//! The bleproxy host application: a Bluetooth LE proxy for Home Assistant
//! that speaks the ESPHome Native API.
//!
//! The binary wires these pieces together; they live in a library so that
//! integration tests can drive them directly.

#[cfg(target_os = "linux")]
pub mod bluez;
pub mod connections;
pub mod diagnostics;
pub mod encryption;
pub mod filter;
pub mod identity;
pub mod mdns;
pub mod scanner;
pub mod server;
pub mod transport;

pub use identity::Identity;
