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

mod cli;
mod message;

use std::env;
use std::net::ToSocketAddrs;

use slog::Drain;

use amq_protocol::types::FieldTable;
use futures::Stream;
use futures::future::Future;
use tokio_core::reactor::Core;
use tokio_core::net::TcpStream;
use lapin::client::ConnectionOptions;
use lapin::channel::{BasicConsumeOptions, ExchangeDeclareOptions, QueueDeclareOptions,
                     QueueBindOptions};

pub fn exit(message: &str) -> ! {
    let err = clap::Error::with_description(message, clap::ErrorKind::InvalidValue);
    err.exit();
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
    config.merge(config::File::new(config_file, config::FileFormat::Yaml)).unwrap();

    let connectin_info = config.get_table("connection").unwrap();
    let addr = connectin_info.get("host").unwrap().clone().into_str();
    let socket_addrs: Vec<_> = match addr.clone() {
        Some(addr) => addr.to_socket_addrs().expect("unable to resolve host").collect(),
        None => exit("Invalid host specified in config"),
    };
    let socket_addr = socket_addrs.get(0).unwrap();
    let user = connectin_info.get("user").unwrap().clone().into_str();
    let pass = connectin_info.get("pass").unwrap().clone().into_str();
    let vhost = connectin_info.get("vhost").unwrap().clone().into_str();
    let exchange = connectin_info.get("exchange").unwrap().clone().into_str();
    let exchange = match exchange {
        Some(e) => e,
        None => exit("Exchange not declared in config file"),
    };

    debug!(log, "attempting to connect"; "host" => addr, "user" => user.clone());
    debug!(log, "socket {:?}", socket_addr.clone());

    let mut core = Core::new().unwrap();
    let handle = core.handle();

    debug!(log, "starting event loop");

    core.run(TcpStream::connect(&socket_addr, &handle)
            .and_then(|stream| {
                let connection_options = ConnectionOptions {
                    username: user.unwrap(),
                    password: pass.unwrap(),
                    vhost: vhost.unwrap(),
                    ..Default::default()
                };
                lapin::client::Client::connect(stream, &connection_options)
            })
            .and_then(|client| client.create_channel())
            .and_then(|channel| {
                let ch = channel.clone();
                let options = QueueDeclareOptions {
                    passive: false,
                    durable: false,
                    exclusive: true,
                    auto_delete: true,
                    nowait: false,
                };
                channel.queue_declare("weenotify", &options, FieldTable::new())
                    .and_then(move |_| {
                        channel.exchange_declare(&exchange,
                                              "fanout",
                                              &ExchangeDeclareOptions::default(),
                                              FieldTable::new())
                            .and_then(move |_| {
                                channel.queue_bind("weenotify",
                                                &exchange,
                                                "derp",
                                                &QueueBindOptions::default(),
                                                FieldTable::new())
                                    .and_then(move |_| {
                                        let consume_options = BasicConsumeOptions::default();
                                        channel.basic_consume("weenotify",
                                                              "my_consumer",
                                                              &consume_options)
                                    })
                                    .and_then(|stream| {
                                        info!(log, "established stream");

                                        stream.for_each(move |message| {
                                            let decoded = std::str::from_utf8(&message.data)
                                                .unwrap();
                                            trace!(log, "decoded message: {:?}", decoded);

                                            let parsed_message: message::Message = serde_json::from_str(decoded).unwrap();
                                            debug!(log, "parsed message: {:?}", parsed_message; "sender" => parsed_message.sender(), "tag" => parsed_message.has_tag("notify_none"));

                                            ch.basic_ack(message.delivery_tag);
                                            Ok(())
                                        })
                                    })
                            })
                    })
            }))
        .unwrap();
}
