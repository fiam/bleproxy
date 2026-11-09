//! Bluetooth LE scanning through BlueZ's D-Bus API (Linux).
//!
//! BlueZ reports each device as a D-Bus object whose properties (RSSI,
//! manufacturer data, service data, ...) change as advertisements arrive.
//! The scanner follows those changes for one adapter and turns them back
//! into raw advertisements with [`AdvDataBuilder`].
//!
//! Two scan modes are supported:
//!
//! - [`ScanMode::Active`] uses regular discovery with `DuplicateData`, so
//!   BlueZ reports every advertisement. BlueZ sends scan requests, which
//!   also collects scan response data but costs battery-powered devices
//!   some energy.
//! - [`ScanMode::Passive`] registers an advertisement monitor instead, so
//!   BlueZ only listens. Monitors need patterns; like Home Assistant's own
//!   passive scanner, the ones used here match the common Flags values,
//!   so devices that advertise without flags go unseen. It needs a BlueZ
//!   with the `AdvertisementMonitor` API enabled.
//!
//! API clients can switch between them at runtime (see
//! [`crate::scanner`]).

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use bleproxy_core::scanner::{ScanMode, ScannerState};
use bleproxy_core::{AddressType, AdvDataBuilder, Advertisement, BdAddr, Uuid};
use futures_util::StreamExt;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

use crate::scanner::ScannerDriver;
use zbus::message::Type as MessageType;
use zbus::{Connection, MatchRule, MessageStream, OwnedObjectPath, OwnedVariant, Variant};

const BLUEZ: &str = "org.bluez";
const ADAPTER: &str = "org.bluez.Adapter1";
const DEVICE: &str = "org.bluez.Device1";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";
const MONITOR_MANAGER: &str = "org.bluez.AdvertisementMonitorManager1";

/// Where the passive scan's advertisement monitor is served.
const MONITOR_ROOT: &str = "/io/github/fiam/bleproxy";
const MONITOR_PATH: &str = "/io/github/fiam/bleproxy/monitor0";

/// How long to wait before restarting a failed scan.
const RETRY_DELAY: Duration = Duration::from_secs(5);

/// Errors produced while talking to BlueZ.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BluezError {
    /// A D-Bus call or the bus connection failed.
    #[error("D-Bus: {0}")]
    DBus(#[from] zbus::Error),
    /// The adapter has no usable address.
    #[error("adapter {adapter} reported an invalid address {address:?}")]
    InvalidAddress {
        /// Adapter name.
        adapter: String,
        /// Address as reported.
        address: String,
    },
    /// The signal streams ended, usually because the bus connection closed.
    #[error("lost the connection to BlueZ")]
    Disconnected,
}

/// Properties of all D-Bus objects, as returned by `GetManagedObjects`.
type ManagedObjects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedVariant>>>;

/// Returns the address of `adapter` (for example `hci0`).
///
/// # Errors
///
/// Returns an error if BlueZ is unreachable or the adapter doesn't exist.
pub async fn adapter_address(adapter: &str) -> Result<BdAddr, BluezError> {
    let connection = Connection::system().await?;
    let reply = connection
        .call_method(
            Some(BLUEZ),
            adapter_path(adapter).as_str(),
            Some(PROPERTIES),
            "Get",
            &(ADAPTER, "Address"),
        )
        .await?;
    let value: OwnedVariant = reply.body().deserialize()?;
    let address = string(&value).unwrap_or_default();
    address.parse().map_err(|_| BluezError::InvalidAddress {
        adapter: adapter.to_owned(),
        address,
    })
}

/// Scans with `adapter` forever, publishing advertisements to `sender`.
///
/// Scans in the mode requested through `driver`, switching when another
/// mode is requested, and reports the scanner's state back through it.
/// Failures (BlueZ restarting, the adapter disappearing) are logged and the
/// scan is restarted after a short delay.
pub async fn scan_forever(
    adapter: String,
    mut driver: ScannerDriver,
    sender: broadcast::Sender<Advertisement>,
) {
    loop {
        let mode = driver.requested_mode();
        driver.report(ScannerState::Starting, mode);
        match scan(&adapter, mode, &sender, &mut driver).await {
            Ok(next) => {
                info!(from = ?mode, to = ?next, "switching scan mode");
                continue;
            }
            Err(err) => {
                warn!(%err, "scan failed");
                driver.report(ScannerState::Failed, mode);
            }
        }
        info!(delay = ?RETRY_DELAY, "restarting the scan");
        // Retry after a delay, or right away in another mode if asked to.
        tokio::select! {
            () = tokio::time::sleep(RETRY_DELAY) => {}
            _ = driver.mode_change(mode) => {}
        }
    }
}

/// Runs one scan session in `mode` until another mode is requested, which
/// it returns, or until the connection to BlueZ fails.
async fn scan(
    adapter: &str,
    mode: ScanMode,
    sender: &broadcast::Sender<Advertisement>,
    driver: &mut ScannerDriver,
) -> Result<ScanMode, BluezError> {
    let connection = Connection::system().await?;
    let path = adapter_path(adapter);

    // Subscribe before starting discovery so no update is missed.
    let mut changed = signals(&connection, PROPERTIES, "PropertiesChanged", &path).await?;
    let mut added = signals(&connection, OBJECT_MANAGER, "InterfacesAdded", "/").await?;
    let mut removed = signals(&connection, OBJECT_MANAGER, "InterfacesRemoved", "/").await?;

    power_on(&connection, &path).await?;
    match mode {
        ScanMode::Active => start_discovery(&connection, &path).await?,
        ScanMode::Passive => register_monitor(&connection, &path).await?,
    }
    info!(adapter, ?mode, "scanning");
    driver.report(ScannerState::Running, mode);

    let mut devices = Devices::new(&path);
    let objects: ManagedObjects = connection
        .call_method(
            Some(BLUEZ),
            "/",
            Some(OBJECT_MANAGER),
            "GetManagedObjects",
            &(),
        )
        .await?
        .body()
        .deserialize()?;
    for (object, interfaces) in &objects {
        if let Some(properties) = interfaces.get(DEVICE) {
            devices.update(object, properties, &[]);
        }
    }

    loop {
        let (message, kind) = tokio::select! {
            Some(message) = changed.next() => (message?, Signal::Changed),
            Some(message) = added.next() => (message?, Signal::Added),
            Some(message) = removed.next() => (message?, Signal::Removed),
            next = driver.mode_change(mode) => {
                driver.report(ScannerState::Stopping, mode);
                stop(&connection, &path, mode).await;
                driver.report(ScannerState::Stopped, mode);
                return Ok(next);
            }
            else => return Err(BluezError::Disconnected),
        };
        let Some(object) = message
            .header()
            .path()
            .map(|path| OwnedObjectPath::from(path.to_owned()))
        else {
            continue;
        };
        let advertisement = match kind {
            Signal::Changed => {
                let (interface, properties, invalidated): (
                    String,
                    HashMap<String, OwnedVariant>,
                    Vec<String>,
                ) = message.body().deserialize()?;
                if interface != DEVICE {
                    continue;
                }
                devices.update(&object, &properties, &invalidated)
            }
            Signal::Added => {
                let (object, mut interfaces): (
                    OwnedObjectPath,
                    HashMap<String, HashMap<String, OwnedVariant>>,
                ) = message.body().deserialize()?;
                interfaces
                    .remove(DEVICE)
                    .and_then(|properties| devices.update(&object, &properties, &[]))
            }
            Signal::Removed => {
                let (object, interfaces): (OwnedObjectPath, Vec<String>) =
                    message.body().deserialize()?;
                if interfaces.iter().any(|interface| interface == DEVICE) {
                    devices.remove(&object);
                }
                None
            }
        };
        if let Some(advertisement) = advertisement {
            // No receivers just means no client is subscribed right now.
            let _ = sender.send(advertisement);
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Signal {
    Changed,
    Added,
    Removed,
}

fn adapter_path(adapter: &str) -> String {
    format!("/org/bluez/{adapter}")
}

async fn signals(
    connection: &Connection,
    interface: &'static str,
    member: &'static str,
    path_namespace: &str,
) -> Result<MessageStream, BluezError> {
    let rule = MatchRule::builder()
        .msg_type(MessageType::Signal)
        .sender(BLUEZ)
        .interface(interface)
        .member(member)
        .path_namespace(path_namespace.to_owned())
        .build()?;
    Ok(MessageStream::for_match_rule(rule, connection, None).await?)
}

async fn start_discovery(connection: &Connection, path: &str) -> Result<(), BluezError> {
    let filter = HashMap::from([
        ("Transport", Variant::from("le")),
        ("DuplicateData", Variant::from(true)),
    ]);
    connection
        .call_method(
            Some(BLUEZ),
            path,
            Some(ADAPTER),
            "SetDiscoveryFilter",
            &(filter,),
        )
        .await?;
    connection
        .call_method(Some(BLUEZ), path, Some(ADAPTER), "StartDiscovery", &())
        .await?;
    Ok(())
}

/// Serves an advertisement monitor and registers it with the adapter,
/// which makes BlueZ scan passively for it.
/// Stops scanning in `mode`. Best effort: BlueZ also stops when our bus
/// connection closes, which follows right after.
async fn stop(connection: &Connection, path: &str, mode: ScanMode) {
    let result = match mode {
        ScanMode::Active => {
            connection
                .call_method(Some(BLUEZ), path, Some(ADAPTER), "StopDiscovery", &())
                .await
        }
        ScanMode::Passive => match zbus::ObjectPath::try_from(MONITOR_ROOT) {
            Ok(root) => {
                connection
                    .call_method(
                        Some(BLUEZ),
                        path,
                        Some(MONITOR_MANAGER),
                        "UnregisterMonitor",
                        &(root,),
                    )
                    .await
            }
            Err(err) => Err(err),
        },
    };
    if let Err(err) = result {
        debug!(%err, ?mode, "stopping the scan failed");
    }
}

async fn register_monitor(connection: &Connection, path: &str) -> Result<(), BluezError> {
    let server = connection.object_server();
    server.at(MONITOR_ROOT, zbus::fdo::ObjectManager).await?;
    server.at(MONITOR_PATH, Monitor).await?;
    let root = zbus::ObjectPath::try_from(MONITOR_ROOT)?;
    connection
        .call_method(
            Some(BLUEZ),
            path,
            Some(MONITOR_MANAGER),
            "RegisterMonitor",
            &(root,),
        )
        .await?;
    Ok(())
}

/// An `or_patterns` advertisement monitor that matches nearly every
/// advertisement. BlueZ calls it as it starts and stops monitoring;
/// matched devices show up as regular device objects either way.
#[derive(Debug)]
struct Monitor;

#[allow(
    clippy::unused_self,
    clippy::needless_pass_by_value,
    reason = "zbus interface methods take self and owned arguments"
)]
#[zbus::interface(name = "org.bluez.AdvertisementMonitor1")]
impl Monitor {
    fn release(&self) {
        warn!("BlueZ released the advertisement monitor");
    }

    fn activate(&self) {
        info!("passive scanning active");
    }

    fn device_found(&self, device: OwnedObjectPath) {
        debug!(device = device.as_str(), "monitor found a device");
    }

    fn device_lost(&self, device: OwnedObjectPath) {
        debug!(device = device.as_str(), "monitor lost a device");
    }

    #[zbus(property, name = "Type")]
    const fn kind(&self) -> &'static str {
        "or_patterns"
    }

    /// Patterns as (start position, AD type, content): the Flags values
    /// Home Assistant's passive scanner matches.
    #[zbus(property)]
    fn patterns(&self) -> Vec<(u8, u8, Vec<u8>)> {
        const FLAGS: u8 = 0x01;
        [0x02, 0x06, 0x1A]
            .into_iter()
            .map(|flags| (0, FLAGS, vec![flags]))
            .collect()
    }
}

async fn power_on(connection: &Connection, path: &str) -> Result<(), BluezError> {
    let reply = connection
        .call_method(
            Some(BLUEZ),
            path,
            Some(PROPERTIES),
            "Get",
            &(ADAPTER, "Powered"),
        )
        .await?;
    let powered: OwnedVariant = reply.body().deserialize()?;
    if matches!(unwrap(&powered), Variant::Bool(true)) {
        return Ok(());
    }
    info!("powering on the adapter");
    connection
        .call_method(
            Some(BLUEZ),
            path,
            Some(PROPERTIES),
            "Set",
            &(ADAPTER, "Powered", Variant::from(true)),
        )
        .await?;
    Ok(())
}

/// What is known about one device, accumulated from property updates.
#[derive(Debug, Default)]
struct Device {
    address: Option<BdAddr>,
    address_type: Option<AddressType>,
    rssi: Option<i8>,
    name: Option<String>,
    tx_power: Option<i8>,
    uuids: Vec<Uuid>,
    manufacturer_data: BTreeMap<u16, Vec<u8>>,
    service_data: BTreeMap<Uuid, Vec<u8>>,
}

impl Device {
    fn apply(&mut self, properties: &HashMap<String, OwnedVariant>, invalidated: &[String]) {
        for (key, value) in properties {
            match key.as_str() {
                "Address" => self.address = string(value).and_then(|s| s.parse().ok()),
                "AddressType" => {
                    self.address_type = match string(value).as_deref() {
                        Some("random") => Some(AddressType::Random),
                        Some(_) => Some(AddressType::Public),
                        None => None,
                    };
                }
                "RSSI" => self.rssi = int16(value).map(saturate),
                "TxPower" => self.tx_power = int16(value).map(saturate),
                "Name" => self.name = string(value),
                "UUIDs" => {
                    self.uuids = strings(value)
                        .iter()
                        .filter_map(|uuid| uuid.parse().ok())
                        .collect();
                }
                "ManufacturerData" => {
                    self.manufacturer_data = entries(value)
                        .filter_map(|(key, value)| match key {
                            Variant::U16(company) => Some((*company, bytes(value)?)),
                            _ => None,
                        })
                        .collect();
                }
                "ServiceData" => {
                    self.service_data = entries(value)
                        .filter_map(|(key, value)| {
                            let uuid = String::try_from(key).ok()?.parse().ok()?;
                            Some((uuid, bytes(value)?))
                        })
                        .collect();
                }
                _ => {}
            }
        }
        for key in invalidated {
            match key.as_str() {
                "RSSI" => self.rssi = None,
                "TxPower" => self.tx_power = None,
                "ManufacturerData" => self.manufacturer_data.clear(),
                "ServiceData" => self.service_data.clear(),
                _ => {}
            }
        }
    }

    /// Rebuilds the device's latest advertisement, if it is in range.
    fn advertisement(&self) -> Option<Advertisement> {
        let mut data = AdvDataBuilder::new();
        // Most specific first: if space runs out, names and UUID lists are
        // the least useful to lose.
        for (company, payload) in &self.manufacturer_data {
            data.manufacturer_data(*company, payload);
        }
        for (uuid, payload) in &self.service_data {
            data.service_data(*uuid, payload);
        }
        data.service_uuids(&self.uuids);
        if let Some(name) = &self.name {
            data.local_name(name);
        }
        if let Some(tx_power) = self.tx_power {
            data.tx_power(tx_power);
        }
        Some(Advertisement {
            address: self.address?,
            address_type: self.address_type.unwrap_or(AddressType::Public),
            rssi: self.rssi?,
            data: data.build(),
        })
    }
}

/// Devices of one adapter.
#[derive(Debug)]
struct Devices {
    prefix: String,
    devices: HashMap<OwnedObjectPath, Device>,
}

impl Devices {
    fn new(adapter_path: &str) -> Self {
        Self {
            prefix: format!("{adapter_path}/"),
            devices: HashMap::new(),
        }
    }

    /// Applies a property update and returns the resulting advertisement
    /// if the update came from a received advertisement.
    fn update(
        &mut self,
        object: &OwnedObjectPath,
        properties: &HashMap<String, OwnedVariant>,
        invalidated: &[String],
    ) -> Option<Advertisement> {
        if !object.as_str().starts_with(&self.prefix) {
            return None;
        }
        let device = self.devices.entry(object.clone()).or_default();
        device.apply(properties, invalidated);
        // Connection state and similar changes don't mean a device was heard.
        let heard = ["RSSI", "ManufacturerData", "ServiceData", "TxPower"]
            .iter()
            .any(|key| properties.contains_key(*key));
        let advertisement = heard.then(|| device.advertisement()).flatten();
        if advertisement.is_none() && heard {
            debug!(object = object.as_str(), "update without address or RSSI");
        }
        advertisement
    }

    fn remove(&mut self, object: &OwnedObjectPath) {
        self.devices.remove(object);
    }
}

/// Unwraps nested variants (`v` inside `v`).
fn unwrap<'a>(value: &'a Variant<'a>) -> &'a Variant<'a> {
    match value {
        Variant::Variant(inner) => unwrap(inner),
        other => other,
    }
}

fn string(value: &Variant<'_>) -> Option<String> {
    String::try_from(unwrap(value)).ok()
}

fn int16(value: &Variant<'_>) -> Option<i16> {
    match unwrap(value) {
        Variant::I16(value) => Some(*value),
        _ => None,
    }
}

fn strings(value: &Variant<'_>) -> Vec<String> {
    match unwrap(value) {
        Variant::Array(array) => array
            .inner()
            .iter()
            .filter_map(|item| String::try_from(item).ok())
            .collect(),
        _ => Vec::new(),
    }
}

fn bytes(value: &Variant<'_>) -> Option<Vec<u8>> {
    match unwrap(value) {
        Variant::Array(array) => array
            .inner()
            .iter()
            .map(|item| match item {
                Variant::U8(byte) => Some(*byte),
                _ => None,
            })
            .collect(),
        _ => None,
    }
}

fn entries<'a>(value: &'a Variant<'a>) -> impl Iterator<Item = (&'a Variant<'a>, &'a Variant<'a>)> {
    let dict = match unwrap(value) {
        Variant::Dict(dict) => Some(dict),
        _ => None,
    };
    dict.into_iter().flat_map(zbus::Dict::iter)
}

/// Clamps a D-Bus `int16` to the `i8` range BLE uses for dBm values.
fn saturate(value: i16) -> i8 {
    i8::try_from(value).unwrap_or(if value < 0 { i8::MIN } else { i8::MAX })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variant(value: impl Into<Variant<'static>>) -> OwnedVariant {
        OwnedVariant::try_from(&Variant::new(value.into())).unwrap()
    }

    fn device_properties() -> HashMap<String, OwnedVariant> {
        let manufacturer: HashMap<u16, Variant<'static>> =
            HashMap::from([(0x004C, Variant::from(vec![0x02_u8, 0x15]))]);
        let service: HashMap<String, Variant<'static>> = HashMap::from([(
            "0000fcd2-0000-1000-8000-00805f9b34fb".to_owned(),
            Variant::from(vec![0x40_u8]),
        )]);
        HashMap::from([
            ("Address".to_owned(), variant("A4:C1:38:00:11:22")),
            ("AddressType".to_owned(), variant("random")),
            ("RSSI".to_owned(), variant(-70_i16)),
            ("Name".to_owned(), variant("ATC")),
            ("ManufacturerData".to_owned(), variant(manufacturer)),
            ("ServiceData".to_owned(), variant(service)),
        ])
    }

    fn path(s: &str) -> OwnedObjectPath {
        OwnedObjectPath::try_from(s).unwrap()
    }

    #[test]
    fn rebuilds_advertisements_from_properties() {
        let mut devices = Devices::new("/org/bluez/hci0");
        let object = path("/org/bluez/hci0/dev_A4_C1_38_00_11_22");
        let advertisement = devices
            .update(&object, &device_properties(), &[])
            .expect("advertisement");
        assert_eq!(advertisement.address.to_string(), "A4:C1:38:00:11:22");
        assert_eq!(advertisement.address_type, AddressType::Random);
        assert_eq!(advertisement.rssi, -70);
        assert_eq!(
            advertisement.data.as_slice(),
            [
                5, 0xFF, 0x4C, 0x00, 0x02, 0x15, // manufacturer data
                4, 0x16, 0xD2, 0xFC, 0x40, // service data
                4, 0x09, b'A', b'T', b'C', // name
            ]
        );
    }

    #[test]
    fn follows_rssi_updates_and_ignores_other_changes() {
        let mut devices = Devices::new("/org/bluez/hci0");
        let object = path("/org/bluez/hci0/dev_A4_C1_38_00_11_22");
        devices.update(&object, &device_properties(), &[]);

        let rssi = HashMap::from([("RSSI".to_owned(), variant(-50_i16))]);
        let advertisement = devices.update(&object, &rssi, &[]).expect("advertisement");
        assert_eq!(advertisement.rssi, -50);

        let connected = HashMap::from([("Connected".to_owned(), variant(true))]);
        assert!(devices.update(&object, &connected, &[]).is_none());
    }

    #[test]
    fn needs_an_rssi_and_the_right_adapter() {
        let mut devices = Devices::new("/org/bluez/hci0");
        let mut properties = device_properties();
        properties.remove("RSSI");
        let object = path("/org/bluez/hci0/dev_A4_C1_38_00_11_22");
        let service = HashMap::from([(
            "ServiceData".to_owned(),
            properties.remove("ServiceData").unwrap(),
        )]);
        devices.update(&object, &properties, &[]);
        assert!(devices.update(&object, &service, &[]).is_none());

        let other = path("/org/bluez/hci1/dev_A4_C1_38_00_11_22");
        assert!(devices.update(&other, &device_properties(), &[]).is_none());
    }

    #[test]
    fn clamps_dbm_values() {
        assert_eq!(saturate(-70), -70);
        assert_eq!(saturate(-300), i8::MIN);
        assert_eq!(saturate(300), i8::MAX);
    }
}
