//! `bleproxy` command-line entrypoint for desktop and server hosts.

use std::error::Error;
use std::io::IsTerminal;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use bleproxy::diagnostics::Diagnostics;
use bleproxy::encryption::EncryptionKey;
use bleproxy::identity::validate_name;
use bleproxy::logs::Logs;
use bleproxy::mdns::Announcement;
use bleproxy::{Identity, server};
use bleproxy_core::BdAddr;
use clap::Parser;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tracing::level_filters::LevelFilter;
use tracing::{error, info};
use tracing_subscriber::Layer as _;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

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

    /// Bluetooth adapter to scan with.
    #[cfg(target_os = "linux")]
    #[arg(long, env = "BLEPROXY_ADAPTER", default_value = "hci0")]
    adapter: String,

    /// Scan mode to start in; Home Assistant can switch it at runtime.
    /// Active scanning sends scan requests and sees every device; passive
    /// scanning only listens, saving battery-powered devices energy, but
    /// needs BlueZ's advertisement monitor support (bleproxy scans actively
    /// without it) and misses devices that advertise without flags.
    #[cfg(target_os = "linux")]
    #[arg(long, env = "BLEPROXY_SCAN_MODE", value_enum, default_value_t)]
    scan_mode: ScanModeArg,

    /// How many devices Home Assistant may connect to through the proxy at
    /// once, for integrations that read from or control devices. 0 turns
    /// active connections off.
    #[cfg(target_os = "linux")]
    #[arg(long, env = "BLEPROXY_CONNECTION_SLOTS", default_value_t = 3)]
    connection_slots: u32,

    /// Only forward advertisements from, and connect to, the device with
    /// this address. Repeat it (or separate addresses with commas) to
    /// allow several devices. Each allowed device gets signal strength and
    /// last seen sensors, named after the label given with `=NAME`, or else
    /// the name BlueZ knows for it when bleproxy starts, or else the
    /// address. [default: every device]
    #[cfg(target_os = "linux")]
    #[arg(
        long = "allow-address",
        env = "BLEPROXY_ALLOW_ADDRESS",
        value_name = "ADDRESS[=NAME]",
        value_delimiter = ','
    )]
    allow_addresses: Vec<bleproxy::filter::AllowedDevice>,

    /// Key that encrypts the API, as 32 bytes of base64 (ESPHome's
    /// `api: encryption: key:`). Generate one with
    /// `openssl rand -base64 32`; Home Assistant asks for it when adding the
    /// proxy.
    #[arg(
        long,
        env = "BLEPROXY_ENCRYPTION_KEY",
        hide_env_values = true,
        conflicts_with = "encryption_key_file"
    )]
    encryption_key: Option<EncryptionKey>,

    /// File holding the encryption key, to keep it out of the environment
    /// (for example a Docker secret, or a file only the service can read).
    #[arg(long, env = "BLEPROXY_ENCRYPTION_KEY_FILE")]
    encryption_key_file: Option<PathBuf>,

    /// Address and port for the ESPHome API. `[::]:6053` also accepts IPv6
    /// clients, and announces the host's IPv6 addresses over mDNS.
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

/// Command-line spelling of [`bleproxy_core::scanner::ScanMode`].
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, Default, clap::ValueEnum)]
enum ScanModeArg {
    /// Send scan requests.
    #[default]
    Active,
    /// Listen only.
    Passive,
}

#[cfg(target_os = "linux")]
impl From<ScanModeArg> for bleproxy_core::scanner::ScanMode {
    fn from(mode: ScanModeArg) -> Self {
        match mode {
            ScanModeArg::Active => Self::Active,
            ScanModeArg::Passive => Self::Passive,
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    // Logs go to stderr and, at the level they ask for, to API clients.
    let logs = Arc::new(Logs::default());
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_writer(std::io::stderr)
                .with_ansi(std::io::stderr().is_terminal())
                .with_filter(cli.log_level),
        )
        .with(logs.layer())
        .init();
    match run(cli, logs).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            error!("{err}");
            ExitCode::FAILURE
        }
    }
}

async fn run(mut cli: Cli, logs: Arc<Logs>) -> Result<(), Box<dyn Error>> {
    // Catch configuration mistakes before touching the Bluetooth stack.
    if let Some(name) = &cli.name {
        validate_name(name)?;
    }
    let encryption = match (cli.encryption_key.take(), &cli.encryption_key_file) {
        (Some(key), _) => Some(key),
        (None, Some(path)) => {
            let text = std::fs::read_to_string(path)
                .map_err(|err| format!("cannot read {}: {err}", path.display()))?;
            Some(text.parse::<EncryptionKey>()?)
        }
        (None, None) => None,
    };
    let (advertisements, _) = broadcast::channel(1024);
    // Count from the first advertisement on.
    let counted = advertisements.subscribe();

    let Bluetooth {
        mac: bluetooth_mac,
        scanner,
        connections,
        devices,
        config: platform_config,
    } = start_bluetooth(&cli, &advertisements).await?;
    let diagnostics = Arc::new(Diagnostics::with_devices(devices));
    tokio::spawn(Arc::clone(&diagnostics).count_advertisements(counted));
    let mac = cli.mac.unwrap_or(bluetooth_mac);
    let mut identity = Identity::new(cli.name, cli.friendly_name, mac, bluetooth_mac)?;
    if let Some(scanner) = &scanner {
        identity = identity.with_scanner(scanner.configured());
    }
    if connections.is_some() {
        identity = identity.with_connections();
    }
    if encryption.is_some() {
        identity = identity.with_encryption();
    }
    let identity = Arc::new(identity);

    let listener = TcpListener::bind(cli.listen).await?;
    let address = listener.local_addr()?;
    info!(
        name = identity.name(),
        mac = identity.mac_address(),
        %address,
        encrypted = identity.encrypted(),
        "bleproxy {} serving the ESPHome API",
        Identity::VERSION,
    );
    let announcement = if cli.no_mdns {
        None
    } else {
        let announcement = Announcement::start(&identity, address)?;
        info!(host = %format_args!("{}.local", identity.name()), "announced over mDNS");
        Some(announcement)
    };

    let config = [
        format!("bleproxy {}", Identity::VERSION),
        format!("Name: {} ({})", identity.name(), identity.friendly_name()),
        format!("MAC address: {}", identity.mac_address()),
        format!("API: {address}"),
        format!(
            "Encryption: {}",
            if encryption.is_some() { "on" } else { "off" }
        ),
        format!(
            "mDNS: {}",
            if announcement.is_some() { "on" } else { "off" }
        ),
    ]
    .into_iter()
    .chain(platform_config)
    .collect();
    let context = server::Context {
        identity,
        advertisements,
        scanner,
        diagnostics,
        encryption,
        connections,
        logs,
        config,
    };
    let served = server::serve(listener, context, shutdown_signal()).await;
    info!("shutting down");
    if let Some(announcement) = announcement {
        tokio::task::spawn_blocking(|| announcement.stop()).await?;
    }
    Ok(served?)
}

/// What the platform's Bluetooth stack offers clients.
struct Bluetooth {
    /// The adapter's address, or one derived from the name without one.
    mac: BdAddr,
    scanner: Option<bleproxy::scanner::ScannerHandle>,
    connections: Option<bleproxy::connections::ConnectionsHandle>,
    /// The devices to report the signal of, with their names.
    devices: Vec<(BdAddr, String)>,
    /// Lines describing the Bluetooth configuration.
    config: Vec<String>,
}

/// Starts scanning and managing connections through BlueZ.
#[cfg(target_os = "linux")]
async fn start_bluetooth(
    cli: &Cli,
    advertisements: &broadcast::Sender<bleproxy_core::Advertisement>,
) -> Result<Bluetooth, Box<dyn Error>> {
    let address = bleproxy::bluez::adapter_address(&cli.adapter)
        .await
        .map_err(|err| format!("cannot use Bluetooth adapter {}: {err}", cli.adapter))?;
    info!(adapter = cli.adapter, %address, "using Bluetooth adapter");
    let filter = bleproxy::filter::AddressFilter::new(
        cli.allow_addresses
            .iter()
            .map(|device| device.address)
            .collect::<Vec<_>>(),
    );
    if filter.is_restricted() {
        info!(devices = %filter, "only forwarding allowed devices");
    }
    let devices = name_devices(&cli.adapter, &cli.allow_addresses).await;
    let config = vec![
        format!("Bluetooth adapter: {} ({address})", cli.adapter),
        format!("Scan mode: {:?}", cli.scan_mode),
        format!("Devices: {filter}"),
        format!("Connection slots: {}", cli.connection_slots),
    ];
    let (handle, driver) = bleproxy::scanner::channel(cli.scan_mode.into());
    tokio::spawn(bleproxy::bluez::scan_forever(
        cli.adapter.clone(),
        driver,
        advertisements.clone(),
        filter.clone(),
    ));
    let connections = (cli.connection_slots > 0).then(|| {
        let (handle, driver) = bleproxy::connections::channel(cli.connection_slots, filter);
        tokio::spawn(bleproxy::bluez::manage_connections(
            cli.adapter.clone(),
            driver,
        ));
        handle
    });
    Ok(Bluetooth {
        mac: address,
        scanner: Some(handle),
        connections,
        devices,
        config,
    })
}

/// Names the allowed devices for their sensors: by label, by the name
/// BlueZ knows, or by address. Names are fixed from here on, since Home
/// Assistant would see a renamed sensor as a new one.
#[cfg(target_os = "linux")]
async fn name_devices(
    adapter: &str,
    devices: &[bleproxy::filter::AllowedDevice],
) -> Vec<(BdAddr, String)> {
    let known = if devices.iter().any(|device| device.label.is_none()) {
        bleproxy::bluez::known_names(adapter)
            .await
            .unwrap_or_else(|err| {
                tracing::warn!(%err, "cannot look up device names; naming devices by address");
                std::collections::HashMap::new()
            })
    } else {
        std::collections::HashMap::new()
    };
    devices
        .iter()
        .map(|device| {
            let address = device.address;
            let name = match (&device.label, known.get(&address)) {
                (Some(label), _) => label.clone(),
                (None, Some(name)) => {
                    info!(
                        %address,
                        name, "naming device after its Bluetooth name; label it to keep the name"
                    );
                    name.clone()
                }
                (None, None) => {
                    info!(
                        %address,
                        "naming device after its address; label it to choose a name"
                    );
                    address.to_string()
                }
            };
            (address, name)
        })
        .collect()
}

/// Serves the API without Bluetooth, which this platform lacks so far.
#[cfg(not(target_os = "linux"))]
#[allow(clippy::unused_async, reason = "matches the Linux version")]
async fn start_bluetooth(
    cli: &Cli,
    _: &broadcast::Sender<bleproxy_core::Advertisement>,
) -> Result<Bluetooth, Box<dyn Error>> {
    tracing::warn!(
        "Bluetooth scanning is not supported on this platform yet; serving the API only"
    );
    Ok(Bluetooth {
        mac: bleproxy::identity::derived_mac(cli.name.as_deref().unwrap_or("bleproxy")),
        scanner: None,
        connections: None,
        devices: Vec::new(),
        config: Vec::new(),
    })
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

    #[cfg(target_os = "linux")]
    #[test]
    fn accepts_repeated_and_comma_separated_addresses() {
        use clap::Parser;

        let cli = Cli::try_parse_from([
            "bleproxy",
            "--allow-address",
            "5C:CA:D3:BA:BA:CC=Scale,A4:C1:38:00:11:22",
            "--allow-address",
            "A4:C1:38:00:33:44",
        ])
        .unwrap();
        let devices: Vec<(String, Option<&str>)> = cli
            .allow_addresses
            .iter()
            .map(|device| (device.address.to_string(), device.label.as_deref()))
            .collect();
        assert_eq!(
            devices,
            [
                ("5C:CA:D3:BA:BA:CC".to_owned(), Some("Scale")),
                ("A4:C1:38:00:11:22".to_owned(), None),
                ("A4:C1:38:00:33:44".to_owned(), None),
            ]
        );
        assert!(Cli::try_parse_from(["bleproxy", "--allow-address", "scale"]).is_err());
        assert!(
            Cli::try_parse_from(["bleproxy", "--allow-address", "5C:CA:D3:BA:BA:CC="]).is_err()
        );
    }
}
