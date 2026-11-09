//! `bleproxy` command-line entrypoint for desktop and server hosts.

use std::error::Error;
use std::io::IsTerminal;
use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;

use bleproxy::identity::derived_mac;
use bleproxy::mdns::Announcement;
use bleproxy::{Identity, server};
use bleproxy_core::BdAddr;
use clap::Parser;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tracing::level_filters::LevelFilter;
use tracing::{error, info, warn};

/// Bluetooth LE proxy for Home Assistant, speaking the ESPHome Native API.
#[derive(Debug, Parser)]
#[command(name = "bleproxy", version, about)]
struct Cli {
    /// Device name, also used as the mDNS host name.
    /// [default: bleproxy-<last 3 bytes of the MAC>]
    #[arg(long, env = "BLEPROXY_NAME")]
    name: Option<String>,

    /// Human-friendly name shown in Home Assistant.
    /// [default: bleproxy <last 3 bytes of the MAC>]
    #[arg(long, env = "BLEPROXY_FRIENDLY_NAME")]
    friendly_name: Option<String>,

    /// MAC address identifying this proxy in Home Assistant.
    /// [default: the Bluetooth adapter's address]
    #[arg(long, env = "BLEPROXY_MAC")]
    mac: Option<BdAddr>,

    /// Address and port for the ESPHome API.
    #[arg(long, env = "BLEPROXY_LISTEN", default_value = "0.0.0.0:6053")]
    listen: SocketAddr,

    /// Don't announce the proxy over mDNS. Home Assistant then needs the
    /// address configured by hand.
    #[arg(long, env = "BLEPROXY_NO_MDNS")]
    no_mdns: bool,

    /// Log level: off, error, warn, info, debug or trace.
    #[arg(long, env = "BLEPROXY_LOG", default_value = "info")]
    log_level: LevelFilter,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_max_level(cli.log_level)
        .with_target(false)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            error!("{err}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<(), Box<dyn Error>> {
    warn!("Bluetooth scanning is not supported on this platform yet; serving the API only");
    let mac = cli
        .mac
        .unwrap_or_else(|| derived_mac(cli.name.as_deref().unwrap_or("bleproxy")));
    let identity = Arc::new(Identity::new(cli.name, cli.friendly_name, mac, mac)?);
    let (advertisements, _) = broadcast::channel(1024);

    let listener = TcpListener::bind(cli.listen).await?;
    let address = listener.local_addr()?;
    info!(
        name = identity.name(),
        mac = identity.mac_address(),
        %address,
        "bleproxy {} serving the ESPHome API",
        Identity::VERSION,
    );
    let announcement = if cli.no_mdns {
        None
    } else {
        let announcement = Announcement::start(&identity, address.port())?;
        info!(host = %format_args!("{}.local", identity.name()), "announced over mDNS");
        Some(announcement)
    };

    let served = server::serve(listener, identity, advertisements, shutdown_signal()).await;
    info!("shutting down");
    if let Some(announcement) = announcement {
        tokio::task::spawn_blocking(|| announcement.stop()).await?;
    }
    Ok(served?)
}

/// Completes on Ctrl-C, or on SIGTERM where it exists.
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            error!(%err, "cannot listen for Ctrl-C");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(err) => {
                error!(%err, "cannot listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
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
