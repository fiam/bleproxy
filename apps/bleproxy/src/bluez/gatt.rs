//! Active connections through BlueZ's D-Bus API (Linux).
//!
//! Devices are connected with `Device1.Connect`, or with the adapter's
//! `ConnectDevice` if BlueZ hasn't seen them, and are reported connected
//! once BlueZ has resolved their services.
//!
//! The ESPHome API addresses GATT attributes by handle, BlueZ by object
//! path. The paths end in the attribute's handle
//! (`.../service0028/char0029/desc002b`), so the two map exactly.
//! Characteristics are identified by their value handle, as ESPHome does
//! (clients share cached services between proxies); it is the one after
//! the characteristic declaration's, which is the one in the path.
//!
//! BlueZ writes the client characteristic configuration descriptor itself
//! in `StartNotify` and `StopNotify`, and refuses writes to it. Clients
//! write it right after asking for notifications, so those writes are
//! acknowledged without reaching BlueZ.
//!
//! Notifications arrive as changes of a characteristic's `Value`, which
//! BlueZ also changes when a read completes. Changes caused by our own
//! reads are skipped so they don't pass for notifications.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use bleproxy_core::{AddressType, BdAddr, Uuid};
use bleproxy_protocol::bluetooth_error;
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};
use zbus::{Connection, Message, OwnedObjectPath, OwnedVariant, Variant};

use super::{
    ADAPTER, BLUEZ, BluezError, DEVICE, ManagedObjects, OBJECT_MANAGER, PROPERTIES, RETRY_DELAY,
    adapter_path, bytes, signals, string, strings, unwrap,
};
use crate::connections::{
    Characteristic, ClientId, Command, ConnectionsDriver, Descriptor, Event, EventSender, Request,
    Service,
};

const GATT_SERVICE: &str = "org.bluez.GattService1";
const GATT_CHARACTERISTIC: &str = "org.bluez.GattCharacteristic1";
const GATT_DESCRIPTOR: &str = "org.bluez.GattDescriptor1";

/// MTU reported when BlueZ doesn't know it: the LE minimum.
const DEFAULT_MTU: u16 = 23;

/// The client characteristic configuration descriptor.
const CCCD: Uuid = Uuid::from_u16(0x2902);

/// Connects to devices with `adapter` for API clients, until no client
/// can make requests anymore.
///
/// If the connection to BlueZ fails, every device connection is reported
/// lost and the manager starts over after a short delay.
pub async fn manage_connections(adapter: String, mut driver: ConnectionsDriver) {
    let path = adapter_path(&adapter);
    info!(
        adapter,
        slots = driver.limit(),
        "accepting device connections"
    );
    loop {
        match run(&path, &mut driver).await {
            Ok(()) => return,
            Err(err) => warn!(%err, "device connections failed"),
        }
        driver.report(Vec::new());
        info!(delay = ?RETRY_DELAY, "restarting device connections");
        tokio::time::sleep(RETRY_DELAY).await;
    }
}

/// Runs the manager until every client is gone (`Ok`) or BlueZ fails.
async fn run(adapter: &str, driver: &mut ConnectionsDriver) -> Result<(), BluezError> {
    let bus = Connection::system().await?;
    // Device connection changes, and characteristic value changes.
    let mut changes = signals(&bus, PROPERTIES, "PropertiesChanged", adapter).await?;
    let (done, mut finished) = mpsc::unbounded_channel();
    let mut manager = Manager {
        bus,
        adapter: adapter.to_owned(),
        limit: driver.limit(),
        links: HashMap::new(),
        next_id: 0,
        echoes: Echoes::default(),
        done,
    };
    let result = loop {
        tokio::select! {
            command = driver.next() => match command {
                Some(command) => manager.command(command),
                None => break Ok(()),
            },
            Some(done) = finished.recv() => manager.finished(done),
            Some(message) = changes.next() => match message {
                Ok(message) => manager.changed(&message),
                Err(err) => break Err(err.into()),
            },
            else => break Err(BluezError::Disconnected),
        }
        driver.report(manager.allocated());
    };
    manager.drop_all();
    result
}

/// A device connection, from the request until it ends. It holds a slot.
#[derive(Debug)]
struct Link {
    /// Tells this link's task results apart from an earlier link's.
    id: u64,
    owner: ClientId,
    events: EventSender,
    device: String,
    state: State,
}

#[derive(Debug)]
enum State {
    Connecting(JoinHandle<()>),
    Connected(Box<Connected>),
    Disconnecting,
}

#[derive(Debug)]
struct Connected {
    gatt: Gatt,
    mtu: u16,
    /// Operations for the link's worker, which runs them in order.
    operations: mpsc::UnboundedSender<Operation>,
    /// Characteristics whose notifications the client wants.
    notifying: HashSet<u32>,
}

/// Results of the tasks the manager starts.
#[derive(Debug)]
enum Finished {
    Connected {
        address: BdAddr,
        id: u64,
        gatt: Gatt,
    },
    Failed {
        address: BdAddr,
        id: u64,
    },
    Disconnected {
        address: BdAddr,
        id: u64,
    },
}

struct Manager {
    bus: Connection,
    adapter: String,
    limit: u32,
    links: HashMap<BdAddr, Link>,
    next_id: u64,
    echoes: Echoes,
    done: mpsc::UnboundedSender<Finished>,
}

impl Manager {
    fn allocated(&self) -> Vec<BdAddr> {
        let mut allocated: Vec<BdAddr> = self.links.keys().copied().collect();
        allocated.sort_unstable();
        allocated
    }

    fn command(&mut self, command: Command) {
        match command {
            Command::Request {
                client,
                events,
                request,
            } => self.request(client, &events, request),
            Command::Release(client) => {
                let owned: Vec<BdAddr> = self
                    .links
                    .iter()
                    .filter(|(_, link)| link.owner == client)
                    .map(|(address, _)| *address)
                    .collect();
                for address in owned {
                    info!(%address, "client went away, disconnecting");
                    self.disconnect(address);
                }
            }
        }
    }

    fn request(&mut self, client: ClientId, events: &EventSender, request: Request) {
        let address = request.address();
        match request {
            Request::Connect { address_type, .. } => {
                self.connect(client, events, address, address_type);
            }
            Request::Disconnect { .. } => match self.links.get(&address) {
                Some(link) if link.owner == client => self.disconnect(address),
                _ => events.send(Event::Connection {
                    address,
                    connected: false,
                    mtu: 0,
                    error: 0,
                }),
            },
            Request::GetServices { .. } => match self.connected(client, address) {
                Some(connected) => events.send(Event::Services {
                    address,
                    services: connected.gatt.services.clone(),
                }),
                None => events.send(not_connected(address, 0)),
            },
            Request::Read { handle, .. } => {
                self.operate(
                    client,
                    events,
                    address,
                    handle,
                    Kind::Characteristic,
                    |path| Operation::Read { handle, path },
                );
            }
            Request::Write {
                handle,
                data,
                response,
                ..
            } => {
                self.operate(
                    client,
                    events,
                    address,
                    handle,
                    Kind::Characteristic,
                    |path| Operation::Write {
                        handle,
                        path,
                        data,
                        response,
                    },
                );
            }
            Request::ReadDescriptor { handle, .. } => {
                self.operate(client, events, address, handle, Kind::Descriptor, |path| {
                    Operation::ReadDescriptor { handle, path }
                });
            }
            Request::WriteDescriptor { handle, data, .. } => {
                let is_cccd = self.connected(client, address).is_some_and(|connected| {
                    connected
                        .gatt
                        .attributes
                        .get(&handle)
                        .is_some_and(|attribute| attribute.uuid == CCCD)
                });
                if is_cccd {
                    // BlueZ configures notifications itself (see the
                    // module documentation).
                    events.send(Event::Written { address, handle });
                    return;
                }
                self.operate(client, events, address, handle, Kind::Descriptor, |path| {
                    Operation::WriteDescriptor { handle, path, data }
                });
            }
            Request::Notify { handle, enable, .. } => {
                if let Some(connected) = self.connected(client, address) {
                    if enable {
                        connected.notifying.insert(handle);
                    } else {
                        connected.notifying.remove(&handle);
                    }
                }
                self.operate(
                    client,
                    events,
                    address,
                    handle,
                    Kind::Characteristic,
                    |path| Operation::Notify {
                        handle,
                        path,
                        enable,
                    },
                );
            }
        }
    }

    /// Returns `address`'s connection if `client` owns it and it is up.
    fn connected(&mut self, client: ClientId, address: BdAddr) -> Option<&mut Connected> {
        match self.links.get_mut(&address) {
            Some(Link {
                owner,
                state: State::Connected(connected),
                ..
            }) if *owner == client => Some(connected),
            _ => None,
        }
    }

    /// Queues an operation on the attribute at `handle`, which must be of
    /// `kind`, for the link's worker.
    fn operate(
        &mut self,
        client: ClientId,
        events: &EventSender,
        address: BdAddr,
        handle: u32,
        kind: Kind,
        operation: impl FnOnce(OwnedObjectPath) -> Operation,
    ) {
        let Some(connected) = self.connected(client, address) else {
            events.send(not_connected(address, handle));
            return;
        };
        match connected.gatt.attributes.get(&handle) {
            Some(attribute) if attribute.kind == kind => {
                let _ = connected.operations.send(operation(attribute.path.clone()));
            }
            _ => events.send(Event::Error {
                address,
                handle,
                error: bluetooth_error::INVALID_HANDLE,
            }),
        }
    }

    fn connect(
        &mut self,
        client: ClientId,
        events: &EventSender,
        address: BdAddr,
        address_type: Option<AddressType>,
    ) {
        if let Some(link) = self.links.get(&address) {
            let event = match &link.state {
                State::Connected(connected) if link.owner == client => Event::Connection {
                    address,
                    connected: true,
                    mtu: connected.mtu,
                    error: 0,
                },
                // Connecting already, or someone else's.
                _ => return debug!(%address, "ignoring a connection request for a busy device"),
            };
            return events.send(event);
        }
        if self.links.len() >= usize::try_from(self.limit).unwrap_or(usize::MAX) {
            info!(%address, "no free connection slot");
            return events.send(Event::Connection {
                address,
                connected: false,
                mtu: 0,
                error: 0,
            });
        }
        let id = self.next_id;
        self.next_id += 1;
        let device = device_path(&self.adapter, address);
        let task = tokio::spawn(connect(
            self.bus.clone(),
            self.adapter.clone(),
            device.clone(),
            address,
            address_type,
            id,
            self.done.clone(),
        ));
        self.links.insert(
            address,
            Link {
                id,
                owner: client,
                events: events.clone(),
                device,
                state: State::Connecting(task),
            },
        );
    }

    /// Disconnects from `address`, or stops connecting to it. The owner
    /// hears about it once BlueZ is done.
    fn disconnect(&mut self, address: BdAddr) {
        let Some(link) = self.links.get_mut(&address) else {
            return;
        };
        match std::mem::replace(&mut link.state, State::Disconnecting) {
            State::Connecting(task) => task.abort(),
            // Dropping the operations queue stops the worker.
            State::Connected(_) => {}
            State::Disconnecting => return,
        }
        let (bus, device, id, done) = (
            self.bus.clone(),
            link.device.clone(),
            link.id,
            self.done.clone(),
        );
        tokio::spawn(async move {
            let result = bus
                .call_method(
                    Some(BLUEZ),
                    device.as_str(),
                    Some(DEVICE),
                    "Disconnect",
                    &(),
                )
                .await;
            if let Err(err) = result {
                debug!(%address, %err, "disconnecting failed");
            }
            let _ = done.send(Finished::Disconnected { address, id });
        });
    }

    fn finished(&mut self, finished: Finished) {
        match finished {
            Finished::Connected { address, id, gatt } => {
                let Some(link) = self.links.get_mut(&address).filter(|link| link.id == id) else {
                    return;
                };
                let (operations, queue) = mpsc::unbounded_channel();
                tokio::spawn(work(
                    self.bus.clone(),
                    address,
                    link.events.clone(),
                    self.echoes.clone(),
                    queue,
                ));
                let mtu = gatt.mtu.unwrap_or(DEFAULT_MTU);
                info!(%address, mtu, services = gatt.services.len(), "device connected");
                link.state = State::Connected(Box::new(Connected {
                    gatt,
                    mtu,
                    operations,
                    notifying: HashSet::new(),
                }));
                link.events.send(Event::Connection {
                    address,
                    connected: true,
                    mtu,
                    error: 0,
                });
            }
            Finished::Failed { address, id } => {
                self.end(address, Some(id), bluetooth_error::CONNECTION_FAILED);
            }
            Finished::Disconnected { address, id } => self.end(address, Some(id), 0),
        }
    }

    /// Frees `address`'s slot and tells its owner the device is
    /// disconnected. With an `id`, only that link is ended.
    fn end(&mut self, address: BdAddr, id: Option<u64>, error: i32) {
        if id.is_some_and(|id| self.links.get(&address).is_none_or(|link| link.id != id)) {
            return;
        }
        if let Some(link) = self.links.remove(&address) {
            if let State::Connecting(task) = &link.state {
                task.abort();
            }
            link.events.send(Event::Connection {
                address,
                connected: false,
                mtu: 0,
                error,
            });
        }
    }

    /// Reports every connection lost, as when BlueZ goes away.
    fn drop_all(&mut self) {
        let addresses: Vec<BdAddr> = self.links.keys().copied().collect();
        for address in addresses {
            self.end(address, None, 0);
        }
    }

    /// Follows device disconnections and characteristic notifications.
    fn changed(&mut self, message: &Message) {
        let Some(path) = message.header().path().map(|path| path.as_str().to_owned()) else {
            return;
        };
        let Ok((interface, properties, _)) =
            message
                .body()
                .deserialize::<(String, HashMap<String, OwnedVariant>, Vec<String>)>()
        else {
            return;
        };
        match interface.as_str() {
            DEVICE => {
                let disconnected = properties
                    .get("Connected")
                    .is_some_and(|value| matches!(unwrap(value), Variant::Bool(false)));
                let link = self
                    .links
                    .iter()
                    .find(|(_, link)| link.device == path)
                    .map(|(address, link)| (*address, &link.state));
                // While connecting, the connecting task follows the device.
                if let (true, Some((address, State::Connected(_) | State::Disconnecting))) =
                    (disconnected, link)
                {
                    info!(%address, "device disconnected");
                    self.end(address, None, 0);
                }
            }
            GATT_CHARACTERISTIC => {
                let Some(value) = properties.get("Value").and_then(|value| bytes(value)) else {
                    return;
                };
                if self.echoes.take(&path) {
                    return;
                }
                for (address, link) in &self.links {
                    let State::Connected(connected) = &link.state else {
                        continue;
                    };
                    if let Some(handle) = connected.gatt.characteristics.get(&path)
                        && connected.notifying.contains(handle)
                    {
                        link.events.send(Event::Notification {
                            address: *address,
                            handle: *handle,
                            data: value,
                        });
                        return;
                    }
                }
            }
            _ => {}
        }
    }
}

const fn not_connected(address: BdAddr, handle: u32) -> Event {
    Event::Error {
        address,
        handle,
        error: bluetooth_error::NOT_CONNECTED,
    }
}

fn device_path(adapter: &str, address: BdAddr) -> String {
    format!("{adapter}/dev_{}", address.to_string().replace(':', "_"))
}

/// Connects to a device and resolves its services, for the link `id`.
async fn connect(
    bus: Connection,
    adapter: String,
    device: String,
    address: BdAddr,
    address_type: Option<AddressType>,
    id: u64,
    done: mpsc::UnboundedSender<Finished>,
) {
    info!(%address, "connecting");
    let finished = match establish(&bus, &adapter, &device, address, address_type).await {
        Ok(gatt) => Finished::Connected { address, id, gatt },
        Err(err) => {
            warn!(%address, %err, "connecting failed");
            // Don't leave BlueZ connecting in the background.
            let _ = bus
                .call_method(
                    Some(BLUEZ),
                    device.as_str(),
                    Some(DEVICE),
                    "Disconnect",
                    &(),
                )
                .await;
            Finished::Failed { address, id }
        }
    };
    let _ = done.send(finished);
}

async fn establish(
    bus: &Connection,
    adapter: &str,
    device: &str,
    address: BdAddr,
    address_type: Option<AddressType>,
) -> Result<Gatt, BluezError> {
    // Subscribe first so that no change is missed.
    let mut changes = signals(bus, PROPERTIES, "PropertiesChanged", device).await?;
    let properties = match device_properties(bus, device).await {
        Err(BluezError::DBus(err)) if is_unknown_object(&err) => {
            // BlueZ hasn't seen the device (lately); connect by address.
            debug!(%address, "unknown device, connecting by address");
            let kind = match address_type {
                Some(AddressType::Random) => "random",
                Some(AddressType::Public) | None => "public",
            };
            let properties = HashMap::from([
                ("Address", Variant::from(address.to_string())),
                ("AddressType", Variant::from(kind)),
            ]);
            bus.call_method(
                Some(BLUEZ),
                adapter,
                Some(ADAPTER),
                "ConnectDevice",
                &(properties,),
            )
            .await?;
            device_properties(bus, device).await?
        }
        other => other?,
    };
    if !flag(&properties, "Connected") {
        bus.call_method(Some(BLUEZ), device, Some(DEVICE), "Connect", &())
            .await?;
    }
    let mut resolved = flag(&device_properties(bus, device).await?, "ServicesResolved");
    while !resolved {
        let message = changes.next().await.ok_or(BluezError::Disconnected)??;
        let (interface, properties, _): (String, HashMap<String, OwnedVariant>, Vec<String>) =
            message.body().deserialize()?;
        if interface != DEVICE {
            continue;
        }
        if properties
            .get("Connected")
            .is_some_and(|value| matches!(unwrap(value), Variant::Bool(false)))
        {
            return Err(BluezError::ConnectionLost);
        }
        resolved = flag(&properties, "ServicesResolved");
    }
    let objects: ManagedObjects = bus
        .call_method(
            Some(BLUEZ),
            "/",
            Some(OBJECT_MANAGER),
            "GetManagedObjects",
            &(),
        )
        .await?
        .body()
        .deserialize()?;
    Ok(Gatt::new(device, &objects))
}

async fn device_properties(
    bus: &Connection,
    device: &str,
) -> Result<HashMap<String, OwnedVariant>, BluezError> {
    Ok(bus
        .call_method(Some(BLUEZ), device, Some(PROPERTIES), "GetAll", &(DEVICE,))
        .await?
        .body()
        .deserialize()?)
}

fn is_unknown_object(err: &zbus::Error) -> bool {
    matches!(
        err,
        zbus::Error::FDO(err) if matches!(
            **err,
            zbus::fdo::Error::UnknownObject(_)
                | zbus::fdo::Error::UnknownMethod(_)
                | zbus::fdo::Error::UnknownInterface(_)
        )
    )
}

fn flag(properties: &HashMap<String, OwnedVariant>, name: &str) -> bool {
    properties
        .get(name)
        .is_some_and(|value| matches!(unwrap(value), Variant::Bool(true)))
}

/// An operation for a link's worker.
#[derive(Debug)]
enum Operation {
    Read {
        handle: u32,
        path: OwnedObjectPath,
    },
    Write {
        handle: u32,
        path: OwnedObjectPath,
        data: Vec<u8>,
        response: bool,
    },
    ReadDescriptor {
        handle: u32,
        path: OwnedObjectPath,
    },
    WriteDescriptor {
        handle: u32,
        path: OwnedObjectPath,
        data: Vec<u8>,
    },
    Notify {
        handle: u32,
        path: OwnedObjectPath,
        enable: bool,
    },
}

/// Runs a link's operations one at a time, so that writes reach the
/// device in the order the client sent them, and reports their results.
async fn work(
    bus: Connection,
    address: BdAddr,
    events: EventSender,
    echoes: Echoes,
    mut operations: mpsc::UnboundedReceiver<Operation>,
) {
    while let Some(operation) = operations.recv().await {
        let handle = operation.handle();
        match perform(&bus, &echoes, operation).await {
            Ok(Some(outcome)) => events.send(outcome.into_event(address, handle)),
            Ok(None) => {}
            Err(err) => {
                debug!(%address, handle, %err, "GATT operation failed");
                events.send(Event::Error {
                    address,
                    handle,
                    error: gatt_error(&err),
                });
            }
        }
    }
}

/// What a successful operation reports.
#[derive(Debug)]
enum Done {
    Read(Vec<u8>),
    Written,
    Notifying,
}

impl Done {
    fn into_event(self, address: BdAddr, handle: u32) -> Event {
        match self {
            Self::Read(data) => Event::Read {
                address,
                handle,
                data,
            },
            Self::Written => Event::Written { address, handle },
            Self::Notifying => Event::Notifying { address, handle },
        }
    }
}

impl Operation {
    const fn handle(&self) -> u32 {
        match self {
            Self::Read { handle, .. }
            | Self::Write { handle, .. }
            | Self::ReadDescriptor { handle, .. }
            | Self::WriteDescriptor { handle, .. }
            | Self::Notify { handle, .. } => *handle,
        }
    }
}

/// Performs one operation, returning what to report, if anything.
async fn perform(
    bus: &Connection,
    echoes: &Echoes,
    operation: Operation,
) -> zbus::Result<Option<Done>> {
    let call = |path: OwnedObjectPath, interface, method| async move {
        bus.call_method(Some(BLUEZ), path.as_str(), Some(interface), method, &())
            .await
    };
    Ok(Some(match operation {
        Operation::Read { path, .. } => {
            echoes.expect(&path);
            let result = read(bus, &path, GATT_CHARACTERISTIC).await;
            if result.is_err() {
                echoes.cancel(&path);
            }
            Done::Read(result?)
        }
        Operation::ReadDescriptor { path, .. } => {
            Done::Read(read(bus, &path, GATT_DESCRIPTOR).await?)
        }
        Operation::Write {
            path,
            data,
            response,
            ..
        } => {
            let kind = if response { "request" } else { "command" };
            write(bus, &path, GATT_CHARACTERISTIC, data, Some(kind)).await?;
            // Writes without response are not confirmed.
            return Ok(response.then_some(Done::Written));
        }
        Operation::WriteDescriptor { path, data, .. } => {
            write(bus, &path, GATT_DESCRIPTOR, data, None).await?;
            Done::Written
        }
        Operation::Notify { path, enable, .. } => {
            let method = if enable { "StartNotify" } else { "StopNotify" };
            call(path, GATT_CHARACTERISTIC, method).await?;
            Done::Notifying
        }
    }))
}

/// Writes a value; `kind` picks a characteristic write's procedure.
async fn write(
    bus: &Connection,
    path: &OwnedObjectPath,
    interface: &str,
    data: Vec<u8>,
    kind: Option<&str>,
) -> zbus::Result<()> {
    let options: HashMap<&str, Variant<'_>> = kind
        .map(|kind| ("type", Variant::from(kind)))
        .into_iter()
        .collect();
    bus.call_method(
        Some(BLUEZ),
        path.as_str(),
        Some(interface),
        "WriteValue",
        &(data, options),
    )
    .await?;
    Ok(())
}

async fn read(bus: &Connection, path: &OwnedObjectPath, interface: &str) -> zbus::Result<Vec<u8>> {
    let options: HashMap<&str, Variant<'_>> = HashMap::new();
    bus.call_method(
        Some(BLUEZ),
        path.as_str(),
        Some(interface),
        "ReadValue",
        &(options,),
    )
    .await?
    .body()
    .deserialize()
}

/// Translates a BlueZ error into the ATT error code it stands for, as
/// ESPHome reports them.
fn gatt_error(err: &zbus::Error) -> i32 {
    use bluetooth_error as e;
    let zbus::Error::MethodError(name, detail, _) = err else {
        return e::ERROR;
    };
    let detail = detail.as_deref().unwrap_or_default();
    match name.as_str() {
        "org.bluez.Error.NotPermitted" => match detail {
            "Read not permitted" => e::READ_NOT_PERMITTED,
            "Write not permitted" => e::WRITE_NOT_PERMITTED,
            // "Not paired": insufficient authentication or encryption.
            _ => e::INSUFFICIENT_AUTHENTICATION,
        },
        "org.bluez.Error.NotAuthorized" => e::INSUFFICIENT_AUTHORIZATION,
        "org.bluez.Error.NotSupported" => e::REQUEST_NOT_SUPPORTED,
        "org.bluez.Error.InvalidArguments" if detail == "Invalid offset" => e::INVALID_OFFSET,
        "org.bluez.Error.InvalidArguments" => e::INVALID_ATTRIBUTE_LENGTH,
        "org.bluez.Error.Failed" if detail == "Not connected" => e::NOT_CONNECTED,
        "org.bluez.Error.Failed" => detail
            .strip_prefix("Operation failed with ATT error: 0x")
            .and_then(|code| i32::from_str_radix(code, 16).ok())
            .unwrap_or(e::ERROR),
        _ => e::ERROR,
    }
}

/// Reads whose `Value` change hasn't been seen yet, per characteristic.
#[derive(Debug, Clone, Default)]
struct Echoes(Arc<Mutex<HashMap<OwnedObjectPath, u32>>>);

impl Echoes {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<OwnedObjectPath, u32>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Notes a read about to start.
    fn expect(&self, path: &OwnedObjectPath) {
        *self.lock().entry(path.clone()).or_default() += 1;
    }

    /// Notes a read that failed, and so changes nothing.
    fn cancel(&self, path: &OwnedObjectPath) {
        self.take(path.as_str());
    }

    /// Returns whether a change of `path`'s value comes from a read, and
    /// consumes it if so.
    fn take(&self, path: &str) -> bool {
        let mut echoes = self.lock();
        let Some((key, count)) = echoes.iter_mut().find(|(key, _)| key.as_str() == path) else {
            return false;
        };
        *count -= 1;
        if *count == 0 {
            let key = key.clone();
            echoes.remove(&key);
        }
        true
    }
}

/// What kind of attribute a handle names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Characteristic,
    Descriptor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Attribute {
    kind: Kind,
    uuid: Uuid,
    path: OwnedObjectPath,
}

/// A connected device's GATT database, as BlueZ resolved it.
#[derive(Debug, Default)]
struct Gatt {
    services: Vec<Service>,
    /// Characteristics (by value handle) and descriptors.
    attributes: HashMap<u32, Attribute>,
    /// Value handles of characteristics, by object path.
    characteristics: HashMap<String, u32>,
    /// MTU of the connection, if BlueZ reports it.
    mtu: Option<u16>,
}

impl Gatt {
    /// Collects the services of `device` from BlueZ's objects.
    fn new(device: &str, objects: &ManagedObjects) -> Self {
        let prefix = format!("{device}/");
        let mut gatt = Self::default();
        let mut services = BTreeMap::new();
        let mut characteristics = BTreeMap::new();
        let mut descriptors = BTreeMap::new();
        for (path, interfaces) in objects {
            let path_str = path.as_str();
            if !path_str.starts_with(&prefix) {
                continue;
            }
            let (parent, name) = path_str.rsplit_once('/').unwrap_or_default();
            let uuid = |properties: &HashMap<String, OwnedVariant>| {
                properties
                    .get("UUID")
                    .and_then(|value| string(value))
                    .and_then(|uuid| uuid.parse::<Uuid>().ok())
            };
            if let Some(properties) = interfaces.get(GATT_SERVICE)
                && let (Some(handle), Some(uuid)) = (path_handle(name, "service"), uuid(properties))
            {
                services.insert(path_str, (handle, uuid));
            } else if let Some(properties) = interfaces.get(GATT_CHARACTERISTIC)
                && let (Some(declaration), Some(uuid)) =
                    (path_handle(name, "char"), uuid(properties))
            {
                let handle = declaration.saturating_add(1);
                if gatt.mtu.is_none() {
                    gatt.mtu = properties.get("MTU").and_then(|value| match unwrap(value) {
                        Variant::U16(mtu) => Some(*mtu),
                        _ => None,
                    });
                }
                let flags = properties.get("Flags").map(|value| strings(value));
                characteristics.insert(
                    path_str,
                    (
                        parent,
                        handle,
                        uuid,
                        properties_mask(&flags.unwrap_or_default()),
                    ),
                );
                gatt.characteristics
                    .insert(path_str.to_owned(), handle.into());
                gatt.attributes.insert(
                    handle.into(),
                    Attribute {
                        kind: Kind::Characteristic,
                        uuid,
                        path: path.clone(),
                    },
                );
            } else if let Some(properties) = interfaces.get(GATT_DESCRIPTOR)
                && let (Some(handle), Some(uuid)) = (path_handle(name, "desc"), uuid(properties))
            {
                descriptors.insert(path_str, (parent, handle, uuid));
                gatt.attributes.insert(
                    handle.into(),
                    Attribute {
                        kind: Kind::Descriptor,
                        uuid,
                        path: path.clone(),
                    },
                );
            }
        }
        let mut by_service: HashMap<&str, Vec<Characteristic>> = HashMap::new();
        for (path, (service, handle, uuid, properties)) in &characteristics {
            let mut list: Vec<Descriptor> = descriptors
                .values()
                .filter(|(parent, ..)| parent == path)
                .map(|(_, handle, uuid)| Descriptor {
                    uuid: *uuid,
                    handle: *handle,
                })
                .collect();
            list.sort_by_key(|descriptor| descriptor.handle);
            by_service.entry(service).or_default().push(Characteristic {
                uuid: *uuid,
                handle: *handle,
                properties: *properties,
                descriptors: list,
            });
        }
        gatt.services = services
            .into_iter()
            .map(|(path, (handle, uuid))| {
                let mut characteristics = by_service.remove(path).unwrap_or_default();
                characteristics.sort_by_key(|characteristic| characteristic.handle);
                Service {
                    uuid,
                    handle,
                    characteristics,
                }
            })
            .collect();
        gatt.services.sort_by_key(|service| service.handle);
        gatt
    }
}

/// Reads the handle at the end of a GATT object's path, such as `0029`
/// in `char0029`.
fn path_handle(name: &str, kind: &str) -> Option<u16> {
    u16::from_str_radix(name.strip_prefix(kind)?, 16).ok()
}

/// Turns BlueZ's characteristic flags into the GATT properties bit mask.
fn properties_mask(flags: &[String]) -> u8 {
    flags
        .iter()
        .map(|flag| match flag.as_str() {
            "broadcast" => 0x01,
            "read" => 0x02,
            "write-without-response" => 0x04,
            "write" => 0x08,
            "notify" => 0x10,
            "indicate" => 0x20,
            "authenticated-signed-writes" => 0x40,
            "extended-properties" => 0x80,
            _ => 0,
        })
        .fold(0, |mask, bit| mask | bit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variant(value: impl Into<Variant<'static>>) -> OwnedVariant {
        OwnedVariant::try_from(&Variant::new(value.into())).unwrap()
    }

    fn object(
        interface: &str,
        properties: &[(&str, OwnedVariant)],
    ) -> HashMap<String, HashMap<String, OwnedVariant>> {
        HashMap::from([(
            interface.to_owned(),
            properties
                .iter()
                .map(|(key, value)| ((*key).to_owned(), value.try_clone().unwrap()))
                .collect(),
        )])
    }

    const DEVICE_PATH: &str = "/org/bluez/hci0/dev_A4_C1_38_00_11_22";

    /// A battery service with a notifying level characteristic, as BlueZ
    /// exports it, next to another device's service.
    fn objects() -> ManagedObjects {
        let path =
            |suffix: &str| OwnedObjectPath::try_from(format!("{DEVICE_PATH}{suffix}")).unwrap();
        let uuid = |alias: u16| variant(Uuid::from_u16(alias).to_string());
        HashMap::from([
            (
                path("/service0028"),
                object(GATT_SERVICE, &[("UUID", uuid(0x180F))]),
            ),
            (
                path("/service0028/char0029"),
                object(
                    GATT_CHARACTERISTIC,
                    &[
                        ("UUID", uuid(0x2A19)),
                        (
                            "Flags",
                            variant(vec!["read".to_owned(), "notify".to_owned()]),
                        ),
                        ("MTU", variant(247_u16)),
                    ],
                ),
            ),
            (
                path("/service0028/char0029/desc002b"),
                object(GATT_DESCRIPTOR, &[("UUID", uuid(0x2902))]),
            ),
            (
                path("/service0010"),
                object(GATT_SERVICE, &[("UUID", uuid(0x1801))]),
            ),
            (
                OwnedObjectPath::try_from("/org/bluez/hci0/dev_11_22_33_44_55_66/service0001")
                    .unwrap(),
                object(GATT_SERVICE, &[("UUID", uuid(0x1800))]),
            ),
        ])
    }

    #[test]
    fn maps_bluez_objects_to_services_by_value_handle() {
        let gatt = Gatt::new(DEVICE_PATH, &objects());
        assert_eq!(
            gatt.services,
            [
                Service {
                    uuid: Uuid::from_u16(0x1801),
                    handle: 0x10,
                    characteristics: Vec::new(),
                },
                Service {
                    uuid: Uuid::from_u16(0x180F),
                    handle: 0x28,
                    characteristics: vec![Characteristic {
                        uuid: Uuid::from_u16(0x2A19),
                        handle: 0x2A,
                        properties: 0x12,
                        descriptors: vec![Descriptor {
                            uuid: CCCD,
                            handle: 0x2B,
                        }],
                    }],
                },
            ]
        );
        assert_eq!(gatt.mtu, Some(247));
        let level = &gatt.attributes[&0x2A];
        assert_eq!(level.kind, Kind::Characteristic);
        assert_eq!(
            level.path.as_str(),
            format!("{DEVICE_PATH}/service0028/char0029")
        );
        assert_eq!(gatt.attributes[&0x2B].kind, Kind::Descriptor);
        assert_eq!(
            gatt.characteristics[&format!("{DEVICE_PATH}/service0028/char0029")],
            0x2A
        );
        // Declarations are not addressable.
        assert!(!gatt.attributes.contains_key(&0x29));
    }

    #[test]
    fn converts_flags_to_properties() {
        let flags = [
            "read",
            "write-without-response",
            "write",
            "indicate",
            "encrypt-read",
        ]
        .map(str::to_owned);
        assert_eq!(properties_mask(&flags), 0x2E);
        assert_eq!(properties_mask(&[]), 0);
    }

    fn method_error(name: &str, detail: &str) -> zbus::Error {
        let message = zbus::Message::method_call("/", "Ping").build(&()).unwrap();
        zbus::Error::MethodError(
            zbus::names::OwnedErrorName::try_from(name).unwrap(),
            Some(detail.to_owned()),
            message,
        )
    }

    #[test]
    fn maps_bluez_errors_to_att_errors() {
        let cases = [
            ("org.bluez.Error.NotPermitted", "Read not permitted", 0x02),
            ("org.bluez.Error.NotPermitted", "Write not permitted", 0x03),
            ("org.bluez.Error.NotPermitted", "Not paired", 0x05),
            (
                "org.bluez.Error.NotAuthorized",
                "Operation Not Authorized",
                0x08,
            ),
            (
                "org.bluez.Error.NotSupported",
                "Operation is not supported",
                0x06,
            ),
            ("org.bluez.Error.InvalidArguments", "Invalid offset", 0x07),
            ("org.bluez.Error.InvalidArguments", "Invalid Length", 0x0D),
            (
                "org.bluez.Error.Failed",
                "Operation failed with ATT error: 0x0e",
                0x0E,
            ),
            ("org.bluez.Error.Failed", "Not connected", -1),
            ("org.bluez.Error.Failed", "Operation failed", 0x85),
            ("org.bluez.Error.InProgress", "In Progress", 0x85),
        ];
        for (name, detail, expected) in cases {
            assert_eq!(
                gatt_error(&method_error(name, detail)),
                expected,
                "{name}: {detail}"
            );
        }
        assert_eq!(gatt_error(&zbus::Error::InvalidReply), 0x85);
    }

    #[test]
    fn counts_read_echoes_per_characteristic() {
        let echoes = Echoes::default();
        let path =
            OwnedObjectPath::try_from(format!("{DEVICE_PATH}/service0028/char0029")).unwrap();
        assert!(!echoes.take(path.as_str()));
        echoes.expect(&path);
        echoes.expect(&path);
        echoes.cancel(&path);
        assert!(echoes.take(path.as_str()));
        assert!(!echoes.take(path.as_str()), "only one read is pending");
    }

    #[test]
    fn derives_device_paths() {
        let address = BdAddr::new([0xA4, 0xC1, 0x38, 0x00, 0x11, 0x22]);
        assert_eq!(device_path("/org/bluez/hci0", address), DEVICE_PATH);
    }
}
