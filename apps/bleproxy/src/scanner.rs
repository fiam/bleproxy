//! The link between API connections and the platform's scanner.
//!
//! Clients request scan modes, turn scanning on and off with the
//! [`SCANNING_SWITCH`], and watch the scanner's status through a
//! [`ScannerHandle`]; the scanner task applies the requests and publishes
//! its status through a [`ScannerDriver`].

use std::sync::Arc;

use bleproxy_core::scanner::{ScanMode, ScannerState, ScannerStatus};
use bleproxy_protocol::{ListEntitiesSwitchResponse, SwitchStateResponse, entity_category};
use tokio::sync::watch;

/// The BLE Scanning switch, as listed to clients. Turning it off stops
/// the scanner until it is turned back on; it starts on. The key differs
/// from the diagnostic sensors' keys.
pub const SCANNING_SWITCH: ListEntitiesSwitchResponse<'static> = ListEntitiesSwitchResponse {
    object_id: "ble_scanning",
    key: 4,
    name: "BLE Scanning",
    icon: "mdi:bluetooth-audio",
    entity_category: entity_category::CONFIG,
};

/// What clients want from the scanner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wanted {
    /// Mode to scan in.
    pub mode: ScanMode,
    /// Whether to scan at all.
    pub enabled: bool,
}

impl Wanted {
    /// Returns the [`SCANNING_SWITCH`] state for this request.
    #[must_use]
    pub const fn switch_state(&self) -> SwitchStateResponse {
        SwitchStateResponse {
            key: SCANNING_SWITCH.key,
            state: self.enabled,
        }
    }
}

/// Creates a connected handle and driver for a scanner configured with
/// `configured`, which starts out scanning.
#[must_use]
pub fn channel(configured: ScanMode) -> (ScannerHandle, ScannerDriver) {
    let (wanted_tx, wanted_rx) = watch::channel(Wanted {
        mode: configured,
        enabled: true,
    });
    let (status_tx, status_rx) = watch::channel(ScannerStatus {
        state: ScannerState::Idle,
        mode: configured,
    });
    let handle = ScannerHandle {
        configured,
        wanted: Arc::new(wanted_tx),
        status: status_rx,
    };
    let driver = ScannerDriver {
        wanted: wanted_rx,
        status: status_tx,
    };
    (handle, driver)
}

/// Used by API connections to control and observe the scanner.
#[derive(Debug, Clone)]
pub struct ScannerHandle {
    configured: ScanMode,
    wanted: Arc<watch::Sender<Wanted>>,
    status: watch::Receiver<ScannerStatus>,
}

impl ScannerHandle {
    /// Mode the scanner was configured with.
    #[must_use]
    pub const fn configured(&self) -> ScanMode {
        self.configured
    }

    /// Asks the scanner to switch to `mode`. Repeating the current request
    /// does nothing.
    pub fn request(&self, mode: ScanMode) {
        self.update(|wanted| wanted.mode = mode);
    }

    /// Turns scanning on or off.
    pub fn enable(&self, enabled: bool) {
        self.update(|wanted| wanted.enabled = enabled);
    }

    /// Returns a receiver that follows what clients want, such as whether
    /// scanning is on.
    #[must_use]
    pub fn wanted(&self) -> watch::Receiver<Wanted> {
        self.wanted.subscribe()
    }

    /// Returns a receiver that follows the scanner's status.
    #[must_use]
    pub fn status(&self) -> watch::Receiver<ScannerStatus> {
        self.status.clone()
    }

    fn update(&self, change: impl FnOnce(&mut Wanted)) {
        self.wanted.send_if_modified(|wanted| {
            let before = *wanted;
            change(wanted);
            *wanted != before
        });
    }
}

/// Used by the scanner task to receive requests and report status.
#[derive(Debug)]
pub struct ScannerDriver {
    wanted: watch::Receiver<Wanted>,
    status: watch::Sender<ScannerStatus>,
}

impl ScannerDriver {
    /// Returns what clients want, marking it as seen.
    pub fn wanted(&mut self) -> Wanted {
        *self.wanted.borrow_and_update()
    }

    /// Waits until clients want something other than `current`, and
    /// returns it. Never completes if requests can no longer arrive.
    pub async fn change(&mut self, current: Wanted) -> Wanted {
        loop {
            if self.wanted.changed().await.is_err() {
                return std::future::pending().await;
            }
            let wanted = *self.wanted.borrow_and_update();
            if wanted != current {
                return wanted;
            }
        }
    }

    /// Publishes the scanner's state while scanning in `mode`.
    pub fn report(&self, state: ScannerState, mode: ScanMode) {
        self.status.send_if_modified(|status| {
            let new = ScannerStatus { state, mode };
            let changed = *status != new;
            *status = new;
            changed
        });
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::diagnostics;

    async fn change(driver: &mut ScannerDriver, current: Wanted) -> Wanted {
        tokio::time::timeout(Duration::from_secs(1), driver.change(current))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn requests_reach_the_driver_and_status_reaches_handles() {
        let (handle, mut driver) = channel(ScanMode::Active);
        assert_eq!(handle.configured(), ScanMode::Active);
        let initial = driver.wanted();
        assert_eq!(
            initial,
            Wanted {
                mode: ScanMode::Active,
                enabled: true
            }
        );

        let mut status = handle.status();
        assert_eq!(status.borrow_and_update().state, ScannerState::Idle);

        // Repeating the current mode is not a change.
        handle.request(ScanMode::Active);
        handle.request(ScanMode::Passive);
        let passive = change(&mut driver, initial).await;
        assert_eq!(passive.mode, ScanMode::Passive);
        assert!(passive.enabled);

        driver.report(ScannerState::Running, ScanMode::Passive);
        status.changed().await.unwrap();
        assert_eq!(
            *status.borrow_and_update(),
            ScannerStatus {
                state: ScannerState::Running,
                mode: ScanMode::Passive,
            }
        );
        // Reporting the same status again does not notify.
        driver.report(ScannerState::Running, ScanMode::Passive);
        assert!(!status.has_changed().unwrap());
    }

    #[tokio::test]
    async fn the_switch_turns_scanning_off_and_on() {
        let (handle, mut driver) = channel(ScanMode::Passive);
        let mut followed = handle.wanted();
        let on = driver.wanted();
        assert!(followed.borrow_and_update().switch_state().state);

        handle.enable(false);
        let off = change(&mut driver, on).await;
        assert_eq!(
            off,
            Wanted {
                mode: ScanMode::Passive,
                enabled: false
            }
        );
        followed.changed().await.unwrap();
        let state = followed.borrow_and_update().switch_state();
        assert_eq!((state.key, state.state), (SCANNING_SWITCH.key, false));

        // Turning it off again changes nothing.
        handle.enable(false);
        assert!(!followed.has_changed().unwrap());
        handle.enable(true);
        assert_eq!(change(&mut driver, off).await, on);
    }

    #[test]
    fn switch_key_is_unique() {
        let mut sensors = diagnostics::TEXT_SENSORS
            .map(|sensor| sensor.key)
            .into_iter()
            .chain(diagnostics::SENSORS.map(|sensor| sensor.key));
        assert!(sensors.all(|key| key != SCANNING_SWITCH.key));
    }
}
