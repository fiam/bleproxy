//! Scanner modes and states, as reported to and requested by API clients.

use bleproxy_protocol::{bluetooth_scanner_mode, bluetooth_scanner_state};

/// How the scanner looks for advertisements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ScanMode {
    /// Listen only.
    Passive,
    /// Send scan requests, which also collects scan response data.
    #[default]
    Active,
}

impl ScanMode {
    /// Returns the mode's `BluetoothScannerMode` value.
    #[must_use]
    pub const fn to_wire(self) -> u32 {
        match self {
            Self::Passive => bluetooth_scanner_mode::PASSIVE,
            Self::Active => bluetooth_scanner_mode::ACTIVE,
        }
    }

    /// Converts a `BluetoothScannerMode` value; unknown values are `None`.
    #[must_use]
    pub const fn from_wire(value: u32) -> Option<Self> {
        match value {
            bluetooth_scanner_mode::PASSIVE => Some(Self::Passive),
            bluetooth_scanner_mode::ACTIVE => Some(Self::Active),
            _ => None,
        }
    }
}

/// Lifecycle of the scanner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ScannerState {
    /// Not scanning yet.
    #[default]
    Idle,
    /// Starting to scan.
    Starting,
    /// Scanning.
    Running,
    /// The last attempt to scan failed; it will be retried.
    Failed,
    /// Stopping, for example to switch modes.
    Stopping,
    /// Stopped.
    Stopped,
}

impl ScannerState {
    /// Returns the state's `BluetoothScannerState` value.
    #[must_use]
    pub const fn to_wire(self) -> u32 {
        match self {
            Self::Idle => bluetooth_scanner_state::IDLE,
            Self::Starting => bluetooth_scanner_state::STARTING,
            Self::Running => bluetooth_scanner_state::RUNNING,
            Self::Failed => bluetooth_scanner_state::FAILED,
            Self::Stopping => bluetooth_scanner_state::STOPPING,
            Self::Stopped => bluetooth_scanner_state::STOPPED,
        }
    }
}

/// What the scanner is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ScannerStatus {
    /// Lifecycle state.
    pub state: ScannerState,
    /// Mode of the current (or upcoming) scan.
    pub mode: ScanMode,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_round_trip_through_wire_values() {
        for mode in [ScanMode::Passive, ScanMode::Active] {
            assert_eq!(ScanMode::from_wire(mode.to_wire()), Some(mode));
        }
        assert_eq!(ScanMode::from_wire(7), None);
    }

    #[test]
    fn states_use_esphome_values() {
        let states = [
            ScannerState::Idle,
            ScannerState::Starting,
            ScannerState::Running,
            ScannerState::Failed,
            ScannerState::Stopping,
            ScannerState::Stopped,
        ];
        for (expected, state) in (0..).zip(states) {
            assert_eq!(state.to_wire(), expected, "{state:?}");
        }
    }
}
