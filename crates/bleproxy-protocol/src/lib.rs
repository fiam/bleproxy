//! ESPHome Native API protocol support for bleproxy.
//!
//! Home Assistant talks to Bluetooth proxies over the ESPHome Native API: a
//! stream of protobuf messages, each preceded by a small frame header. This
//! crate owns the wire format and nothing else; it never touches sockets or
//! radios, and it is `no_std` so the same code runs on hosts and
//! microcontrollers.
//!
//! It covers plaintext and Noise-encrypted framing, and the messages a
//! Bluetooth proxy exchanges.

#![no_std]
#![forbid(unsafe_code)]

mod frame;
mod messages;
pub mod noise;
mod varint;
pub mod wire;

pub use frame::{FrameError, FrameHeader, MAX_HEADER_LEN, PLAINTEXT_INDICATOR, encode_frame};
pub use messages::{
    API_VERSION, BluetoothConnectionsFreeResponse, BluetoothDeviceConnectionResponse,
    BluetoothDeviceRequest, BluetoothGattCharacteristic, BluetoothGattDescriptor,
    BluetoothGattErrorResponse, BluetoothGattGetServicesDoneResponse,
    BluetoothGattGetServicesResponse, BluetoothGattNotifyDataResponse, BluetoothGattNotifyResponse,
    BluetoothGattReadResponse, BluetoothGattService, BluetoothGattWriteResponse,
    BluetoothLeRawAdvertisementsResponse, BluetoothScannerStateResponse, ClientMessage,
    DeviceInfoResponse, DisconnectRequest, DisconnectResponse, HelloRequest, HelloResponse,
    ListEntitiesDoneResponse, ListEntitiesSensorResponse, ListEntitiesSwitchResponse,
    ListEntitiesTextSensorResponse, Message, PingRequest, PingResponse, RawAdvertisement,
    SensorStateResponse, SwitchStateResponse, TextSensorStateResponse,
    bluetooth_device_request_type, bluetooth_error, bluetooth_proxy_feature,
    bluetooth_proxy_subscription, bluetooth_scanner_mode, bluetooth_scanner_state, entity_category,
    message_type, sensor_state_class,
};
