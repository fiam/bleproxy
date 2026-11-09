//! Platform-independent building blocks for bleproxy.
//!
//! This crate holds the Bluetooth Low Energy domain types and the proxy
//! behavior shared by every bleproxy target, from desktop hosts to
//! microcontrollers. It is `no_std` and performs no I/O: platform crates
//! translate their native BLE events into these types, and drive a
//! [`session::Session`] per ESPHome API connection.

#![no_std]
#![forbid(unsafe_code)]

pub mod ad;
mod address;
mod advertisement;
pub mod session;
mod uuid;

pub use ad::AdvDataBuilder;
pub use address::{AddressError, BdAddr};
pub use advertisement::{AddressType, AdvData, Advertisement, AdvertisementError};
pub use uuid::{CompactUuid, Uuid, UuidParseError};
