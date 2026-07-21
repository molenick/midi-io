use std::sync::atomic::AtomicU64;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;

use futures_channel::mpsc;
use futures_channel::oneshot;

use super::log_error;
use super::map_send_err;
use super::MutexExt;
use crate::midi::stream_parser::DecodedEvent;
use crate::name::Name;
use crate::port::VirtualPortId;
use crate::time::Instant;
use crate::Destination;
use crate::DestinationChange;
use crate::Error;
use crate::IoError;
use crate::MidiMessage;
use crate::PortId;
use crate::RawMidiMessage;
use crate::Source;
use crate::SourceChange;
use crate::SysEx;
use crate::Timed;

/// Command channel has a maximum capacity to prevent unlimited memory allocations. Once the limit is reached, old messages drop to accomodate for new.
pub(super) const COMMAND_CHANNEL_CAPACITY: usize = 512;
/// Inbound channels have a maximum capacity to prevent unlimited memory allocations. Once the limit is reached, old messages drop to accomodate for new.
pub(super) const INBOUND_CHANNEL_CAPACITY: usize = 1024;

pub(super) type SourceSubscribers = Arc<Mutex<Vec<mpsc::UnboundedSender<SourceChange>>>>;
pub(super) type DestinationSubscribers = Arc<Mutex<Vec<mpsc::UnboundedSender<DestinationChange>>>>;

pub(super) type StreamReceivers = (
    mpsc::Receiver<Timed<MidiMessage>>,
    mpsc::Receiver<Timed<SysEx>>,
    mpsc::Receiver<Timed<Error>>,
);

#[derive(Clone)]
pub(super) struct StreamSenders {
    message: Arc<Mutex<mpsc::Sender<Timed<MidiMessage>>>>,
    sysex: Arc<Mutex<mpsc::Sender<Timed<SysEx>>>>,
    error: Arc<Mutex<mpsc::Sender<Timed<Error>>>>,
    dropped: Arc<AtomicUsize>,
}

impl StreamSenders {
    pub(super) fn channel() -> (StreamSenders, StreamReceivers) {
        let (message, message_rx) = mpsc::channel(INBOUND_CHANNEL_CAPACITY);
        let (sysex, sysex_rx) = mpsc::channel(INBOUND_CHANNEL_CAPACITY);
        let (error, error_rx) = mpsc::channel(INBOUND_CHANNEL_CAPACITY);
        (
            StreamSenders {
                message: Arc::new(Mutex::new(message)),
                sysex: Arc::new(Mutex::new(sysex)),
                error: Arc::new(Mutex::new(error)),
                dropped: Arc::new(AtomicUsize::new(0)),
            },
            (message_rx, sysex_rx, error_rx),
        )
    }

    fn report_overflow(&self, timestamp: Instant) {
        let n = self.dropped.load(Ordering::Relaxed);
        if n > 0 {
            if let Ok(mut tx) = self.error.try_lock() {
                if tx
                    .try_send(Timed {
                        timestamp,
                        payload: IoError::InboundOverflow { dropped: n }.into(),
                    })
                    .is_ok()
                {
                    self.dropped.fetch_sub(n, Ordering::Relaxed);
                }
            }
        }
    }

    fn try_send_counted<T>(
        &self,
        channel: &Mutex<mpsc::Sender<Timed<T>>>,
        timestamp: Instant,
        payload: T,
    ) {
        match channel.try_lock() {
            Ok(mut tx) => match tx.try_send(Timed { timestamp, payload }) {
                Ok(()) => {
                    drop(tx);
                    self.report_overflow(timestamp);
                }
                Err(e) if e.is_full() => {
                    self.dropped.fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => {}
            },
            Err(_) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(super) fn lifecycle_error(&self, error: impl Into<Error>) {
        let mut tx = self.error.lock_unpoisoned();
        let _ = tx.try_send(Timed {
            timestamp: Instant::now(),
            payload: error.into(),
        });
    }

    #[cfg(target_os = "linux")]
    pub(super) fn send_message(&self, timestamp: Instant, msg: MidiMessage) {
        self.try_send_counted(&self.message, timestamp, msg);
    }

    #[cfg(target_os = "linux")]
    pub(super) fn send_error(&self, timestamp: Instant, error: Error) {
        let mut tx = self.error.lock_unpoisoned();
        let _ = tx.try_send(Timed {
            timestamp,
            payload: error,
        });
    }

    pub(super) fn emit(&self, timestamp: Instant, event: DecodedEvent) {
        match event {
            DecodedEvent::Message(msg) => self.try_send_counted(&self.message, timestamp, msg),
            DecodedEvent::Sysex(sysex) => self.try_send_counted(&self.sysex, timestamp, sysex),
            DecodedEvent::Error(error) => {
                let mut tx = self.error.lock_unpoisoned();
                let _ = tx.try_send(Timed {
                    timestamp,
                    payload: error,
                });
            }
        }
    }
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(super) enum Command {
    ConnectSource {
        port_id: PortId,
        reply: oneshot::Sender<Result<StreamReceivers, Error>>,
    },
    Disconnect(PortId),
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    Shutdown,
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    BackendDied,
    ConnectDestination {
        port_id: PortId,
        reply: oneshot::Sender<Result<(), Error>>,
    },
    SendMidi {
        port_id: PortId,
        msg: RawMidiMessage,
        reply: oneshot::Sender<Result<(), Error>>,
    },
    SendSysex {
        port_id: PortId,
        data: Vec<u8>,
        reply: oneshot::Sender<Result<(), Error>>,
    },
    DisconnectDestination(PortId),
    CreateVirtualSource {
        id: VirtualPortId,
        name: Name,
        reply: oneshot::Sender<Result<PortId, Error>>,
    },
    DestroyVirtualSource(VirtualPortId),
    CreateVirtualDestination {
        id: VirtualPortId,
        name: Name,
        reply: oneshot::Sender<Result<(PortId, StreamReceivers), Error>>,
    },
    SendVirtualMidi {
        id: VirtualPortId,
        msg: RawMidiMessage,
        reply: oneshot::Sender<Result<(), Error>>,
    },
    SendVirtualSysex {
        id: VirtualPortId,
        data: Vec<u8>,
        reply: oneshot::Sender<Result<(), Error>>,
    },
    DestroyVirtualDestination(VirtualPortId),
    ListSources {
        reply: oneshot::Sender<Result<Vec<Source>, Error>>,
    },
    ListDestinations {
        reply: oneshot::Sender<Result<Vec<Destination>, Error>>,
    },
}

pub(super) async fn request<R>(
    cmd_tx: &std::sync::mpsc::SyncSender<Command>,
    make: impl FnOnce(oneshot::Sender<Result<R, Error>>) -> Command,
    wake: impl FnOnce(),
) -> Result<R, Error> {
    let (reply_tx, reply_rx) = oneshot::channel();
    cmd_tx.try_send(make(reply_tx)).map_err(map_send_err)?;
    wake();
    reply_rx.await.map_err(|_| IoError::BackendThreadDied)?
}

pub(super) fn notify(
    cmd_tx: &std::sync::mpsc::SyncSender<Command>,
    cmd: Command,
    wake: impl FnOnce(),
) {
    if let Err(std::sync::mpsc::TrySendError::Full(cmd)) = cmd_tx.try_send(cmd) {
        match cmd {
            Command::Disconnect(port_id) => {
                log_error!("command channel full; source disconnect lost for {port_id:?}");
            }
            Command::DisconnectDestination(port_id) => {
                log_error!("command channel full; destination disconnect lost for {port_id:?}");
            }
            Command::DestroyVirtualSource(id) => {
                log_error!("command channel full; virtual source destroy lost for {id:?}");
            }
            Command::DestroyVirtualDestination(id) => {
                log_error!("command channel full; virtual destination destroy lost for {id:?}");
            }
            _ => {
                log_error!("command channel full; command lost");
            }
        }
    }
    wake();
}

pub(super) fn prune_send<T: Clone>(subs: &mut Vec<mpsc::UnboundedSender<T>>, change: &T) {
    subs.retain(|tx| tx.unbounded_send(change.clone()).is_ok());
}

pub(crate) struct PlatformClient {
    source_subs: SourceSubscribers,
    destination_subs: DestinationSubscribers,
    cmd_tx: std::sync::mpsc::SyncSender<Command>,
    next_virtual_id: AtomicU64,
    backend: super::Backend,
}

impl std::fmt::Debug for PlatformClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformClient").finish_non_exhaustive()
    }
}

impl PlatformClient {
    pub(crate) fn new(name: Name) -> Result<(Self, oneshot::Receiver<Result<(), Error>>), Error> {
        let source_subs: SourceSubscribers = Arc::new(Mutex::new(Vec::new()));
        let destination_subs: DestinationSubscribers = Arc::new(Mutex::new(Vec::new()));
        let (cmd_tx, cmd_rx) = std::sync::mpsc::sync_channel::<Command>(COMMAND_CHANNEL_CAPACITY);
        let (ready_tx, ready_rx) = oneshot::channel();
        let backend = super::Backend::start(
            name,
            Arc::clone(&source_subs),
            Arc::clone(&destination_subs),
            cmd_rx,
            &cmd_tx,
            ready_tx,
        )?;
        Ok((
            Self {
                source_subs,
                destination_subs,
                cmd_tx,
                next_virtual_id: AtomicU64::new(0),
                backend,
            },
            ready_rx,
        ))
    }

    async fn request<R>(
        &self,
        make: impl FnOnce(oneshot::Sender<Result<R, Error>>) -> Command,
    ) -> Result<R, Error> {
        request(&self.cmd_tx, make, || self.backend.wake()).await
    }

    fn notify(&self, cmd: Command) {
        notify(&self.cmd_tx, cmd, || self.backend.wake());
    }

    pub(crate) async fn sources(&self) -> Result<Vec<Source>, Error> {
        self.request(|reply| Command::ListSources { reply }).await
    }

    pub(crate) async fn destinations(&self) -> Result<Vec<Destination>, Error> {
        self.request(|reply| Command::ListDestinations { reply })
            .await
    }

    pub(crate) fn source_changes_rx(&self) -> mpsc::UnboundedReceiver<SourceChange> {
        let (tx, rx) = mpsc::unbounded();
        self.source_subs.lock_unpoisoned().push(tx);
        rx
    }

    pub(crate) fn destination_changes_rx(&self) -> mpsc::UnboundedReceiver<DestinationChange> {
        let (tx, rx) = mpsc::unbounded();
        self.destination_subs.lock_unpoisoned().push(tx);
        rx
    }

    pub(crate) fn alloc_virtual_id(&self) -> VirtualPortId {
        VirtualPortId(self.next_virtual_id.fetch_add(1, Ordering::Relaxed))
    }

    pub(crate) async fn create_virtual_source(
        &self,
        id: VirtualPortId,
        name: Name,
    ) -> Result<PortId, Error> {
        self.request(|reply| Command::CreateVirtualSource { id, name, reply })
            .await
    }

    pub(crate) async fn create_virtual_destination(
        &self,
        id: VirtualPortId,
        name: Name,
    ) -> Result<(PortId, StreamReceivers), Error> {
        self.request(|reply| Command::CreateVirtualDestination { id, name, reply })
            .await
    }

    pub(crate) async fn connect_source(&self, port: &Source) -> Result<StreamReceivers, Error> {
        self.request(|reply| Command::ConnectSource {
            port_id: port.id,
            reply,
        })
        .await
    }

    pub(crate) async fn connect_destination(&self, port: &Destination) -> Result<(), Error> {
        self.request(|reply| Command::ConnectDestination {
            port_id: port.id,
            reply,
        })
        .await
    }

    pub(crate) fn disconnect(&self, port_id: PortId) {
        self.notify(Command::Disconnect(port_id));
    }

    pub(crate) fn disconnect_destination(&self, port_id: PortId) {
        self.notify(Command::DisconnectDestination(port_id));
    }

    pub(crate) fn destroy_virtual_source(&self, id: VirtualPortId) {
        self.notify(Command::DestroyVirtualSource(id));
    }

    pub(crate) fn destroy_virtual_destination(&self, id: VirtualPortId) {
        self.notify(Command::DestroyVirtualDestination(id));
    }

    pub(crate) async fn send_midi(
        &self,
        port_id: PortId,
        msg: RawMidiMessage,
    ) -> Result<(), Error> {
        self.request(|reply| Command::SendMidi {
            port_id,
            msg,
            reply,
        })
        .await
    }

    pub(crate) async fn send_sysex(&self, port_id: PortId, data: Vec<u8>) -> Result<(), Error> {
        self.request(|reply| Command::SendSysex {
            port_id,
            data,
            reply,
        })
        .await
    }

    pub(crate) async fn send_virtual_midi(
        &self,
        id: VirtualPortId,
        msg: RawMidiMessage,
    ) -> Result<(), Error> {
        self.request(|reply| Command::SendVirtualMidi { id, msg, reply })
            .await
    }

    pub(crate) async fn send_virtual_sysex(
        &self,
        id: VirtualPortId,
        data: Vec<u8>,
    ) -> Result<(), Error> {
        self.request(|reply| Command::SendVirtualSysex { id, data, reply })
            .await
    }
}

impl Drop for PlatformClient {
    fn drop(&mut self) {
        self.backend.on_drop(&self.cmd_tx);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn list_sources_command() -> Command {
        let (reply, _reply_rx) = oneshot::channel();
        Command::ListSources { reply }
    }

    #[test]
    fn notify_does_not_block_on_full_command_channel() {
        let (cmd_tx, cmd_rx) = std::sync::mpsc::sync_channel::<Command>(1);
        cmd_tx.try_send(list_sources_command()).unwrap();

        let (done_tx, done_rx) = std::sync::mpsc::sync_channel::<()>(1);
        let worker = std::thread::spawn(move || {
            notify(&cmd_tx, list_sources_command(), || {});
            let _ = done_tx.send(());
        });

        done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("notify blocked on a full command channel");
        worker.join().unwrap();
        drop(cmd_rx);
    }
}
