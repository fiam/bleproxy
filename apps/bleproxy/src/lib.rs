//! The bleproxy host application: a Bluetooth LE proxy for Home Assistant
//! that speaks the ESPHome Native API.
//!
//! The binary wires these pieces together; they live in a library so that
//! integration tests can drive them directly.

pub mod identity;
pub mod mdns;
pub mod server;

pub use identity::Identity;
