//! `roc-slack-door`: the door. It reads the host's configuration, will hold the Slack Socket
//! Mode connection, write the inbox and serve the `roc_slack_door` space over a Unix socket or
//! QUIC (slack-resources 4.2 to 4.4). For now it checks the configuration and stops there.

use std::collections::BTreeMap;
use std::process::ExitCode;

mod config;
mod socket;
mod web_api;

use config::Config;

fn main() -> ExitCode {
    // The one argument is the host's configuration file: `roc-slack-door --config PATH`.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let file: BTreeMap<String, String> = match args.as_slice() {
        [flag, path] if flag == "--config" => match std::fs::read_to_string(path) {
            Ok(text) => config::parse_file(&text),
            Err(e) => {
                eprintln!("roc-slack-door: cannot read {path}: {e}");
                return ExitCode::from(2);
            }
        },
        [] => BTreeMap::new(),
        _ => {
            eprintln!("usage: roc-slack-door [--config PATH]");
            return ExitCode::from(2);
        }
    };
    match Config::resolve(&file, |name| std::env::var(name).ok()) {
        Ok(_) => {
            println!("roc-slack-door: configuration ok; the door does not serve yet");
            ExitCode::SUCCESS
        }
        Err(missing) => {
            let names: Vec<&str> = missing.0;
            eprintln!("roc-slack-door: refusing to start, missing: {}", names.join(", "));
            ExitCode::from(2)
        }
    }
}
