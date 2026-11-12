//! ESPHome Native API messages used by a Bluetooth proxy.
//!
//! Field numbers and message type identifiers follow ESPHome's `api.proto`.
//! Only the messages a Bluetooth proxy exchanges are modeled; any other
//! client message decodes as [`ClientMessage::Unknown`].

use crate::wire::{DecodeError, Encode, EncodeError, Encoder, Field, Fields, Sink};

/// API version reported to clients.
///
/// 1.14 is the newest version for which clients still derive the device's
/// capabilities (Bluetooth proxy feature flags and MAC address) from
/// [`DeviceInfoResponse`]; from 1.15 on they expect a separate capabilities
/// request. Everything a proxy uses predates it.
pub const API_VERSION: (u32, u32) = (1, 14);

/// Message type identifiers sent in frame headers.
pub mod message_type {
    /// [`ClientMessage::Hello`](super::ClientMessage::Hello).
    pub const HELLO_REQUEST: u16 = 1;
    /// [`HelloResponse`](super::HelloResponse).
    pub const HELLO_RESPONSE: u16 = 2;
    /// [`ClientMessage::Authentication`](super::ClientMessage::Authentication).
    pub const AUTHENTICATION_REQUEST: u16 = 3;
    /// [`DisconnectRequest`](super::DisconnectRequest), from either side.
    pub const DISCONNECT_REQUEST: u16 = 5;
    /// [`DisconnectResponse`](super::DisconnectResponse), from either side.
    pub const DISCONNECT_RESPONSE: u16 = 6;
    /// [`PingRequest`](super::PingRequest), from either side.
    pub const PING_REQUEST: u16 = 7;
    /// [`PingResponse`](super::PingResponse), from either side.
    pub const PING_RESPONSE: u16 = 8;
    /// [`ClientMessage::DeviceInfo`](super::ClientMessage::DeviceInfo).
    pub const DEVICE_INFO_REQUEST: u16 = 9;
    /// [`DeviceInfoResponse`](super::DeviceInfoResponse).
    pub const DEVICE_INFO_RESPONSE: u16 = 10;
    /// [`ClientMessage::ListEntities`](super::ClientMessage::ListEntities).
    pub const LIST_ENTITIES_REQUEST: u16 = 11;
    /// [`ListEntitiesSensorResponse`](super::ListEntitiesSensorResponse).
    pub const LIST_ENTITIES_SENSOR_RESPONSE: u16 = 16;
    /// [`ListEntitiesSwitchResponse`](super::ListEntitiesSwitchResponse).
    pub const LIST_ENTITIES_SWITCH_RESPONSE: u16 = 17;
    /// [`ListEntitiesTextSensorResponse`](super::ListEntitiesTextSensorResponse).
    pub const LIST_ENTITIES_TEXT_SENSOR_RESPONSE: u16 = 18;
    /// [`ListEntitiesDoneResponse`](super::ListEntitiesDoneResponse).
    pub const LIST_ENTITIES_DONE_RESPONSE: u16 = 19;
    /// [`ClientMessage::SubscribeStates`](super::ClientMessage::SubscribeStates).
    pub const SUBSCRIBE_STATES_REQUEST: u16 = 20;
    /// [`SensorStateResponse`](super::SensorStateResponse).
    pub const SENSOR_STATE_RESPONSE: u16 = 25;
    /// [`SwitchStateResponse`](super::SwitchStateResponse).
    pub const SWITCH_STATE_RESPONSE: u16 = 26;
    /// [`TextSensorStateResponse`](super::TextSensorStateResponse).
    pub const TEXT_SENSOR_STATE_RESPONSE: u16 = 27;
    /// [`ClientMessage::SubscribeLogs`](super::ClientMessage::SubscribeLogs).
    pub const SUBSCRIBE_LOGS_REQUEST: u16 = 28;
    /// [`SubscribeLogsResponse`](super::SubscribeLogsResponse).
    pub const SUBSCRIBE_LOGS_RESPONSE: u16 = 29;
    /// [`ClientMessage::SwitchCommand`](super::ClientMessage::SwitchCommand).
    pub const SWITCH_COMMAND_REQUEST: u16 = 33;
    /// [`ClientMessage::SubscribeHomeassistantServices`](super::ClientMessage::SubscribeHomeassistantServices).
    pub const SUBSCRIBE_HOMEASSISTANT_SERVICES_REQUEST: u16 = 34;
    /// [`ClientMessage::SubscribeHomeAssistantStates`](super::ClientMessage::SubscribeHomeAssistantStates).
    pub const SUBSCRIBE_HOME_ASSISTANT_STATES_REQUEST: u16 = 38;
    /// [`ClientMessage::SubscribeBluetoothLeAdvertisements`](super::ClientMessage::SubscribeBluetoothLeAdvertisements).
    pub const SUBSCRIBE_BLUETOOTH_LE_ADVERTISEMENTS_REQUEST: u16 = 66;
    /// [`ClientMessage::BluetoothDeviceRequest`](super::ClientMessage::BluetoothDeviceRequest).
    pub const BLUETOOTH_DEVICE_REQUEST: u16 = 68;
    /// [`BluetoothDeviceConnectionResponse`](super::BluetoothDeviceConnectionResponse).
    pub const BLUETOOTH_DEVICE_CONNECTION_RESPONSE: u16 = 69;
    /// [`ClientMessage::BluetoothGattGetServices`](super::ClientMessage::BluetoothGattGetServices).
    pub const BLUETOOTH_GATT_GET_SERVICES_REQUEST: u16 = 70;
    /// [`BluetoothGattGetServicesResponse`](super::BluetoothGattGetServicesResponse).
    pub const BLUETOOTH_GATT_GET_SERVICES_RESPONSE: u16 = 71;
    /// [`BluetoothGattGetServicesDoneResponse`](super::BluetoothGattGetServicesDoneResponse).
    pub const BLUETOOTH_GATT_GET_SERVICES_DONE_RESPONSE: u16 = 72;
    /// [`ClientMessage::BluetoothGattRead`](super::ClientMessage::BluetoothGattRead).
    pub const BLUETOOTH_GATT_READ_REQUEST: u16 = 73;
    /// [`BluetoothGattReadResponse`](super::BluetoothGattReadResponse).
    pub const BLUETOOTH_GATT_READ_RESPONSE: u16 = 74;
    /// [`ClientMessage::BluetoothGattWrite`](super::ClientMessage::BluetoothGattWrite).
    pub const BLUETOOTH_GATT_WRITE_REQUEST: u16 = 75;
    /// [`ClientMessage::BluetoothGattReadDescriptor`](super::ClientMessage::BluetoothGattReadDescriptor).
    pub const BLUETOOTH_GATT_READ_DESCRIPTOR_REQUEST: u16 = 76;
    /// [`ClientMessage::BluetoothGattWriteDescriptor`](super::ClientMessage::BluetoothGattWriteDescriptor).
    pub const BLUETOOTH_GATT_WRITE_DESCRIPTOR_REQUEST: u16 = 77;
    /// [`ClientMessage::BluetoothGattNotify`](super::ClientMessage::BluetoothGattNotify).
    pub const BLUETOOTH_GATT_NOTIFY_REQUEST: u16 = 78;
    /// [`BluetoothGattNotifyDataResponse`](super::BluetoothGattNotifyDataResponse).
    pub const BLUETOOTH_GATT_NOTIFY_DATA_RESPONSE: u16 = 79;
    /// [`ClientMessage::SubscribeBluetoothConnectionsFree`](super::ClientMessage::SubscribeBluetoothConnectionsFree).
    pub const SUBSCRIBE_BLUETOOTH_CONNECTIONS_FREE_REQUEST: u16 = 80;
    /// [`BluetoothConnectionsFreeResponse`](super::BluetoothConnectionsFreeResponse).
    pub const BLUETOOTH_CONNECTIONS_FREE_RESPONSE: u16 = 81;
    /// [`BluetoothGattErrorResponse`](super::BluetoothGattErrorResponse).
    pub const BLUETOOTH_GATT_ERROR_RESPONSE: u16 = 82;
    /// [`BluetoothGattWriteResponse`](super::BluetoothGattWriteResponse).
    pub const BLUETOOTH_GATT_WRITE_RESPONSE: u16 = 83;
    /// [`BluetoothGattNotifyResponse`](super::BluetoothGattNotifyResponse).
    pub const BLUETOOTH_GATT_NOTIFY_RESPONSE: u16 = 84;
    /// [`ClientMessage::UnsubscribeBluetoothLeAdvertisements`](super::ClientMessage::UnsubscribeBluetoothLeAdvertisements).
    pub const UNSUBSCRIBE_BLUETOOTH_LE_ADVERTISEMENTS_REQUEST: u16 = 87;
    /// [`BluetoothLeRawAdvertisementsResponse`](super::BluetoothLeRawAdvertisementsResponse).
    pub const BLUETOOTH_LE_RAW_ADVERTISEMENTS_RESPONSE: u16 = 93;
    /// [`BluetoothScannerStateResponse`](super::BluetoothScannerStateResponse).
    pub const BLUETOOTH_SCANNER_STATE_RESPONSE: u16 = 126;
    /// [`ClientMessage::BluetoothScannerSetMode`](super::ClientMessage::BluetoothScannerSetMode).
    pub const BLUETOOTH_SCANNER_SET_MODE_REQUEST: u16 = 127;
}

/// Bluetooth proxy capabilities, reported in
/// [`DeviceInfoResponse::bluetooth_proxy_feature_flags`].
pub mod bluetooth_proxy_feature {
    /// Forwards advertisements received while scanning.
    pub const PASSIVE_SCAN: u32 = 1 << 0;
    /// Connects to devices and proxies their GATT services.
    pub const ACTIVE_CONNECTIONS: u32 = 1 << 1;
    /// Lets clients cache GATT services and handle the client
    /// characteristic configuration descriptor themselves (version 3
    /// connections). Current clients require it for active connections.
    pub const REMOTE_CACHING: u32 = 1 << 2;
    /// Forwards advertisements in their raw form (see
    /// [`BluetoothLeRawAdvertisementsResponse`](super::BluetoothLeRawAdvertisementsResponse)).
    pub const RAW_ADVERTISEMENTS: u32 = 1 << 5;
    /// Reports scanner state (see
    /// [`BluetoothScannerStateResponse`](super::BluetoothScannerStateResponse))
    /// and accepts scan mode changes.
    pub const STATE_AND_MODE: u32 = 1 << 6;
}

/// Values of the `EntityCategory` enum, which tells clients where to show
/// an entity.
pub mod entity_category {
    /// A regular entity.
    pub const NONE: u32 = 0;
    /// A setting.
    pub const CONFIG: u32 = 1;
    /// Information about the device itself, rather than what it controls.
    pub const DIAGNOSTIC: u32 = 2;
}

/// Values of the `SensorStateClass` enum, which tells clients how to keep
/// statistics for a sensor.
pub mod sensor_state_class {
    /// No statistics.
    pub const NONE: u32 = 0;
    /// A current value, such as a temperature.
    pub const MEASUREMENT: u32 = 1;
    /// A counter that only grows, except for resets.
    pub const TOTAL_INCREASING: u32 = 2;
    /// A total that may grow and shrink.
    pub const TOTAL: u32 = 3;
}

/// Values of the `LogLevel` enum. Each level includes the ones before it.
pub mod log_level {
    /// No logs.
    pub const NONE: u32 = 0;
    /// Errors.
    pub const ERROR: u32 = 1;
    /// Warnings.
    pub const WARN: u32 = 2;
    /// Notable events.
    pub const INFO: u32 = 3;
    /// The device's configuration, logged on request.
    pub const CONFIG: u32 = 4;
    /// Details for debugging.
    pub const DEBUG: u32 = 5;
    /// More details.
    pub const VERBOSE: u32 = 6;
    /// Everything.
    pub const VERY_VERBOSE: u32 = 7;
}

/// Values of the `BluetoothScannerMode` enum.
pub mod bluetooth_scanner_mode {
    /// Listen only.
    pub const PASSIVE: u32 = 0;
    /// Send scan requests.
    pub const ACTIVE: u32 = 1;
}

/// Values of the `BluetoothScannerState` enum.
pub mod bluetooth_scanner_state {
    /// Not scanning.
    pub const IDLE: u32 = 0;
    /// Starting to scan.
    pub const STARTING: u32 = 1;
    /// Scanning.
    pub const RUNNING: u32 = 2;
    /// Scanning failed.
    pub const FAILED: u32 = 3;
    /// Stopping the scan.
    pub const STOPPING: u32 = 4;
    /// The scan stopped.
    pub const STOPPED: u32 = 5;
}

/// Values of the `BluetoothDeviceRequestType` enum, carried by
/// [`BluetoothDeviceRequest::request_type`].
pub mod bluetooth_device_request_type {
    /// Connect (version 1); no longer used by clients.
    pub const CONNECT: u32 = 0;
    /// Disconnect, or cancel a pending connection.
    pub const DISCONNECT: u32 = 1;
    /// Pair with the device.
    pub const PAIR: u32 = 2;
    /// Remove the pairing.
    pub const UNPAIR: u32 = 3;
    /// Connect (version 3); the client has the device's services cached.
    pub const CONNECT_V3_WITH_CACHE: u32 = 4;
    /// Connect (version 3); the client will ask for the services.
    pub const CONNECT_V3_WITHOUT_CACHE: u32 = 5;
    /// Clear the proxy's cache of the device's services.
    pub const CLEAR_CACHE: u32 = 6;
}

/// Error codes of connection and GATT responses, as ESPHome reports them
/// (ESP-IDF's GATT status and connection reason codes). Clients turn them
/// into messages.
pub mod bluetooth_error {
    /// The device is not connected (an ESPHome extension).
    pub const NOT_CONNECTED: i32 = -1;
    /// No attribute has the handle.
    pub const INVALID_HANDLE: i32 = 0x01;
    /// The attribute cannot be read.
    pub const READ_NOT_PERMITTED: i32 = 0x02;
    /// The attribute cannot be written.
    pub const WRITE_NOT_PERMITTED: i32 = 0x03;
    /// The device requires pairing.
    pub const INSUFFICIENT_AUTHENTICATION: i32 = 0x05;
    /// The device doesn't support the request.
    pub const REQUEST_NOT_SUPPORTED: i32 = 0x06;
    /// The offset is past the end of the value.
    pub const INVALID_OFFSET: i32 = 0x07;
    /// The device requires authorization.
    pub const INSUFFICIENT_AUTHORIZATION: i32 = 0x08;
    /// The value has the wrong length.
    pub const INVALID_ATTRIBUTE_LENGTH: i32 = 0x0D;
    /// The connection could not be established.
    pub const CONNECTION_FAILED: i32 = 0x3E;
    /// Any other failure.
    pub const ERROR: i32 = 0x85;
}

/// Flags of [`ClientMessage::SubscribeBluetoothLeAdvertisements`].
pub mod bluetooth_proxy_subscription {
    /// The client wants raw advertisements.
    pub const RAW_ADVERTISEMENTS: u32 = 1 << 0;
}

/// A message the server sends.
pub trait Message: Encode {
    /// Message type identifier sent in the frame header.
    const TYPE: u16;
}

/// A message received from an API client, decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClientMessage<'a> {
    /// Opens the session; answered with [`HelloResponse`].
    Hello(HelloRequest<'a>),
    /// Legacy password authentication; ignored, since no password is used.
    Authentication,
    /// The client is closing the connection.
    Disconnect,
    /// Acknowledges a server-initiated [`DisconnectRequest`].
    DisconnectResponse,
    /// Keepalive; answered with [`PingResponse`].
    Ping,
    /// Answers a server-initiated [`PingRequest`].
    PingResponse,
    /// Asks for [`DeviceInfoResponse`].
    DeviceInfo,
    /// Asks for the device's entities, ending with
    /// [`ListEntitiesDoneResponse`].
    ListEntities,
    /// Subscribes to entity state updates.
    SubscribeStates,
    /// Subscribes to device logs, answered with [`SubscribeLogsResponse`]
    /// messages. A later subscription replaces the level.
    SubscribeLogs {
        /// The most detailed [`log_level`] to send.
        level: u32,
        /// Whether to log the device's configuration now.
        dump_config: bool,
    },
    /// Subscribes to Home Assistant service calls made by the device.
    SubscribeHomeassistantServices,
    /// Subscribes to Home Assistant states the device wants to follow.
    SubscribeHomeAssistantStates,
    /// Starts forwarding Bluetooth advertisements.
    SubscribeBluetoothLeAdvertisements {
        /// Combination of [`bluetooth_proxy_subscription`] flags.
        flags: u32,
    },
    /// Stops forwarding Bluetooth advertisements.
    UnsubscribeBluetoothLeAdvertisements,
    /// Subscribes to the number of free active connection slots.
    SubscribeBluetoothConnectionsFree,
    /// Connects to, disconnects from or pairs with a device.
    BluetoothDeviceRequest(BluetoothDeviceRequest),
    /// Asks for a connected device's GATT services.
    BluetoothGattGetServices {
        /// Device address as a 48-bit integer.
        address: u64,
    },
    /// Reads a characteristic.
    BluetoothGattRead {
        /// Device address as a 48-bit integer.
        address: u64,
        /// Characteristic handle.
        handle: u32,
    },
    /// Writes a characteristic.
    BluetoothGattWrite {
        /// Device address as a 48-bit integer.
        address: u64,
        /// Characteristic handle.
        handle: u32,
        /// Whether to write with a response (a write request rather than a
        /// write command), and confirm with [`BluetoothGattWriteResponse`].
        response: bool,
        /// Value to write.
        data: &'a [u8],
    },
    /// Reads a descriptor.
    BluetoothGattReadDescriptor {
        /// Device address as a 48-bit integer.
        address: u64,
        /// Descriptor handle.
        handle: u32,
    },
    /// Writes a descriptor.
    BluetoothGattWriteDescriptor {
        /// Device address as a 48-bit integer.
        address: u64,
        /// Descriptor handle.
        handle: u32,
        /// Value to write.
        data: &'a [u8],
    },
    /// Starts or stops forwarding a characteristic's notifications.
    BluetoothGattNotify {
        /// Device address as a 48-bit integer.
        address: u64,
        /// Characteristic handle.
        handle: u32,
        /// Whether to start (or stop) forwarding.
        enable: bool,
    },
    /// Asks the proxy to scan in another mode.
    BluetoothScannerSetMode {
        /// One of the [`bluetooth_scanner_mode`] values.
        mode: u32,
    },
    /// Turns a switch on or off.
    SwitchCommand {
        /// The switch's [`ListEntitiesSwitchResponse::key`].
        key: u32,
        /// Whether to turn it on.
        state: bool,
    },
    /// A message type this crate does not model.
    Unknown(u16),
}

/// Asks the proxy to connect to, disconnect from or pair with a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BluetoothDeviceRequest {
    /// Device address as a 48-bit integer.
    pub address: u64,
    /// One of the [`bluetooth_device_request_type`] values.
    pub request_type: u32,
    /// Address type (0 public, 1 random), if the client knows it.
    pub address_type: Option<u32>,
}

/// The first message of every session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HelloRequest<'a> {
    /// Free-form description of the client, for logs.
    pub client_info: &'a str,
    /// Major API version the client speaks.
    pub api_version_major: u32,
    /// Minor API version the client speaks.
    pub api_version_minor: u32,
}

impl<'a> ClientMessage<'a> {
    /// Decodes the payload of a frame whose header carries `type_id`.
    ///
    /// Unknown fields are skipped, so newer clients remain compatible.
    ///
    /// ```
    /// use bleproxy_protocol::ClientMessage;
    ///
    /// // SubscribeBluetoothLEAdvertisementsRequest { flags: 1 }
    /// let message = ClientMessage::decode(66, &[0x08, 0x01])?;
    /// assert_eq!(message, ClientMessage::SubscribeBluetoothLeAdvertisements { flags: 1 });
    /// # Ok::<(), bleproxy_protocol::wire::DecodeError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a [`DecodeError`] if the payload of a modeled message is
    /// malformed.
    pub fn decode(type_id: u16, payload: &'a [u8]) -> Result<Self, DecodeError> {
        use message_type as t;
        Ok(match type_id {
            t::HELLO_REQUEST => Self::Hello(HelloRequest::decode(payload)?),
            t::AUTHENTICATION_REQUEST => Self::Authentication,
            t::DISCONNECT_REQUEST => Self::Disconnect,
            t::DISCONNECT_RESPONSE => Self::DisconnectResponse,
            t::PING_REQUEST => Self::Ping,
            t::PING_RESPONSE => Self::PingResponse,
            t::DEVICE_INFO_REQUEST => Self::DeviceInfo,
            t::LIST_ENTITIES_REQUEST => Self::ListEntities,
            t::SUBSCRIBE_STATES_REQUEST => Self::SubscribeStates,
            t::SUBSCRIBE_LOGS_REQUEST => {
                let (mut level, mut dump_config) = (log_level::NONE, false);
                for field in Fields::new(payload) {
                    let field = field?;
                    match field.number {
                        1 => level = field.as_u32()?,
                        2 => dump_config = field.as_bool()?,
                        _ => {}
                    }
                }
                Self::SubscribeLogs { level, dump_config }
            }
            t::SUBSCRIBE_HOMEASSISTANT_SERVICES_REQUEST => Self::SubscribeHomeassistantServices,
            t::SUBSCRIBE_HOME_ASSISTANT_STATES_REQUEST => Self::SubscribeHomeAssistantStates,
            t::SUBSCRIBE_BLUETOOTH_LE_ADVERTISEMENTS_REQUEST => {
                let mut flags = 0;
                for field in Fields::new(payload) {
                    let field = field?;
                    if field.number == 1 {
                        flags = field.as_u32()?;
                    }
                }
                Self::SubscribeBluetoothLeAdvertisements { flags }
            }
            t::UNSUBSCRIBE_BLUETOOTH_LE_ADVERTISEMENTS_REQUEST => {
                Self::UnsubscribeBluetoothLeAdvertisements
            }
            t::SUBSCRIBE_BLUETOOTH_CONNECTIONS_FREE_REQUEST => {
                Self::SubscribeBluetoothConnectionsFree
            }
            t::BLUETOOTH_DEVICE_REQUEST => {
                Self::BluetoothDeviceRequest(BluetoothDeviceRequest::decode(payload)?)
            }
            // The GATT requests, and the responses in between.
            t::BLUETOOTH_GATT_GET_SERVICES_REQUEST..=t::BLUETOOTH_GATT_NOTIFY_REQUEST => {
                Self::decode_gatt_request(type_id, payload)?
            }
            t::BLUETOOTH_SCANNER_SET_MODE_REQUEST => {
                let mut mode = bluetooth_scanner_mode::PASSIVE;
                for field in Fields::new(payload) {
                    let field = field?;
                    if field.number == 1 {
                        mode = field.as_u32()?;
                    }
                }
                Self::BluetoothScannerSetMode { mode }
            }
            t::SWITCH_COMMAND_REQUEST => {
                let (mut key, mut state) = (0, false);
                for field in Fields::new(payload) {
                    let field = field?;
                    match field.number {
                        1 => key = field.as_fixed32()?,
                        2 => state = field.as_bool()?,
                        _ => {}
                    }
                }
                Self::SwitchCommand { key, state }
            }
            other => Self::Unknown(other),
        })
    }
}

impl<'a> ClientMessage<'a> {
    fn decode_gatt_request(type_id: u16, payload: &'a [u8]) -> Result<Self, DecodeError> {
        use message_type as t;
        Ok(match type_id {
            t::BLUETOOTH_GATT_GET_SERVICES_REQUEST => {
                let (address, _) = decode_gatt(payload, |_| Ok(()))?;
                Self::BluetoothGattGetServices { address }
            }
            t::BLUETOOTH_GATT_READ_REQUEST => {
                let (address, handle) = decode_gatt(payload, |_| Ok(()))?;
                Self::BluetoothGattRead { address, handle }
            }
            t::BLUETOOTH_GATT_WRITE_REQUEST => {
                let (mut response, mut data) = (false, &[][..]);
                let (address, handle) = decode_gatt(payload, |field| {
                    match field.number {
                        3 => response = field.as_bool()?,
                        4 => data = field.as_bytes()?,
                        _ => {}
                    }
                    Ok(())
                })?;
                Self::BluetoothGattWrite {
                    address,
                    handle,
                    response,
                    data,
                }
            }
            t::BLUETOOTH_GATT_READ_DESCRIPTOR_REQUEST => {
                let (address, handle) = decode_gatt(payload, |_| Ok(()))?;
                Self::BluetoothGattReadDescriptor { address, handle }
            }
            t::BLUETOOTH_GATT_WRITE_DESCRIPTOR_REQUEST => {
                let mut data = &[][..];
                let (address, handle) = decode_gatt(payload, |field| {
                    if field.number == 3 {
                        data = field.as_bytes()?;
                    }
                    Ok(())
                })?;
                Self::BluetoothGattWriteDescriptor {
                    address,
                    handle,
                    data,
                }
            }
            t::BLUETOOTH_GATT_NOTIFY_REQUEST => {
                let mut enable = false;
                let (address, handle) = decode_gatt(payload, |field| {
                    if field.number == 3 {
                        enable = field.as_bool()?;
                    }
                    Ok(())
                })?;
                Self::BluetoothGattNotify {
                    address,
                    handle,
                    enable,
                }
            }
            other => Self::Unknown(other),
        })
    }
}

/// Decodes a GATT request: an address (field 1) and a handle (field 2),
/// with any other field passed to `other`.
fn decode_gatt<'a>(
    payload: &'a [u8],
    mut other: impl FnMut(Field<'a>) -> Result<(), DecodeError>,
) -> Result<(u64, u32), DecodeError> {
    let (mut address, mut handle) = (0, 0);
    for field in Fields::new(payload) {
        let field = field?;
        match field.number {
            1 => address = field.as_u64()?,
            2 => handle = field.as_u32()?,
            _ => other(field)?,
        }
    }
    Ok((address, handle))
}

impl BluetoothDeviceRequest {
    fn decode(payload: &[u8]) -> Result<Self, DecodeError> {
        let mut request = Self::default();
        let (mut has_address_type, mut address_type) = (false, 0);
        for field in Fields::new(payload) {
            let field = field?;
            match field.number {
                1 => request.address = field.as_u64()?,
                2 => request.request_type = field.as_u32()?,
                3 => has_address_type = field.as_bool()?,
                4 => address_type = field.as_u32()?,
                _ => {}
            }
        }
        request.address_type = has_address_type.then_some(address_type);
        Ok(request)
    }
}

impl<'a> HelloRequest<'a> {
    fn decode(payload: &'a [u8]) -> Result<Self, DecodeError> {
        let mut hello = Self::default();
        for field in Fields::new(payload) {
            let field = field?;
            match field.number {
                1 => hello.client_info = field.as_str()?,
                2 => hello.api_version_major = field.as_u32()?,
                3 => hello.api_version_minor = field.as_u32()?,
                _ => {}
            }
        }
        Ok(hello)
    }
}

/// Answers [`ClientMessage::Hello`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HelloResponse<'a> {
    /// Major API version the server speaks.
    pub api_version_major: u32,
    /// Minor API version the server speaks.
    pub api_version_minor: u32,
    /// Free-form description of the server, for logs.
    pub server_info: &'a str,
    /// Device name. Clients compare it with the name they expect, so it
    /// must match the advertised mDNS host name.
    pub name: &'a str,
}

impl Encode for HelloResponse<'_> {
    fn encode<S: Sink>(&self, e: &mut Encoder<S>) -> Result<(), EncodeError> {
        e.uint32(1, self.api_version_major)?;
        e.uint32(2, self.api_version_minor)?;
        e.string(3, self.server_info)?;
        e.string(4, self.name)
    }
}

impl Message for HelloResponse<'_> {
    const TYPE: u16 = message_type::HELLO_RESPONSE;
}

/// Describes the device; answers [`ClientMessage::DeviceInfo`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeviceInfoResponse<'a> {
    /// Device name, as in [`HelloResponse::name`].
    pub name: &'a str,
    /// Device MAC address (`AA:BB:CC:DD:EE:FF`). Home Assistant uses it as
    /// the device's unique identifier.
    pub mac_address: &'a str,
    /// Firmware version.
    pub esphome_version: &'a str,
    /// Hardware or platform model.
    pub model: &'a str,
    /// Project name in `author.project` form. Identifies firmware that is
    /// not ESPHome itself.
    pub project_name: &'a str,
    /// Project version.
    pub project_version: &'a str,
    /// Combination of [`bluetooth_proxy_feature`] flags.
    pub bluetooth_proxy_feature_flags: u32,
    /// Manufacturer name.
    pub manufacturer: &'a str,
    /// Human-friendly device name.
    pub friendly_name: &'a str,
    /// MAC address of the Bluetooth adapter (`AA:BB:CC:DD:EE:FF`).
    pub bluetooth_mac_address: &'a str,
    /// Whether the API is encrypted (see [`crate::noise`]).
    pub api_encryption_supported: bool,
}

impl Encode for DeviceInfoResponse<'_> {
    fn encode<S: Sink>(&self, e: &mut Encoder<S>) -> Result<(), EncodeError> {
        e.string(2, self.name)?;
        e.string(3, self.mac_address)?;
        e.string(4, self.esphome_version)?;
        e.string(6, self.model)?;
        e.string(8, self.project_name)?;
        e.string(9, self.project_version)?;
        e.string(12, self.manufacturer)?;
        e.string(13, self.friendly_name)?;
        e.uint32(15, self.bluetooth_proxy_feature_flags)?;
        e.string(18, self.bluetooth_mac_address)?;
        e.bool(19, self.api_encryption_supported)
    }
}

impl Message for DeviceInfoResponse<'_> {
    const TYPE: u16 = message_type::DEVICE_INFO_RESPONSE;
}

/// Describes a numeric sensor, in answer to
/// [`ClientMessage::ListEntities`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ListEntitiesSensorResponse<'a> {
    /// Identifier, unique among the device's sensors. Clients derive the
    /// entity's identity from it.
    pub object_id: &'a str,
    /// Key that state updates refer to.
    pub key: u32,
    /// Display name.
    pub name: &'a str,
    /// Icon, such as `mdi:counter`.
    pub icon: &'a str,
    /// Unit of the values.
    pub unit_of_measurement: &'a str,
    /// Decimal places to show.
    pub accuracy_decimals: i32,
    /// One of the [`sensor_state_class`] values.
    pub state_class: u32,
    /// One of the [`entity_category`] values.
    pub entity_category: u32,
}

impl Encode for ListEntitiesSensorResponse<'_> {
    fn encode<S: Sink>(&self, e: &mut Encoder<S>) -> Result<(), EncodeError> {
        e.string(1, self.object_id)?;
        e.fixed32(2, self.key)?;
        e.string(3, self.name)?;
        e.string(5, self.icon)?;
        e.string(6, self.unit_of_measurement)?;
        e.int32(7, self.accuracy_decimals)?;
        e.uint32(10, self.state_class)?;
        e.uint32(13, self.entity_category)
    }
}

impl Message for ListEntitiesSensorResponse<'_> {
    const TYPE: u16 = message_type::LIST_ENTITIES_SENSOR_RESPONSE;
}

/// Describes a text sensor, in answer to [`ClientMessage::ListEntities`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ListEntitiesTextSensorResponse<'a> {
    /// Identifier, unique among the device's text sensors. Clients derive
    /// the entity's identity from it.
    pub object_id: &'a str,
    /// Key that state updates refer to.
    pub key: u32,
    /// Display name.
    pub name: &'a str,
    /// Icon, such as `mdi:clock-outline`.
    pub icon: &'a str,
    /// One of the [`entity_category`] values.
    pub entity_category: u32,
}

impl Encode for ListEntitiesTextSensorResponse<'_> {
    fn encode<S: Sink>(&self, e: &mut Encoder<S>) -> Result<(), EncodeError> {
        e.string(1, self.object_id)?;
        e.fixed32(2, self.key)?;
        e.string(3, self.name)?;
        e.string(5, self.icon)?;
        e.uint32(7, self.entity_category)
    }
}

impl Message for ListEntitiesTextSensorResponse<'_> {
    const TYPE: u16 = message_type::LIST_ENTITIES_TEXT_SENSOR_RESPONSE;
}

/// Describes a switch, in answer to [`ClientMessage::ListEntities`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ListEntitiesSwitchResponse<'a> {
    /// Identifier, unique among the device's switches. Clients derive the
    /// entity's identity from it.
    pub object_id: &'a str,
    /// Key that states and commands refer to.
    pub key: u32,
    /// Display name.
    pub name: &'a str,
    /// Icon, such as `mdi:toggle-switch`.
    pub icon: &'a str,
    /// One of the [`entity_category`] values.
    pub entity_category: u32,
}

impl Encode for ListEntitiesSwitchResponse<'_> {
    fn encode<S: Sink>(&self, e: &mut Encoder<S>) -> Result<(), EncodeError> {
        e.string(1, self.object_id)?;
        e.fixed32(2, self.key)?;
        e.string(3, self.name)?;
        e.string(5, self.icon)?;
        e.uint32(8, self.entity_category)
    }
}

impl Message for ListEntitiesSwitchResponse<'_> {
    const TYPE: u16 = message_type::LIST_ENTITIES_SWITCH_RESPONSE;
}

/// A switch's state, sent once the client subscribed to states, and
/// whenever it changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SwitchStateResponse {
    /// The switch's [`ListEntitiesSwitchResponse::key`].
    pub key: u32,
    /// Whether it is on.
    pub state: bool,
}

impl Encode for SwitchStateResponse {
    fn encode<S: Sink>(&self, e: &mut Encoder<S>) -> Result<(), EncodeError> {
        e.fixed32(1, self.key)?;
        e.bool(2, self.state)
    }
}

impl Message for SwitchStateResponse {
    const TYPE: u16 = message_type::SWITCH_STATE_RESPONSE;
}

/// A numeric sensor's state, sent once the client subscribed to states.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SensorStateResponse {
    /// The sensor's [`ListEntitiesSensorResponse::key`].
    pub key: u32,
    /// Its value.
    pub state: f32,
}

impl Encode for SensorStateResponse {
    fn encode<S: Sink>(&self, e: &mut Encoder<S>) -> Result<(), EncodeError> {
        e.fixed32(1, self.key)?;
        e.float(2, self.state)
    }
}

impl Message for SensorStateResponse {
    const TYPE: u16 = message_type::SENSOR_STATE_RESPONSE;
}

/// A text sensor's state, sent once the client subscribed to states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextSensorStateResponse<'a> {
    /// The sensor's [`ListEntitiesTextSensorResponse::key`].
    pub key: u32,
    /// Its value.
    pub state: &'a str,
}

impl Encode for TextSensorStateResponse<'_> {
    fn encode<S: Sink>(&self, e: &mut Encoder<S>) -> Result<(), EncodeError> {
        e.fixed32(1, self.key)?;
        e.string(2, self.state)
    }
}

impl Message for TextSensorStateResponse<'_> {
    const TYPE: u16 = message_type::TEXT_SENSOR_STATE_RESPONSE;
}

/// A log line, sent to clients that subscribed with
/// [`ClientMessage::SubscribeLogs`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SubscribeLogsResponse<'a> {
    /// The line's [`log_level`].
    pub level: u32,
    /// The line, as ESPHome formats it (`[I][tag]: message`). Clients
    /// decode it as UTF-8.
    pub message: &'a [u8],
}

impl Encode for SubscribeLogsResponse<'_> {
    fn encode<S: Sink>(&self, e: &mut Encoder<S>) -> Result<(), EncodeError> {
        e.uint32(1, self.level)?;
        e.bytes(3, self.message)
    }
}

impl Message for SubscribeLogsResponse<'_> {
    const TYPE: u16 = message_type::SUBSCRIBE_LOGS_RESPONSE;
}

/// Declares a message without fields.
macro_rules! empty_message {
    ($(#[$doc:meta])* $name:ident = $type:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
        pub struct $name;

        impl Encode for $name {
            fn encode<S: Sink>(&self, _: &mut Encoder<S>) -> Result<(), EncodeError> {
                Ok(())
            }
        }

        impl Message for $name {
            const TYPE: u16 = $type;
        }
    };
}

empty_message!(
    /// Asks the client to close the connection.
    DisconnectRequest = message_type::DISCONNECT_REQUEST
);
empty_message!(
    /// Acknowledges [`ClientMessage::Disconnect`].
    DisconnectResponse = message_type::DISCONNECT_RESPONSE
);
empty_message!(
    /// Server-initiated keepalive.
    PingRequest = message_type::PING_REQUEST
);
empty_message!(
    /// Answers [`ClientMessage::Ping`].
    PingResponse = message_type::PING_RESPONSE
);
empty_message!(
    /// Ends the (empty, for a proxy) entity list.
    ListEntitiesDoneResponse = message_type::LIST_ENTITIES_DONE_RESPONSE
);

/// Reports free active connection slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BluetoothConnectionsFreeResponse<'a> {
    /// Free connection slots.
    pub free: u32,
    /// Total connection slots.
    pub limit: u32,
    /// Addresses of the devices holding the other slots.
    pub allocated: &'a [u64],
}

impl Encode for BluetoothConnectionsFreeResponse<'_> {
    fn encode<S: Sink>(&self, e: &mut Encoder<S>) -> Result<(), EncodeError> {
        e.uint32(1, self.free)?;
        e.uint32(2, self.limit)?;
        e.packed_uint64(3, self.allocated)
    }
}

impl Message for BluetoothConnectionsFreeResponse<'_> {
    const TYPE: u16 = message_type::BLUETOOTH_CONNECTIONS_FREE_RESPONSE;
}

/// One advertisement in a [`BluetoothLeRawAdvertisementsResponse`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawAdvertisement<'a> {
    /// Device address as a 48-bit integer.
    pub address: u64,
    /// Received signal strength in dBm.
    pub rssi: i32,
    /// Address type (0 public, 1 random).
    pub address_type: u32,
    /// Advertising data followed by scan response data, at most 62 bytes.
    pub data: &'a [u8],
}

impl Encode for RawAdvertisement<'_> {
    fn encode<S: Sink>(&self, e: &mut Encoder<S>) -> Result<(), EncodeError> {
        e.uint64(1, self.address)?;
        e.sint32(2, self.rssi)?;
        e.uint32(3, self.address_type)?;
        e.bytes(4, self.data)
    }
}

/// A batch of forwarded advertisements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BluetoothLeRawAdvertisementsResponse<'a> {
    /// The advertisements, oldest first.
    pub advertisements: &'a [RawAdvertisement<'a>],
}

impl Encode for BluetoothLeRawAdvertisementsResponse<'_> {
    fn encode<S: Sink>(&self, e: &mut Encoder<S>) -> Result<(), EncodeError> {
        for advertisement in self.advertisements {
            e.message(1, advertisement)?;
        }
        Ok(())
    }
}

impl Message for BluetoothLeRawAdvertisementsResponse<'_> {
    const TYPE: u16 = message_type::BLUETOOTH_LE_RAW_ADVERTISEMENTS_RESPONSE;
}

/// Reports the scanner's state and mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BluetoothScannerStateResponse {
    /// One of the [`bluetooth_scanner_state`] values.
    pub state: u32,
    /// Current mode, one of the [`bluetooth_scanner_mode`] values.
    pub mode: u32,
    /// Mode the proxy was configured with, one of the
    /// [`bluetooth_scanner_mode`] values.
    pub configured_mode: u32,
}

impl Encode for BluetoothScannerStateResponse {
    fn encode<S: Sink>(&self, e: &mut Encoder<S>) -> Result<(), EncodeError> {
        e.uint32(1, self.state)?;
        e.uint32(2, self.mode)?;
        e.uint32(3, self.configured_mode)
    }
}

impl Message for BluetoothScannerStateResponse {
    const TYPE: u16 = message_type::BLUETOOTH_SCANNER_STATE_RESPONSE;
}

/// Reports a device's connection state: answers connection and
/// disconnection requests, and announces disconnections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BluetoothDeviceConnectionResponse {
    /// Device address as a 48-bit integer.
    pub address: u64,
    /// Whether the device is connected (with its services resolved).
    pub connected: bool,
    /// ATT MTU of the connection.
    pub mtu: u32,
    /// Why connecting failed or the connection ended, one of the
    /// [`bluetooth_error`] values, or 0.
    pub error: i32,
}

impl Encode for BluetoothDeviceConnectionResponse {
    fn encode<S: Sink>(&self, e: &mut Encoder<S>) -> Result<(), EncodeError> {
        e.uint64(1, self.address)?;
        e.bool(2, self.connected)?;
        e.uint32(3, self.mtu)?;
        e.int32(4, self.error)
    }
}

impl Message for BluetoothDeviceConnectionResponse {
    const TYPE: u16 = message_type::BLUETOOTH_DEVICE_CONNECTION_RESPONSE;
}

/// Writes a 128-bit UUID as the pair of `uint64` ESPHome uses: the high
/// half, then the low half.
#[allow(
    clippy::cast_possible_truncation,
    reason = "splits the UUID into its halves"
)]
fn encode_uuid<S: Sink>(e: &mut Encoder<S>, field: u32, uuid: u128) -> Result<(), EncodeError> {
    e.packed_uint64(field, &[(uuid >> 64) as u64, uuid as u64])
}

/// A GATT descriptor in a [`BluetoothGattGetServicesResponse`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BluetoothGattDescriptor {
    /// Descriptor UUID.
    pub uuid: u128,
    /// Descriptor handle.
    pub handle: u32,
}

impl Encode for BluetoothGattDescriptor {
    fn encode<S: Sink>(&self, e: &mut Encoder<S>) -> Result<(), EncodeError> {
        encode_uuid(e, 1, self.uuid)?;
        e.uint32(2, self.handle)
    }
}

/// A GATT characteristic in a [`BluetoothGattGetServicesResponse`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BluetoothGattCharacteristic<'a> {
    /// Characteristic UUID.
    pub uuid: u128,
    /// Handle of the characteristic's value.
    pub handle: u32,
    /// Characteristic properties (read, write, notify...) as the GATT
    /// bit mask.
    pub properties: u32,
    /// The characteristic's descriptors.
    pub descriptors: &'a [BluetoothGattDescriptor],
}

impl Encode for BluetoothGattCharacteristic<'_> {
    fn encode<S: Sink>(&self, e: &mut Encoder<S>) -> Result<(), EncodeError> {
        encode_uuid(e, 1, self.uuid)?;
        e.uint32(2, self.handle)?;
        e.uint32(3, self.properties)?;
        for descriptor in self.descriptors {
            e.message(4, descriptor)?;
        }
        Ok(())
    }
}

/// A GATT service in a [`BluetoothGattGetServicesResponse`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BluetoothGattService<'a> {
    /// Service UUID.
    pub uuid: u128,
    /// Handle of the service declaration.
    pub handle: u32,
    /// The service's characteristics.
    pub characteristics: &'a [BluetoothGattCharacteristic<'a>],
}

impl Encode for BluetoothGattService<'_> {
    fn encode<S: Sink>(&self, e: &mut Encoder<S>) -> Result<(), EncodeError> {
        encode_uuid(e, 1, self.uuid)?;
        e.uint32(2, self.handle)?;
        for characteristic in self.characteristics {
            e.message(3, characteristic)?;
        }
        Ok(())
    }
}

/// Some of a device's GATT services; answers
/// [`ClientMessage::BluetoothGattGetServices`] together with more of these
/// and a final [`BluetoothGattGetServicesDoneResponse`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BluetoothGattGetServicesResponse<'a> {
    /// Device address as a 48-bit integer.
    pub address: u64,
    /// The services.
    pub services: &'a [BluetoothGattService<'a>],
}

impl Encode for BluetoothGattGetServicesResponse<'_> {
    fn encode<S: Sink>(&self, e: &mut Encoder<S>) -> Result<(), EncodeError> {
        e.uint64(1, self.address)?;
        for service in self.services {
            e.message(2, service)?;
        }
        Ok(())
    }
}

impl Message for BluetoothGattGetServicesResponse<'_> {
    const TYPE: u16 = message_type::BLUETOOTH_GATT_GET_SERVICES_RESPONSE;
}

/// Ends the answer to [`ClientMessage::BluetoothGattGetServices`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BluetoothGattGetServicesDoneResponse {
    /// Device address as a 48-bit integer.
    pub address: u64,
}

impl Encode for BluetoothGattGetServicesDoneResponse {
    fn encode<S: Sink>(&self, e: &mut Encoder<S>) -> Result<(), EncodeError> {
        e.uint64(1, self.address)
    }
}

impl Message for BluetoothGattGetServicesDoneResponse {
    const TYPE: u16 = message_type::BLUETOOTH_GATT_GET_SERVICES_DONE_RESPONSE;
}

/// Declares a message that carries an attribute's value.
macro_rules! gatt_data_message {
    ($(#[$doc:meta])* $name:ident = $type:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
        pub struct $name<'a> {
            /// Device address as a 48-bit integer.
            pub address: u64,
            /// Attribute handle.
            pub handle: u32,
            /// The value.
            pub data: &'a [u8],
        }

        impl Encode for $name<'_> {
            fn encode<S: Sink>(&self, e: &mut Encoder<S>) -> Result<(), EncodeError> {
                e.uint64(1, self.address)?;
                e.uint32(2, self.handle)?;
                e.bytes(3, self.data)
            }
        }

        impl Message for $name<'_> {
            const TYPE: u16 = $type;
        }
    };
}

gatt_data_message!(
    /// Answers [`ClientMessage::BluetoothGattRead`] and
    /// [`ClientMessage::BluetoothGattReadDescriptor`].
    BluetoothGattReadResponse = message_type::BLUETOOTH_GATT_READ_RESPONSE
);
gatt_data_message!(
    /// Forwards a notification or indication.
    BluetoothGattNotifyDataResponse = message_type::BLUETOOTH_GATT_NOTIFY_DATA_RESPONSE
);

/// Declares a message that acknowledges an operation on an attribute.
macro_rules! gatt_ack_message {
    ($(#[$doc:meta])* $name:ident = $type:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
        pub struct $name {
            /// Device address as a 48-bit integer.
            pub address: u64,
            /// Attribute handle.
            pub handle: u32,
        }

        impl Encode for $name {
            fn encode<S: Sink>(&self, e: &mut Encoder<S>) -> Result<(), EncodeError> {
                e.uint64(1, self.address)?;
                e.uint32(2, self.handle)
            }
        }

        impl Message for $name {
            const TYPE: u16 = $type;
        }
    };
}

gatt_ack_message!(
    /// Confirms a [`ClientMessage::BluetoothGattWrite`] that asked for a
    /// response, or a [`ClientMessage::BluetoothGattWriteDescriptor`].
    BluetoothGattWriteResponse = message_type::BLUETOOTH_GATT_WRITE_RESPONSE
);
gatt_ack_message!(
    /// Confirms a [`ClientMessage::BluetoothGattNotify`].
    BluetoothGattNotifyResponse = message_type::BLUETOOTH_GATT_NOTIFY_RESPONSE
);

/// Reports a failed GATT operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BluetoothGattErrorResponse {
    /// Device address as a 48-bit integer.
    pub address: u64,
    /// Handle of the attribute the operation was for.
    pub handle: u32,
    /// One of the [`bluetooth_error`] values.
    pub error: i32,
}

impl Encode for BluetoothGattErrorResponse {
    fn encode<S: Sink>(&self, e: &mut Encoder<S>) -> Result<(), EncodeError> {
        e.uint64(1, self.address)?;
        e.uint32(2, self.handle)?;
        e.int32(3, self.error)
    }
}

impl Message for BluetoothGattErrorResponse {
    const TYPE: u16 = message_type::BLUETOOTH_GATT_ERROR_RESPONSE;
}

#[cfg(test)]
mod tests {
    //! Expected bytes were produced by the protobuf classes generated from
    //! ESPHome's `api.proto` (aioesphomeapi 46.28.0).

    use super::*;

    fn encode<M: Message>(message: &M) -> ([u8; 256], usize) {
        let mut buf = [0; 256];
        let len = message.encode_to_slice(&mut buf).unwrap();
        (buf, len)
    }

    fn hex(s: &str) -> ([u8; 256], usize) {
        let mut buf = [0; 256];
        let digits = s.as_bytes();
        for (i, pair) in digits.chunks(2).enumerate() {
            let text = core::str::from_utf8(pair).unwrap();
            buf[i] = u8::from_str_radix(text, 16).unwrap();
        }
        (buf, digits.len() / 2)
    }

    fn assert_encodes<M: Message>(message: &M, expected_hex: &str) {
        let (actual, actual_len) = encode(message);
        let (expected, expected_len) = hex(expected_hex);
        assert_eq!(&actual[..actual_len], &expected[..expected_len]);
    }

    #[test]
    fn decodes_hello_request() {
        let (payload, len) = hex("0a0d61696f657370686f6d6561706910011813");
        let message = ClientMessage::decode(1, &payload[..len]).unwrap();
        assert_eq!(
            message,
            ClientMessage::Hello(HelloRequest {
                client_info: "aioesphomeapi",
                api_version_major: 1,
                api_version_minor: 19,
            })
        );
    }

    #[test]
    fn encodes_hello_response() {
        let response = HelloResponse {
            api_version_major: 1,
            api_version_minor: 19,
            server_info: "bleproxy 0.1.0",
            name: "bleproxy-a1b2c3",
        };
        assert_encodes(
            &response,
            "080110131a0e626c6570726f787920302e312e30220f626c6570726f78792d613162326333",
        );
    }

    #[test]
    fn encodes_device_info_response() {
        let response = DeviceInfoResponse {
            name: "bleproxy-a1b2c3",
            mac_address: "AA:BB:CC:A1:B2:C3",
            esphome_version: "0.1.0",
            model: "Linux",
            project_name: "fiam.bleproxy",
            project_version: "0.1.0",
            bluetooth_proxy_feature_flags: bluetooth_proxy_feature::PASSIVE_SCAN
                | bluetooth_proxy_feature::RAW_ADVERTISEMENTS,
            manufacturer: "bleproxy",
            friendly_name: "Bluetooth Proxy",
            bluetooth_mac_address: "AA:BB:CC:A1:B2:C3",
            api_encryption_supported: false,
        };
        assert_encodes(
            &response,
            concat!(
                "120f626c6570726f78792d6131623263331a1141413a42423a43433a41313a42",
                "323a43332205302e312e3032054c696e7578420d6669616d2e626c6570726f78",
                "794a05302e312e306208626c6570726f78796a0f426c7565746f6f7468205072",
                "6f7879782192011141413a42423a43433a41313a42323a4333",
            ),
        );
    }

    #[test]
    fn encodes_encryption_support() {
        let response = DeviceInfoResponse {
            name: "x",
            bluetooth_mac_address: "y",
            api_encryption_supported: true,
            ..DeviceInfoResponse::default()
        };
        assert_encodes(&response, "12017892010179980101");
    }

    #[test]
    fn encodes_raw_advertisements() {
        let advertisements = [
            RawAdvertisement {
                address: 0xA4C1_3800_1122,
                rssi: -70,
                address_type: 1,
                data: &[0x02, 0x01, 0x06, 0x03, 0x03, 0xAA, 0xFE],
            },
            // Default values are omitted, leaving only the address.
            RawAdvertisement {
                address: 1,
                rssi: 0,
                address_type: 0,
                data: &[],
            },
        ];
        let response = BluetoothLeRawAdvertisementsResponse {
            advertisements: &advertisements,
        };
        assert_encodes(
            &response,
            "0a1608a2a280c0939829108b01180122070201060303aafe0a020801",
        );
    }

    #[test]
    fn encodes_empty_messages() {
        assert_eq!(encode(&BluetoothConnectionsFreeResponse::default()).1, 0);
        assert_eq!(encode(&PingResponse).1, 0);
        assert_eq!(ListEntitiesDoneResponse::TYPE, 19);
    }

    #[test]
    fn decodes_bluetooth_subscription() {
        assert_eq!(
            ClientMessage::decode(66, &[0x08, 0x01]),
            Ok(ClientMessage::SubscribeBluetoothLeAdvertisements {
                flags: bluetooth_proxy_subscription::RAW_ADVERTISEMENTS
            })
        );
        assert_eq!(
            ClientMessage::decode(66, &[]),
            Ok(ClientMessage::SubscribeBluetoothLeAdvertisements { flags: 0 })
        );
    }

    #[test]
    fn encodes_entity_listings() {
        let advertisements = ListEntitiesSensorResponse {
            object_id: "advertisements",
            key: 2,
            name: "Advertisements",
            icon: "mdi:bluetooth-searching",
            unit_of_measurement: "adv",
            accuracy_decimals: 0,
            state_class: sensor_state_class::TOTAL_INCREASING,
            entity_category: entity_category::DIAGNOSTIC,
        };
        assert_encodes(
            &advertisements,
            "0a0e6164766572746973656d656e747315020000001a0e4164766572746973656d656e74732a176d64693a\
             626c7565746f6f74682d736561726368696e67320361647650026802",
        );
        let clients = ListEntitiesSensorResponse {
            object_id: "connected_clients",
            key: 3,
            name: "Connected Clients",
            icon: "mdi:account-network",
            state_class: sensor_state_class::MEASUREMENT,
            entity_category: entity_category::DIAGNOSTIC,
            ..ListEntitiesSensorResponse::default()
        };
        assert_encodes(
            &clients,
            "0a11636f6e6e65637465645f636c69656e747315030000001a11436f6e6e656374656420436c69656e7473\
             2a136d64693a6163636f756e742d6e6574776f726b50016802",
        );
        let uptime = ListEntitiesTextSensorResponse {
            object_id: "uptime",
            key: 1,
            name: "Uptime",
            icon: "mdi:clock-outline",
            entity_category: entity_category::DIAGNOSTIC,
        };
        assert_encodes(
            &uptime,
            "0a06757074696d6515010000001a06557074696d652a116d64693a636c6f636b2d6f75746c696e653802",
        );
        assert_eq!(ListEntitiesSensorResponse::TYPE, 16);
        assert_eq!(ListEntitiesTextSensorResponse::TYPE, 18);
    }

    #[test]
    fn encodes_switches() {
        let scanning = ListEntitiesSwitchResponse {
            object_id: "ble_scanning",
            key: 4,
            name: "BLE Scanning",
            icon: "mdi:bluetooth-searching",
            entity_category: entity_category::CONFIG,
        };
        assert_encodes(
            &scanning,
            "0a0c626c655f7363616e6e696e6715040000001a0c424c45205363616e6e696e672a176d64693a626c75\
             65746f6f74682d736561726368696e674001",
        );
        let on = SwitchStateResponse {
            key: 4,
            state: true,
        };
        assert_encodes(&on, "0d040000001001");
        let off = SwitchStateResponse {
            key: 4,
            state: false,
        };
        assert_encodes(&off, "0d04000000");
        assert_eq!(ListEntitiesSwitchResponse::TYPE, 17);
        assert_eq!(SwitchStateResponse::TYPE, 26);
    }

    #[test]
    fn decodes_switch_commands() {
        let cases = [
            ("0d040000001001", 4, true),
            ("0d04000000", 4, false),
            // Sub-device IDs are ignored.
            ("0defbeadde10011807", 0xDEAD_BEEF, true),
        ];
        for (payload, key, state) in cases {
            let (payload, len) = hex(payload);
            assert_eq!(
                ClientMessage::decode(33, &payload[..len]),
                Ok(ClientMessage::SwitchCommand { key, state })
            );
        }
        // The key is a fixed32; a varint there is malformed.
        let (payload, len) = hex("0804");
        assert!(ClientMessage::decode(33, &payload[..len]).is_err());
    }

    #[test]
    fn encodes_entity_states() {
        let count = SensorStateResponse {
            key: 2,
            state: 1234.0,
        };
        assert_encodes(&count, "0d020000001500409a44");
        // Zero is the default, so only the key is sent.
        let none = SensorStateResponse { key: 3, state: 0.0 };
        assert_encodes(&none, "0d03000000");
        let uptime = TextSensorStateResponse {
            key: 1,
            state: "1d 2h 3m 45s",
        };
        assert_encodes(&uptime, "0d01000000120c316420326820336d20343573");
        assert_eq!(SensorStateResponse::TYPE, 25);
        assert_eq!(TextSensorStateResponse::TYPE, 27);
    }

    #[test]
    fn decodes_log_subscriptions() {
        let cases = [
            ("08051001", log_level::DEBUG, true),
            ("0803", log_level::INFO, false),
        ];
        for (payload, level, dump_config) in cases {
            let (payload, len) = hex(payload);
            assert_eq!(
                ClientMessage::decode(28, &payload[..len]),
                Ok(ClientMessage::SubscribeLogs { level, dump_config })
            );
        }
    }

    #[test]
    fn encodes_log_lines() {
        let line = SubscribeLogsResponse {
            level: log_level::INFO,
            message: b"[I][bleproxy]: client connected",
        };
        assert_encodes(
            &line,
            "08031a1f5b495d5b626c6570726f78795d3a20636c69656e7420636f6e6e6563746564",
        );
        assert_eq!(SubscribeLogsResponse::TYPE, 29);
    }

    #[test]
    fn encodes_scanner_state() {
        let running = BluetoothScannerStateResponse {
            state: bluetooth_scanner_state::RUNNING,
            mode: bluetooth_scanner_mode::ACTIVE,
            configured_mode: bluetooth_scanner_mode::PASSIVE,
        };
        assert_encodes(&running, "08021001");
        let stopped = BluetoothScannerStateResponse {
            state: bluetooth_scanner_state::STOPPED,
            mode: bluetooth_scanner_mode::PASSIVE,
            configured_mode: bluetooth_scanner_mode::ACTIVE,
        };
        assert_encodes(&stopped, "08051801");
    }

    #[test]
    fn decodes_scanner_mode_requests() {
        assert_eq!(
            ClientMessage::decode(127, &[0x08, 0x01]),
            Ok(ClientMessage::BluetoothScannerSetMode {
                mode: bluetooth_scanner_mode::ACTIVE
            })
        );
        // Passive is the default value, so it is sent as an empty message.
        assert_eq!(
            ClientMessage::decode(127, &[]),
            Ok(ClientMessage::BluetoothScannerSetMode {
                mode: bluetooth_scanner_mode::PASSIVE
            })
        );
    }

    #[test]
    fn decodes_fieldless_requests() {
        let cases = [
            (3, ClientMessage::Authentication),
            (5, ClientMessage::Disconnect),
            (7, ClientMessage::Ping),
            (9, ClientMessage::DeviceInfo),
            (11, ClientMessage::ListEntities),
            (20, ClientMessage::SubscribeStates),
            (
                28,
                ClientMessage::SubscribeLogs {
                    level: log_level::NONE,
                    dump_config: false,
                },
            ),
            (34, ClientMessage::SubscribeHomeassistantServices),
            (38, ClientMessage::SubscribeHomeAssistantStates),
            (80, ClientMessage::SubscribeBluetoothConnectionsFree),
            (87, ClientMessage::UnsubscribeBluetoothLeAdvertisements),
            // Server messages among the GATT requests.
            (71, ClientMessage::Unknown(71)),
            (150, ClientMessage::Unknown(150)),
        ];
        for (message_type, expected) in cases {
            assert_eq!(ClientMessage::decode(message_type, &[]), Ok(expected));
        }
    }

    const ADDRESS: u64 = 0xA4C1_3800_1122;

    fn assert_decodes(message_type: u16, payload_hex: &str, expected: ClientMessage<'_>) {
        let (payload, len) = hex(payload_hex);
        assert_eq!(
            ClientMessage::decode(message_type, &payload[..len]),
            Ok(expected),
            "message type {message_type}"
        );
    }

    #[test]
    fn decodes_device_requests() {
        assert_decodes(
            68,
            "08a2a280c0939829100418012001",
            ClientMessage::BluetoothDeviceRequest(BluetoothDeviceRequest {
                address: ADDRESS,
                request_type: bluetooth_device_request_type::CONNECT_V3_WITH_CACHE,
                address_type: Some(1),
            }),
        );
        // Disconnect requests don't carry an address type.
        assert_decodes(
            68,
            "08a2a280c09398291001",
            ClientMessage::BluetoothDeviceRequest(BluetoothDeviceRequest {
                address: ADDRESS,
                request_type: bluetooth_device_request_type::DISCONNECT,
                address_type: None,
            }),
        );
    }

    #[test]
    fn decodes_gatt_requests() {
        let cases = [
            (
                70,
                "08a2a280c0939829",
                ClientMessage::BluetoothGattGetServices { address: ADDRESS },
            ),
            (
                73,
                "08a2a280c0939829102a",
                ClientMessage::BluetoothGattRead {
                    address: ADDRESS,
                    handle: 0x2A,
                },
            ),
            (
                75,
                "08a2a280c0939829102a180122020102",
                ClientMessage::BluetoothGattWrite {
                    address: ADDRESS,
                    handle: 0x2A,
                    response: true,
                    data: &[0x01, 0x02],
                },
            ),
            (
                76,
                "08a2a280c0939829102c",
                ClientMessage::BluetoothGattReadDescriptor {
                    address: ADDRESS,
                    handle: 0x2C,
                },
            ),
            (
                77,
                "08a2a280c0939829102c1a020100",
                ClientMessage::BluetoothGattWriteDescriptor {
                    address: ADDRESS,
                    handle: 0x2C,
                    data: &[0x01, 0x00],
                },
            ),
            (
                78,
                "08a2a280c0939829102a1801",
                ClientMessage::BluetoothGattNotify {
                    address: ADDRESS,
                    handle: 0x2A,
                    enable: true,
                },
            ),
        ];
        for (message_type, payload, expected) in cases {
            assert_decodes(message_type, payload, expected);
        }
    }

    #[test]
    fn encodes_connection_responses() {
        let connected = BluetoothDeviceConnectionResponse {
            address: ADDRESS,
            connected: true,
            mtu: 247,
            error: 0,
        };
        assert_encodes(&connected, "08a2a280c0939829100118f701");
        let not_connected = BluetoothDeviceConnectionResponse {
            address: ADDRESS,
            error: bluetooth_error::NOT_CONNECTED,
            ..BluetoothDeviceConnectionResponse::default()
        };
        assert_encodes(&not_connected, "08a2a280c093982920ffffffffffffffffff01");
        let failed = BluetoothDeviceConnectionResponse {
            address: ADDRESS,
            error: bluetooth_error::CONNECTION_FAILED,
            ..BluetoothDeviceConnectionResponse::default()
        };
        assert_encodes(&failed, "08a2a280c0939829203e");
    }

    #[test]
    fn encodes_services() {
        let descriptors = [BluetoothGattDescriptor {
            uuid: 0x0000_2902_0000_1000_8000_0080_5F9B_34FB,
            handle: 0x2B,
        }];
        let characteristics = [BluetoothGattCharacteristic {
            uuid: 0x0000_2A19_0000_1000_8000_0080_5F9B_34FB,
            handle: 0x2A,
            properties: 0x12,
            descriptors: &descriptors,
        }];
        let services = [BluetoothGattService {
            uuid: 0x0000_180F_0000_1000_8000_0080_5F9B_34FB,
            handle: 0x28,
            characteristics: &characteristics,
        }];
        let response = BluetoothGattGetServicesResponse {
            address: ADDRESS,
            services: &services,
        };
        assert_encodes(
            &response,
            concat!(
                "08a2a280c093982912450a1180a08080f08106fbe9ecfc85908080800110281a",
                "2e0a1180a0808090c30afbe9ecfc859080808001102a181222150a1180a08080",
                "a0a00afbe9ecfc859080808001102b",
            ),
        );
        let done = BluetoothGattGetServicesDoneResponse { address: ADDRESS };
        assert_encodes(&done, "08a2a280c0939829");
    }

    #[test]
    fn encodes_gatt_responses() {
        let read = BluetoothGattReadResponse {
            address: ADDRESS,
            handle: 0x2A,
            data: &[0x64],
        };
        assert_encodes(&read, "08a2a280c0939829102a1a0164");
        let notification = BluetoothGattNotifyDataResponse {
            address: ADDRESS,
            handle: 0x2A,
            data: &[0x63],
        };
        assert_encodes(&notification, "08a2a280c0939829102a1a0163");
        let written = BluetoothGattWriteResponse {
            address: ADDRESS,
            handle: 0x2A,
        };
        assert_encodes(&written, "08a2a280c0939829102a");
        let notifying = BluetoothGattNotifyResponse {
            address: ADDRESS,
            handle: 0x2A,
        };
        assert_encodes(&notifying, "08a2a280c0939829102a");
        let error = BluetoothGattErrorResponse {
            address: ADDRESS,
            handle: 0x2A,
            error: bluetooth_error::NOT_CONNECTED,
        };
        assert_encodes(&error, "08a2a280c0939829102a18ffffffffffffffffff01");
        let error = BluetoothGattErrorResponse {
            error: bluetooth_error::INSUFFICIENT_AUTHENTICATION,
            ..error
        };
        assert_encodes(&error, "08a2a280c0939829102a1805");
    }

    #[test]
    fn encodes_free_connection_slots() {
        let free = BluetoothConnectionsFreeResponse {
            free: 2,
            limit: 3,
            allocated: &[ADDRESS],
        };
        assert_encodes(&free, "080210031a07a2a280c0939829");
    }

    #[test]
    fn skips_unknown_fields() {
        // Hello with an extra field 4 (outgoing_connection_target = true).
        let (payload, len) = hex("0a0178100118132001");
        let message = ClientMessage::decode(1, &payload[..len]).unwrap();
        assert_eq!(
            message,
            ClientMessage::Hello(HelloRequest {
                client_info: "x",
                api_version_major: 1,
                api_version_minor: 19,
            })
        );
    }

    #[test]
    fn rejects_malformed_payloads() {
        assert_eq!(
            ClientMessage::decode(1, &[0x0A, 0x05, b'x']),
            Err(DecodeError::Truncated)
        );
        assert_eq!(
            ClientMessage::decode(66, &[0x0A, 0x00]),
            Err(DecodeError::UnexpectedWireType(1))
        );
    }
}
