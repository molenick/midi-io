use std::collections::HashMap;
use std::os::fd::AsRawFd;
use std::os::fd::FromRawFd;
use std::os::fd::OwnedFd;
use std::os::raw::c_void;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

use alsa::poll::Descriptors as _;
use alsa::seq::Addr;
use alsa::seq::ClientIter;
use alsa::seq::EvCtrl;
use alsa::seq::EvNote;
use alsa::seq::EventType;
use alsa::seq::MidiEvent;
use alsa::seq::PortCap;
use alsa::seq::PortIter;
use alsa::seq::PortSubscribe;
use alsa::seq::PortType;
use alsa::seq::Seq;
use alsa::Direction;
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
use crate::midi::stream_parser::StreamParser;
#[cfg(test)]
use crate::midi::sys_ex::MAX_SYSEX_BYTES;
#[cfg(test)]
use crate::midi::sys_ex::ORPHAN_PREFIX_BYTES;
use crate::name::Name;
use crate::Channel;
use crate::CodecError;
use crate::DataByte;
use crate::Destination;
use crate::DestinationChange;
use crate::Error;
use crate::IoError;
use crate::MidiMessage;
use crate::ParseError;
use crate::PitchBend;
use crate::PlatformError;
use crate::PortId;
use crate::RawMidiMessage;
use crate::SongPosition;
use crate::Source;
use crate::SourceChange;

const SND_SEQ_ADDRESS_SUBSCRIBERS: i32 = 254;
const SND_SEQ_ADDRESS_UNKNOWN: i32 = 253;
const SNDRV_SEQ_DYNAMIC_CLIENTS_BEGIN: i32 = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct AlsaPortKey(i32, i32);

struct ConnectionState {
    senders: StreamSenders,
    parser: StreamParser,
}

struct DestinationConnectionState {
    disconnected: bool,
}

struct VirtualDestinationState {
    senders: StreamSenders,
    parser: StreamParser,
}

struct SeqContext {
    seq: Seq,
    our_client: i32,
    our_port: i32,
    our_send_port: i32,
    queue_id: i32,
    queue_start_instant: Instant,
    source_caps: PortCap,
    destination_caps: PortCap,
}

struct PortRegistry {
    source_cache: HashMap<AlsaPortKey, Source>,
    destination_cache: HashMap<AlsaPortKey, Destination>,
    connections: HashMap<AlsaPortKey, ConnectionState>,
    destination_connections: HashMap<AlsaPortKey, DestinationConnectionState>,
    vdest_recv: HashMap<i32, VirtualDestinationState>,
    vdest_ports: HashMap<u64, i32>,
    vsrc_send: HashMap<u64, i32>,
    source_subs: SourceSubscribers,
    destination_subs: DestinationSubscribers,
    send_coder: MidiEvent,
}

trait AlsaPort {
    fn from_alsa(key: AlsaPortKey, name: &str) -> Self;
}

impl AlsaPort for Source {
    fn from_alsa(key: AlsaPortKey, name: &str) -> Self {
        Source {
            id: key_to_id(key),
            name: name.to_string(),
            is_virtual: key.0 >= SNDRV_SEQ_DYNAMIC_CLIENTS_BEGIN,
        }
    }
}

impl AlsaPort for Destination {
    fn from_alsa(key: AlsaPortKey, name: &str) -> Self {
        Destination {
            id: key_to_id(key),
            name: name.to_string(),
            is_virtual: key.0 >= SNDRV_SEQ_DYNAMIC_CLIENTS_BEGIN,
        }
    }
}

fn subscribers_addr() -> Addr {
    Addr {
        client: SND_SEQ_ADDRESS_SUBSCRIBERS,
        port: SND_SEQ_ADDRESS_UNKNOWN,
    }
}

fn notify_subscribers<T: Clone>(subs: &Mutex<Vec<mpsc::UnboundedSender<T>>>, change: T) {
    let mut subs_lock = subs.lock_unpoisoned();
    prune_send(&mut subs_lock, &change);
}

pub(super) struct Backend {
    stop: Arc<AtomicBool>,
    efd: Arc<OwnedFd>,
}

impl Backend {
    pub(super) fn start(
        name: Name,
        source_subs: SourceSubscribers,
        destination_subs: DestinationSubscribers,
        cmd_rx: std::sync::mpsc::Receiver<Command>,
        _cmd_tx: &std::sync::mpsc::SyncSender<Command>,
        ready_tx: oneshot::Sender<Result<(), Error>>,
    ) -> Result<Self, Error> {
        let efd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if efd < 0 {
            return Err(IoError::Platform(PlatformError::ThreadInit).into());
        }
        let efd = Arc::new(unsafe { OwnedFd::from_raw_fd(efd) });

        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = stop.clone();
        let thread_efd = Arc::clone(&efd);

        std::thread::spawn(move || {
            run_thread(
                name,
                source_subs,
                destination_subs,
                cmd_rx,
                thread_efd,
                stop_clone,
                ready_tx,
            );
        });

        Ok(Self { stop, efd })
    }

    pub(super) fn wake(&self) {
        let one = 1u64;
        let ret =
            unsafe { libc::write(self.efd.as_raw_fd(), &one as *const u64 as *const c_void, 8) };
        if ret != 8 {
            log_error!("eventfd write failed: {}", std::io::Error::last_os_error());
        }
    }

    pub(super) fn on_drop(&self, _cmd_tx: &std::sync::mpsc::SyncSender<Command>) {
        self.stop.store(true, Ordering::Release);
        self.wake();
    }
}

fn init_seq(name: &Name) -> Result<SeqContext, Error> {
    let seq = Seq::open(None, None, true)
        .map_err(|e| IoError::Platform(PlatformError::ClientInit(e.errno())))?;
    seq.set_client_name(name.as_c_str())
        .map_err(|e| IoError::Platform(PlatformError::ClientInit(e.errno())))?;
    let our_port = seq
        .create_simple_port(
            c"midi-io-input",
            PortCap::WRITE | PortCap::SUBS_WRITE,
            PortType::APPLICATION,
        )
        .map_err(|e| IoError::Platform(PlatformError::ClientInit(e.errno())))?;
    let our_send_port = seq
        .create_simple_port(
            c"midi-io-output",
            PortCap::READ | PortCap::SUBS_READ,
            PortType::APPLICATION,
        )
        .map_err(|e| IoError::Platform(PlatformError::ClientInit(e.errno())))?;
    let our_client = seq
        .client_id()
        .map_err(|e| IoError::Platform(PlatformError::ClientInit(e.errno())))?;
    let queue_id = seq
        .alloc_named_queue(c"midi-io-ts")
        .map_err(|e| IoError::Platform(PlatformError::ClientInit(e.errno())))?;
    seq.control_queue(queue_id, EventType::Start, 0, None::<&mut alsa::seq::Event>)
        .map_err(|e| IoError::Platform(PlatformError::ClientInit(e.errno())))?;
    let queue_start_instant = Instant::now();
    let sub = PortSubscribe::empty()
        .map_err(|e| IoError::Platform(PlatformError::ClientInit(e.errno())))?;
    sub.set_sender(Addr::system_announce());
    sub.set_dest(Addr {
        client: our_client,
        port: our_port,
    });
    seq.subscribe_port(&sub)
        .map_err(|e| IoError::Platform(PlatformError::ClientInit(e.errno())))?;
    Ok(SeqContext {
        seq,
        our_client,
        our_port,
        our_send_port,
        queue_id,
        queue_start_instant,
        source_caps: PortCap::READ | PortCap::SUBS_READ,
        destination_caps: PortCap::WRITE | PortCap::SUBS_WRITE,
    })
}

fn create_virtual_destination_port(
    ctx: &SeqContext,
    name: &std::ffi::CStr,
) -> Result<i32, alsa::Error> {
    let alsa_port = ctx.seq.create_simple_port(
        name,
        PortCap::WRITE | PortCap::SUBS_WRITE,
        PortType::APPLICATION,
    )?;
    if let Err(e) = enable_port_timestamping(ctx, alsa_port) {
        if let Err(del) = ctx.seq.delete_port(alsa_port) {
            log_error!(
                "failed to delete port {} after timestamping setup failed: {}",
                alsa_port,
                del
            );
        }
        return Err(e);
    }
    Ok(alsa_port)
}

fn enable_port_timestamping(ctx: &SeqContext, alsa_port: i32) -> Result<(), alsa::Error> {
    let mut info = ctx.seq.get_any_port_info(Addr {
        client: ctx.our_client,
        port: alsa_port,
    })?;
    info.set_timestamping(true);
    info.set_timestamp_real(true);
    info.set_timestamp_queue(ctx.queue_id);
    ctx.seq.set_port_info(alsa_port, &mut info)
}

fn build_cache<P: AlsaPort>(
    seq: &Seq,
    our_client: i32,
    needed_caps: PortCap,
) -> HashMap<AlsaPortKey, P> {
    let mut cache = HashMap::new();
    for client in ClientIter::new(seq) {
        let cid = client.get_client();
        if cid == 0 || cid == our_client {
            continue;
        }
        for port in PortIter::new(seq, cid) {
            if !port.get_capability().contains(needed_caps) {
                continue;
            }
            let key = AlsaPortKey(cid, port.get_port());
            let pname = port.get_name().unwrap_or("").to_string();
            cache.insert(key, P::from_alsa(key, &pname));
        }
    }
    cache
}

impl PortRegistry {
    fn new(
        source_subs: SourceSubscribers,
        destination_subs: DestinationSubscribers,
        send_coder: MidiEvent,
    ) -> Self {
        Self {
            source_cache: HashMap::new(),
            destination_cache: HashMap::new(),
            connections: HashMap::new(),
            destination_connections: HashMap::new(),
            vdest_recv: HashMap::new(),
            vdest_ports: HashMap::new(),
            vsrc_send: HashMap::new(),
            source_subs,
            destination_subs,
            send_coder,
        }
    }

    fn add_source(&mut self, key: AlsaPortKey, port: Source) {
        self.source_cache.insert(key, port.clone());
        notify_subscribers(&self.source_subs, SourceChange::Added(port));
    }

    fn add_destination(&mut self, key: AlsaPortKey, port: Destination) {
        self.destination_cache.insert(key, port.clone());
        notify_subscribers(&self.destination_subs, DestinationChange::Added(port));
    }

    fn remove_source(&mut self, key: AlsaPortKey) {
        if let Some(port) = self.source_cache.remove(&key) {
            if let Some(conn) = self.connections.get(&key) {
                conn.senders.lifecycle_error(IoError::PortDisconnected);
            }
            notify_subscribers(&self.source_subs, SourceChange::Removed(port));
        }
    }

    fn remove_destination(&mut self, key: AlsaPortKey) {
        if let Some(port) = self.destination_cache.remove(&key) {
            if let Some(conn) = self.destination_connections.get_mut(&key) {
                conn.disconnected = true;
            }
            notify_subscribers(&self.destination_subs, DestinationChange::Removed(port));
        }
    }

    fn handle_port_start(&mut self, addr: Addr, ctx: &SeqContext) {
        if addr.client == 0 || addr.client == ctx.our_client {
            return;
        }
        if let Ok(pinfo) = ctx.seq.get_any_port_info(addr) {
            let caps = pinfo.get_capability();
            let pname = pinfo.get_name().unwrap_or("").to_string();
            let key = AlsaPortKey(addr.client, addr.port);
            if caps.contains(ctx.source_caps) {
                self.add_source(key, Source::from_alsa(key, &pname));
            }
            if caps.contains(ctx.destination_caps) {
                self.add_destination(key, Destination::from_alsa(key, &pname));
            }
        }
    }

    fn handle_port_exit(&mut self, addr: Addr) {
        let key = AlsaPortKey(addr.client, addr.port);
        self.remove_source(key);
        self.remove_destination(key);
    }

    fn handle_client_exit(&mut self, client_id: i32) {
        let source_keys: Vec<AlsaPortKey> = self
            .source_cache
            .keys()
            .filter(|k| k.0 == client_id)
            .copied()
            .collect();
        for key in source_keys {
            self.remove_source(key);
        }
        let destination_keys: Vec<AlsaPortKey> = self
            .destination_cache
            .keys()
            .filter(|k| k.0 == client_id)
            .copied()
            .collect();
        for key in destination_keys {
            self.remove_destination(key);
        }
    }

    fn route_message(&self, msg: MidiMessage, src: Addr, dest: Addr, our_client: i32, ts: Instant) {
        if dest.client == our_client {
            if let Some(vi) = self.vdest_recv.get(&dest.port) {
                vi.senders.send_message(ts, msg);
            }
        }
        if let Some(conn) = self.connections.get(&AlsaPortKey(src.client, src.port)) {
            conn.senders.send_message(ts, msg);
        }
    }

    fn route_error(&self, err: CodecError, src: Addr, dest: Addr, our_client: i32, ts: Instant) {
        if dest.client == our_client {
            if let Some(vi) = self.vdest_recv.get(&dest.port) {
                vi.senders.send_error(ts, err.clone().into());
            }
        }
        if let Some(conn) = self.connections.get(&AlsaPortKey(src.client, src.port)) {
            conn.senders.send_error(ts, err.into());
        }
    }

    fn dispatch_commands(&mut self, cmd_rx: &std::sync::mpsc::Receiver<Command>, ctx: &SeqContext) {
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                Command::ConnectSource { port_id, reply } => {
                    let result = self.handle_connect(&port_id, ctx);
                    let _ = reply.send(result);
                }
                Command::Disconnect(port_id) => {
                    let key = port_to_key(&port_id);
                    if self.connections.remove(&key).is_some() {
                        let _ = ctx.seq.unsubscribe_port(
                            Addr {
                                client: key.0,
                                port: key.1,
                            },
                            Addr {
                                client: ctx.our_client,
                                port: ctx.our_port,
                            },
                        );
                    }
                }
                Command::ConnectDestination { port_id, reply } => {
                    let result = self.handle_connect_destination(&port_id, ctx);
                    let _ = reply.send(result);
                }
                Command::SendMidi {
                    port_id,
                    msg,
                    reply,
                } => {
                    let key = port_to_key(&port_id);
                    let result = self.resolve_output_state(&key).and_then(|()| {
                        handle_send_midi(
                            &port_id,
                            msg,
                            ctx.our_send_port,
                            &mut self.send_coder,
                            &ctx.seq,
                        )
                    });
                    let _ = reply.send(result);
                }
                Command::SendSysex {
                    port_id,
                    data,
                    reply,
                } => {
                    let key = port_to_key(&port_id);
                    let result = self.resolve_output_state(&key).and_then(|()| {
                        handle_send_sysex(&port_id, data, ctx.our_send_port, &ctx.seq)
                    });
                    let _ = reply.send(result);
                }
                Command::DisconnectDestination(port_id) => {
                    let key = port_to_key(&port_id);
                    self.destination_connections.remove(&key);
                }
                Command::CreateVirtualSource { id, name, reply } => {
                    match ctx.seq.create_simple_port(
                        name.as_c_str(),
                        PortCap::READ | PortCap::SUBS_READ,
                        PortType::APPLICATION,
                    ) {
                        Ok(alsa_port) => {
                            let key = AlsaPortKey(ctx.our_client, alsa_port);
                            let source = Source::from_alsa(key, name.as_str());
                            let port = source.id;
                            self.add_source(key, source);
                            self.vsrc_send.insert(id.0, alsa_port);
                            let _ = reply.send(Ok(port));
                        }
                        Err(e) => {
                            let _ = reply.send(Err(Error::from(IoError::Platform(
                                PlatformError::VirtualPortCreate(e.errno()),
                            ))));
                        }
                    }
                }
                Command::DestroyVirtualSource(id) => {
                    if let Some(alsa_port) = self.vsrc_send.remove(&id.0) {
                        self.remove_source(AlsaPortKey(ctx.our_client, alsa_port));
                        if let Err(e) = ctx.seq.delete_port(alsa_port) {
                            log_error!("failed to delete port {}: {}", alsa_port, e);
                        }
                    }
                }
                Command::CreateVirtualDestination { id, name, reply } => {
                    match create_virtual_destination_port(ctx, name.as_c_str()) {
                        Ok(alsa_port) => {
                            let key = AlsaPortKey(ctx.our_client, alsa_port);
                            let destination = Destination::from_alsa(key, name.as_str());
                            let port = destination.id;
                            self.add_destination(key, destination);
                            let (senders, receivers) = StreamSenders::channel();
                            self.vdest_recv.insert(
                                alsa_port,
                                VirtualDestinationState {
                                    senders,
                                    parser: StreamParser::new(),
                                },
                            );
                            self.vdest_ports.insert(id.0, alsa_port);
                            let _ = reply.send(Ok((port, receivers)));
                        }
                        Err(e) => {
                            let _ = reply.send(Err(Error::from(IoError::Platform(
                                PlatformError::VirtualPortCreate(e.errno()),
                            ))));
                        }
                    }
                }
                Command::SendVirtualMidi { id, msg, reply } => {
                    let result = send_virtual_midi(
                        id.0,
                        msg,
                        &self.vsrc_send,
                        &mut self.send_coder,
                        &ctx.seq,
                    );
                    let _ = reply.send(result);
                }
                Command::SendVirtualSysex { id, data, reply } => {
                    let result = send_virtual_sysex(id.0, data, &self.vsrc_send, &ctx.seq);
                    let _ = reply.send(result);
                }
                Command::DestroyVirtualDestination(id) => {
                    if let Some(alsa_port) = self.vdest_ports.remove(&id.0) {
                        if let Some(state) = self.vdest_recv.remove(&alsa_port) {
                            state.senders.lifecycle_error(IoError::PortDisconnected);
                        }
                        self.remove_destination(AlsaPortKey(ctx.our_client, alsa_port));
                        let _ = ctx.seq.delete_port(alsa_port);
                    }
                }
                Command::ListSources { reply } => {
                    let ports = self.source_cache.values().cloned().collect();
                    let _ = reply.send(Ok(ports));
                }
                Command::ListDestinations { reply } => {
                    let ports = self.destination_cache.values().cloned().collect();
                    let _ = reply.send(Ok(ports));
                }
            }
        }
    }

    fn drain_seq_events(&mut self, ctx: &SeqContext, stop: &AtomicBool) {
        let mut inp = ctx.seq.input();
        loop {
            if stop.load(Ordering::Acquire) {
                break;
            }
            match inp.event_input_pending(true) {
                Ok(n) if n > 0 => {}
                _ => break,
            }
            let ev = match inp.event_input() {
                Ok(ev) => ev,
                Err(e) => {
                    log_error!("event_input failed, stopping event drain: {}", e);
                    break;
                }
            };

            let ev_type = ev.get_type();
            let src = ev.get_source();
            let dest = ev.get_dest();

            let ts = ev
                .get_time()
                .map(|d| ctx.queue_start_instant + d)
                .unwrap_or_else(Instant::now);

            match ev_type {
                EventType::PortStart => {
                    if let Some(addr) = ev.get_data::<Addr>() {
                        self.handle_port_start(addr, ctx);
                    }
                }
                EventType::PortExit => {
                    if let Some(addr) = ev.get_data::<Addr>() {
                        self.handle_port_exit(addr);
                    }
                }
                EventType::ClientExit => {
                    if let Some(addr) = ev.get_data::<Addr>() {
                        self.handle_client_exit(addr.client);
                    }
                }
                EventType::Noteon
                | EventType::Noteoff
                | EventType::Controller
                | EventType::Pgmchange
                | EventType::Chanpress
                | EventType::Pitchbend
                | EventType::Keypress
                | EventType::Qframe
                | EventType::Songpos
                | EventType::Songsel
                | EventType::TuneRequest
                | EventType::Clock
                | EventType::Start
                | EventType::Continue
                | EventType::Stop
                | EventType::Sensing
                | EventType::Reset => match message_from_event(&ev) {
                    Some(Ok(msg)) => self.route_message(msg, src, dest, ctx.our_client, ts),
                    Some(Err(err)) => self.route_error(err, src, dest, ctx.our_client, ts),
                    None => {}
                },
                EventType::Control14 => {
                    if let Some(c) = ev.get_data::<EvCtrl>() {
                        match control14_to_cc_pair(&c) {
                            Ok(msgs) => {
                                for msg in msgs {
                                    self.route_message(msg, src, dest, ctx.our_client, ts);
                                }
                            }
                            Err(err) => self.route_error(err, src, dest, ctx.our_client, ts),
                        }
                    }
                }
                EventType::Nonregparam | EventType::Regparam => {
                    if let Some(c) = ev.get_data::<EvCtrl>() {
                        let param_controllers = if ev.get_type() == EventType::Nonregparam {
                            [99, 98]
                        } else {
                            [101, 100]
                        };
                        match param_cc_messages(&c, param_controllers) {
                            Ok(msgs) => {
                                for msg in msgs {
                                    self.route_message(msg, src, dest, ctx.our_client, ts);
                                }
                            }
                            Err(err) => self.route_error(err, src, dest, ctx.our_client, ts),
                        }
                    }
                }
                EventType::Sysex => {
                    if dest.client == ctx.our_client {
                        if let Some(vi) = self.vdest_recv.get_mut(&dest.port) {
                            if let Some(data) = ev.get_ext() {
                                handle_sysex_virtual_destination(data, vi, ts);
                            }
                        }
                    }
                    if let Some(conn) = self.connections.get_mut(&AlsaPortKey(src.client, src.port))
                    {
                        if let Some(data) = ev.get_ext() {
                            handle_sysex_data(data, conn, ts);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn handle_connect_destination(
        &mut self,
        port_id: &PortId,
        ctx: &SeqContext,
    ) -> Result<(), Error> {
        let key = port_to_key(port_id);
        if self.destination_connections.contains_key(&key) {
            return Err(IoError::AlreadyConnected.into());
        }
        let pinfo = ctx
            .seq
            .get_any_port_info(Addr {
                client: key.0,
                port: key.1,
            })
            .map_err(|_| IoError::PortNotFound)?;
        if !pinfo.get_capability().contains(ctx.destination_caps) {
            return Err(IoError::PortNotFound.into());
        }
        self.destination_connections.insert(
            key,
            DestinationConnectionState {
                disconnected: false,
            },
        );
        Ok(())
    }

    fn resolve_output_state(&self, key: &AlsaPortKey) -> Result<(), Error> {
        match self.destination_connections.get(key) {
            None => Err(IoError::PortNotFound.into()),
            Some(conn) if conn.disconnected => Err(IoError::PortDisconnected.into()),
            Some(_) => Ok(()),
        }
    }

    fn handle_connect(
        &mut self,
        port_id: &PortId,
        ctx: &SeqContext,
    ) -> Result<StreamReceivers, Error> {
        let key = port_to_key(port_id);

        if self.connections.contains_key(&key) {
            return Err(IoError::AlreadyConnected.into());
        }

        let pinfo = ctx
            .seq
            .get_any_port_info(Addr {
                client: key.0,
                port: key.1,
            })
            .map_err(|_| IoError::PortNotFound)?;
        if !pinfo.get_capability().contains(ctx.source_caps) {
            return Err(IoError::PortNotFound.into());
        }

        let sub = PortSubscribe::empty()
            .map_err(|e| IoError::Platform(PlatformError::ClientInit(e.errno())))?;
        sub.set_sender(Addr {
            client: key.0,
            port: key.1,
        });
        sub.set_dest(Addr {
            client: ctx.our_client,
            port: ctx.our_port,
        });
        sub.set_queue(ctx.queue_id);
        sub.set_time_update(true);
        sub.set_time_real(true);
        ctx.seq
            .subscribe_port(&sub)
            .map_err(|e| IoError::Platform(PlatformError::Connect(e.errno())))?;

        let (senders, receivers) = StreamSenders::channel();
        self.connections.insert(
            key,
            ConnectionState {
                senders,
                parser: StreamParser::new(),
            },
        );
        Ok(receivers)
    }
}

fn handle_sysex_data(data: &[u8], conn: &mut ConnectionState, timestamp: Instant) {
    let ConnectionState { senders, parser } = conn;
    parser.push(data, &mut |event| senders.emit(timestamp, event));
}

fn handle_sysex_virtual_destination(
    data: &[u8],
    vi: &mut VirtualDestinationState,
    timestamp: Instant,
) {
    let VirtualDestinationState { senders, parser } = vi;
    parser.push(data, &mut |event| senders.emit(timestamp, event));
}

fn send_encoded(
    seq: &Seq,
    coder: &mut MidiEvent,
    msg: &RawMidiMessage,
    source: i32,
    dest: Addr,
) -> Result<(), Error> {
    coder.reset_encode();
    let (_, maybe_ev) = coder
        .encode(msg)
        .map_err(|e| IoError::Platform(PlatformError::Send(e.errno())))?;
    let mut ev = maybe_ev.ok_or(IoError::Platform(PlatformError::Encode))?;
    ev.set_source(source);
    ev.set_dest(dest);
    ev.set_direct();
    seq.event_output_direct(&mut ev)
        .map_err(|e| IoError::Platform(PlatformError::Send(e.errno())))?;
    Ok(())
}

fn send_sysex_event(seq: &Seq, data: Vec<u8>, source: i32, dest: Addr) -> Result<(), Error> {
    let mut ev = alsa::seq::Event::new_ext(EventType::Sysex, data);
    ev.set_source(source);
    ev.set_dest(dest);
    let mut ev = ev.into_owned();
    ev.set_direct();
    seq.event_output_direct(&mut ev)
        .map_err(|e| IoError::Platform(PlatformError::Send(e.errno())))?;
    Ok(())
}

fn send_virtual_midi(
    virtual_id: u64,
    msg: RawMidiMessage,
    ports: &HashMap<u64, i32>,
    coder: &mut MidiEvent,
    seq: &Seq,
) -> Result<(), Error> {
    let alsa_port = *ports.get(&virtual_id).ok_or(IoError::PortNotFound)?;
    send_encoded(seq, coder, &msg, alsa_port, subscribers_addr())
}

fn send_virtual_sysex(
    virtual_id: u64,
    data: Vec<u8>,
    ports: &HashMap<u64, i32>,
    seq: &Seq,
) -> Result<(), Error> {
    let alsa_port = *ports.get(&virtual_id).ok_or(IoError::PortNotFound)?;
    send_sysex_event(seq, data, alsa_port, subscribers_addr())
}

fn run_thread(
    name: Name,
    source_subs: SourceSubscribers,
    destination_subs: DestinationSubscribers,
    cmd_rx: std::sync::mpsc::Receiver<Command>,
    efd: Arc<OwnedFd>,
    stop: Arc<AtomicBool>,
    ready_tx: oneshot::Sender<Result<(), Error>>,
) {
    let ctx = match init_seq(&name) {
        Ok(v) => v,
        Err(e) => {
            let _ = ready_tx.send(Err(e));
            return;
        }
    };

    let send_coder = match MidiEvent::new(4) {
        Ok(v) => v,
        Err(e) => {
            let _ = ready_tx.send(Err(Error::from(IoError::Platform(
                PlatformError::ClientInit(e.errno()),
            ))));
            return;
        }
    };
    send_coder.enable_running_status(false);
    let mut registry = PortRegistry::new(source_subs, destination_subs, send_coder);
    registry.source_cache = build_cache(&ctx.seq, ctx.our_client, ctx.source_caps);
    registry.destination_cache = build_cache(&ctx.seq, ctx.our_client, ctx.destination_caps);

    let alsa_pfds = match (&ctx.seq, Some(Direction::Capture)).get() {
        Ok(p) => p,
        Err(e) => {
            let _ = ready_tx.send(Err(Error::from(IoError::Platform(
                PlatformError::ClientInit(e.errno()),
            ))));
            return;
        }
    };
    let mut pfds = alsa_pfds;
    add_efd_poll(&mut pfds, efd.as_raw_fd());

    let _ = ready_tx.send(Ok(()));

    loop {
        for pfd in &mut pfds {
            pfd.revents = 0;
        }

        let r = unsafe { libc::poll(pfds.as_mut_ptr(), pfds.len() as libc::nfds_t, -1) };
        if r < 0 {
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            log_error!(
                "poll failed, backend thread exiting: {}",
                std::io::Error::last_os_error()
            );
            break;
        }

        let efd_revents = pfds.last().map(|p| p.revents).unwrap_or(0);
        if efd_revents & (libc::POLLIN | libc::POLLERR | libc::POLLHUP) != 0 {
            clear_efd(efd.as_raw_fd());
            if stop.load(Ordering::Acquire) {
                break;
            }
        }
        registry.dispatch_commands(&cmd_rx, &ctx);

        for pfd in pfds.iter().take(pfds.len() - 1) {
            if pfd.revents & (libc::POLLERR | libc::POLLHUP) != 0 {
                log_error!("sequencer fd error/hup, backend thread exiting");
                stop.store(true, Ordering::Release);
                break;
            }
        }

        if stop.load(Ordering::Acquire) {
            break;
        }

        registry.drain_seq_events(&ctx, &stop);
    }

    for (_, conn) in registry.connections.drain() {
        conn.senders.lifecycle_error(IoError::BackendThreadDied);
    }
    for (_, state) in registry.vdest_recv.drain() {
        state.senders.lifecycle_error(IoError::BackendThreadDied);
    }
}

fn handle_send_midi(
    port_id: &PortId,
    msg: RawMidiMessage,
    our_send_port: i32,
    coder: &mut MidiEvent,
    seq: &Seq,
) -> Result<(), Error> {
    let key = port_to_key(port_id);
    send_encoded(
        seq,
        coder,
        &msg,
        our_send_port,
        Addr {
            client: key.0,
            port: key.1,
        },
    )
}

fn handle_send_sysex(
    port_id: &PortId,
    data: Vec<u8>,
    our_send_port: i32,
    seq: &Seq,
) -> Result<(), Error> {
    let key = port_to_key(port_id);
    send_sysex_event(
        seq,
        data,
        our_send_port,
        Addr {
            client: key.0,
            port: key.1,
        },
    )
}

fn port_to_key(port_id: &PortId) -> AlsaPortKey {
    AlsaPortKey((port_id.0 >> 32) as u32 as i32, port_id.0 as u32 as i32)
}

fn key_to_id(key: AlsaPortKey) -> PortId {
    PortId(((key.0 as u32 as u64) << 32) | (key.1 as u32 as u64))
}

fn add_efd_poll(pfds: &mut Vec<libc::pollfd>, efd: i32) {
    pfds.push(libc::pollfd {
        fd: efd,
        events: libc::POLLIN,
        revents: 0,
    });
}

fn clear_efd(efd: i32) {
    let mut val = 0u64;
    let ret = unsafe { libc::read(efd, &mut val as *mut u64 as *mut c_void, 8) };
    if ret != 8 {
        log_warn!(
            "eventfd read returned {ret}: {}",
            std::io::Error::last_os_error()
        );
    }
}

fn control14_to_cc_pair(c: &EvCtrl) -> Result<[MidiMessage; 2], CodecError> {
    let Ok(channel) = Channel::from_index(c.channel) else {
        return Err(channel_oob(0xB0, c.channel));
    };
    if c.param > 31 || !(0..=0x3FFF).contains(&c.value) {
        return Err(data_oob(
            0xB0 | channel.index(),
            &[c.param as i64, c.value as i64],
        ));
    }
    let data = |v: u8| DataByte::try_from(v).expect("validated to fit in 7 bits");
    Ok([
        MidiMessage::ControlChange {
            channel,
            controller: data(c.param as u8),
            value: data((c.value >> 7) as u8),
        },
        MidiMessage::ControlChange {
            channel,
            controller: data(c.param as u8 + 32),
            value: data((c.value & 0x7F) as u8),
        },
    ])
}

fn param_cc_messages(
    c: &EvCtrl,
    [param_msb, param_lsb]: [u8; 2],
) -> Result<[MidiMessage; 4], CodecError> {
    let Ok(channel) = Channel::from_index(c.channel) else {
        return Err(channel_oob(0xB0, c.channel));
    };
    if c.param > 0x3FFF || !(0..=0x3FFF).contains(&c.value) {
        return Err(data_oob(
            0xB0 | channel.index(),
            &[c.param as i64, c.value as i64],
        ));
    }
    let cc = |controller: u8, value: u8| MidiMessage::ControlChange {
        channel,
        controller: DataByte::try_from(controller).expect("validated to fit in 7 bits"),
        value: DataByte::try_from(value).expect("validated to fit in 7 bits"),
    };
    Ok([
        cc(param_msb, (c.param >> 7) as u8),
        cc(param_lsb, (c.param & 0x7F) as u8),
        cc(6, (c.value >> 7) as u8),
        cc(38, (c.value & 0x7F) as u8),
    ])
}

fn data7(value: i64) -> Option<DataByte> {
    u8::try_from(value)
        .ok()
        .and_then(|b| DataByte::try_from(b).ok())
}

fn channel_oob(base: u8, channel: u8) -> CodecError {
    CodecError::Parse {
        reason: ParseError::ChannelOutOfRange,
        bytes: vec![base, channel],
    }
}

fn data_oob(status: u8, data: &[i64]) -> CodecError {
    let mut bytes = Vec::with_capacity(1 + data.len());
    bytes.push(status);
    bytes.extend(data.iter().map(|&d| d.clamp(0, 0xFF) as u8));
    CodecError::Parse {
        reason: ParseError::DataByteOutOfRange,
        bytes,
    }
}

fn note_message(base: u8, channel: u8, note: u8, velocity: u8) -> Result<MidiMessage, CodecError> {
    let Ok(channel) = Channel::from_index(channel) else {
        return Err(channel_oob(base, channel));
    };
    let (Some(key), Some(vel)) = (data7(note as i64), data7(velocity as i64)) else {
        return Err(data_oob(
            base | channel.index(),
            &[note as i64, velocity as i64],
        ));
    };
    Ok(if base == 0x90 && velocity != 0 {
        MidiMessage::NoteOn {
            channel,
            key,
            velocity: vel,
        }
    } else {
        MidiMessage::NoteOff {
            channel,
            key,
            velocity: vel,
        }
    })
}

fn message_from_event(ev: &alsa::seq::Event) -> Option<Result<MidiMessage, CodecError>> {
    Some(match ev.get_type() {
        EventType::Noteon => {
            let n = ev.get_data::<EvNote>()?;
            note_message(0x90, n.channel, n.note, n.velocity)
        }
        EventType::Noteoff => {
            let n = ev.get_data::<EvNote>()?;
            note_message(0x80, n.channel, n.note, n.velocity)
        }
        EventType::Controller => {
            let c = ev.get_data::<EvCtrl>()?;
            match (
                Channel::from_index(c.channel),
                data7(c.param as i64),
                data7(c.value as i64),
            ) {
                (Ok(channel), Some(controller), Some(value)) => Ok(MidiMessage::ControlChange {
                    channel,
                    controller,
                    value,
                }),
                (Err(_), ..) => Err(channel_oob(0xB0, c.channel)),
                _ => Err(data_oob(
                    0xB0 | (c.channel & 0x0F),
                    &[c.param as i64, c.value as i64],
                )),
            }
        }
        EventType::Pgmchange => {
            let c = ev.get_data::<EvCtrl>()?;
            match (Channel::from_index(c.channel), data7(c.value as i64)) {
                (Ok(channel), Some(program)) => Ok(MidiMessage::ProgramChange { channel, program }),
                (Err(_), _) => Err(channel_oob(0xC0, c.channel)),
                _ => Err(data_oob(0xC0 | (c.channel & 0x0F), &[c.value as i64])),
            }
        }
        EventType::Chanpress => {
            let c = ev.get_data::<EvCtrl>()?;
            match (Channel::from_index(c.channel), data7(c.value as i64)) {
                (Ok(channel), Some(pressure)) => {
                    Ok(MidiMessage::ChannelPressure { channel, pressure })
                }
                (Err(_), _) => Err(channel_oob(0xD0, c.channel)),
                _ => Err(data_oob(0xD0 | (c.channel & 0x0F), &[c.value as i64])),
            }
        }
        EventType::Pitchbend => {
            let c = ev.get_data::<EvCtrl>()?;
            match (
                Channel::from_index(c.channel),
                i16::try_from(c.value)
                    .ok()
                    .and_then(|v| PitchBend::from_signed(v).ok()),
            ) {
                (Ok(channel), Some(value)) => Ok(MidiMessage::PitchBend { channel, value }),
                (Err(_), _) => Err(channel_oob(0xE0, c.channel)),
                _ => Err(data_oob(0xE0 | (c.channel & 0x0F), &[c.value as i64])),
            }
        }
        EventType::Keypress => {
            let n = ev.get_data::<EvNote>()?;
            match (
                Channel::from_index(n.channel),
                data7(n.note as i64),
                data7(n.velocity as i64),
            ) {
                (Ok(channel), Some(key), Some(pressure)) => Ok(MidiMessage::PolyKeyPressure {
                    channel,
                    key,
                    pressure,
                }),
                (Err(_), ..) => Err(channel_oob(0xA0, n.channel)),
                _ => Err(data_oob(
                    0xA0 | (n.channel & 0x0F),
                    &[n.note as i64, n.velocity as i64],
                )),
            }
        }
        EventType::Qframe => {
            let c = ev.get_data::<EvCtrl>()?;
            match data7(c.value as i64) {
                Some(frame) => Ok(MidiMessage::MtcQuarterFrame(frame)),
                None => Err(data_oob(0xF1, &[c.value as i64])),
            }
        }
        EventType::Songpos => {
            let c = ev.get_data::<EvCtrl>()?;
            match u16::try_from(c.value)
                .ok()
                .and_then(|v| SongPosition::try_from(v).ok())
            {
                Some(position) => Ok(MidiMessage::SongPositionPointer(position)),
                None => Err(data_oob(0xF2, &[c.value as i64])),
            }
        }
        EventType::Songsel => {
            let c = ev.get_data::<EvCtrl>()?;
            match data7(c.value as i64) {
                Some(song) => Ok(MidiMessage::SongSelect(song)),
                None => Err(data_oob(0xF3, &[c.value as i64])),
            }
        }
        EventType::TuneRequest => Ok(MidiMessage::TuneRequest),
        EventType::Clock => Ok(MidiMessage::TimingClock),
        EventType::Start => Ok(MidiMessage::Start),
        EventType::Continue => Ok(MidiMessage::Continue),
        EventType::Stop => Ok(MidiMessage::Stop),
        EventType::Sensing => Ok(MidiMessage::ActiveSensing),
        EventType::Reset => Ok(MidiMessage::Reset),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Channel;

    fn make_conn() -> (ConnectionState, StreamReceivers) {
        let (senders, receivers) = StreamSenders::channel();
        (
            ConnectionState {
                senders,
                parser: StreamParser::new(),
            },
            receivers,
        )
    }

    #[test]
    fn key_round_trips_through_port_id() {
        for (client, port) in [(0, 0), (1, 2), (-1, -1), (i32::MAX, i32::MIN), (128, 3)] {
            let round = port_to_key(&key_to_id(AlsaPortKey(client, port)));
            assert_eq!((round.0, round.1), (client, port));
        }
    }

    #[test]
    fn add_efd_poll_appends_pollin_entry() {
        let mut pfds: Vec<libc::pollfd> = Vec::new();
        add_efd_poll(&mut pfds, 99);
        assert_eq!(pfds.len(), 1);
        assert_eq!(pfds[0].fd, 99);
        assert_eq!(pfds[0].events, libc::POLLIN);
        assert_eq!(pfds[0].revents, 0);
    }

    #[test]
    fn message_from_event_returns_none_for_unmapped_type() {
        let ev = alsa::seq::Event::new(alsa::seq::EventType::None, &());
        assert!(message_from_event(&ev).is_none());
    }

    #[test]
    fn noteoff_preserves_velocity() {
        let ev = alsa::seq::Event::new(
            alsa::seq::EventType::Noteoff,
            &EvNote {
                channel: 0,
                note: 60,
                velocity: 64,
                off_velocity: 0,
                duration: 0,
            },
        );
        let msg = message_from_event(&ev).unwrap().unwrap();
        assert_eq!(
            msg,
            MidiMessage::NoteOff {
                channel: Channel::Ch1,
                key: DataByte::try_from(60).unwrap(),
                velocity: DataByte::try_from(64).unwrap(),
            }
        );
    }

    #[test]
    fn noteon_out_of_range_channel_is_error() {
        let ev = alsa::seq::Event::new(
            alsa::seq::EventType::Noteon,
            &EvNote {
                channel: 16,
                note: 60,
                velocity: 64,
                off_velocity: 0,
                duration: 0,
            },
        );
        let err = message_from_event(&ev).unwrap().unwrap_err();
        assert!(matches!(
            err,
            CodecError::Parse {
                reason: ParseError::ChannelOutOfRange,
                ..
            }
        ));
    }

    #[test]
    fn controller_out_of_range_channel_is_error() {
        let ev = alsa::seq::Event::new(
            alsa::seq::EventType::Controller,
            &EvCtrl {
                channel: 200,
                param: 7,
                value: 64,
            },
        );
        let err = message_from_event(&ev).unwrap().unwrap_err();
        assert!(matches!(
            err,
            CodecError::Parse {
                reason: ParseError::ChannelOutOfRange,
                ..
            }
        ));
    }

    #[test]
    fn controller_out_of_range_value_is_error() {
        let ev = alsa::seq::Event::new(
            alsa::seq::EventType::Controller,
            &EvCtrl {
                channel: 0,
                param: 7,
                value: 200,
            },
        );
        let err = message_from_event(&ev).unwrap().unwrap_err();
        assert!(matches!(
            err,
            CodecError::Parse {
                reason: ParseError::DataByteOutOfRange,
                ..
            }
        ));
    }

    #[test]
    fn noteon_out_of_range_key_is_error() {
        let ev = alsa::seq::Event::new(
            alsa::seq::EventType::Noteon,
            &EvNote {
                channel: 0,
                note: 200,
                velocity: 64,
                off_velocity: 0,
                duration: 0,
            },
        );
        let err = message_from_event(&ev).unwrap().unwrap_err();
        assert!(matches!(
            err,
            CodecError::Parse {
                reason: ParseError::DataByteOutOfRange,
                ..
            }
        ));
    }

    #[test]
    fn sysex_complete_message_sends_body() {
        let (mut conn, (_msg_rx, mut sysex_rx, _err_rx)) = make_conn();
        handle_sysex_data(&[0xF0, 0x01, 0x02, 0xF7], &mut conn, Instant::now());
        let sysex = sysex_rx.try_recv().unwrap().payload;
        assert_eq!(sysex.bytes(), &[0x01, 0x02]);
    }

    #[test]
    fn sysex_orphaned_fragment_emits_single_error() {
        let (mut conn, (_msg_rx, mut sysex_rx, mut err_rx)) = make_conn();
        handle_sysex_data(&[0x02, 0x03, 0xF7], &mut conn, Instant::now());
        assert!(sysex_rx.try_recv().is_err());
        let err = err_rx.try_recv().unwrap();
        assert!(matches!(
            err.payload,
            Error::Codec(CodecError::Parse {
                reason: ParseError::OrphanedData { len: 3 },
                ref bytes,
            }) if bytes == &vec![0x02, 0x03, 0xF7]
        ));
        assert!(err_rx.try_recv().is_err());
    }

    #[test]
    fn sysex_orphan_run_flushes_on_new_message() {
        let (mut conn, (_msg_rx, mut sysex_rx, mut err_rx)) = make_conn();
        handle_sysex_data(&[0x02, 0x03], &mut conn, Instant::now());
        assert!(err_rx.try_recv().is_err());
        handle_sysex_data(&[0xF0, 0x05, 0xF7], &mut conn, Instant::now());
        let err = err_rx.try_recv().unwrap();
        assert!(matches!(
            err.payload,
            Error::Codec(CodecError::Parse {
                reason: ParseError::OrphanedData { len: 2 },
                ref bytes,
            }) if bytes == &vec![0x02, 0x03]
        ));
        let sysex = sysex_rx.try_recv().unwrap().payload;
        assert_eq!(sysex.bytes(), &[0x05]);
    }

    #[test]
    fn sysex_orphan_error_caps_stored_bytes() {
        let (mut conn, (_msg_rx, _sysex_rx, mut err_rx)) = make_conn();
        handle_sysex_data(&[0x01; 100], &mut conn, Instant::now());
        assert!(err_rx.try_recv().is_err());
        let mut tail = vec![0x02u8; 49];
        tail.push(0xF7);
        handle_sysex_data(&tail, &mut conn, Instant::now());
        let err = err_rx.try_recv().unwrap();
        assert!(matches!(
            err.payload,
            Error::Codec(CodecError::Parse {
                reason: ParseError::OrphanedData { len: 150 },
                ref bytes,
            }) if bytes == &vec![0x01; ORPHAN_PREFIX_BYTES]
        ));
        assert!(err_rx.try_recv().is_err());
    }

    #[test]
    fn sysex_oversized_multichunk_single_error_then_recovers() {
        let (mut conn, (_msg_rx, mut sysex_rx, mut err_rx)) = make_conn();
        let big: Vec<u8> = std::iter::once(0xF0)
            .chain(std::iter::repeat(0x01).take(MAX_SYSEX_BYTES))
            .collect();
        handle_sysex_data(&big, &mut conn, Instant::now());
        assert!(err_rx.try_recv().is_err());
        handle_sysex_data(&[0x01, 0x01], &mut conn, Instant::now());
        let err = err_rx.try_recv().unwrap();
        assert!(matches!(
            err.payload,
            Error::Codec(CodecError::SysexTooLong { .. })
        ));
        handle_sysex_data(&[0x01, 0xF7], &mut conn, Instant::now());
        assert!(err_rx.try_recv().is_err());
        handle_sysex_data(&[0xF0, 0x05, 0xF7], &mut conn, Instant::now());
        assert!(err_rx.try_recv().is_err());
        let sysex = sysex_rx.try_recv().unwrap().payload;
        assert_eq!(sysex.bytes(), &[0x05]);
    }

    #[test]
    fn sysex_two_chunk_message_sends_after_f7() {
        let (mut conn, (_msg_rx, mut sysex_rx, _err_rx)) = make_conn();
        handle_sysex_data(&[0xF0, 0x01], &mut conn, Instant::now());
        assert!(sysex_rx.try_recv().is_err());
        handle_sysex_data(&[0x02, 0xF7], &mut conn, Instant::now());
        let sysex = sysex_rx.try_recv().unwrap().payload;
        assert_eq!(sysex.bytes(), &[0x01, 0x02]);
    }

    #[test]
    fn sysex_restart_emits_unterminated_error() {
        let (mut conn, (_msg_rx, mut sysex_rx, mut err_rx)) = make_conn();
        handle_sysex_data(&[0xF0, 0x01], &mut conn, Instant::now());
        handle_sysex_data(&[0xF0, 0x02, 0xF7], &mut conn, Instant::now());
        let err = err_rx.try_recv().unwrap();
        assert!(matches!(
            err.payload,
            Error::Codec(CodecError::Parse {
                reason: ParseError::UnterminatedSysex,
                ref bytes,
            }) if bytes == &vec![0xF0, 0x01]
        ));
        let sysex = sysex_rx.try_recv().unwrap().payload;
        assert_eq!(sysex.bytes(), &[0x02]);
    }

    #[test]
    fn sysex_max_size_body_accepted() {
        let (mut conn, (_msg_rx, mut sysex_rx, mut err_rx)) = make_conn();
        let big: Vec<u8> = std::iter::once(0xF0)
            .chain(std::iter::repeat(0x01).take(MAX_SYSEX_BYTES))
            .chain(std::iter::once(0xF7))
            .collect();
        handle_sysex_data(&big, &mut conn, Instant::now());
        assert!(err_rx.try_recv().is_err());
        let sysex = sysex_rx.try_recv().unwrap().payload;
        assert_eq!(sysex.bytes().len(), MAX_SYSEX_BYTES);
    }

    #[test]
    fn sysex_oversized_clears_buffer() {
        let (mut conn, (_msg_rx, _sysex_rx, mut err_rx)) = make_conn();
        let big: Vec<u8> = std::iter::once(0xF0)
            .chain(std::iter::repeat(0x01).take(MAX_SYSEX_BYTES + 1))
            .chain(std::iter::once(0xF7))
            .collect();
        handle_sysex_data(&big, &mut conn, Instant::now());
        let err = err_rx.try_recv().unwrap();
        assert!(matches!(
            err.payload,
            Error::Codec(CodecError::SysexTooLong {
                len,
                max: MAX_SYSEX_BYTES,
            }) if len == MAX_SYSEX_BYTES + 1
        ));
        assert!(err_rx.try_recv().is_err());
    }

    #[test]
    fn sysex_partial_accumulates_no_send() {
        let (mut conn, (_msg_rx, mut sysex_rx, _err_rx)) = make_conn();
        handle_sysex_data(&[0xF0, 0x01, 0x02], &mut conn, Instant::now());
        assert!(sysex_rx.try_recv().is_err());
        handle_sysex_data(&[0xF7], &mut conn, Instant::now());
        let sysex = sysex_rx.try_recv().unwrap().payload;
        assert_eq!(sysex.bytes(), &[0x01, 0x02]);
    }

    #[test]
    fn conformance_message_from_event_matches_corpus() {
        for (bytes, expected) in crate::midi::conformance::all() {
            let mut coder = MidiEvent::new(8).unwrap();
            coder.enable_running_status(false);
            let (_, ev) = coder.encode(&bytes).unwrap();
            let ev = ev.unwrap_or_else(|| panic!("no event for {bytes:02X?}"));
            let msg = message_from_event(&ev)
                .unwrap_or_else(|| panic!("message_from_event None for {bytes:02X?}"))
                .unwrap_or_else(|e| panic!("message_from_event Err {e:?} for {bytes:02X?}"));
            assert_eq!(msg, expected, "alsa decode {bytes:02X?}");
        }
    }

    #[test]
    fn conformance_sysex_push_matches_codec_body() {
        let (mut conn, (_msg_rx, mut sysex_rx, _err_rx)) = make_conn();
        handle_sysex_data(
            &crate::midi::conformance::SYSEX_FRAME,
            &mut conn,
            Instant::now(),
        );
        let body = sysex_rx.try_recv().unwrap().payload;
        let codec = match crate::decode(&crate::midi::conformance::SYSEX_FRAME) {
            Ok(crate::Decoded::SysEx(s)) => s,
            other => panic!("codec sysex: {other:?}"),
        };
        assert_eq!(body.bytes(), codec.bytes());
    }

    #[test]
    fn sysex_chunk_with_status_byte_aborts_and_parses_message() {
        let (mut conn, (mut msg_rx, _sysex_rx, mut err_rx)) = make_conn();
        handle_sysex_data(&[0xF0, 0x01, 0x90, 0x3C, 0x64], &mut conn, Instant::now());
        let err = err_rx.try_recv().unwrap();
        assert!(matches!(
            err.payload,
            Error::Codec(CodecError::Parse {
                reason: ParseError::UnterminatedSysex,
                ref bytes,
            }) if bytes == &vec![0xF0, 0x01]
        ));
        let msg = msg_rx.try_recv().unwrap().payload;
        assert_eq!(
            msg,
            MidiMessage::NoteOn {
                channel: Channel::Ch1,
                key: DataByte::try_from(0x3C).unwrap(),
                velocity: DataByte::try_from(0x64).unwrap(),
            }
        );
    }

    fn test_registry(
        source_tx: mpsc::UnboundedSender<SourceChange>,
        dest_tx: mpsc::UnboundedSender<DestinationChange>,
    ) -> PortRegistry {
        PortRegistry::new(
            Arc::new(Mutex::new(vec![source_tx])),
            Arc::new(Mutex::new(vec![dest_tx])),
            MidiEvent::new(4).unwrap(),
        )
    }

    #[test]
    fn is_virtual_reflects_dynamic_client_range() {
        assert!(Source::from_alsa(AlsaPortKey(SNDRV_SEQ_DYNAMIC_CLIENTS_BEGIN, 0), "P").is_virtual);
        assert!(
            !Source::from_alsa(AlsaPortKey(SNDRV_SEQ_DYNAMIC_CLIENTS_BEGIN - 1, 0), "P").is_virtual
        );
        assert!(Destination::from_alsa(AlsaPortKey(200, 3), "P").is_virtual);
        assert!(!Destination::from_alsa(AlsaPortKey(14, 0), "P").is_virtual);
    }

    #[test]
    fn handle_port_exit_removes_port_and_notifies() {
        let (tx, mut rx) = mpsc::unbounded();
        let (out_tx, _out_rx) = mpsc::unbounded();
        let mut reg = test_registry(tx, out_tx);
        let key = AlsaPortKey(10, 1);
        reg.source_cache.insert(key, Source::from_alsa(key, "Port"));
        reg.handle_port_exit(Addr {
            client: 10,
            port: 1,
        });
        assert!(!reg.source_cache.contains_key(&key));
        let change = rx.try_recv().map(Some).unwrap();
        assert!(matches!(change, Some(SourceChange::Removed(_))));
    }

    #[test]
    fn handle_port_exit_retains_connection_and_emits_disconnect() {
        let (changes_tx, _changes_rx) = mpsc::unbounded();
        let (out_tx, _out_rx) = mpsc::unbounded();
        let mut reg = test_registry(changes_tx, out_tx);
        let key = AlsaPortKey(10, 1);
        reg.source_cache.insert(key, Source::from_alsa(key, "Port"));
        let (conn, (_msg_rx, _sysex_rx, mut err_rx)) = make_conn();
        reg.connections.insert(key, conn);
        reg.handle_port_exit(Addr {
            client: 10,
            port: 1,
        });
        assert!(reg.connections.contains_key(&key));
        let err = err_rx.try_recv().unwrap();
        assert!(matches!(err.payload, Error::Io(IoError::PortDisconnected)));
    }

    #[test]
    fn handle_port_exit_unknown_port_is_noop() {
        let (tx, mut rx) = mpsc::unbounded();
        let (out_tx, _out_rx) = mpsc::unbounded();
        let mut reg = test_registry(tx, out_tx);
        reg.handle_port_exit(Addr {
            client: 99,
            port: 5,
        });
        assert!(rx.try_recv().map(Some).is_err());
    }

    #[test]
    fn handle_client_exit_removes_all_client_ports() {
        let (tx, mut rx) = mpsc::unbounded();
        let (out_tx, _out_rx) = mpsc::unbounded();
        let mut reg = test_registry(tx, out_tx);
        for key in [AlsaPortKey(5, 0), AlsaPortKey(5, 1), AlsaPortKey(7, 0)] {
            reg.source_cache.insert(key, Source::from_alsa(key, "P"));
        }
        reg.handle_client_exit(5);
        assert!(!reg.source_cache.contains_key(&AlsaPortKey(5, 0)));
        assert!(!reg.source_cache.contains_key(&AlsaPortKey(5, 1)));
        assert!(reg.source_cache.contains_key(&AlsaPortKey(7, 0)));
        let mut count = 0;
        while rx.try_recv().map(Some).is_ok() {
            count += 1;
        }
        assert_eq!(count, 2);
    }

    #[test]
    fn resolve_output_state_requires_live_connection() {
        let (tx, _rx) = mpsc::unbounded();
        let (out_tx, _out_rx) = mpsc::unbounded();
        let mut reg = test_registry(tx, out_tx);
        let key = AlsaPortKey(30, 0);
        assert!(matches!(
            reg.resolve_output_state(&key),
            Err(Error::Io(IoError::PortNotFound))
        ));
        reg.destination_connections.insert(
            key,
            DestinationConnectionState {
                disconnected: false,
            },
        );
        assert!(reg.resolve_output_state(&key).is_ok());
        reg.destination_connections
            .get_mut(&key)
            .unwrap()
            .disconnected = true;
        assert!(matches!(
            reg.resolve_output_state(&key),
            Err(Error::Io(IoError::PortDisconnected))
        ));
    }

    #[test]
    fn handle_port_exit_marks_active_output_connection_disconnected() {
        let (tx, _rx) = mpsc::unbounded();
        let (out_tx, _out_rx) = mpsc::unbounded();
        let mut reg = test_registry(tx, out_tx);
        let key = AlsaPortKey(20, 2);
        reg.destination_cache
            .insert(key, Destination::from_alsa(key, "Port 0"));
        reg.destination_connections.insert(
            key,
            DestinationConnectionState {
                disconnected: false,
            },
        );
        reg.handle_port_exit(Addr {
            client: 20,
            port: 2,
        });
        assert!(!reg.destination_cache.contains_key(&key));
        let conn = reg.destination_connections.get(&key).unwrap();
        assert!(
            conn.disconnected,
            "output connection must be marked disconnected"
        );
    }

    #[test]
    fn handle_client_exit_marks_destination_connections_disconnected() {
        let (tx, _rx) = mpsc::unbounded();
        let (out_tx, _out_rx) = mpsc::unbounded();
        let mut reg = test_registry(tx, out_tx);
        for key in [AlsaPortKey(8, 0), AlsaPortKey(8, 1)] {
            reg.destination_cache
                .insert(key, Destination::from_alsa(key, "P"));
        }
        reg.destination_connections.insert(
            AlsaPortKey(8, 0),
            DestinationConnectionState {
                disconnected: false,
            },
        );
        reg.handle_client_exit(8);
        assert!(!reg.destination_cache.contains_key(&AlsaPortKey(8, 0)));
        assert!(!reg.destination_cache.contains_key(&AlsaPortKey(8, 1)));
        let conn = reg.destination_connections.get(&AlsaPortKey(8, 0)).unwrap();
        assert!(conn.disconnected);
    }

    #[test]
    fn control14_event_not_handled_by_message_from_event() {
        let ev = alsa::seq::Event::new(
            alsa::seq::EventType::Control14,
            &EvCtrl {
                channel: 1,
                param: 7,
                value: 8192,
            },
        );
        assert!(message_from_event(&ev).is_none());
    }

    #[test]
    fn nonregparam_event_not_handled_by_message_from_event() {
        let ev = alsa::seq::Event::new(
            alsa::seq::EventType::Nonregparam,
            &EvCtrl {
                channel: 0,
                param: 0,
                value: 0,
            },
        );
        assert!(message_from_event(&ev).is_none());
    }

    #[test]
    fn regparam_event_not_handled_by_message_from_event() {
        let ev = alsa::seq::Event::new(
            alsa::seq::EventType::Regparam,
            &EvCtrl {
                channel: 5,
                param: 42,
                value: 16383,
            },
        );
        assert!(message_from_event(&ev).is_none());
    }

    #[test]
    fn control14_to_cc_pair_splits_msb_and_lsb() {
        let c = EvCtrl {
            channel: 1,
            param: 7,
            value: 8192,
        };
        let [msb, lsb] = control14_to_cc_pair(&c).unwrap();
        assert_eq!(
            msb,
            MidiMessage::ControlChange {
                channel: Channel::Ch2,
                controller: DataByte::try_from(7).unwrap(),
                value: DataByte::try_from(64).unwrap(),
            }
        );
        assert_eq!(
            lsb,
            MidiMessage::ControlChange {
                channel: Channel::Ch2,
                controller: DataByte::try_from(39).unwrap(),
                value: DataByte::try_from(0).unwrap(),
            }
        );
    }

    #[test]
    fn control14_to_cc_pair_preserves_lsb() {
        let c = EvCtrl {
            channel: 0,
            param: 1,
            value: 0b_0000001_1111111,
        };
        let [msb, lsb] = control14_to_cc_pair(&c).unwrap();
        assert_eq!(
            msb,
            MidiMessage::ControlChange {
                channel: Channel::Ch1,
                controller: DataByte::try_from(1).unwrap(),
                value: DataByte::try_from(1).unwrap(),
            }
        );
        assert_eq!(
            lsb,
            MidiMessage::ControlChange {
                channel: Channel::Ch1,
                controller: DataByte::try_from(33).unwrap(),
                value: DataByte::try_from(127).unwrap(),
            }
        );
    }

    #[test]
    fn control14_to_cc_pair_rejects_out_of_range_param() {
        let c = EvCtrl {
            channel: 0,
            param: 32,
            value: 0,
        };
        let err = control14_to_cc_pair(&c).unwrap_err();
        assert!(matches!(
            err,
            CodecError::Parse {
                reason: ParseError::DataByteOutOfRange,
                ..
            }
        ));
    }

    #[test]
    fn control14_to_cc_pair_rejects_out_of_range_value() {
        for value in [-1, 0x4000] {
            let c = EvCtrl {
                channel: 0,
                param: 7,
                value,
            };
            let err = control14_to_cc_pair(&c).unwrap_err();
            assert!(matches!(
                err,
                CodecError::Parse {
                    reason: ParseError::DataByteOutOfRange,
                    ..
                }
            ));
        }
    }

    #[test]
    fn control14_to_cc_pair_rejects_out_of_range_channel() {
        let c = EvCtrl {
            channel: 16,
            param: 7,
            value: 0,
        };
        let err = control14_to_cc_pair(&c).unwrap_err();
        assert!(matches!(
            err,
            CodecError::Parse {
                reason: ParseError::ChannelOutOfRange,
                ..
            }
        ));
    }

    #[test]
    fn param_cc_messages_splits_param_and_value() {
        let c = EvCtrl {
            channel: 1,
            param: 0b_0000010_0000011,
            value: 0b_0000100_0000101,
        };
        let msgs = param_cc_messages(&c, [99, 98]).unwrap();
        let expected = [(99u8, 2u8), (98, 3), (6, 4), (38, 5)];
        for (msg, (controller, value)) in msgs.iter().zip(expected) {
            assert_eq!(
                *msg,
                MidiMessage::ControlChange {
                    channel: Channel::Ch2,
                    controller: DataByte::try_from(controller).unwrap(),
                    value: DataByte::try_from(value).unwrap(),
                }
            );
        }
    }

    #[test]
    fn param_cc_messages_rejects_out_of_range_fields() {
        let oob = [
            EvCtrl {
                channel: 0,
                param: 0x4000,
                value: 0,
            },
            EvCtrl {
                channel: 0,
                param: 0,
                value: -1,
            },
            EvCtrl {
                channel: 0,
                param: 0,
                value: 0x4000,
            },
        ];
        for c in oob {
            let err = param_cc_messages(&c, [101, 100]).unwrap_err();
            assert!(matches!(
                err,
                CodecError::Parse {
                    reason: ParseError::DataByteOutOfRange,
                    ..
                }
            ));
        }
        let bad_channel = EvCtrl {
            channel: 16,
            param: 0,
            value: 0,
        };
        let err = param_cc_messages(&bad_channel, [101, 100]).unwrap_err();
        assert!(matches!(
            err,
            CodecError::Parse {
                reason: ParseError::ChannelOutOfRange,
                ..
            }
        ));
    }

    #[test]
    fn inbound_overflow_dropped_and_coalesced() {
        let (senders, (mut m, _x, mut e)) = StreamSenders::channel();
        let ts = Instant::now();

        for _ in 0..(INBOUND_CHANNEL_CAPACITY + 5) {
            senders.send_message(ts, crate::MidiMessage::Reset);
        }

        for _ in 0..INBOUND_CHANNEL_CAPACITY {
            let _ = m.try_recv();
        }

        senders.send_message(ts, crate::MidiMessage::Reset);

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

        senders.send_message(ts, crate::MidiMessage::Reset);
        let _ = m.try_recv();

        senders.send_message(ts, crate::MidiMessage::Reset);
        let _ = m.try_recv();

        assert!(
            e.try_recv().is_err(),
            "no overflow should occur in steady state"
        );
    }
}
