use clap::{App, Arg, ArgMatches, SubCommand};

pub fn get_matches<'a>(default_config_path: &'a str) -> ArgMatches<'a> {
    let app =
        App::new("weenotify")
            .about("Display received weechat AMQP messages as desktop notifications")
            .author(crate_authors!())
            .version(crate_version!())
            .arg(Arg::with_name("config")
                     .help("sets the config file to use")
                     .takes_value(true)
                     .default_value(default_config_path)
                     .short("c")
                     .long("config")
                     .global(true))
            .subcommand(SubCommand::with_name("start").about("Starts listening for messages"));

    app.get_matches()
}
