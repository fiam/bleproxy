//! ESPHome Native API messages used by a passive Bluetooth proxy.
//!
//! Field numbers and message type identifiers follow ESPHome's `api.proto`.
//! Only the messages a passive proxy exchanges are modeled; any other
//! client message decodes as [`ClientMessage::Unknown`].

use crate::wire::{DecodeError, Encode, EncodeError, Encoder, Fields, Sink};

/// API version reported to clients.
///
/// 1.14 is the newest version for which clients still derive the device's
/// capabilities (Bluetooth proxy feature flags and MAC address) from
/// [`DeviceInfoResponse`]; from 1.15 on they expect a separate capabilities
/// request. Everything a passive proxy uses predates it.
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
    /// [`ListEntitiesDoneResponse`](super::ListEntitiesDoneResponse).
    pub const LIST_ENTITIES_DONE_RESPONSE: u16 = 19;
    /// [`ClientMessage::SubscribeStates`](super::ClientMessage::SubscribeStates).
    pub const SUBSCRIBE_STATES_REQUEST: u16 = 20;
    /// [`ClientMessage::SubscribeLogs`](super::ClientMessage::SubscribeLogs).
    pub const SUBSCRIBE_LOGS_REQUEST: u16 = 28;
    /// [`ClientMessage::SubscribeHomeassistantServices`](super::ClientMessage::SubscribeHomeassistantServices).
    pub const SUBSCRIBE_HOMEASSISTANT_SERVICES_REQUEST: u16 = 34;
    /// [`ClientMessage::SubscribeHomeAssistantStates`](super::ClientMessage::SubscribeHomeAssistantStates).
    pub const SUBSCRIBE_HOME_ASSISTANT_STATES_REQUEST: u16 = 38;
    /// [`ClientMessage::SubscribeBluetoothLeAdvertisements`](super::ClientMessage::SubscribeBluetoothLeAdvertisements).
    pub const SUBSCRIBE_BLUETOOTH_LE_ADVERTISEMENTS_REQUEST: u16 = 66;
    /// [`ClientMessage::SubscribeBluetoothConnectionsFree`](super::ClientMessage::SubscribeBluetoothConnectionsFree).
    pub const SUBSCRIBE_BLUETOOTH_CONNECTIONS_FREE_REQUEST: u16 = 80;
    /// [`BluetoothConnectionsFreeResponse`](super::BluetoothConnectionsFreeResponse).
    pub const BLUETOOTH_CONNECTIONS_FREE_RESPONSE: u16 = 81;
    /// [`ClientMessage::UnsubscribeBluetoothLeAdvertisements`](super::ClientMessage::UnsubscribeBluetoothLeAdvertisements).
    pub const UNSUBSCRIBE_BLUETOOTH_LE_ADVERTISEMENTS_REQUEST: u16 = 87;
    /// [`BluetoothLeRawAdvertisementsResponse`](super::BluetoothLeRawAdvertisementsResponse).
    pub const BLUETOOTH_LE_RAW_ADVERTISEMENTS_RESPONSE: u16 = 93;
}

/// Bluetooth proxy capabilities, reported in
/// [`DeviceInfoResponse::bluetooth_proxy_feature_flags`].
pub mod bluetooth_proxy_feature {
    /// Forwards advertisements received while scanning.
    pub const PASSIVE_SCAN: u32 = 1 << 0;
    /// Forwards advertisements in their raw form (see
    /// [`BluetoothLeRawAdvertisementsResponse`](super::BluetoothLeRawAdvertisementsResponse)).
    pub const RAW_ADVERTISEMENTS: u32 = 1 << 5;
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
    /// Subscribes to device logs.
    SubscribeLogs,
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
    /// A message type this crate does not model.
    Unknown(u16),
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
            t::SUBSCRIBE_LOGS_REQUEST => Self::SubscribeLogs,
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
            other => Self::Unknown(other),
        })
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
        e.string(18, self.bluetooth_mac_address)
    }
}

impl Message for DeviceInfoResponse<'_> {
    const TYPE: u16 = message_type::DEVICE_INFO_RESPONSE;
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
pub struct BluetoothConnectionsFreeResponse {
    /// Free connection slots.
    pub free: u32,
    /// Total connection slots.
    pub limit: u32,
}

impl Encode for BluetoothConnectionsFreeResponse {
    fn encode<S: Sink>(&self, e: &mut Encoder<S>) -> Result<(), EncodeError> {
        e.uint32(1, self.free)?;
        e.uint32(2, self.limit)
    }
}

impl Message for BluetoothConnectionsFreeResponse {
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
    fn decodes_fieldless_requests() {
        let cases = [
            (3, ClientMessage::Authentication),
            (5, ClientMessage::Disconnect),
            (7, ClientMessage::Ping),
            (9, ClientMessage::DeviceInfo),
            (11, ClientMessage::ListEntities),
            (20, ClientMessage::SubscribeStates),
            (28, ClientMessage::SubscribeLogs),
            (34, ClientMessage::SubscribeHomeassistantServices),
            (38, ClientMessage::SubscribeHomeAssistantStates),
            (80, ClientMessage::SubscribeBluetoothConnectionsFree),
            (87, ClientMessage::UnsubscribeBluetoothLeAdvertisements),
            (150, ClientMessage::Unknown(150)),
        ];
        for (message_type, expected) in cases {
            assert_eq!(ClientMessage::decode(message_type, &[]), Ok(expected));
        }
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
