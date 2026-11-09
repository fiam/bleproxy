//! ESPHome Native API protocol support for bleproxy.
//!
//! Home Assistant talks to Bluetooth proxies over the ESPHome Native API: a
//! stream of protobuf messages, each preceded by a small frame header. This
//! crate owns the wire format and nothing else; it never touches sockets or
//! radios, and it is `no_std` so the same code runs on hosts and
//! microcontrollers.
//!
//! It covers plaintext framing and the messages a passive Bluetooth proxy
//! exchanges. Noise-encrypted framing will live here as well.

#![no_std]
#![forbid(unsafe_code)]

mod frame;
mod messages;
mod varint;
pub mod wire;

pub use frame::{FrameError, FrameHeader, MAX_HEADER_LEN, PLAINTEXT_INDICATOR, encode_frame};
pub use messages::{
    API_VERSION, BluetoothConnectionsFreeResponse, BluetoothLeRawAdvertisementsResponse,
    ClientMessage, DeviceInfoResponse, DisconnectRequest, DisconnectResponse, HelloRequest,
    HelloResponse, ListEntitiesDoneResponse, Message, PingRequest, PingResponse, RawAdvertisement,
    bluetooth_proxy_feature, bluetooth_proxy_subscription, message_type,
};
