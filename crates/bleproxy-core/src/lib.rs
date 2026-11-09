//! Platform-independent building blocks for bleproxy.
//!
//! This crate holds the Bluetooth Low Energy domain types shared by every
//! bleproxy target, from desktop hosts to microcontrollers. It is `no_std`
//! and performs no I/O: platform crates translate their native BLE events
//! into these types and hand them to the shared proxy logic.

#![no_std]
#![forbid(unsafe_code)]

pub mod ad;
mod address;
mod advertisement;
mod uuid;

pub use ad::AdvDataBuilder;
pub use address::{AddressError, BdAddr};
pub use advertisement::{AddressType, AdvData, Advertisement, AdvertisementError};
pub use uuid::{CompactUuid, Uuid, UuidParseError};
