//! Active connections: operations on connected devices that clients ask
//! for and the platform carries out.

use bleproxy_protocol::{BluetoothDeviceRequest, ClientMessage, bluetooth_device_request_type};

use crate::{AddressType, BdAddr};

/// An operation on a device connection that a client asked for.
///
/// `D` holds written values: a slice of the message while decoding, and
/// usually an owned buffer once passed on (see [`Self::map_data`]).
/// Handles are kept as the client sent them; the platform reports an
/// invalid handle error for those that don't match an attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionRequest<D> {
    /// Connect to a device and resolve its GATT services.
    Connect {
        /// Device address.
        address: BdAddr,
        /// Address type, if the client knows it.
        address_type: Option<AddressType>,
    },
    /// Disconnect from a device, or stop connecting to it.
    Disconnect {
        /// Device address.
        address: BdAddr,
    },
    /// List a connected device's GATT services.
    GetServices {
        /// Device address.
        address: BdAddr,
    },
    /// Read a characteristic.
    Read {
        /// Device address.
        address: BdAddr,
        /// Characteristic value handle.
        handle: u32,
    },
    /// Write a characteristic.
    Write {
        /// Device address.
        address: BdAddr,
        /// Characteristic value handle.
        handle: u32,
        /// Value to write.
        data: D,
        /// Whether to wait for the device to confirm the write.
        response: bool,
    },
    /// Read a descriptor.
    ReadDescriptor {
        /// Device address.
        address: BdAddr,
        /// Descriptor handle.
        handle: u32,
    },
    /// Write a descriptor.
    WriteDescriptor {
        /// Device address.
        address: BdAddr,
        /// Descriptor handle.
        handle: u32,
        /// Value to write.
        data: D,
    },
    /// Start or stop forwarding a characteristic's notifications and
    /// indications.
    Notify {
        /// Device address.
        address: BdAddr,
        /// Characteristic value handle.
        handle: u32,
        /// Whether to start forwarding.
        enable: bool,
    },
}

impl<D> ConnectionRequest<D> {
    /// Returns the address of the device the request is for.
    #[must_use]
    pub const fn address(&self) -> BdAddr {
        match self {
            Self::Connect { address, .. }
            | Self::Disconnect { address }
            | Self::GetServices { address }
            | Self::Read { address, .. }
            | Self::Write { address, .. }
            | Self::ReadDescriptor { address, .. }
            | Self::WriteDescriptor { address, .. }
            | Self::Notify { address, .. } => *address,
        }
    }

    /// Converts the written value, for example into an owned buffer.
    ///
    /// ```
    /// use bleproxy_core::BdAddr;
    /// use bleproxy_core::connection::ConnectionRequest;
    ///
    /// let request = ConnectionRequest::WriteDescriptor {
    ///     address: BdAddr::new([0xA4, 0xC1, 0x38, 0x00, 0x11, 0x22]),
    ///     handle: 0x2C,
    ///     data: &[0x01, 0x00][..],
    /// };
    /// let owned: ConnectionRequest<[u8; 2]> = request.map_data(|data| [data[0], data[1]]);
    /// assert!(matches!(owned, ConnectionRequest::WriteDescriptor { data: [1, 0], .. }));
    /// ```
    pub fn map_data<E>(self, f: impl FnOnce(D) -> E) -> ConnectionRequest<E> {
        match self {
            Self::Connect {
                address,
                address_type,
            } => ConnectionRequest::Connect {
                address,
                address_type,
            },
            Self::Disconnect { address } => ConnectionRequest::Disconnect { address },
            Self::GetServices { address } => ConnectionRequest::GetServices { address },
            Self::Read { address, handle } => ConnectionRequest::Read { address, handle },
            Self::Write {
                address,
                handle,
                data,
                response,
            } => ConnectionRequest::Write {
                address,
                handle,
                data: f(data),
                response,
            },
            Self::ReadDescriptor { address, handle } => {
                ConnectionRequest::ReadDescriptor { address, handle }
            }
            Self::WriteDescriptor {
                address,
                handle,
                data,
            } => ConnectionRequest::WriteDescriptor {
                address,
                handle,
                data: f(data),
            },
            Self::Notify {
                address,
                handle,
                enable,
            } => ConnectionRequest::Notify {
                address,
                handle,
                enable,
            },
        }
    }
}

impl<'a> ConnectionRequest<&'a [u8]> {
    /// Returns the request `message` makes, if it is one this proxy
    /// carries out.
    ///
    /// Pairing and cache clearing are not offered, so those requests are
    /// ignored, as are messages with invalid addresses.
    #[must_use]
    pub fn from_message(message: &ClientMessage<'a>) -> Option<Self> {
        let device = |address| BdAddr::from_u64(address).ok();
        Some(match *message {
            ClientMessage::BluetoothDeviceRequest(BluetoothDeviceRequest {
                address,
                request_type,
                address_type,
            }) => {
                use bluetooth_device_request_type as t;
                let address = device(address)?;
                match request_type {
                    t::CONNECT | t::CONNECT_V3_WITH_CACHE | t::CONNECT_V3_WITHOUT_CACHE => {
                        Self::Connect {
                            address,
                            address_type: address_type
                                .and_then(|kind| u8::try_from(kind).ok())
                                .and_then(|kind| AddressType::try_from(kind).ok()),
                        }
                    }
                    t::DISCONNECT => Self::Disconnect { address },
                    _ => return None,
                }
            }
            ClientMessage::BluetoothGattGetServices { address } => Self::GetServices {
                address: device(address)?,
            },
            ClientMessage::BluetoothGattRead { address, handle } => Self::Read {
                address: device(address)?,
                handle,
            },
            ClientMessage::BluetoothGattWrite {
                address,
                handle,
                response,
                data,
            } => Self::Write {
                address: device(address)?,
                handle,
                data,
                response,
            },
            ClientMessage::BluetoothGattReadDescriptor { address, handle } => {
                Self::ReadDescriptor {
                    address: device(address)?,
                    handle,
                }
            }
            ClientMessage::BluetoothGattWriteDescriptor {
                address,
                handle,
                data,
            } => Self::WriteDescriptor {
                address: device(address)?,
                handle,
                data,
            },
            ClientMessage::BluetoothGattNotify {
                address,
                handle,
                enable,
            } => Self::Notify {
                address: device(address)?,
                handle,
                enable,
            },
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADDRESS: BdAddr = BdAddr::new([0xA4, 0xC1, 0x38, 0x00, 0x11, 0x22]);
    const RAW: u64 = 0xA4C1_3800_1122;

    fn device_request(request_type: u32, address_type: Option<u32>) -> ClientMessage<'static> {
        ClientMessage::BluetoothDeviceRequest(BluetoothDeviceRequest {
            address: RAW,
            request_type,
            address_type,
        })
    }

    #[test]
    fn connects_with_any_connection_version() {
        use bluetooth_device_request_type as t;
        for request_type in [
            t::CONNECT,
            t::CONNECT_V3_WITH_CACHE,
            t::CONNECT_V3_WITHOUT_CACHE,
        ] {
            assert_eq!(
                ConnectionRequest::from_message(&device_request(request_type, Some(1))),
                Some(ConnectionRequest::Connect {
                    address: ADDRESS,
                    address_type: Some(AddressType::Random),
                })
            );
        }
        // Unknown address types are left for the platform to work out.
        assert_eq!(
            ConnectionRequest::from_message(&device_request(t::CONNECT_V3_WITH_CACHE, Some(7))),
            Some(ConnectionRequest::Connect {
                address: ADDRESS,
                address_type: None,
            })
        );
    }

    #[test]
    fn ignores_requests_that_are_not_offered() {
        use bluetooth_device_request_type as t;
        for request_type in [t::PAIR, t::UNPAIR, t::CLEAR_CACHE, 99] {
            assert_eq!(
                ConnectionRequest::from_message(&device_request(request_type, None)),
                None
            );
        }
        let invalid = ClientMessage::BluetoothGattRead {
            address: u64::MAX,
            handle: 1,
        };
        assert_eq!(ConnectionRequest::from_message(&invalid), None);
        assert_eq!(ConnectionRequest::from_message(&ClientMessage::Ping), None);
    }

    #[test]
    fn converts_gatt_requests() {
        let cases: [(_, ConnectionRequest<&[u8]>); 7] = [
            (
                device_request(bluetooth_device_request_type::DISCONNECT, None),
                ConnectionRequest::Disconnect { address: ADDRESS },
            ),
            (
                ClientMessage::BluetoothGattGetServices { address: RAW },
                ConnectionRequest::GetServices { address: ADDRESS },
            ),
            (
                ClientMessage::BluetoothGattRead {
                    address: RAW,
                    handle: 0x2A,
                },
                ConnectionRequest::Read {
                    address: ADDRESS,
                    handle: 0x2A,
                },
            ),
            (
                ClientMessage::BluetoothGattWrite {
                    address: RAW,
                    handle: 0x2A,
                    response: true,
                    data: &[1, 2],
                },
                ConnectionRequest::Write {
                    address: ADDRESS,
                    handle: 0x2A,
                    data: &[1, 2],
                    response: true,
                },
            ),
            (
                ClientMessage::BluetoothGattReadDescriptor {
                    address: RAW,
                    handle: 0x2C,
                },
                ConnectionRequest::ReadDescriptor {
                    address: ADDRESS,
                    handle: 0x2C,
                },
            ),
            (
                ClientMessage::BluetoothGattWriteDescriptor {
                    address: RAW,
                    handle: 0x2C,
                    data: &[1, 0],
                },
                ConnectionRequest::WriteDescriptor {
                    address: ADDRESS,
                    handle: 0x2C,
                    data: &[1, 0],
                },
            ),
            (
                ClientMessage::BluetoothGattNotify {
                    address: RAW,
                    handle: 0x2A,
                    enable: true,
                },
                ConnectionRequest::Notify {
                    address: ADDRESS,
                    handle: 0x2A,
                    enable: true,
                },
            ),
        ];
        for (message, expected) in cases {
            let request = ConnectionRequest::from_message(&message);
            assert_eq!(request, Some(expected), "{message:?}");
            assert_eq!(expected.address(), ADDRESS);
        }
    }

    #[test]
    fn maps_written_values() {
        let request = ConnectionRequest::Write {
            address: ADDRESS,
            handle: 1,
            data: &[1_u8, 2, 3][..],
            response: false,
        };
        assert_eq!(
            request.map_data(<[u8]>::len),
            ConnectionRequest::Write {
                address: ADDRESS,
                handle: 1,
                data: 3,
                response: false,
            }
        );
        let read: ConnectionRequest<&[u8]> = ConnectionRequest::Read {
            address: ADDRESS,
            handle: 1,
        };
        assert_eq!(
            read.map_data(<[u8]>::len),
            ConnectionRequest::Read {
                address: ADDRESS,
                handle: 1,
            }
        );
    }
}
