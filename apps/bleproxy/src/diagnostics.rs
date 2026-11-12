//! Diagnostic sensors the proxy reports about itself: how long it has been
//! running, how many advertisements it forwarded, and how many clients are
//! connected.
//!
//! With an address filter, each allowed device also gets its signal
//! strength and when it was last heard.
//!
//! Clients list them like any ESPHome entity and receive their states
//! once they subscribe, then every [`STATE_INTERVAL`].

use std::collections::HashMap;
use std::fmt::Write as _;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use bleproxy_core::{Advertisement, BdAddr};
use bleproxy_protocol::{
    ListEntitiesSensorResponse, ListEntitiesTextSensorResponse, SensorStateResponse,
    TextSensorStateResponse, entity_category, sensor_state_class,
};
use tokio::sync::broadcast;

/// How often clients get fresh states.
pub const STATE_INTERVAL: Duration = Duration::from_secs(60);

const UPTIME: u32 = 1;
const ADVERTISEMENTS: u32 = 2;
const CONNECTED_CLIENTS: u32 = 3;
/// Key of the first allowed device's signal sensor. Each device takes two
/// keys, clear of the proxy's own entities.
const FIRST_DEVICE: u32 = 16;

/// The text sensors, as listed to clients. Object IDs match btproxy's, so
/// Home Assistant keeps the entities of a proxy that replaced it.
pub const TEXT_SENSORS: [ListEntitiesTextSensorResponse<'static>; 1] =
    [ListEntitiesTextSensorResponse {
        object_id: "uptime",
        key: UPTIME,
        name: "Uptime",
        icon: "mdi:clock-outline",
        entity_category: entity_category::DIAGNOSTIC,
        device_class: "",
    }];

/// The numeric sensors, as listed to clients.
pub const SENSORS: [ListEntitiesSensorResponse<'static>; 2] = [
    ListEntitiesSensorResponse {
        object_id: "advertisements",
        key: ADVERTISEMENTS,
        name: "Advertisements",
        icon: "mdi:bluetooth-transfer",
        unit_of_measurement: "adv",
        accuracy_decimals: 0,
        device_class: "",
        state_class: sensor_state_class::TOTAL_INCREASING,
        entity_category: entity_category::DIAGNOSTIC,
    },
    ListEntitiesSensorResponse {
        object_id: "connected_clients",
        key: CONNECTED_CLIENTS,
        name: "Connected Clients",
        icon: "mdi:account-network",
        unit_of_measurement: "",
        accuracy_decimals: 0,
        device_class: "",
        state_class: sensor_state_class::MEASUREMENT,
        entity_category: entity_category::DIAGNOSTIC,
    },
];

/// What the diagnostic sensors measure, shared by all connections.
#[derive(Debug)]
pub struct Diagnostics {
    started: Instant,
    advertisements: AtomicU64,
    /// Open connections by client host.
    clients: Mutex<HashMap<IpAddr, usize>>,
    devices: Vec<Device>,
    /// When each device was last heard, and how well.
    sightings: Mutex<HashMap<BdAddr, Sighting>>,
}

/// The sensors of an allowed device. Home Assistant identifies entities
/// by name, so names never change while the proxy runs.
#[derive(Debug)]
struct Device {
    address: BdAddr,
    key: u32,
    rssi_id: String,
    rssi_name: String,
    last_seen_id: String,
    last_seen_name: String,
}

impl Device {
    fn new(address: BdAddr, name: &str, key: u32) -> Self {
        let id = address.to_string().replace(':', "").to_lowercase();
        Self {
            address,
            key,
            rssi_id: format!("rssi_{id}"),
            rssi_name: format!("{name} RSSI"),
            last_seen_id: format!("last_seen_{id}"),
            last_seen_name: format!("{name} Last Seen"),
        }
    }

    const fn last_seen_key(&self) -> u32 {
        self.key + 1
    }
}

#[derive(Debug, Clone, Copy)]
struct Sighting {
    rssi: i8,
    at: SystemTime,
}

/// The states of an allowed device's sensors.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceStates {
    /// Its signal strength when last heard.
    pub rssi: SensorStateResponse,
    last_seen_key: u32,
    last_seen: Option<String>,
}

impl DeviceStates {
    /// Returns when it was last heard, as an RFC 3339 time.
    #[must_use]
    pub fn last_seen(&self) -> TextSensorStateResponse<'_> {
        TextSensorStateResponse {
            key: self.last_seen_key,
            state: self.last_seen.as_deref().unwrap_or_default(),
            missing_state: self.last_seen.is_none(),
        }
    }
}

impl Default for Diagnostics {
    fn default() -> Self {
        Self::new()
    }
}

impl Diagnostics {
    /// Starts measuring; uptime counts from now.
    #[must_use]
    pub fn new() -> Self {
        Self::with_devices([])
    }

    /// Starts measuring, also following the given devices, named for
    /// their sensors' names.
    #[must_use]
    pub fn with_devices(devices: impl IntoIterator<Item = (BdAddr, String)>) -> Self {
        let devices = (FIRST_DEVICE..)
            .step_by(2)
            .zip(devices)
            .map(|(key, (address, name))| Device::new(address, &name, key))
            .collect();
        Self {
            started: Instant::now(),
            advertisements: AtomicU64::new(0),
            clients: Mutex::new(HashMap::new()),
            devices,
            sightings: Mutex::new(HashMap::new()),
        }
    }

    /// Counts the advertisements published on `receiver`, until the
    /// channel closes. Each one counts once, however many clients it goes
    /// to.
    pub async fn count_advertisements(
        self: Arc<Self>,
        mut receiver: broadcast::Receiver<Advertisement>,
    ) {
        loop {
            let count = match receiver.recv().await {
                Ok(advertisement) => {
                    self.record(&advertisement);
                    1
                }
                // Missed ones were published all the same.
                Err(broadcast::error::RecvError::Lagged(missed)) => missed,
                Err(broadcast::error::RecvError::Closed) => return,
            };
            self.advertisements.fetch_add(count, Ordering::Relaxed);
        }
    }

    /// Remembers when a followed device was heard.
    fn record(&self, advertisement: &Advertisement) {
        let address = advertisement.address;
        if self.devices.iter().any(|device| device.address == address) {
            let sighting = Sighting {
                rssi: advertisement.rssi,
                at: SystemTime::now(),
            };
            lock(&self.sightings).insert(address, sighting);
        }
    }

    /// Records a connection from `host` until the returned guard is
    /// dropped. Hosts count once, however many connections they open.
    #[must_use]
    pub fn connect(self: &Arc<Self>, host: IpAddr) -> Connected {
        *self.lock_clients().entry(host).or_default() += 1;
        Connected {
            diagnostics: Arc::clone(self),
            host,
        }
    }

    /// Returns the uptime state, as text such as `1d 2h 3m 45s`.
    #[must_use]
    pub fn uptime(&self) -> String {
        format_uptime(self.started.elapsed())
    }

    /// Returns the numeric sensors' states.
    #[must_use]
    #[allow(
        clippy::cast_precision_loss,
        reason = "sensor states are floats; counts beyond 2^24 lose precision"
    )]
    pub fn sensor_states(&self) -> [SensorStateResponse; 2] {
        let clients = self.lock_clients().len();
        [
            SensorStateResponse {
                key: ADVERTISEMENTS,
                state: self.advertisements.load(Ordering::Relaxed) as f32,
                missing_state: false,
            },
            SensorStateResponse {
                key: CONNECTED_CLIENTS,
                state: clients as f32,
                missing_state: false,
            },
        ]
    }

    /// Returns the text sensors' state for `uptime`, as returned by
    /// [`Self::uptime`].
    #[must_use]
    pub const fn text_states(uptime: &str) -> [TextSensorStateResponse<'_>; 1] {
        [TextSensorStateResponse {
            key: UPTIME,
            state: uptime,
            missing_state: false,
        }]
    }

    /// Lists each followed device's signal and last-seen sensors.
    pub fn device_entities(
        &self,
    ) -> impl Iterator<
        Item = (
            ListEntitiesSensorResponse<'_>,
            ListEntitiesTextSensorResponse<'_>,
        ),
    > {
        self.devices.iter().map(|device| {
            let rssi = ListEntitiesSensorResponse {
                object_id: &device.rssi_id,
                key: device.key,
                name: &device.rssi_name,
                icon: "",
                unit_of_measurement: "dBm",
                accuracy_decimals: 0,
                device_class: "signal_strength",
                state_class: sensor_state_class::MEASUREMENT,
                entity_category: entity_category::DIAGNOSTIC,
            };
            let last_seen = ListEntitiesTextSensorResponse {
                object_id: &device.last_seen_id,
                key: device.last_seen_key(),
                name: &device.last_seen_name,
                icon: "",
                entity_category: entity_category::DIAGNOSTIC,
                device_class: "timestamp",
            };
            (rssi, last_seen)
        })
    }

    /// Returns the states of each followed device's sensors, missing until
    /// it is first heard.
    #[must_use]
    pub fn device_states(&self) -> Vec<DeviceStates> {
        let sightings = lock(&self.sightings);
        self.devices
            .iter()
            .map(|device| {
                let sighting = sightings.get(&device.address);
                DeviceStates {
                    rssi: SensorStateResponse {
                        key: device.key,
                        state: sighting.map_or(0.0, |s| f32::from(s.rssi)),
                        missing_state: sighting.is_none(),
                    },
                    last_seen_key: device.last_seen_key(),
                    last_seen: sighting.map(|s| format_timestamp(s.at)),
                }
            })
            .collect()
    }

    fn lock_clients(&self) -> std::sync::MutexGuard<'_, HashMap<IpAddr, usize>> {
        lock(&self.clients)
    }
}

/// Locks `mutex`; the maps it guards stay consistent even if a holder
/// panicked.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A client connection, counted until dropped.
#[derive(Debug)]
pub struct Connected {
    diagnostics: Arc<Diagnostics>,
    host: IpAddr,
}

impl Drop for Connected {
    fn drop(&mut self) {
        let mut clients = self.diagnostics.lock_clients();
        if let Some(count) = clients.get_mut(&self.host) {
            *count -= 1;
            if *count == 0 {
                clients.remove(&self.host);
            }
        }
    }
}

/// Formats `time` as RFC 3339 in UTC, such as `2026-11-12T15:55:06Z`, the
/// form clients parse timestamp sensors in.
fn format_timestamp(time: SystemTime) -> String {
    let seconds = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (year, month, day) = civil_from_days(seconds / 86_400);
    let time = seconds % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        time / 3_600,
        time % 3_600 / 60,
        time % 60
    )
}

/// Converts days since 1970-01-01 to a proleptic Gregorian date, with
/// Howard Hinnant's `civil_from_days`.
const fn civil_from_days(days: u64) -> (u64, u64, u64) {
    // Count from 0000-03-01, so leap days end the year.
    let days = days + 719_468;
    let era = days / 146_097;
    let day_of_era = days % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

/// Formats `uptime` the way ESPHome's uptime text sensor does: `45s`,
/// `3m 45s`, `2h 3m 45s`, `1d 2h 3m 45s`.
fn format_uptime(uptime: Duration) -> String {
    let total = uptime.as_secs();
    let (days, hours, minutes, seconds) = (
        total / 86_400,
        total % 86_400 / 3_600,
        total % 3_600 / 60,
        total % 60,
    );
    let mut text = String::new();
    if days > 0 {
        let _ = write!(text, "{days}d ");
    }
    if total >= 3_600 {
        let _ = write!(text, "{hours}h ");
    }
    if total >= 60 {
        let _ = write!(text, "{minutes}m ");
    }
    let _ = write!(text, "{seconds}s");
    text
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use bleproxy_core::{AddressType, AdvData};

    use super::*;

    const HOME_ASSISTANT: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 104));
    const LAPTOP: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20));

    const SCALE: BdAddr = BdAddr::new([0x5C, 0xCA, 0xD3, 0xBA, 0xBA, 0xCC]);

    fn advertisement(address: BdAddr, rssi: i8) -> Advertisement {
        Advertisement {
            address,
            address_type: AddressType::Public,
            rssi,
            data: AdvData::default(),
        }
    }

    fn states(diagnostics: &Diagnostics) -> (f32, f32) {
        let [advertisements, clients] = diagnostics.sensor_states();
        (advertisements.state, clients.state)
    }

    #[test]
    fn formats_uptime_like_esphome() {
        let cases = [
            (0, "0s"),
            (45, "45s"),
            (60, "1m 0s"),
            (3 * 60 + 45, "3m 45s"),
            (3_600, "1h 0m 0s"),
            (2 * 3_600 + 3 * 60 + 45, "2h 3m 45s"),
            (86_400 + 2 * 3_600 + 3 * 60 + 45, "1d 2h 3m 45s"),
            (3 * 86_400 + 5, "3d 0h 0m 5s"),
        ];
        for (seconds, expected) in cases {
            assert_eq!(format_uptime(Duration::from_secs(seconds)), expected);
        }
    }

    #[test]
    fn counts_client_hosts_once() {
        let diagnostics = Arc::new(Diagnostics::new());
        assert_eq!(states(&diagnostics).1, 0.0);
        let first = diagnostics.connect(HOME_ASSISTANT);
        let second = diagnostics.connect(HOME_ASSISTANT);
        let laptop = diagnostics.connect(LAPTOP);
        assert_eq!(states(&diagnostics).1, 2.0);
        drop(first);
        assert_eq!(states(&diagnostics).1, 2.0, "one connection left");
        drop(second);
        drop(laptop);
        assert_eq!(states(&diagnostics).1, 0.0);
    }

    #[tokio::test]
    async fn counts_published_advertisements() {
        let diagnostics = Arc::new(Diagnostics::new());
        let (sender, receiver) = broadcast::channel(2);
        let counting = tokio::spawn(Arc::clone(&diagnostics).count_advertisements(receiver));
        // More than the channel holds, so some are counted as missed.
        for _ in 0..5 {
            sender.send(advertisement(SCALE, -60)).unwrap();
        }
        drop(sender);
        counting.await.unwrap();
        assert_eq!(states(&diagnostics).0, 5.0);
    }

    #[test]
    fn formats_timestamps_in_utc() {
        let cases = [
            (0, "1970-01-01T00:00:00Z"),
            (951_782_400, "2000-02-29T00:00:00Z"),
            (1_794_498_906, "2026-11-12T15:55:06Z"),
            (4_107_628_799, "2100-03-01T23:59:59Z"),
        ];
        for (seconds, expected) in cases {
            let time = SystemTime::UNIX_EPOCH + Duration::from_secs(seconds);
            assert_eq!(format_timestamp(time), expected);
        }
    }

    #[tokio::test]
    async fn follows_allowed_devices() {
        const OTHER: BdAddr = BdAddr::new([0xA4, 0xC1, 0x38, 0x00, 0x11, 0x22]);
        let diagnostics = Arc::new(Diagnostics::with_devices([
            (SCALE, "Scale".to_owned()),
            (OTHER, OTHER.to_string()),
        ]));
        let listed: Vec<_> = diagnostics
            .device_entities()
            .map(|(rssi, last_seen)| {
                (
                    (rssi.key, rssi.object_id, rssi.name),
                    (last_seen.key, last_seen.object_id, last_seen.name),
                )
            })
            .collect();
        assert_eq!(
            listed,
            [
                (
                    (16, "rssi_5ccad3babacc", "Scale RSSI"),
                    (17, "last_seen_5ccad3babacc", "Scale Last Seen")
                ),
                (
                    (18, "rssi_a4c138001122", "A4:C1:38:00:11:22 RSSI"),
                    (19, "last_seen_a4c138001122", "A4:C1:38:00:11:22 Last Seen")
                ),
            ]
        );

        // Nothing heard yet.
        let states = diagnostics.device_states();
        assert!(states.iter().all(|s| s.rssi.missing_state));
        assert!(states.iter().all(|s| s.last_seen().missing_state));

        let (sender, receiver) = broadcast::channel(4);
        let counting = tokio::spawn(Arc::clone(&diagnostics).count_advertisements(receiver));
        let before = SystemTime::now();
        sender.send(advertisement(SCALE, -67)).unwrap();
        sender.send(advertisement(SCALE, -60)).unwrap();
        drop(sender);
        counting.await.unwrap();

        let [scale, other] = <[DeviceStates; 2]>::try_from(diagnostics.device_states()).unwrap();
        assert_eq!((scale.rssi.key, scale.rssi.state), (16, -60.0));
        assert!(!scale.rssi.missing_state);
        let last_seen = scale.last_seen();
        assert_eq!(last_seen.key, 17);
        assert!(!last_seen.missing_state);
        // Seen at or after `before`, to the second.
        assert!(last_seen.state >= format_timestamp(before).as_str());
        assert!(other.rssi.missing_state && other.last_seen().missing_state);
    }

    #[test]
    fn lists_entities_with_matching_keys() {
        let diagnostics = Diagnostics::new();
        let uptime = diagnostics.uptime();
        let text_keys = Diagnostics::text_states(&uptime).map(|state| state.key);
        assert_eq!(text_keys, TEXT_SENSORS.map(|sensor| sensor.key));
        let keys = diagnostics.sensor_states().map(|state| state.key);
        assert_eq!(keys, SENSORS.map(|sensor| sensor.key));
    }
}
