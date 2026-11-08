//! ESPHome Native API protocol support for bleproxy.
//!
//! Home Assistant talks to Bluetooth proxies over the ESPHome Native API: a
//! stream of protobuf messages, each preceded by a small frame header. This
//! crate owns the wire format and nothing else; it never touches sockets or
//! radios, and it is `no_std` so the same code runs on hosts and
//! microcontrollers.
//!
//! Only the plaintext frame header is implemented so far. Noise-encrypted
//! frames and the protobuf messages themselves will live here as well.

#![no_std]
#![forbid(unsafe_code)]

mod frame;
mod varint;

pub use frame::{FrameError, FrameHeader, MAX_HEADER_LEN, PLAINTEXT_INDICATOR};
