use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::Duration;
use std::time::Instant;

use core_foundation::date::CFAbsoluteTimeGetCurrent;
use core_foundation::runloop::kCFRunLoopDefaultMode;
use core_foundation::runloop::CFRunLoop;
use core_foundation::runloop::CFRunLoopTimer;
use core_foundation::runloop::CFRunLoopTimerRef;
use coremidi::AnyObject;
use coremidi::Destination as CoreMidiDestination;
use coremidi::InputPort as CoreMidiInputPort;
use coremidi::Notification;
use coremidi::OutputPort as CoreMidiOutputPort;
use coremidi::PacketBuffer;
use coremidi::Source as CoreMidiSource;
use coremidi::VirtualDestination as CoreMidiVirtualDestination;
use coremidi::VirtualSource as CoreMidiVirtualSource;
use coremidi_sys::MIDIEndpointGetEntity;
use coremidi_sys::MIDIObjectFindByUniqueID;
#[cfg(test)]
use futures_channel::mpsc;
use futures_channel::oneshot;

use super::common::prune_send;
use super::common::Command;
use super::common::DestinationSubscribers;
use super::common::SourceSubscribers;
use super::common::StreamReceivers;
use super::common::StreamSenders;
#[cfg(test)]
use super::common::INBOUND_CHANNEL_CAPACITY;
use super::log_error;
use super::log_warn;
use super::MutexExt;
#[cfg(test)]
use crate::midi::stream_parser::DecodedEvent;
use crate::midi::stream_parser::StreamParser;
use crate::name::Name;
use crate::port::PortIdInner;
use crate::Destination;
use crate::DestinationChange;
use crate::Error;
use crate::IoError;
use crate::PlatformError;
use crate::PortId;
use crate::RawMidiMessage;
use crate::Source;
use crate::SourceChange;
#[cfg(test)]
use crate::Timed;

const COREMIDI_KEEPALIVE_SECS: f64 = 1.0e9;

#[repr(C)]
struct MachTimebaseInfo {
    numer: u32,
    denom: u32,
}

extern "C" {
    fn mach_absolute_time() -> u64;
    fn mach_timebase_info(info: *mut MachTimebaseInfo) -> i32;
}

fn current_host_time() -> u64 {
    unsafe { mach_absolute_time() }
}

fn host_time_to_nanos(host_time: u64) -> u64 {
    static TIMEBASE: OnceLock<(u32, u32)> = OnceLock::new();
    let (numer, denom) = *TIMEBASE.get_or_init(|| {
        let mut tb = MachTimebaseInfo { numer: 0, denom: 0 };
        let ret = unsafe { mach_timebase_info(&mut tb) };
        debug_assert_eq!(ret, 0);
        (tb.numer, tb.denom.max(1))
    });
    (host_time as u128 * numer as u128 / denom as u128) as u64
}

#[derive(Clone, Copy)]
struct TimeCalibration {
    ref_host_time: u64,
    ref_instant: Instant,
}

impl TimeCalibration {
    fn capture() -> Self {
        let ref_instant = Instant::now();
        let ref_host_time = current_host_time();
        Self {
            ref_host_time,
            ref_instant,
        }
    }

    fn to_instant(self, host_time: u64) -> Instant {
        if host_time == 0 {
            return Instant::now();
        }
        if host_time >= self.ref_host_time {
            let delta_ticks = host_time - self.ref_host_time;
            let delta_ns = host_time_to_nanos(delta_ticks);
            self.ref_instant
                .checked_add(Duration::from_nanos(delta_ns))
                .unwrap_or(self.ref_instant)
        } else {
            let delta_ticks = self.ref_host_time - host_time;
            let delta_ns = host_time_to_nanos(delta_ticks);
            self.ref_instant
                .checked_sub(Duration::from_nanos(delta_ns))
                .unwrap_or(self.ref_instant)
        }
    }
}

type DisconnectFn = Box<dyn FnOnce() + Send>;
type ClientDisconnectHandlers = HashMap<i32, Vec<(u64, DisconnectFn)>>;
type SourceSubscriberMap = Arc<Mutex<HashMap<u64, SourceSubscribers>>>;
type DestinationSubscriberMap = Arc<Mutex<HashMap<u64, DestinationSubscribers>>>;

#[derive(Clone)]
struct GlobalContext {
    senders: SourceSubscriberMap,
    output_senders: DestinationSubscriberMap,
    source_cache: Arc<Mutex<HashMap<i32, (CoreMidiSource, Source)>>>,
    dest_cache: Arc<Mutex<HashMap<i32, (CoreMidiDestination, Destination)>>>,
    disconnect_handlers: Arc<Mutex<ClientDisconnectHandlers>>,
    output_disconnect_handlers: Arc<Mutex<ClientDisconnectHandlers>>,
    command_senders: Arc<Mutex<HashMap<u64, std::sync::mpsc::SyncSender<Command>>>>,
}

fn remove_client_handlers(handlers: &mut ClientDisconnectHandlers, uid: i32, client_id: u64) {
    if let Some(v) = handlers.get_mut(&uid) {
        v.retain(|(cid, _)| *cid != client_id);
        if v.is_empty() {
            handlers.remove(&uid);
        }
    }
}

impl GlobalContext {
    fn notify_source_subscribers(&self, change: SourceChange) {
        let senders = self.senders.lock_unpoisoned().clone();
        for subs in senders.values() {
            let mut subs_lock = subs.lock_unpoisoned();
            prune_send(&mut subs_lock, &change);
        }
    }

    fn signal_backend_death(&self) {
        let senders: Vec<_> = self
            .command_senders
            .lock_unpoisoned()
            .values()
            .cloned()
            .collect();
        for tx in senders {
            let _ = tx.try_send(Command::BackendDied);
        }
    }

    fn notify_destination_subscribers(&self, change: DestinationChange) {
        let senders = self.output_senders.lock_unpoisoned().clone();
        for subs in senders.values() {
            let mut subs_lock = subs.lock_unpoisoned();
            prune_send(&mut subs_lock, &change);
        }
    }
}

struct GlobalIo {
    #[allow(dead_code)]
    io_client: coremidi::Client,
    ctx: GlobalContext,
    next_id: AtomicU64,
    calibration: TimeCalibration,
}

static GLOBAL_IO: Mutex<Option<Arc<GlobalIo>>> = Mutex::new(None);

extern "C" fn keep_alive_noop(_timer: CFRunLoopTimerRef, _info: *mut std::ffi::c_void) {}

fn init_global_io() -> Result<GlobalIo, PlatformError> {
    let ctx = GlobalContext {
        senders: Arc::new(Mutex::new(HashMap::new())),
        output_senders: Arc::new(Mutex::new(HashMap::new())),
        source_cache: Arc::new(Mutex::new(HashMap::new())),
        dest_cache: Arc::new(Mutex::new(HashMap::new())),
        disconnect_handlers: Arc::new(Mutex::new(HashMap::new())),
        output_disconnect_handlers: Arc::new(Mutex::new(HashMap::new())),
        command_senders: Arc::new(Mutex::new(HashMap::new())),
    };

    let ctx_thread = ctx.clone();
    let (client_tx, client_rx) = std::sync::mpsc::channel::<Result<coremidi::Client, i32>>();

    if std::thread::Builder::new()
        .name("midi-io-global".to_string())
        .spawn(move || {
            *ctx_thread.source_cache.lock_unpoisoned() = init_source_cache();
            *ctx_thread.dest_cache.lock_unpoisoned() = init_dest_cache();

            let io_client = coremidi::Client::new_with_notifications(
                "global",
                move |notification: &Notification| {
                    on_global_notification(&ctx_thread, notification);
                },
            );
            if let Ok(c) = io_client {
                let _ = client_tx.send(Ok(c));
            } else {
                let _ = client_tx.send(Err(io_client.unwrap_err()));
                return;
            }

            let keep_alive = CFRunLoopTimer::new(
                unsafe { CFAbsoluteTimeGetCurrent() } + COREMIDI_KEEPALIVE_SECS,
                COREMIDI_KEEPALIVE_SECS,
                0,
                0,
                keep_alive_noop,
                std::ptr::null_mut(),
            );
            CFRunLoop::get_current().add_timer(&keep_alive, unsafe { kCFRunLoopDefaultMode });
            loop {
                CFRunLoop::run_current();
                log_warn!("CoreMIDI global run loop returned unexpectedly; re-entering");
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        })
        .is_err()
    {
        return Err(PlatformError::ThreadInit);
    }

    match client_rx.recv() {
        Ok(Ok(io_client)) => Ok(GlobalIo {
            io_client,
            ctx,
            next_id: AtomicU64::new(0),
            calibration: TimeCalibration::capture(),
        }),
        Ok(Err(status)) => Err(PlatformError::ClientInit(status)),
        Err(_) => Err(PlatformError::ThreadInit),
    }
}

fn ensure_global_io() -> Result<Arc<GlobalIo>, Error> {
    let mut guard = GLOBAL_IO.lock_unpoisoned();
    if let Some(io) = guard.as_ref() {
        return Ok(Arc::clone(io));
    }
    let io = Arc::new(init_global_io().map_err(IoError::Platform)?);
    *guard = Some(Arc::clone(&io));
    Ok(io)
}

fn endpoint_is_virtual(uid: i32) -> bool {
    let mut obj_ref: coremidi_sys::MIDIObjectRef = 0;
    let mut obj_type: coremidi_sys::MIDIObjectType = 0;
    let found = unsafe { MIDIObjectFindByUniqueID(uid, &mut obj_ref, &mut obj_type) };
    if found != 0 {
        return false;
    }
    let mut entity_ref: coremidi_sys::MIDIEntityRef = 0;
    let status = unsafe { MIDIEndpointGetEntity(obj_ref, &mut entity_ref) };
    status != 0 || entity_ref == 0
}

fn coremidi_dest_to_destination(dest: &CoreMidiDestination) -> Option<(i32, Destination)> {
    let name = dest.display_name().or_else(|| dest.name())?;
    let uid = dest.unique_id()? as i32;
    let port = Destination {
        id: PortId(PortIdInner::CoreMidi(uid)),
        name,
        is_virtual: endpoint_is_virtual(uid),
    };
    Some((uid, port))
}

fn init_dest_cache() -> HashMap<i32, (CoreMidiDestination, Destination)> {
    coremidi::Destinations
        .into_iter()
        .filter_map(|d| {
            let (uid, port) = coremidi_dest_to_destination(&d)?;
            Some((uid, (d, port)))
        })
        .collect()
}

fn on_dest_added(ctx: &GlobalContext, dest: &CoreMidiDestination) {
    let (uid, port) = match coremidi_dest_to_destination(dest) {
        Some(p) => p,
        None => return,
    };
    {
        let mut cache = ctx.dest_cache.lock_unpoisoned();
        if cache.contains_key(&uid) {
            return;
        }
        cache.insert(uid, (dest.clone(), port.clone()));
    }
    ctx.notify_destination_subscribers(DestinationChange::Added(port));
}

fn on_dest_removed(ctx: &GlobalContext, dest: &CoreMidiDestination) {
    let pair = {
        let mut cache = ctx.dest_cache.lock_unpoisoned();
        let uid = dest.unique_id().map(|id| id as i32).or_else(|| {
            let live_uids: std::collections::HashSet<i32> = coremidi::Destinations
                .into_iter()
                .filter_map(|d| d.unique_id().map(|id| id as i32))
                .collect();
            find_stale_uid(cache.keys().copied(), &live_uids)
        });
        uid.and_then(|uid| cache.remove(&uid).map(|(_, p)| (uid, p)))
    };
    let (uid, port) = match pair {
        Some(p) => p,
        None => return,
    };
    let fns = ctx
        .output_disconnect_handlers
        .lock_unpoisoned()
        .remove(&uid);
    if let Some(fns) = fns {
        for (_, f) in fns {
            f();
        }
    }
    ctx.notify_destination_subscribers(DestinationChange::Removed(port));
}

fn coremidi_source_to_source(source: &CoreMidiSource) -> Option<(i32, Source)> {
    let name = source.display_name().or_else(|| source.name())?;
    let uid = source.unique_id()? as i32;
    let port = Source {
        id: PortId(PortIdInner::CoreMidi(uid)),
        name,
        is_virtual: endpoint_is_virtual(uid),
    };
    Some((uid, port))
}

fn init_source_cache() -> HashMap<i32, (CoreMidiSource, Source)> {
    coremidi::Sources
        .into_iter()
        .filter_map(|s| {
            let (uid, port) = coremidi_source_to_source(&s)?;
            Some((uid, (s, port)))
        })
        .collect()
}

fn on_global_notification(ctx: &GlobalContext, notification: &Notification) {
    match notification {
        Notification::ObjectAdded(info) => match &info.child {
            AnyObject::Source(source) | AnyObject::ExternalSource(source) => {
                on_source_added(ctx, source);
            }
            AnyObject::Destination(dest) | AnyObject::ExternalDestination(dest) => {
                on_dest_added(ctx, dest);
            }
            _ => {}
        },
        Notification::ObjectRemoved(info) => match &info.child {
            AnyObject::Source(source) | AnyObject::ExternalSource(source) => {
                on_source_removed(ctx, source);
            }
            AnyObject::Destination(dest) | AnyObject::ExternalDestination(dest) => {
                on_dest_removed(ctx, dest);
            }
            _ => {}
        },
        Notification::IoError(e) => {
            log_error!("{e:?}");
            ctx.signal_backend_death()
        }
        Notification::SetupChanged => {}
        Notification::PropertyChanged(_) => {}
        Notification::ThruConnectionsChanged => {}
        Notification::SerialPortOwnerChanged => {}
    }
}

fn on_source_added(ctx: &GlobalContext, source: &CoreMidiSource) {
    let (uid, port) = match coremidi_source_to_source(source) {
        Some(p) => p,
        None => return,
    };
    {
        let mut cache = ctx.source_cache.lock_unpoisoned();
        if cache.contains_key(&uid) {
            return;
        }
        cache.insert(uid, (source.clone(), port.clone()));
    }
    ctx.notify_source_subscribers(SourceChange::Added(port));
}

fn find_stale_uid(
    mut cached_uids: impl Iterator<Item = i32>,
    live_uids: &std::collections::HashSet<i32>,
) -> Option<i32> {
    cached_uids.find(|uid| !live_uids.contains(uid))
}

fn on_source_removed(ctx: &GlobalContext, source: &CoreMidiSource) {
    let pair = {
        let mut cache = ctx.source_cache.lock_unpoisoned();
        let uid = source.unique_id().map(|id| id as i32).or_else(|| {
            let live_uids: std::collections::HashSet<i32> = coremidi::Sources
                .into_iter()
                .filter_map(|s| s.unique_id().map(|id| id as i32))
                .collect();
            find_stale_uid(cache.keys().copied(), &live_uids)
        });
        uid.and_then(|uid| cache.remove(&uid).map(|(_, p)| (uid, p)))
    };
    let (uid, port) = match pair {
        Some(p) => p,
        None => return,
    };
    let fns = ctx.disconnect_handlers.lock_unpoisoned().remove(&uid);
    if let Some(fns) = fns {
        for (_, f) in fns {
            f();
        }
    }
    ctx.notify_source_subscribers(SourceChange::Removed(port));
}

struct ConnectionState {
    _source_port: CoreMidiInputPort,
}

struct DestinationConnectionState {
    destination_port: CoreMidiOutputPort,
    destination: CoreMidiDestination,
}

pub(super) struct Backend {
    client_id: u64,
    connections: Arc<Mutex<HashMap<i32, ConnectionState>>>,
    global: Arc<GlobalIo>,
}

impl Backend {
    pub(super) fn start(
        name: Name,
        source_subs: SourceSubscribers,
        destination_subs: DestinationSubscribers,
        cmd_rx: std::sync::mpsc::Receiver<Command>,
        cmd_tx: &std::sync::mpsc::SyncSender<Command>,
        ready_tx: oneshot::Sender<Result<(), Error>>,
    ) -> Result<Self, Error> {
        let global: Arc<GlobalIo> = ensure_global_io()?;

        let client_id = global.next_id.fetch_add(1, Ordering::Relaxed);
        global
            .ctx
            .senders
            .lock_unpoisoned()
            .insert(client_id, Arc::clone(&source_subs));
        global
            .ctx
            .output_senders
            .lock_unpoisoned()
            .insert(client_id, Arc::clone(&destination_subs));

        let connections: Arc<Mutex<HashMap<i32, ConnectionState>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let connections_bg = connections.clone();

        let disconnected_outputs: Arc<Mutex<std::collections::HashSet<i32>>> =
            Arc::new(Mutex::new(std::collections::HashSet::new()));
        let disconnected_outputs_bg = disconnected_outputs.clone();

        let source_streams: Arc<Mutex<HashMap<i32, StreamSenders>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let source_streams_bg = source_streams.clone();

        let global_bg = Arc::clone(&global);
        std::thread::Builder::new()
            .name(format!("midi-io-cmd-{}", name.as_str()))
            .spawn(move || {
                let client = match coremidi::Client::new(name.as_str()) {
                    Ok(c) => c,
                    Err(status) => {
                        let _ = ready_tx.send(Err(Error::from(IoError::Platform(
                            PlatformError::ClientInit(status),
                        ))));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(()));
                let mut destination_connections: HashMap<i32, DestinationConnectionState> =
                    HashMap::new();
                let mut virtual_sources: HashMap<u64, CoreMidiVirtualSource> = HashMap::new();
                let mut virtual_destinations: HashMap<
                    u64,
                    (CoreMidiVirtualDestination, StreamSenders),
                > = HashMap::new();
                while let Ok(cmd) = cmd_rx.recv() {
                    match cmd {
                        Command::Disconnect(port_id) => {
                            let PortIdInner::CoreMidi(uid) = port_id.0;
                            connections_bg.lock_unpoisoned().remove(&uid);
                            source_streams_bg.lock_unpoisoned().remove(&uid);
                            remove_client_handlers(
                                &mut global_bg.ctx.disconnect_handlers.lock_unpoisoned(),
                                uid,
                                client_id,
                            );
                        }
                        Command::ConnectSource { port_id, reply } => {
                            let PortIdInner::CoreMidi(uid) = port_id.0;
                            let result = connect_source(
                                uid,
                                &connections_bg,
                                &source_streams_bg,
                                &global_bg,
                                &client,
                                client_id,
                            );
                            let _ = reply.send(result);
                        }
                        Command::Shutdown => {
                            break;
                        }
                        Command::BackendDied => {
                            drain_streams_backend_died(&source_streams_bg.lock_unpoisoned());
                            for (_, senders) in virtual_destinations.values() {
                                senders.lifecycle_error(IoError::BackendThreadDied);
                            }
                        }
                        Command::ConnectDestination { port_id, reply } => {
                            let result = handle_connect_destination(
                                &port_id,
                                &mut destination_connections,
                                &global_bg,
                                &client,
                                disconnected_outputs_bg.clone(),
                                client_id,
                            );
                            let _ = reply.send(result);
                        }
                        Command::SendMidi {
                            port_id,
                            msg,
                            reply,
                        } => {
                            let result = handle_send_midi(
                                &port_id,
                                msg,
                                &destination_connections,
                                &disconnected_outputs_bg,
                            );
                            let _ = reply.send(result);
                        }
                        Command::SendSysex {
                            port_id,
                            data,
                            reply,
                        } => {
                            let result = handle_send_sysex(
                                &port_id,
                                &data,
                                &destination_connections,
                                &disconnected_outputs_bg,
                            );
                            let _ = reply.send(result);
                        }
                        Command::DisconnectDestination(port_id) => {
                            let PortIdInner::CoreMidi(uid) = port_id.0;
                            destination_connections.remove(&uid);
                            disconnected_outputs_bg.lock_unpoisoned().remove(&uid);
                            remove_client_handlers(
                                &mut global_bg.ctx.output_disconnect_handlers.lock_unpoisoned(),
                                uid,
                                client_id,
                            );
                        }
                        Command::CreateVirtualSource { id, name, reply } => {
                            match client.virtual_source(name.as_str()) {
                                Ok(vsource) => match vsource.unique_id().map(|uid| uid as i32) {
                                    Some(uid) => {
                                        virtual_sources.insert(id.0, vsource);
                                        if let Some(src) = coremidi::Sources
                                            .into_iter()
                                            .find(|s| s.unique_id().map(|u| u as i32) == Some(uid))
                                        {
                                            on_source_added(&global_bg.ctx, &src);
                                        }
                                        let _ = reply.send(Ok(PortId(PortIdInner::CoreMidi(uid))));
                                    }
                                    None => {
                                        let _ = reply.send(Err(Error::from(IoError::PortNotFound)));
                                    }
                                },
                                Err(status) => {
                                    let _ = reply.send(Err(Error::from(IoError::Platform(
                                        PlatformError::VirtualPortCreate(status),
                                    ))));
                                }
                            }
                        }
                        Command::DestroyVirtualSource(id) => {
                            virtual_sources.remove(&id.0);
                        }
                        Command::CreateVirtualDestination { id, name, reply } => {
                            let (senders, receivers) = StreamSenders::channel();

                            let notify_senders = senders.clone();
                            let cb_senders = senders;
                            let mut parser = StreamParser::new();
                            let cal = global_bg.calibration;

                            match client.virtual_destination(name.as_str(), move |packet_list| {
                                for packet in packet_list.iter() {
                                    let ts = cal.to_instant(packet.timestamp());
                                    parser.push(packet.data(), &mut |event| {
                                        cb_senders.emit(ts, event);
                                    });
                                }
                            }) {
                                Ok(vdest) => match vdest.unique_id().map(|uid| uid as i32) {
                                    Some(uid) => {
                                        virtual_destinations.insert(id.0, (vdest, notify_senders));
                                        if let Some(dest) = coremidi::Destinations
                                            .into_iter()
                                            .find(|d| d.unique_id().map(|u| u as i32) == Some(uid))
                                        {
                                            on_dest_added(&global_bg.ctx, &dest);
                                        }
                                        let _ = reply.send(Ok((
                                            PortId(PortIdInner::CoreMidi(uid)),
                                            receivers,
                                        )));
                                    }
                                    None => {
                                        let _ = reply.send(Err(Error::from(IoError::PortNotFound)));
                                    }
                                },
                                Err(status) => {
                                    let _ = reply.send(Err(Error::from(IoError::Platform(
                                        PlatformError::VirtualPortCreate(status),
                                    ))));
                                }
                            }
                        }
                        Command::DestroyVirtualDestination(id) => {
                            if let Some((vdest, senders)) = virtual_destinations.remove(&id.0) {
                                senders.lifecycle_error(IoError::PortDisconnected);
                                drop(vdest);
                            }
                        }
                        Command::ListSources { reply } => {
                            let _ = reply.send(enumerate_sources_with_cache(
                                global_bg.ctx.source_cache.clone(),
                            ));
                        }
                        Command::ListDestinations { reply } => {
                            let _ = reply.send(enumerate_destinations_with_cache(
                                global_bg.ctx.dest_cache.clone(),
                            ));
                        }
                        Command::SendVirtualMidi { id, msg, reply } => {
                            if let Some(source) = virtual_sources.get(&id.0) {
                                let buf = PacketBuffer::new(0, &msg);
                                let _ = reply.send(source.received(&buf).map_err(|status| {
                                    Error::from(IoError::Platform(PlatformError::Send(status)))
                                }));
                            } else {
                                let _ = reply.send(Err(Error::from(IoError::PortNotFound)));
                            }
                        }
                        Command::SendVirtualSysex { id, data, reply } => {
                            if let Some(source) = virtual_sources.get(&id.0) {
                                let buf = PacketBuffer::new(0, &data);
                                let _ = reply.send(source.received(&buf).map_err(|status| {
                                    Error::from(IoError::Platform(PlatformError::Send(status)))
                                }));
                            } else {
                                let _ = reply.send(Err(Error::from(IoError::PortNotFound)));
                            }
                        }
                    }
                }
                let mut handlers = global_bg.ctx.output_disconnect_handlers.lock_unpoisoned();
                for uid in destination_connections.keys() {
                    remove_client_handlers(&mut handlers, *uid, client_id);
                }
            })
            .map_err(|_| IoError::BackendThreadDied)?;

        global
            .ctx
            .command_senders
            .lock_unpoisoned()
            .insert(client_id, cmd_tx.clone());

        Ok(Self {
            client_id,
            connections,
            global,
        })
    }

    pub(super) fn wake(&self) {}

    pub(super) fn on_drop(&self, cmd_tx: &std::sync::mpsc::SyncSender<Command>) {
        let client_id = self.client_id;
        self.global.ctx.senders.lock_unpoisoned().remove(&client_id);
        self.global
            .ctx
            .output_senders
            .lock_unpoisoned()
            .remove(&client_id);
        self.global
            .ctx
            .command_senders
            .lock_unpoisoned()
            .remove(&client_id);
        let conns: Vec<i32> = self.connections.lock_unpoisoned().keys().copied().collect();
        {
            let mut disc = self.global.ctx.disconnect_handlers.lock_unpoisoned();
            for uid in conns {
                remove_client_handlers(&mut disc, uid, client_id);
            }
        }
        if let Err(std::sync::mpsc::TrySendError::Full(_)) = cmd_tx.try_send(Command::Shutdown) {
            log_error!("command channel full; shutdown lost");
        }
    }
}

fn handle_connect_destination(
    port_id: &PortId,
    destination_connections: &mut HashMap<i32, DestinationConnectionState>,
    global: &GlobalIo,
    client: &coremidi::Client,
    disconnected_outputs: Arc<Mutex<std::collections::HashSet<i32>>>,
    client_id: u64,
) -> Result<(), Error> {
    let PortIdInner::CoreMidi(uid) = port_id.0;
    if destination_connections.contains_key(&uid) {
        return Err(IoError::AlreadyConnected.into());
    }
    let (destination, added_port) = {
        let mut cache = global.ctx.dest_cache.lock_unpoisoned();
        if let Some((d, _)) = cache.get(&uid) {
            (d.clone(), None)
        } else {
            let found = coremidi::Destinations
                .into_iter()
                .find(|d| d.unique_id().map(|id| id as i32) == Some(uid));
            match found {
                Some(d) => {
                    let (_, out_port) =
                        coremidi_dest_to_destination(&d).ok_or(IoError::PortNotFound)?;
                    cache.insert(uid, (d.clone(), out_port.clone()));
                    (d, Some(out_port))
                }
                None => return Err(IoError::PortNotFound.into()),
            }
        }
    };
    if let Some(p) = added_port {
        global
            .ctx
            .notify_destination_subscribers(DestinationChange::Added(p));
    }
    let destination_port = client
        .output_port(&format!("midi-io-out-{uid}"))
        .map_err(|status| IoError::Platform(PlatformError::Connect(status)))?;

    disconnected_outputs.lock_unpoisoned().remove(&uid);
    global
        .ctx
        .output_disconnect_handlers
        .lock_unpoisoned()
        .entry(uid)
        .or_default()
        .push((
            client_id,
            Box::new(move || {
                disconnected_outputs.lock_unpoisoned().insert(uid);
            }),
        ));
    destination_connections.insert(
        uid,
        DestinationConnectionState {
            destination_port,
            destination,
        },
    );
    Ok(())
}

fn resolve_output_state<'a>(
    port_id: &PortId,
    destination_connections: &'a HashMap<i32, DestinationConnectionState>,
    disconnected_outputs: &Mutex<std::collections::HashSet<i32>>,
) -> Result<&'a DestinationConnectionState, Error> {
    let PortIdInner::CoreMidi(uid) = port_id.0;
    if disconnected_outputs.lock_unpoisoned().contains(&uid) {
        return Err(IoError::PortDisconnected.into());
    }
    destination_connections
        .get(&uid)
        .ok_or_else(|| IoError::PortNotFound.into())
}

fn handle_send_midi(
    port_id: &PortId,
    msg: RawMidiMessage,
    destination_connections: &HashMap<i32, DestinationConnectionState>,
    disconnected_outputs: &Mutex<std::collections::HashSet<i32>>,
) -> Result<(), Error> {
    let state = resolve_output_state(port_id, destination_connections, disconnected_outputs)?;
    let buf = PacketBuffer::new(0, &msg);
    state
        .destination_port
        .send(&state.destination, &buf)
        .map_err(|status| Error::from(IoError::Platform(PlatformError::Send(status))))
}

fn handle_send_sysex(
    port_id: &PortId,
    data: &[u8],
    destination_connections: &HashMap<i32, DestinationConnectionState>,
    disconnected_outputs: &Mutex<std::collections::HashSet<i32>>,
) -> Result<(), Error> {
    let state = resolve_output_state(port_id, destination_connections, disconnected_outputs)?;
    let buf = PacketBuffer::new(0, data);
    state
        .destination_port
        .send(&state.destination, &buf)
        .map_err(|status| Error::from(IoError::Platform(PlatformError::Send(status))))
}

fn drain_streams_backend_died(source_streams: &HashMap<i32, StreamSenders>) {
    for senders in source_streams.values() {
        senders.lifecycle_error(IoError::BackendThreadDied);
    }
}

fn connect_source(
    uid: i32,
    connections: &Arc<Mutex<HashMap<i32, ConnectionState>>>,
    source_streams: &Arc<Mutex<HashMap<i32, StreamSenders>>>,
    global: &GlobalIo,
    client: &coremidi::Client,
    client_id: u64,
) -> Result<StreamReceivers, Error> {
    if connections.lock_unpoisoned().contains_key(&uid) {
        return Err(IoError::AlreadyConnected.into());
    }

    let (senders, receivers) = StreamSenders::channel();

    let (source, added_port) = {
        let mut cache = global.ctx.source_cache.lock_unpoisoned();
        if let Some((s, _)) = cache.get(&uid) {
            (s.clone(), None)
        } else {
            let found = coremidi::Sources
                .into_iter()
                .find(|s| s.unique_id().map(|id| id as i32) == Some(uid));
            match found {
                Some(s) => {
                    let (_, midi_port) =
                        coremidi_source_to_source(&s).ok_or(IoError::PortNotFound)?;
                    cache.insert(uid, (s.clone(), midi_port.clone()));
                    (s, Some(midi_port))
                }
                None => return Err(IoError::PortNotFound.into()),
            }
        }
    };

    if let Some(p) = added_port {
        global.ctx.notify_source_subscribers(SourceChange::Added(p));
    }

    let cb_senders = senders.clone();
    let mut parser = StreamParser::new();
    let cal = global.calibration;
    let source_port = client
        .input_port(&format!("midi-io-in-{uid}"), move |packet_list| {
            for packet in packet_list.iter() {
                let ts = cal.to_instant(packet.timestamp());
                parser.push(packet.data(), &mut |event| {
                    cb_senders.emit(ts, event);
                });
            }
        })
        .map_err(|status| IoError::Platform(PlatformError::Connect(status)))?;

    let disc_senders = senders.clone();
    global
        .ctx
        .disconnect_handlers
        .lock_unpoisoned()
        .entry(uid)
        .or_default()
        .push((
            client_id,
            Box::new(move || {
                disc_senders.lifecycle_error(IoError::PortDisconnected);
            }),
        ));

    if let Err(status) = source_port.connect_source(&source) {
        remove_client_handlers(
            &mut global.ctx.disconnect_handlers.lock_unpoisoned(),
            uid,
            client_id,
        );
        return Err(IoError::Platform(PlatformError::Connect(status)).into());
    }

    connections.lock_unpoisoned().insert(
        uid,
        ConnectionState {
            _source_port: source_port,
        },
    );

    source_streams.lock_unpoisoned().insert(uid, senders);

    Ok(receivers)
}

fn enumerate_sources_with_cache(
    cache: Arc<Mutex<HashMap<i32, (CoreMidiSource, Source)>>>,
) -> Result<Vec<Source>, Error> {
    let mut uid_set: HashMap<i32, Source> = coremidi::Sources
        .into_iter()
        .filter_map(|s| coremidi_source_to_source(&s).map(|(_, p)| p))
        .map(|p| (id_to_uid(&p.id), p))
        .collect();
    for (_, p) in cache.lock_unpoisoned().values() {
        let uid = id_to_uid(&p.id);
        uid_set.entry(uid).or_insert(p.clone());
    }
    Ok(uid_set.into_values().collect())
}

fn enumerate_destinations_with_cache(
    cache: Arc<Mutex<HashMap<i32, (CoreMidiDestination, Destination)>>>,
) -> Result<Vec<Destination>, Error> {
    let mut uid_set: HashMap<i32, Destination> = coremidi::Destinations
        .into_iter()
        .filter_map(|d| coremidi_dest_to_destination(&d).map(|(_, p)| p))
        .map(|p| (id_to_uid(&p.id), p))
        .collect();
    for (_, p) in cache.lock_unpoisoned().values() {
        let uid = id_to_uid(&p.id);
        uid_set.entry(uid).or_insert(p.clone());
    }
    Ok(uid_set.into_values().collect())
}

fn id_to_uid(id: &PortId) -> i32 {
    match id.0 {
        PortIdInner::CoreMidi(uid) => uid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_global_ctx() -> GlobalContext {
        GlobalContext {
            senders: Arc::new(Mutex::new(HashMap::new())),
            output_senders: Arc::new(Mutex::new(HashMap::new())),
            source_cache: Arc::new(Mutex::new(HashMap::new())),
            dest_cache: Arc::new(Mutex::new(HashMap::new())),
            disconnect_handlers: Arc::new(Mutex::new(HashMap::new())),
            output_disconnect_handlers: Arc::new(Mutex::new(HashMap::new())),
            command_senders: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn test_source(uid: i32) -> Source {
        Source {
            id: PortId(PortIdInner::CoreMidi(uid)),
            name: format!("Test:{uid}"),
            is_virtual: false,
        }
    }

    fn test_destination(uid: i32) -> Destination {
        Destination {
            id: PortId(PortIdInner::CoreMidi(uid)),
            name: format!("Test:{uid}"),
            is_virtual: false,
        }
    }

    #[test]
    fn notify_subscribers_reaches_all_registered_senders() {
        let ctx = make_global_ctx();
        let (tx1, mut rx1) = mpsc::unbounded();
        let (tx2, mut rx2) = mpsc::unbounded();
        ctx.senders
            .lock()
            .unwrap()
            .insert(0, Arc::new(Mutex::new(vec![tx1])));
        ctx.senders
            .lock()
            .unwrap()
            .insert(1, Arc::new(Mutex::new(vec![tx2])));
        ctx.notify_source_subscribers(SourceChange::Added(test_source(1)));
        assert!(matches!(
            rx1.try_recv().map(Some),
            Ok(Some(SourceChange::Added(_)))
        ));
        assert!(matches!(
            rx2.try_recv().map(Some),
            Ok(Some(SourceChange::Added(_)))
        ));
    }

    #[test]
    fn notify_subscribers_tolerates_no_senders() {
        let ctx = make_global_ctx();
        ctx.notify_source_subscribers(SourceChange::Added(test_source(2)));
    }

    #[test]
    fn notify_output_subscribers_reaches_all_registered_senders() {
        let ctx = make_global_ctx();
        let (tx1, mut rx1) = mpsc::unbounded();
        ctx.output_senders
            .lock()
            .unwrap()
            .insert(0, Arc::new(Mutex::new(vec![tx1])));
        ctx.notify_destination_subscribers(DestinationChange::Added(test_destination(3)));
        assert!(matches!(
            rx1.try_recv().map(Some),
            Ok(Some(DestinationChange::Added(_)))
        ));
    }

    #[test]
    fn calibration_converts_current_time_accurately() {
        let cal = TimeCalibration::capture();
        let ts = current_host_time();
        let after = Instant::now();
        let converted = cal.to_instant(ts);
        assert!(converted >= cal.ref_instant);
        assert!(converted <= after + std::time::Duration::from_millis(1));
    }

    #[test]
    fn calibration_zero_host_time_falls_back_to_arrival_time() {
        let cal = TimeCalibration::capture();
        let before = Instant::now();
        let converted = cal.to_instant(0);
        let after = Instant::now();
        assert!(converted >= before);
        assert!(converted <= after);
    }

    #[test]
    fn remove_client_handlers_only_removes_target_client() {
        let fired = Arc::new(Mutex::new(Vec::new()));
        let mut handlers: HashMap<i32, Vec<(u64, DisconnectFn)>> = HashMap::new();
        let f0 = fired.clone();
        let f1 = fired.clone();
        handlers
            .entry(5)
            .or_default()
            .push((0, Box::new(move || f0.lock().unwrap().push(0u64))));
        handlers
            .entry(5)
            .or_default()
            .push((1, Box::new(move || f1.lock().unwrap().push(1u64))));
        remove_client_handlers(&mut handlers, 5, 0);
        let remaining = handlers.remove(&5).unwrap();
        for (_, f) in remaining {
            f();
        }
        assert_eq!(*fired.lock().unwrap(), vec![1u64]);
    }

    #[test]
    fn remove_client_handlers_drops_entry_when_empty() {
        let mut handlers: HashMap<i32, Vec<(u64, DisconnectFn)>> = HashMap::new();
        handlers.entry(7).or_default().push((3, Box::new(|| {})));
        remove_client_handlers(&mut handlers, 7, 3);
        assert!(!handlers.contains_key(&7));
    }

    #[test]
    fn find_stale_uid_detects_missing_entry() {
        let cached = vec![1i32, 2, 3];
        let live: std::collections::HashSet<i32> = [2, 3].into_iter().collect();
        assert_eq!(find_stale_uid(cached.into_iter(), &live), Some(1));
    }

    #[test]
    fn find_stale_uid_returns_none_when_all_present() {
        let cached = vec![1i32, 2, 3];
        let live: std::collections::HashSet<i32> = [1, 2, 3].into_iter().collect();
        assert_eq!(find_stale_uid(cached.into_iter(), &live), None);
    }

    #[test]
    fn find_stale_uid_returns_none_on_empty_cache() {
        let live: std::collections::HashSet<i32> = [1, 2].into_iter().collect();
        assert_eq!(find_stale_uid(std::iter::empty(), &live), None);
    }

    #[test]
    fn drain_streams_backend_died_delivers_terminal_error_to_all_streams() {
        let (s1, (_m1, _x1, mut e1)) = StreamSenders::channel();
        let (s2, (_m2, _x2, mut e2)) = StreamSenders::channel();
        let mut streams: HashMap<i32, StreamSenders> = HashMap::new();
        streams.insert(1, s1);
        streams.insert(2, s2);

        drain_streams_backend_died(&streams);

        assert!(matches!(
            e1.try_recv(),
            Ok(Timed {
                payload: Error::Io(IoError::BackendThreadDied),
                ..
            })
        ));
        assert!(matches!(
            e2.try_recv(),
            Ok(Timed {
                payload: Error::Io(IoError::BackendThreadDied),
                ..
            })
        ));
    }

    #[test]
    fn drain_streams_backend_died_tolerates_closed_receiver() {
        let (senders, (_m, _x, e)) = StreamSenders::channel();
        drop(e);
        let mut streams: HashMap<i32, StreamSenders> = HashMap::new();
        streams.insert(7, senders);
        drain_streams_backend_died(&streams);
    }

    #[test]
    fn signal_backend_death_broadcasts_to_every_command_pump() {
        let ctx = make_global_ctx();
        let (tx_a, rx_a) = std::sync::mpsc::sync_channel::<Command>(1);
        let (tx_b, rx_b) = std::sync::mpsc::sync_channel::<Command>(1);
        ctx.command_senders.lock().unwrap().insert(0, tx_a);
        ctx.command_senders.lock().unwrap().insert(1, tx_b);

        ctx.signal_backend_death();

        assert!(matches!(rx_a.try_recv(), Ok(Command::BackendDied)));
        assert!(matches!(rx_b.try_recv(), Ok(Command::BackendDied)));
    }

    #[test]
    fn signal_backend_death_tolerates_no_command_pumps() {
        let ctx = make_global_ctx();
        ctx.signal_backend_death();
    }

    #[test]
    fn connect_destination_clears_stale_disconnected_flag() {
        let global = ensure_global_io().expect("global io");
        let vdest = match global
            .io_client
            .virtual_destination("readd-unit", |_: &coremidi::PacketList| {})
        {
            Ok(vdest) => vdest,
            Err(_) => {
                eprintln!("skip: virtual MIDI endpoints not permitted on this platform");
                return;
            }
        };
        let uid = vdest.unique_id().expect("unique id") as i32;
        let cmdest = coremidi::Destinations
            .into_iter()
            .find(|d| d.unique_id().map(|u| u as i32) == Some(uid))
            .expect("destination present");
        let (_, port) = coremidi_dest_to_destination(&cmdest).expect("port");
        global
            .ctx
            .dest_cache
            .lock_unpoisoned()
            .insert(uid, (cmdest, port));

        let disconnected: Arc<Mutex<std::collections::HashSet<i32>>> =
            Arc::new(Mutex::new(std::collections::HashSet::from([uid])));
        let mut destination_connections: HashMap<i32, DestinationConnectionState> = HashMap::new();
        let port_id = PortId(PortIdInner::CoreMidi(uid));

        handle_connect_destination(
            &port_id,
            &mut destination_connections,
            &global,
            &global.io_client,
            disconnected.clone(),
            u64::MAX,
        )
        .expect("reconnect should succeed");

        assert!(
            !disconnected.lock_unpoisoned().contains(&uid),
            "reconnecting a destination must clear its stale disconnected flag"
        );
        assert!(destination_connections.contains_key(&uid));

        remove_client_handlers(
            &mut global.ctx.output_disconnect_handlers.lock_unpoisoned(),
            uid,
            u64::MAX,
        );
        global.ctx.dest_cache.lock_unpoisoned().remove(&uid);
    }

    #[test]
    fn inbound_overflow_dropped_and_coalesced() {
        let (senders, (mut m, _x, mut e)) = StreamSenders::channel();
        let ts = Instant::now();

        for _ in 0..(INBOUND_CHANNEL_CAPACITY + 5) {
            senders.emit(ts, DecodedEvent::Message(crate::MidiMessage::Reset));
        }

        for _ in 0..INBOUND_CHANNEL_CAPACITY {
            let _ = m.try_recv();
        }

        senders.emit(ts, DecodedEvent::Message(crate::MidiMessage::Reset));

        let overflow = loop {
            match e.try_recv() {
                Ok(timed) => match &timed.payload {
                    Error::Io(IoError::InboundOverflow { dropped }) => {
                        break *dropped;
                    }
                    _ => continue,
                },
                Err(_) => {
                    panic!("expected InboundOverflow error on stream");
                }
            }
        };
        assert!(
            (4..=5).contains(&overflow),
            "expected 4-5 dropped, got {}",
            overflow
        );
    }

    #[test]
    fn inbound_steady_state_no_overflow() {
        let (senders, (mut m, _x, mut e)) = StreamSenders::channel();
        let ts = Instant::now();

        senders.emit(ts, DecodedEvent::Message(crate::MidiMessage::Reset));
        let _ = m.try_recv();

        senders.emit(ts, DecodedEvent::Message(crate::MidiMessage::Reset));
        let _ = m.try_recv();

        assert!(
            e.try_recv().is_err(),
            "no overflow should occur in steady state"
        );
    }
}
