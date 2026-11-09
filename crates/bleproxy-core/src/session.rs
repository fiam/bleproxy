//! The server side of an ESPHome API session, independent of any I/O.
//!
//! A platform feeds each decoded [`ClientMessage`] to [`Session::handle`],
//! sends back the returned [`Reply`], and closes the connection when told
//! to. Forwarding advertisements is left to the platform, which asks
//! [`Session::wants_raw_advertisements`] whether the client subscribed.

use bleproxy_protocol::wire::EncodeError;
use bleproxy_protocol::{
    API_VERSION, BluetoothConnectionsFreeResponse, ClientMessage, DeviceInfoResponse,
    DisconnectResponse, HelloResponse, ListEntitiesDoneResponse, PingResponse, RawAdvertisement,
    bluetooth_proxy_feature, bluetooth_proxy_subscription, encode_frame,
};

use crate::Advertisement;

/// Bluetooth proxy features this implementation offers.
pub const FEATURE_FLAGS: u32 =
    bluetooth_proxy_feature::PASSIVE_SCAN | bluetooth_proxy_feature::RAW_ADVERTISEMENTS;

/// How the proxy describes itself to clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeviceIdentity<'a> {
    /// Device name; must match the advertised mDNS host name.
    pub name: &'a str,
    /// Human-friendly name.
    pub friendly_name: &'a str,
    /// Stable MAC address (`AA:BB:CC:DD:EE:FF`) identifying the device.
    pub mac_address: &'a str,
    /// MAC address of the Bluetooth adapter (`AA:BB:CC:DD:EE:FF`).
    pub bluetooth_mac_address: &'a str,
    /// Hardware or platform model.
    pub model: &'a str,
    /// Manufacturer name.
    pub manufacturer: &'a str,
    /// Project name in `author.project` form.
    pub project_name: &'a str,
    /// Software version.
    pub version: &'a str,
    /// Free-form server description sent in the hello response.
    pub server_info: &'a str,
}

/// A message to send back to the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply<'a> {
    /// Answers the client's hello.
    Hello(HelloResponse<'a>),
    /// Describes the device.
    DeviceInfo(DeviceInfoResponse<'a>),
    /// Answers a ping.
    Ping(PingResponse),
    /// Acknowledges a disconnect request.
    Disconnect(DisconnectResponse),
    /// Ends the (empty) entity list.
    ListEntitiesDone(ListEntitiesDoneResponse),
    /// Reports that no active connections are available.
    ConnectionsFree(BluetoothConnectionsFreeResponse),
}

impl Reply<'_> {
    /// Encodes the reply as a complete frame at the start of `buf`.
    ///
    /// # Errors
    ///
    /// Returns [`EncodeError::BufferTooSmall`] if `buf` cannot hold it.
    pub fn encode_frame(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        match self {
            Self::Hello(message) => encode_frame(message, buf),
            Self::DeviceInfo(message) => encode_frame(message, buf),
            Self::Ping(message) => encode_frame(message, buf),
            Self::Disconnect(message) => encode_frame(message, buf),
            Self::ListEntitiesDone(message) => encode_frame(message, buf),
            Self::ConnectionsFree(message) => encode_frame(message, buf),
        }
    }
}

/// What to do after handling a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcome<'a> {
    /// Message to send back, if any.
    pub reply: Option<Reply<'a>>,
    /// Whether to close the connection (after sending `reply`).
    pub close: bool,
}

impl<'a> Outcome<'a> {
    const NOTHING: Self = Self {
        reply: None,
        close: false,
    };
    const CLOSE: Self = Self {
        reply: None,
        close: true,
    };

    const fn reply(reply: Reply<'a>) -> Self {
        Self {
            reply: Some(reply),
            close: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    AwaitingHello,
    Established,
}

/// State of one client connection.
#[derive(Debug, Clone)]
pub struct Session<'a> {
    identity: &'a DeviceIdentity<'a>,
    phase: Phase,
    raw_advertisements: bool,
}

impl<'a> Session<'a> {
    /// Starts a session for a newly accepted connection.
    #[must_use]
    pub const fn new(identity: &'a DeviceIdentity<'a>) -> Self {
        Self {
            identity,
            phase: Phase::AwaitingHello,
            raw_advertisements: false,
        }
    }

    /// Returns whether the client wants raw advertisements forwarded.
    #[must_use]
    pub const fn wants_raw_advertisements(&self) -> bool {
        self.raw_advertisements
    }

    /// Handles one message from the client.
    pub fn handle(&mut self, message: &ClientMessage<'_>) -> Outcome<'a> {
        let identity = self.identity;
        if self.phase == Phase::AwaitingHello {
            // A session must start with a hello; anything else is a client
            // that doesn't speak the protocol.
            return if matches!(message, ClientMessage::Hello(_)) {
                self.phase = Phase::Established;
                Outcome::reply(Reply::Hello(hello_response(identity)))
            } else {
                Outcome::CLOSE
            };
        }
        match message {
            ClientMessage::Hello(_) => Outcome::reply(Reply::Hello(hello_response(identity))),
            ClientMessage::Ping => Outcome::reply(Reply::Ping(PingResponse)),
            ClientMessage::Disconnect => Outcome {
                reply: Some(Reply::Disconnect(DisconnectResponse)),
                close: true,
            },
            ClientMessage::DisconnectResponse => Outcome::CLOSE,
            ClientMessage::DeviceInfo => Outcome::reply(Reply::DeviceInfo(DeviceInfoResponse {
                name: identity.name,
                mac_address: identity.mac_address,
                esphome_version: identity.version,
                model: identity.model,
                project_name: identity.project_name,
                project_version: identity.version,
                bluetooth_proxy_feature_flags: FEATURE_FLAGS,
                manufacturer: identity.manufacturer,
                friendly_name: identity.friendly_name,
                bluetooth_mac_address: identity.bluetooth_mac_address,
            })),
            ClientMessage::ListEntities => {
                Outcome::reply(Reply::ListEntitiesDone(ListEntitiesDoneResponse))
            }
            ClientMessage::SubscribeBluetoothLeAdvertisements { flags } => {
                // Only the raw format is implemented; it is the one clients
                // request when RAW_ADVERTISEMENTS is advertised.
                self.raw_advertisements =
                    flags & bluetooth_proxy_subscription::RAW_ADVERTISEMENTS != 0;
                Outcome::NOTHING
            }
            ClientMessage::UnsubscribeBluetoothLeAdvertisements => {
                self.raw_advertisements = false;
                Outcome::NOTHING
            }
            ClientMessage::SubscribeBluetoothConnectionsFree => Outcome::reply(
                Reply::ConnectionsFree(BluetoothConnectionsFreeResponse::default()),
            ),
            // No password, no entities, no logs or services to stream.
            _ => Outcome::NOTHING,
        }
    }
}

const fn hello_response<'a>(identity: &DeviceIdentity<'a>) -> HelloResponse<'a> {
    HelloResponse {
        api_version_major: API_VERSION.0,
        api_version_minor: API_VERSION.1,
        server_info: identity.server_info,
        name: identity.name,
    }
}

impl Advertisement {
    /// Returns the advertisement in the form the ESPHome API forwards.
    #[must_use]
    pub fn as_raw(&self) -> RawAdvertisement<'_> {
        RawAdvertisement {
            address: self.address.to_u64(),
            rssi: self.rssi.into(),
            address_type: u8::from(self.address_type).into(),
            data: self.data.as_slice(),
        }
    }
}

#[cfg(test)]
mod tests {
    use bleproxy_protocol::HelloRequest;

    use super::*;
    use crate::{AddressType, AdvData, BdAddr};

    const IDENTITY: DeviceIdentity<'static> = DeviceIdentity {
        name: "bleproxy-a1b2c3",
        friendly_name: "Kitchen proxy",
        mac_address: "AA:BB:CC:A1:B2:C3",
        bluetooth_mac_address: "AA:BB:CC:A1:B2:C3",
        model: "Linux",
        manufacturer: "bleproxy",
        project_name: "fiam.bleproxy",
        version: "0.1.0",
        server_info: "bleproxy 0.1.0",
    };

    fn established() -> Session<'static> {
        let mut session = Session::new(&IDENTITY);
        session.handle(&ClientMessage::Hello(HelloRequest::default()));
        session
    }

    #[test]
    fn answers_hello_with_identity() {
        let mut session = Session::new(&IDENTITY);
        let outcome = session.handle(&ClientMessage::Hello(HelloRequest::default()));
        assert_eq!(
            outcome.reply,
            Some(Reply::Hello(HelloResponse {
                api_version_major: 1,
                api_version_minor: 14,
                server_info: "bleproxy 0.1.0",
                name: "bleproxy-a1b2c3",
            }))
        );
        assert!(!outcome.close);
    }

    #[test]
    fn closes_when_the_first_message_is_not_hello() {
        for message in [
            ClientMessage::Ping,
            ClientMessage::DeviceInfo,
            ClientMessage::Unknown(150),
        ] {
            let outcome = Session::new(&IDENTITY).handle(&message);
            assert_eq!(outcome, Outcome::CLOSE, "{message:?}");
        }
    }

    #[test]
    fn describes_the_device_as_a_passive_proxy() {
        let outcome = established().handle(&ClientMessage::DeviceInfo);
        let Some(Reply::DeviceInfo(info)) = outcome.reply else {
            panic!("expected device info, got {outcome:?}");
        };
        assert_eq!(info.name, IDENTITY.name);
        assert_eq!(info.mac_address, IDENTITY.mac_address);
        assert_eq!(info.bluetooth_mac_address, IDENTITY.bluetooth_mac_address);
        assert_eq!(info.project_name, "fiam.bleproxy");
        assert_eq!(info.bluetooth_proxy_feature_flags, 0b10_0001);
    }

    #[test]
    fn tracks_the_advertisement_subscription() {
        let mut session = established();
        assert!(!session.wants_raw_advertisements());
        session.handle(&ClientMessage::SubscribeBluetoothLeAdvertisements { flags: 1 });
        assert!(session.wants_raw_advertisements());
        session.handle(&ClientMessage::UnsubscribeBluetoothLeAdvertisements);
        assert!(!session.wants_raw_advertisements());
        // The legacy (parsed) format is not offered.
        session.handle(&ClientMessage::SubscribeBluetoothLeAdvertisements { flags: 0 });
        assert!(!session.wants_raw_advertisements());
    }

    #[test]
    fn answers_routine_requests() {
        let mut session = established();
        let cases = [
            (ClientMessage::Ping, Some(Reply::Ping(PingResponse))),
            (
                ClientMessage::ListEntities,
                Some(Reply::ListEntitiesDone(ListEntitiesDoneResponse)),
            ),
            (
                ClientMessage::SubscribeBluetoothConnectionsFree,
                Some(Reply::ConnectionsFree(BluetoothConnectionsFreeResponse {
                    free: 0,
                    limit: 0,
                })),
            ),
            (ClientMessage::Authentication, None),
            (ClientMessage::SubscribeStates, None),
            (ClientMessage::SubscribeLogs, None),
            (ClientMessage::SubscribeHomeassistantServices, None),
            (ClientMessage::SubscribeHomeAssistantStates, None),
            (ClientMessage::PingResponse, None),
            (ClientMessage::Unknown(150), None),
        ];
        for (message, reply) in cases {
            let outcome = session.handle(&message);
            assert_eq!(outcome.reply, reply, "{message:?}");
            assert!(!outcome.close, "{message:?}");
        }
    }

    #[test]
    fn disconnects_on_request() {
        let outcome = established().handle(&ClientMessage::Disconnect);
        assert_eq!(outcome.reply, Some(Reply::Disconnect(DisconnectResponse)));
        assert!(outcome.close);
        assert_eq!(
            established().handle(&ClientMessage::DisconnectResponse),
            Outcome::CLOSE
        );
    }

    #[test]
    fn encodes_replies_as_frames() {
        let mut buf = [0; 64];
        let len = Reply::Ping(PingResponse).encode_frame(&mut buf).unwrap();
        assert_eq!(&buf[..len], [0x00, 0x00, 0x08]);
    }

    #[test]
    fn converts_advertisements() {
        let advertisement = Advertisement {
            address: BdAddr::new([0xA4, 0xC1, 0x38, 0x00, 0x11, 0x22]),
            address_type: AddressType::Random,
            rssi: -70,
            data: AdvData::new(&[0x02, 0x01, 0x06]).unwrap(),
        };
        assert_eq!(
            advertisement.as_raw(),
            RawAdvertisement {
                address: 0xA4C1_3800_1122,
                rssi: -70,
                address_type: 1,
                data: &[0x02, 0x01, 0x06],
            }
        );
    }
}
