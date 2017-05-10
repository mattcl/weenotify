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
use notifier::Notifier;
use slog::Drain;
use tokio_core::net::TcpStream;
use tokio_core::reactor::Core;
use uuid::Uuid;

mod cli;
mod filter;
mod message;
mod notifier;

fn exit(message: &str) -> ! {
    let err = clap::Error::with_description(message, clap::ErrorKind::InvalidValue);
    err.exit();
}

fn extract<F, T>(config: &HashMap<String, config::Value>, key: &str, transform: F) -> T
    where F: Fn(config::Value) -> Option<T>,
          T: std::clone::Clone
{
    match config.get(key).and_then(|v| transform(v.clone())) {
        Some(value) => value.to_owned(),
        None => exit(&format!("Config missing {}", key)),
    }
}

fn extract_or_default<F, T>(config: &HashMap<String, config::Value>,
                            key: &str,
                            default: T,
                            f: F)
                            -> T
    where F: Fn(config::Value) -> Option<T>,
          T: std::clone::Clone
{
    match config.get(key).and_then(|v| f(v.clone())) {
        Some(value) => value.to_owned(),
        None => default,
    }
}

fn make_socket_addr(log: &slog::Logger, config: &HashMap<String, config::Value>) -> SocketAddr {
    let raw = extract(config, "host", |v| v.into_str());
    debug!(log, "determining ip address"; "hostname" => raw.clone());
    let socket_addrs: Vec<_> = raw.to_socket_addrs()
        .expect("unable to resolve host")
        .collect();

    match socket_addrs.get(0) {
        Some(addr) => addr.to_owned(),
        None => exit("Could not determine ip address of host"),
    }
}

fn main() {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();

    let log = slog::Logger::root(drain, o!());

    let default_config_path_raw = env::home_dir()
        .expect("could not determine home directory")
        .join(".weenotify.yml");
    let default_config_path = default_config_path_raw.to_str().unwrap();
    let matches = cli::get_matches(default_config_path);

    // this is safe since clap already checked
    let config_file = matches.value_of("config").unwrap();

    info!(log, "loading config"; "file" => config_file);
    let mut config = config::Config::new();
    config
        .merge(config::File::new(config_file, config::FileFormat::Yaml))
        .expect(&format!("could not load config file: {}", config_file));

    let connection_info = match config.get_table("connection") {
        Some(info) => info,
        None => exit("Config missing connection section"),
    };

    let user = extract(&connection_info, "user", |v| v.into_str());
    let pass = extract(&connection_info, "pass", |v| v.into_str());
    let vhost = extract(&connection_info, "vhost", |v| v.into_str());
    let exchange = extract(&connection_info, "exchange", |v| v.into_str());

    let behavior_info = match config.get_table("behavior") {
        Some(info) => info,
        None => exit("Config missing behavior section"),
    };

    let notice_duration =
        extract_or_default(&behavior_info, "notice_duration", 5000, |v| v.into_int());
    let notifier = Notifier::new(&log, notice_duration as i32);

    let ignored_channels = extract_or_default(&behavior_info,
                                              "ignored_channels",
                                              Vec::new(),
                                              |v| v.into_array());
    let ignored_senders = extract_or_default(&behavior_info,
                                             "ignored_senders",
                                             Vec::new(),
                                             |v| v.into_array());
    let ignored_tags = extract_or_default(&behavior_info,
                                          "ignored_tags",
                                          Vec::new(),
                                          |v| v.into_array());

    let socket_addr = make_socket_addr(&log, &connection_info);
    let filter = filter::Filter::new(&log, &ignored_channels, &ignored_senders, &ignored_tags);

    notifier.show("weenotify", "weenotify started");

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
                                                notifier.show(&parsed_message.summary(), &parsed_message.body());
                                            }

                                            Ok(())
                                        })
                                    })
                            })
                    })
            }))
        .unwrap();
}
