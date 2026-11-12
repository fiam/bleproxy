//! Diagnostic sensors the proxy reports about itself: how long it has been
//! running, how many advertisements it forwarded, and how many clients are
//! connected.
//!
//! Clients list them like any ESPHome entity and receive their states
//! once they subscribe, then every [`STATE_INTERVAL`].

use std::collections::HashMap;
use std::fmt::Write as _;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use bleproxy_core::Advertisement;
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

/// The text sensors, as listed to clients. Object IDs match btproxy's, so
/// Home Assistant keeps the entities of a proxy that replaced it.
pub const TEXT_SENSORS: [ListEntitiesTextSensorResponse<'static>; 1] =
    [ListEntitiesTextSensorResponse {
        object_id: "uptime",
        key: UPTIME,
        name: "Uptime",
        icon: "mdi:clock-outline",
        entity_category: entity_category::DIAGNOSTIC,
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
        Self {
            started: Instant::now(),
            advertisements: AtomicU64::new(0),
            clients: Mutex::new(HashMap::new()),
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
                Ok(_) => 1,
                // Missed ones were published all the same.
                Err(broadcast::error::RecvError::Lagged(missed)) => missed,
                Err(broadcast::error::RecvError::Closed) => return,
            };
            self.advertisements.fetch_add(count, Ordering::Relaxed);
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
            },
            SensorStateResponse {
                key: CONNECTED_CLIENTS,
                state: clients as f32,
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
        }]
    }

    fn lock_clients(&self) -> std::sync::MutexGuard<'_, HashMap<IpAddr, usize>> {
        // The map stays consistent even if a holder panicked.
        self.clients.lock().unwrap_or_else(PoisonError::into_inner)
    }
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

    use bleproxy_core::{AddressType, AdvData, BdAddr};

    use super::*;

    const HOME_ASSISTANT: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 104));
    const LAPTOP: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20));

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
        let advertisement = Advertisement {
            address: BdAddr::new([0x5C, 0xCA, 0xD3, 0xBA, 0xBA, 0xCC]),
            address_type: AddressType::Public,
            rssi: -60,
            data: AdvData::default(),
        };
        // More than the channel holds, so some are counted as missed.
        for _ in 0..5 {
            sender.send(advertisement).unwrap();
        }
        drop(sender);
        counting.await.unwrap();
        assert_eq!(states(&diagnostics).0, 5.0);
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
