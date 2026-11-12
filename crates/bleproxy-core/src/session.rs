//! The server side of an ESPHome API session, independent of any I/O.
//!
//! A platform feeds each decoded [`ClientMessage`] to [`Session::handle`],
//! sends back the returned [`Reply`], and closes the connection when told
//! to. Forwarding advertisements is left to the platform, which asks
//! [`Session::wants_raw_advertisements`] whether the client subscribed.
//! Likewise, scan mode changes are passed to the platform through
//! [`Outcome::scan_mode`], and the platform reports its scanner's state
//! with [`Session::scanner_state`]. Operations on device connections reach
//! the platform as [`Outcome::connection`]; it sends their results, and
//! the free connection slots once [`Session::wants_connections_free`],
//! itself. The platform's own entities, such as diagnostic sensors, are
//! listed when [`Outcome::list_entities`] says so, and their states sent
//! once [`Session::wants_states`]. Commands for them arrive as
//! [`Outcome::switch`].

use bleproxy_protocol::Message;
use bleproxy_protocol::wire::{Encode, EncodeError};
use bleproxy_protocol::{
    API_VERSION, BluetoothConnectionsFreeResponse, BluetoothScannerStateResponse, ClientMessage,
    DeviceInfoResponse, DisconnectResponse, HelloResponse, ListEntitiesDoneResponse, PingResponse,
    RawAdvertisement, bluetooth_proxy_feature, bluetooth_proxy_subscription, encode_frame,
};

use crate::Advertisement;
use crate::connection::ConnectionRequest;
use crate::scanner::{ScanMode, ScannerStatus};

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
    /// The configured scan mode, if the proxy has a scanner whose mode
    /// clients may change. Without one, scanner state and mode switching
    /// are not offered.
    pub scanner: Option<ScanMode>,
    /// Whether the API is encrypted.
    pub encrypted: bool,
    /// Whether the proxy connects to devices for clients.
    pub connections: bool,
}

impl DeviceIdentity<'_> {
    /// Returns the Bluetooth proxy features this proxy offers.
    #[must_use]
    pub const fn feature_flags(&self) -> u32 {
        let mut flags =
            bluetooth_proxy_feature::PASSIVE_SCAN | bluetooth_proxy_feature::RAW_ADVERTISEMENTS;
        if self.scanner.is_some() {
            flags |= bluetooth_proxy_feature::STATE_AND_MODE;
        }
        if self.connections {
            // Services are always resolved by the proxy's Bluetooth stack,
            // so clients may cache them (and must, to use connections).
            flags |= bluetooth_proxy_feature::ACTIVE_CONNECTIONS
                | bluetooth_proxy_feature::REMOTE_CACHING;
        }
        flags
    }
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
    /// Reports that no active connections are offered.
    ConnectionsFree(BluetoothConnectionsFreeResponse<'a>),
    /// Reports the scanner's state and mode.
    ScannerState(BluetoothScannerStateResponse),
}

impl Reply<'_> {
    /// Returns the reply's message type identifier.
    #[must_use]
    pub const fn message_type(&self) -> u16 {
        match self {
            Self::Hello(_) => HelloResponse::TYPE,
            Self::DeviceInfo(_) => DeviceInfoResponse::TYPE,
            Self::Ping(_) => PingResponse::TYPE,
            Self::Disconnect(_) => DisconnectResponse::TYPE,
            Self::ListEntitiesDone(_) => ListEntitiesDoneResponse::TYPE,
            Self::ConnectionsFree(_) => BluetoothConnectionsFreeResponse::TYPE,
            Self::ScannerState(_) => BluetoothScannerStateResponse::TYPE,
        }
    }

    /// Encodes just the reply's protobuf payload at the start of `buf`, for
    /// transports that frame messages themselves.
    ///
    /// # Errors
    ///
    /// Returns [`EncodeError::BufferTooSmall`] if `buf` cannot hold it.
    pub fn encode_payload(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        match self {
            Self::Hello(message) => message.encode_to_slice(buf),
            Self::DeviceInfo(message) => message.encode_to_slice(buf),
            Self::Ping(message) => message.encode_to_slice(buf),
            Self::Disconnect(message) => message.encode_to_slice(buf),
            Self::ListEntitiesDone(message) => message.encode_to_slice(buf),
            Self::ConnectionsFree(message) => message.encode_to_slice(buf),
            Self::ScannerState(message) => message.encode_to_slice(buf),
        }
    }

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
            Self::ScannerState(message) => encode_frame(message, buf),
        }
    }
}

/// What to do after handling a message.
///
/// Replies borrow from the device identity (`'a`), connection requests
/// from the message (`'m`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcome<'a, 'm> {
    /// Message to send back, if any.
    pub reply: Option<Reply<'a>>,
    /// Whether to close the connection (after sending `reply`).
    pub close: bool,
    /// Scan mode the client asked the scanner to switch to.
    pub scan_mode: Option<ScanMode>,
    /// Operation on a device connection for the platform to carry out.
    pub connection: Option<ConnectionRequest<&'m [u8]>>,
    /// Whether to describe the platform's entities before sending `reply`,
    /// which ends the list.
    pub list_entities: bool,
    /// A switch the client wants turned on or off.
    pub switch: Option<SwitchCommand>,
}

/// A client's request to turn one of the platform's switches on or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwitchCommand {
    /// The switch's key, as listed.
    pub key: u32,
    /// Whether to turn it on.
    pub state: bool,
}

impl<'a> Outcome<'a, '_> {
    const NOTHING: Self = Self {
        reply: None,
        close: false,
        scan_mode: None,
        connection: None,
        list_entities: false,
        switch: None,
    };
    const CLOSE: Self = Self {
        close: true,
        ..Self::NOTHING
    };

    const fn reply(reply: Reply<'a>) -> Self {
        Self {
            reply: Some(reply),
            ..Self::NOTHING
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
    connections_free: bool,
    states: bool,
}

impl<'a> Session<'a> {
    /// Starts a session for a newly accepted connection.
    #[must_use]
    pub const fn new(identity: &'a DeviceIdentity<'a>) -> Self {
        Self {
            identity,
            phase: Phase::AwaitingHello,
            raw_advertisements: false,
            connections_free: false,
            states: false,
        }
    }

    /// Returns whether the client wants raw advertisements forwarded.
    #[must_use]
    pub const fn wants_raw_advertisements(&self) -> bool {
        self.raw_advertisements
    }

    /// Returns whether the client wants the states of the platform's
    /// entities: their current states, and then their changes.
    #[must_use]
    pub const fn wants_states(&self) -> bool {
        self.states
    }

    /// Returns whether the client wants to follow the free connection
    /// slots. Only clients of a proxy that offers connections do.
    #[must_use]
    pub const fn wants_connections_free(&self) -> bool {
        self.connections_free
    }

    /// Returns the reply that reports `status` to the client, or `None` if
    /// the proxy has no switchable scanner.
    #[must_use]
    pub const fn scanner_state(&self, status: ScannerStatus) -> Option<Reply<'a>> {
        match self.identity.scanner {
            Some(configured) => Some(Reply::ScannerState(BluetoothScannerStateResponse {
                state: status.state.to_wire(),
                mode: status.mode.to_wire(),
                configured_mode: configured.to_wire(),
            })),
            None => None,
        }
    }

    /// Handles one message from the client.
    pub fn handle<'m>(&mut self, message: &ClientMessage<'m>) -> Outcome<'a, 'm> {
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
                close: true,
                ..Outcome::reply(Reply::Disconnect(DisconnectResponse))
            },
            ClientMessage::DisconnectResponse => Outcome::CLOSE,
            ClientMessage::DeviceInfo => Outcome::reply(Reply::DeviceInfo(DeviceInfoResponse {
                name: identity.name,
                mac_address: identity.mac_address,
                esphome_version: identity.version,
                model: identity.model,
                project_name: identity.project_name,
                project_version: identity.version,
                bluetooth_proxy_feature_flags: identity.feature_flags(),
                manufacturer: identity.manufacturer,
                friendly_name: identity.friendly_name,
                bluetooth_mac_address: identity.bluetooth_mac_address,
                api_encryption_supported: identity.encrypted,
            })),
            ClientMessage::ListEntities => Outcome {
                list_entities: true,
                ..Outcome::reply(Reply::ListEntitiesDone(ListEntitiesDoneResponse))
            },
            ClientMessage::SubscribeStates => {
                self.states = true;
                Outcome::NOTHING
            }
            &ClientMessage::SwitchCommand { key, state } => Outcome {
                switch: Some(SwitchCommand { key, state }),
                ..Outcome::NOTHING
            },
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
            ClientMessage::SubscribeBluetoothConnectionsFree if identity.connections => {
                self.connections_free = true;
                Outcome::NOTHING
            }
            ClientMessage::SubscribeBluetoothConnectionsFree => Outcome::reply(
                Reply::ConnectionsFree(BluetoothConnectionsFreeResponse::default()),
            ),
            ClientMessage::BluetoothScannerSetMode { mode } => Outcome {
                scan_mode: identity.scanner.and_then(|_| ScanMode::from_wire(*mode)),
                ..Outcome::NOTHING
            },
            _ if identity.connections => Outcome {
                connection: ConnectionRequest::from_message(message),
                ..Outcome::NOTHING
            },
            // No password, no logs or services to stream.
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
        scanner: None,
        encrypted: false,
        connections: false,
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

    const WITH_SCANNER: DeviceIdentity<'static> = DeviceIdentity {
        scanner: Some(ScanMode::Passive),
        ..IDENTITY
    };

    #[test]
    fn offers_state_and_mode_only_with_a_scanner() {
        assert_eq!(IDENTITY.feature_flags(), 0b010_0001);
        assert_eq!(WITH_SCANNER.feature_flags(), 0b110_0001);
    }

    #[test]
    fn passes_scan_mode_requests_to_the_platform() {
        let mut session = Session::new(&WITH_SCANNER);
        session.handle(&ClientMessage::Hello(HelloRequest::default()));
        let outcome = session.handle(&ClientMessage::BluetoothScannerSetMode { mode: 1 });
        assert_eq!(outcome.scan_mode, Some(ScanMode::Active));
        assert_eq!((outcome.reply, outcome.close), (None, false));
        // Unknown modes are ignored.
        let outcome = session.handle(&ClientMessage::BluetoothScannerSetMode { mode: 9 });
        assert_eq!(outcome.scan_mode, None);
        // Without a switchable scanner, requests are ignored too.
        let outcome = established().handle(&ClientMessage::BluetoothScannerSetMode { mode: 1 });
        assert_eq!(outcome, Outcome::NOTHING);
    }

    #[test]
    fn reports_scanner_state_with_the_configured_mode() {
        use crate::scanner::ScannerState;

        let status = ScannerStatus {
            state: ScannerState::Running,
            mode: ScanMode::Active,
        };
        assert_eq!(
            Session::new(&WITH_SCANNER).scanner_state(status),
            Some(Reply::ScannerState(BluetoothScannerStateResponse {
                state: 2,
                mode: 1,
                configured_mode: 0,
            }))
        );
        assert_eq!(Session::new(&IDENTITY).scanner_state(status), None);
    }

    const WITH_CONNECTIONS: DeviceIdentity<'static> = DeviceIdentity {
        connections: true,
        ..IDENTITY
    };

    #[test]
    fn offers_connections_with_remote_caching() {
        assert_eq!(WITH_CONNECTIONS.feature_flags(), 0b010_0111);
    }

    #[test]
    fn passes_connection_requests_to_the_platform() {
        let mut session = Session::new(&WITH_CONNECTIONS);
        session.handle(&ClientMessage::Hello(HelloRequest::default()));
        let read = ClientMessage::BluetoothGattRead {
            address: 0xA4C1_3800_1122,
            handle: 0x2A,
        };
        let outcome = session.handle(&read);
        assert_eq!(
            outcome.connection,
            Some(ConnectionRequest::Read {
                address: BdAddr::new([0xA4, 0xC1, 0x38, 0x00, 0x11, 0x22]),
                handle: 0x2A,
            })
        );
        assert_eq!((outcome.reply, outcome.close), (None, false));

        // Without connections, requests are ignored.
        assert_eq!(established().handle(&read), Outcome::NOTHING);
        // Before the handshake, they end the session like anything else.
        assert_eq!(
            Session::new(&WITH_CONNECTIONS).handle(&read),
            Outcome::CLOSE
        );
    }

    #[test]
    fn leaves_free_slots_to_the_platform_when_offering_connections() {
        let mut session = Session::new(&WITH_CONNECTIONS);
        session.handle(&ClientMessage::Hello(HelloRequest::default()));
        assert!(!session.wants_connections_free());
        let outcome = session.handle(&ClientMessage::SubscribeBluetoothConnectionsFree);
        assert_eq!(outcome, Outcome::NOTHING);
        assert!(session.wants_connections_free());

        // Without connections, the session reports no slots itself.
        let mut session = established();
        let outcome = session.handle(&ClientMessage::SubscribeBluetoothConnectionsFree);
        assert_eq!(
            outcome.reply,
            Some(Reply::ConnectionsFree(
                BluetoothConnectionsFreeResponse::default()
            ))
        );
        assert!(!session.wants_connections_free());
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
    fn leaves_entities_to_the_platform() {
        let mut session = established();
        let outcome = session.handle(&ClientMessage::ListEntities);
        assert!(outcome.list_entities);
        assert_eq!(
            outcome.reply,
            Some(Reply::ListEntitiesDone(ListEntitiesDoneResponse))
        );
        assert!(!session.wants_states());
        let outcome = session.handle(&ClientMessage::SubscribeStates);
        assert_eq!(outcome, Outcome::NOTHING);
        assert!(session.wants_states());
        // Other messages don't ask for entities.
        assert!(!session.handle(&ClientMessage::Ping).list_entities);

        let outcome = session.handle(&ClientMessage::SwitchCommand {
            key: 4,
            state: false,
        });
        assert_eq!(
            outcome.switch,
            Some(SwitchCommand {
                key: 4,
                state: false
            })
        );
        assert_eq!((outcome.reply, outcome.close), (None, false));
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
                    allocated: &[],
                })),
            ),
            (ClientMessage::Authentication, None),
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
    fn reports_encryption_in_device_info() {
        let encrypted = DeviceIdentity {
            encrypted: true,
            ..IDENTITY
        };
        let mut session = Session::new(&encrypted);
        session.handle(&ClientMessage::Hello(HelloRequest::default()));
        let Some(Reply::DeviceInfo(info)) = session.handle(&ClientMessage::DeviceInfo).reply else {
            panic!("expected device info");
        };
        assert!(info.api_encryption_supported);
    }

    #[test]
    fn encodes_reply_payloads_for_other_transports() {
        let reply = Reply::ScannerState(BluetoothScannerStateResponse {
            state: 2,
            mode: 1,
            configured_mode: 0,
        });
        assert_eq!(reply.message_type(), 126);
        let mut buf = [0; 16];
        let len = reply.encode_payload(&mut buf).unwrap();
        assert_eq!(&buf[..len], [0x08, 0x02, 0x10, 0x01]);
        assert_eq!(Reply::Ping(PingResponse).message_type(), 8);
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
