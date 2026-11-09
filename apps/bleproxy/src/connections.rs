//! The link between API connections and the platform's connection
//! manager.
//!
//! Each API connection gets a [`Client`] from the [`ConnectionsHandle`] and
//! sends it the connection requests its client makes. The connection
//! manager receives them, tagged with the client, through the
//! [`ConnectionsDriver`], answers with [`Event`]s, and reports which
//! devices hold the connection slots. A device connection belongs to the
//! client that asked for it: its events go to that client, and it is
//! closed when that client goes away.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bleproxy_core::connection::ConnectionRequest;
use bleproxy_core::{BdAddr, Uuid};
use tokio::sync::{mpsc, watch};
use tracing::warn;

/// A connection request with its written value copied out of the message.
pub type Request = ConnectionRequest<Vec<u8>>;

/// How many events may wait for a slow client before new ones are dropped.
const EVENT_BACKLOG: usize = 256;

/// Creates a connected handle and driver for a connection manager with
/// `limit` connection slots.
#[must_use]
pub fn channel(limit: u32) -> (ConnectionsHandle, ConnectionsDriver) {
    let (commands_tx, commands_rx) = mpsc::unbounded_channel();
    let (slots_tx, slots_rx) = watch::channel(Slots {
        limit,
        allocated: Vec::new(),
    });
    let handle = ConnectionsHandle {
        commands: commands_tx,
        slots: slots_rx,
        next_client: Arc::new(AtomicU64::new(0)),
    };
    let driver = ConnectionsDriver {
        commands: commands_rx,
        slots: slots_tx,
    };
    (handle, driver)
}

/// Used by API connections to reach the connection manager.
#[derive(Debug, Clone)]
pub struct ConnectionsHandle {
    commands: mpsc::UnboundedSender<Command>,
    slots: watch::Receiver<Slots>,
    next_client: Arc<AtomicU64>,
}

impl ConnectionsHandle {
    /// Registers a new API connection.
    #[must_use]
    pub fn client(&self) -> Client {
        let (events_tx, events) = mpsc::channel(EVENT_BACKLOG);
        Client {
            id: ClientId(self.next_client.fetch_add(1, Ordering::Relaxed)),
            commands: self.commands.clone(),
            events_tx: EventSender(events_tx),
            events,
        }
    }

    /// Returns a receiver that follows the connection slots.
    #[must_use]
    pub fn slots(&self) -> watch::Receiver<Slots> {
        self.slots.clone()
    }
}

/// The connection slots and the devices holding them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Slots {
    /// Total connection slots.
    pub limit: u32,
    /// Devices holding a slot, connected or connecting.
    pub allocated: Vec<BdAddr>,
}

impl Slots {
    /// Returns the number of free slots.
    #[must_use]
    pub fn free(&self) -> u32 {
        let allocated = u32::try_from(self.allocated.len()).unwrap_or(u32::MAX);
        self.limit.saturating_sub(allocated)
    }
}

/// Identifies an API connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClientId(u64);

/// One API connection's link to the connection manager. Dropping it
/// releases the client's device connections.
#[derive(Debug)]
pub struct Client {
    id: ClientId,
    commands: mpsc::UnboundedSender<Command>,
    events_tx: EventSender,
    events: mpsc::Receiver<Event>,
}

impl Client {
    /// Passes `request` on to the connection manager. Requests are dropped
    /// if it has stopped.
    pub fn request(&self, request: Request) {
        let _ = self.commands.send(Command::Request {
            client: self.id,
            events: self.events_tx.clone(),
            request,
        });
    }

    /// Waits for the next event for this client.
    pub async fn event(&mut self) -> Event {
        match self.events.recv().await {
            Some(event) => event,
            // The client holds a sender, so the channel never closes.
            None => std::future::pending().await,
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Release(self.id));
    }
}

/// Sends events to one client.
#[derive(Debug, Clone)]
pub struct EventSender(mpsc::Sender<Event>);

impl EventSender {
    /// Sends `event`, dropping it if the client is gone or too far behind.
    pub fn send(&self, event: Event) {
        if let Err(mpsc::error::TrySendError::Full(event)) = self.0.try_send(event) {
            warn!(?event, "client is too slow, dropped a connection event");
        }
    }
}

/// What the connection manager receives.
#[derive(Debug)]
pub enum Command {
    /// A client's request.
    Request {
        /// The client that made it.
        client: ClientId,
        /// Where to send the results, and later events of the connection.
        events: EventSender,
        /// The request.
        request: Request,
    },
    /// The client is gone; close its device connections.
    Release(ClientId),
}

/// Used by the connection manager to receive commands and report slots.
#[derive(Debug)]
pub struct ConnectionsDriver {
    commands: mpsc::UnboundedReceiver<Command>,
    slots: watch::Sender<Slots>,
}

impl ConnectionsDriver {
    /// Number of connection slots.
    #[must_use]
    pub fn limit(&self) -> u32 {
        self.slots.borrow().limit
    }

    /// Waits for the next command, or returns `None` once every handle and
    /// client is gone.
    pub async fn next(&mut self) -> Option<Command> {
        self.commands.recv().await
    }

    /// Publishes the devices holding connection slots.
    pub fn report(&self, allocated: Vec<BdAddr>) {
        self.slots.send_if_modified(|slots| {
            let changed = slots.allocated != allocated;
            slots.allocated = allocated;
            changed
        });
    }
}

/// What the connection manager tells a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A device connected (with its services resolved), failed to, or
    /// disconnected.
    Connection {
        /// Device address.
        address: BdAddr,
        /// Whether the device is connected.
        connected: bool,
        /// ATT MTU of the connection.
        mtu: u16,
        /// Why connecting failed or the connection ended, as a
        /// [`bleproxy_protocol::bluetooth_error`] value, or 0.
        error: i32,
    },
    /// A connected device's GATT services.
    Services {
        /// Device address.
        address: BdAddr,
        /// The services, by handle.
        services: Vec<Service>,
    },
    /// A characteristic or descriptor value that was read.
    Read {
        /// Device address.
        address: BdAddr,
        /// Attribute handle.
        handle: u32,
        /// The value.
        data: Vec<u8>,
    },
    /// A write was confirmed.
    Written {
        /// Device address.
        address: BdAddr,
        /// Attribute handle.
        handle: u32,
    },
    /// Notifications were started or stopped.
    Notifying {
        /// Device address.
        address: BdAddr,
        /// Characteristic value handle.
        handle: u32,
    },
    /// A notification or indication.
    Notification {
        /// Device address.
        address: BdAddr,
        /// Characteristic value handle.
        handle: u32,
        /// The value.
        data: Vec<u8>,
    },
    /// An operation on an attribute failed.
    Error {
        /// Device address.
        address: BdAddr,
        /// Attribute handle.
        handle: u32,
        /// A [`bleproxy_protocol::bluetooth_error`] value.
        error: i32,
    },
}

/// A GATT service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    /// Service UUID.
    pub uuid: Uuid,
    /// Handle of the service declaration.
    pub handle: u16,
    /// The service's characteristics, by handle.
    pub characteristics: Vec<Characteristic>,
}

/// A GATT characteristic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Characteristic {
    /// Characteristic UUID.
    pub uuid: Uuid,
    /// Handle of the characteristic's value, which clients use to refer to
    /// it.
    pub handle: u16,
    /// Properties (read, write, notify...) as the GATT bit mask.
    pub properties: u8,
    /// The characteristic's descriptors, by handle.
    pub descriptors: Vec<Descriptor>,
}

/// A GATT descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Descriptor {
    /// Descriptor UUID.
    pub uuid: Uuid,
    /// Descriptor handle.
    pub handle: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEVICE: BdAddr = BdAddr::new([0xA4, 0xC1, 0x38, 0x00, 0x11, 0x22]);

    #[tokio::test]
    async fn routes_requests_and_events_per_client() {
        let (handle, mut driver) = channel(3);
        assert_eq!(driver.limit(), 3);
        let mut client = handle.client();
        let other = handle.client();

        client.request(ConnectionRequest::Disconnect { address: DEVICE });
        let Some(Command::Request {
            client: id,
            events,
            request,
        }) = driver.next().await
        else {
            panic!("expected a request");
        };
        assert_eq!(id, client.id);
        assert_ne!(id, other.id);
        assert_eq!(request, ConnectionRequest::Disconnect { address: DEVICE });

        let event = Event::Connection {
            address: DEVICE,
            connected: false,
            mtu: 0,
            error: 0,
        };
        events.send(event.clone());
        assert_eq!(client.event().await, event);

        drop(other);
        assert!(matches!(driver.next().await, Some(Command::Release(_))));
    }

    #[tokio::test]
    async fn publishes_slot_changes() {
        let (handle, driver) = channel(2);
        let mut slots = handle.slots();
        assert_eq!(slots.borrow_and_update().free(), 2);

        driver.report(vec![DEVICE]);
        slots.changed().await.unwrap();
        let current = slots.borrow_and_update().clone();
        assert_eq!(current.free(), 1);
        assert_eq!(current.allocated, [DEVICE]);

        // Reporting the same devices again does not notify.
        driver.report(vec![DEVICE]);
        assert!(!slots.has_changed().unwrap());
    }

    #[test]
    fn never_reports_negative_free_slots() {
        let slots = Slots {
            limit: 1,
            allocated: vec![DEVICE, DEVICE],
        };
        assert_eq!(slots.free(), 0);
    }

    #[tokio::test]
    async fn drops_events_for_clients_that_fall_behind() {
        let (handle, _driver) = channel(1);
        let mut client = handle.client();
        let read = |data: u8| Event::Read {
            address: DEVICE,
            handle: 1,
            data: vec![data],
        };
        for i in 0..=EVENT_BACKLOG {
            client.events_tx.send(read(u8::try_from(i % 256).unwrap()));
        }
        for _ in 0..EVENT_BACKLOG {
            client.event().await;
        }
        assert!(
            client.events.try_recv().is_err(),
            "the extra event was dropped"
        );
    }
}
