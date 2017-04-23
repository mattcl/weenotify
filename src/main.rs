extern crate amq_protocol;
#[macro_use]
extern crate clap;
extern crate config;
#[macro_use]
extern crate error_chain;
extern crate futures;
extern crate tokio_core;
extern crate lapin_futures as lapin;
#[macro_use]
extern crate lazy_static;
extern crate regex;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;
#[macro_use]
extern crate slog;
extern crate slog_term;
extern crate slog_async;
extern crate uuid;

mod cli;
mod filter;
mod message;

use std::env;
use std::net::{SocketAddr, ToSocketAddrs};
use std::collections::HashMap;

use slog::Drain;

use amq_protocol::types::FieldTable;
use futures::Stream;
use futures::future::Future;
use tokio_core::reactor::Core;
use tokio_core::net::TcpStream;
use lapin::client::ConnectionOptions;
use lapin::channel::{BasicConsumeOptions, ExchangeDeclareOptions, QueueDeclareOptions,
                     QueueBindOptions};
use uuid::Uuid;

fn exit(message: &str) -> ! {
    let err = clap::Error::with_description(message, clap::ErrorKind::InvalidValue);
    err.exit();
}

fn extract_value(config: &HashMap<String, config::Value>, key: &str) -> String {
    match config.get(key).and_then(|v| v.clone().into_str()) {
        Some(value) => value.to_owned(),
        None => exit(&format!("Config missing {}", key)),
    }
}

fn make_socket_addr(config: &HashMap<String, config::Value>) -> SocketAddr {
    let raw = extract_value(config, "host");
    let socket_addrs: Vec<_> = raw.to_socket_addrs()
        .expect("unable to resolve host")
        .collect();

    match socket_addrs.get(0) {
        Some(addr) => addr.to_owned(),
        None => exit("Could not determine ip address of host")
    }
}

fn main() {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();

    let log = slog::Logger::root(drain, o!());

    let default_config_path_raw = env::home_dir().unwrap().join(".weenotify.yml");
    let default_config_path = default_config_path_raw.to_str().unwrap();
    let matches = cli::get_matches(default_config_path);

    // this is safe since clap already checked
    let config_file = matches.value_of("config").unwrap();

    info!(log, "loading config"; "file" => config_file);
    let mut config = config::Config::new();
    config
        .merge(config::File::new(config_file, config::FileFormat::Yaml))
        .unwrap();

    let connection_info = config.get_table("connection").unwrap();
    let addr = extract_value(&connection_info, "host");
    let socket_addrs: Vec<_> = addr.to_socket_addrs()
        .expect("unable to resolve host")
        .collect();
    let socket_addr = socket_addrs.get(0).unwrap();

    let user = extract_value(&connection_info, "user");
    let pass = extract_value(&connection_info, "pass");
    let vhost = extract_value(&connection_info, "vhost");
    let exchange = extract_value(&connection_info, "exchange");

    // let filter = filter::Filter::from_config();

    debug!(log, "attempting to connect"; "host" => addr, "user" => user.clone());
    debug!(log, "socket {:?}", socket_addr.clone());

    let mut core = Core::new().unwrap();
    let handle = core.handle();

    debug!(log, "starting event loop");

    let stream_log = log.new(o!());

    core.run(TcpStream::connect(&socket_addr, &handle)
            .and_then(|stream| {
                let connection_options = ConnectionOptions {
                    username: user,
                    password: pass,
                    vhost: vhost,
                    ..Default::default()
                };
                lapin::client::Client::connect(stream, &connection_options)
            })
            .and_then(|client| client.create_channel())
            .and_then(|channel| {
                let uuid = Uuid::new_v4();
                let queue_name = format!("weenotify-{}", uuid.simple().to_string());
                let qn = queue_name.clone();
                let ch = channel.clone();
                let options = QueueDeclareOptions {
                    passive: false,
                    durable: false,
                    exclusive: true,
                    auto_delete: true,
                    nowait: false,
                };
                channel.queue_declare(&queue_name, &options, FieldTable::new())
                    .and_then(move |_| {
                        channel.exchange_declare(&exchange,
                                              "fanout",
                                              &ExchangeDeclareOptions::default(),
                                              FieldTable::new())
                            .and_then(move |_| {
                                channel.queue_bind(&queue_name,
                                                &exchange,
                                                "derp",
                                                &QueueBindOptions::default(),
                                                FieldTable::new())
                                    .and_then(move |_| {
                                        let consume_options = BasicConsumeOptions::default();
                                        channel.basic_consume(&queue_name,
                                                              "my_consumer",
                                                              &consume_options)
                                    })
                                    .and_then(|stream| {
                                        info!(stream_log, "established stream"; "queue" => qn);

                                        stream.for_each(move |message| {
                                            let decoded = std::str::from_utf8(&message.data)
                                                .unwrap();
                                            trace!(stream_log, "decoded message: {:?}", decoded);

                                            let parsed_message: message::Message = serde_json::from_str(decoded).unwrap();
                                            debug!(stream_log, "parsed message: {:?}", parsed_message; "sender" => parsed_message.sender());

                                            ch.basic_ack(message.delivery_tag);
                                            Ok(())
                                        })
                                    })
                            })
                    })
            }))
        .unwrap();
}
