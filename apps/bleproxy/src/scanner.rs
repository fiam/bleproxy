//! The link between API connections and the platform's scanner.
//!
//! Clients request scan modes and watch the scanner's status through a
//! [`ScannerHandle`]; the scanner task applies the requests and publishes
//! its status through a [`ScannerDriver`].

use std::sync::Arc;

use bleproxy_core::scanner::{ScanMode, ScannerState, ScannerStatus};
use tokio::sync::watch;

/// Creates a connected handle and driver for a scanner configured with
/// `configured`.
#[must_use]
pub fn channel(configured: ScanMode) -> (ScannerHandle, ScannerDriver) {
    let (requested_tx, requested_rx) = watch::channel(configured);
    let (status_tx, status_rx) = watch::channel(ScannerStatus {
        state: ScannerState::Idle,
        mode: configured,
    });
    let handle = ScannerHandle {
        configured,
        requested: Arc::new(requested_tx),
        status: status_rx,
    };
    let driver = ScannerDriver {
        requested: requested_rx,
        status: status_tx,
    };
    (handle, driver)
}

/// Used by API connections to control and observe the scanner.
#[derive(Debug, Clone)]
pub struct ScannerHandle {
    configured: ScanMode,
    requested: Arc<watch::Sender<ScanMode>>,
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
        self.requested.send_if_modified(|requested| {
            let changed = *requested != mode;
            *requested = mode;
            changed
        });
    }

    /// Returns a receiver that follows the scanner's status.
    #[must_use]
    pub fn status(&self) -> watch::Receiver<ScannerStatus> {
        self.status.clone()
    }
}

/// Used by the scanner task to receive mode requests and report status.
#[derive(Debug)]
pub struct ScannerDriver {
    requested: watch::Receiver<ScanMode>,
    status: watch::Sender<ScannerStatus>,
}

impl ScannerDriver {
    /// Returns the requested mode, marking it as seen.
    pub fn requested_mode(&mut self) -> ScanMode {
        *self.requested.borrow_and_update()
    }

    /// Waits until a mode other than `current` is requested, and returns
    /// it. Never completes if requests can no longer arrive.
    pub async fn mode_change(&mut self, current: ScanMode) -> ScanMode {
        loop {
            if self.requested.changed().await.is_err() {
                return std::future::pending().await;
            }
            let mode = *self.requested.borrow_and_update();
            if mode != current {
                return mode;
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

    #[tokio::test]
    async fn requests_reach_the_driver_and_status_reaches_handles() {
        let (handle, mut driver) = channel(ScanMode::Active);
        assert_eq!(handle.configured(), ScanMode::Active);
        assert_eq!(driver.requested_mode(), ScanMode::Active);

        let mut status = handle.status();
        assert_eq!(status.borrow_and_update().state, ScannerState::Idle);

        // Repeating the current mode is not a change.
        handle.request(ScanMode::Active);
        handle.request(ScanMode::Passive);
        let changed =
            tokio::time::timeout(Duration::from_secs(1), driver.mode_change(ScanMode::Active))
                .await
                .unwrap();
        assert_eq!(changed, ScanMode::Passive);

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
}
