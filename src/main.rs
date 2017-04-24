extern crate amq_protocol;
#[macro_use]
extern crate clap;
extern crate config;
extern crate futures;
extern crate tokio_core;
extern crate lapin_futures as lapin;
#[macro_use]
extern crate lazy_static;
extern crate notify_rust;
extern crate regex;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;
#[macro_use]
extern crate slog;
extern crate slog_term;
extern crate slog_async;
extern crate uuid;

use std::env;
use std::net::{SocketAddr, ToSocketAddrs};
use std::collections::HashMap;

use amq_protocol::types::FieldTable;
use futures::Stream;
use futures::future::Future;
use lapin::channel::{BasicConsumeOptions, ExchangeDeclareOptions, QueueDeclareOptions,
                     QueueBindOptions};
use lapin::client::ConnectionOptions;
use notify_rust::Notification;
use slog::Drain;
use tokio_core::net::TcpStream;
use tokio_core::reactor::Core;
use uuid::Uuid;

mod cli;
mod filter;
mod message;

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
        None => exit("Could not determine ip address of host"),
    }
}

fn display_notification(message: &message::Message, timeout: i64) {
    match Notification::new()
              .summary(&message.summary())
              .body(&message.body())
              .timeout(timeout as i32)
              .show() {
        _ => (), // ignore result
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

    let connection_info = match config.get_table("connection") {
        Some(info) => info,
        None => exit("Config missing connection section"),
    };

    let socket_addr = make_socket_addr(&connection_info);
    let user = extract_value(&connection_info, "user");
    let pass = extract_value(&connection_info, "pass");
    let vhost = extract_value(&connection_info, "vhost");
    let exchange = extract_value(&connection_info, "exchange");

    let behavior_info = match config.get_table("behavior") {
        Some(info) => info,
        None => exit("Config missing behavior section"),
    };

    let notice_duration = behavior_info
        .get("notice_duration")
        .unwrap()
        .clone()
        .into_int()
        .unwrap_or(5000);

    let ignored_senders = behavior_info
        .get("ignored_senders")
        .unwrap()
        .clone()
        .into_array()
        .unwrap_or(Vec::new());
    let ignored_tags = behavior_info
        .get("ignored_tags")
        .unwrap()
        .clone()
        .into_array()
        .unwrap_or(Vec::new());

    let filter = filter::Filter::new(&log, &ignored_senders, &ignored_tags);

    match Notification::new()
              .summary("weenotify")
              .body("weenotify started")
              .timeout(5000)
              .show() {
        Ok(_) => (), // ignore result
        Err(_) => error!(log, "could not display notification")
    }

    debug!(log, "attempting to connect";
           "host" => format!("{:?}", socket_addr.clone()),
           "user" => user.clone());

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
                                    .and_then(move |stream| {
                                        info!(stream_log, "established stream"; "queue" => qn);

                                        stream.for_each(move |message| {
                                            let decoded = std::str::from_utf8(&message.data)
                                                .unwrap();
                                            trace!(stream_log, "decoded message: {:?}", decoded);

                                            let parsed_message: message::Message = serde_json::from_str(decoded).unwrap();
                                            debug!(stream_log, "parsed message: {:?}", parsed_message; "sender" => parsed_message.sender());

                                            ch.basic_ack(message.delivery_tag);

                                            if !filter.should_filter(&parsed_message) {
                                                display_notification(&parsed_message, notice_duration);
                                            }

                                            Ok(())
                                        })
                                    })
                            })
                    })
            }))
        .unwrap();
}
