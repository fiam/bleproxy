//! `bleproxy` command-line entrypoint for desktop and server hosts.
//!
//! This binary will bridge the host's Bluetooth adapter to Home Assistant
//! via the ESPHome Native API. For now it only identifies itself.

use clap::Parser;

/// Bluetooth LE proxy for Home Assistant, speaking the ESPHome Native API.
#[derive(Debug, Parser)]
#[command(name = "bleproxy", version, about)]
struct Cli {}

fn main() {
    let Cli {} = Cli::parse();
    println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::Cli;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
