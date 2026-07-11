use std::pin::Pin;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;

use futures_channel::mpsc;
use futures_util::stream::select;
use futures_util::Stream;
use futures_util::StreamExt;

use crate::platform::PlatformClient;
use crate::port::VirtualPortId;
use crate::Decoded;
use crate::Destination;
use crate::DestinationChange;
use crate::Error;
use crate::MidiMessage;
use crate::PortId;
use crate::RawMidiMessage;
use crate::Source;
use crate::SourceChange;
use crate::SysEx;

#[derive(Debug, Clone)]
pub struct Timed<T> {
    pub timestamp: std::time::Instant,
    pub payload: T,
}

impl<T> Timed<T> {
    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> Timed<U> {
        Timed {
            timestamp: self.timestamp,
            payload: f(self.payload),
        }
    }
}

#[derive(Debug)]
enum ConnectionId {
    Source(PortId),
    Destination(PortId),
    VirtualSource(VirtualPortId),
    VirtualDestination(VirtualPortId),
}

#[derive(Debug)]
pub(crate) struct ConnectionGuard {
    target: ConnectionId,
    platform: Arc<PlatformClient>,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        match &self.target {
            ConnectionId::Source(port) => self.platform.disconnect(*port),
            ConnectionId::Destination(port) => self.platform.disconnect_destination(*port),
            ConnectionId::VirtualSource(id) => self.platform.destroy_virtual_source(*id),
            ConnectionId::VirtualDestination(id) => self.platform.destroy_virtual_destination(*id),
        }
    }
}

#[derive(Debug)]
struct ReceiveStreams {
    message_rx: mpsc::Receiver<Timed<MidiMessage>>,
    sysex_rx: mpsc::Receiver<Timed<SysEx>>,
    error_rx: mpsc::Receiver<Timed<Error>>,
    guard: Arc<ConnectionGuard>,
}

impl ReceiveStreams {
    fn into_streams(self) -> Streams {
        Streams {
            messages: MessageStream {
                inner: self.message_rx,
                _guard: self.guard.clone(),
            },
            sysex: SysexStream {
                inner: self.sysex_rx,
                _guard: self.guard.clone(),
            },
            errors: ErrorStream {
                inner: self.error_rx,
                _guard: self.guard,
            },
        }
    }

    fn into_messages(self) -> MessageStream {
        MessageStream {
            inner: self.message_rx,
            _guard: self.guard,
        }
    }

    fn into_sysex(self) -> SysexStream {
        SysexStream {
            inner: self.sysex_rx,
            _guard: self.guard,
        }
    }

    fn into_errors(self) -> ErrorStream {
        ErrorStream {
            inner: self.error_rx,
            _guard: self.guard,
        }
    }

    fn into_events(self) -> EventStream {
        EventStream::new(self.message_rx, self.sysex_rx, self.error_rx, self.guard)
    }
}

#[derive(Debug)]
pub struct SourceConnection {
    streams: ReceiveStreams,
}

impl SourceConnection {
    pub(crate) fn new(
        message_rx: mpsc::Receiver<Timed<MidiMessage>>,
        sysex_rx: mpsc::Receiver<Timed<SysEx>>,
        error_rx: mpsc::Receiver<Timed<Error>>,
        port: PortId,
        platform: Arc<crate::platform::PlatformClient>,
    ) -> Self {
        Self {
            streams: ReceiveStreams {
                message_rx,
                sysex_rx,
                error_rx,
                guard: Arc::new(ConnectionGuard {
                    target: ConnectionId::Source(port),
                    platform,
                }),
            },
        }
    }

    pub fn into_streams(self) -> Streams {
        self.streams.into_streams()
    }

    pub fn into_messages(self) -> MessageStream {
        self.streams.into_messages()
    }

    pub fn into_sysex(self) -> SysexStream {
        self.streams.into_sysex()
    }

    pub fn into_errors(self) -> ErrorStream {
        self.streams.into_errors()
    }

    pub fn into_events(self) -> EventStream {
        self.streams.into_events()
    }
}

#[derive(Debug)]
pub struct MessageStream {
    inner: mpsc::Receiver<Timed<MidiMessage>>,
    _guard: Arc<ConnectionGuard>,
}

impl MessageStream {
    pub async fn recv(&mut self) -> Option<Timed<MidiMessage>> {
        self.next().await
    }
}

impl Stream for MessageStream {
    type Item = Timed<MidiMessage>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}

#[derive(Debug)]
pub struct SysexStream {
    inner: mpsc::Receiver<Timed<SysEx>>,
    _guard: Arc<ConnectionGuard>,
}

impl SysexStream {
    pub async fn recv(&mut self) -> Option<Timed<SysEx>> {
        self.next().await
    }
}

impl Stream for SysexStream {
    type Item = Timed<SysEx>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}

#[derive(Debug)]
pub struct ErrorStream {
    inner: mpsc::Receiver<Timed<Error>>,
    _guard: Arc<ConnectionGuard>,
}

impl ErrorStream {
    pub async fn recv(&mut self) -> Option<Timed<Error>> {
        self.next().await
    }
}

impl Stream for ErrorStream {
    type Item = Timed<Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}

#[derive(Debug)]
pub struct Streams {
    pub messages: MessageStream,
    pub sysex: SysexStream,
    pub errors: ErrorStream,
}

#[allow(clippy::type_complexity)]
pub struct EventStream {
    inner: Pin<Box<dyn Stream<Item = Timed<Result<Decoded, Error>>> + Send>>,
    _guard: Arc<ConnectionGuard>,
}

impl EventStream {
    fn new(
        message_rx: mpsc::Receiver<Timed<MidiMessage>>,
        sysex_rx: mpsc::Receiver<Timed<SysEx>>,
        error_rx: mpsc::Receiver<Timed<Error>>,
        guard: Arc<ConnectionGuard>,
    ) -> Self {
        let messages = message_rx.map(|t| t.map(|m| Ok::<Decoded, Error>(Decoded::Message(m))));
        let sysex = sysex_rx.map(|t| t.map(|s| Ok::<Decoded, Error>(Decoded::SysEx(s))));
        let errors = error_rx.map(|t| t.map(Err::<Decoded, Error>));
        Self {
            inner: Box::pin(select(select(messages, sysex), errors)),
            _guard: guard,
        }
    }

    pub async fn recv(&mut self) -> Option<Timed<Result<Decoded, Error>>> {
        self.next().await
    }
}

impl Stream for EventStream {
    type Item = Timed<Result<Decoded, Error>>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

impl std::fmt::Debug for EventStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventStream").finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct DestinationConnection {
    pub(crate) port: PortId,
    pub(crate) guard: Arc<ConnectionGuard>,
}

impl DestinationConnection {
    pub(crate) fn new(port: PortId, platform: Arc<crate::platform::PlatformClient>) -> Self {
        Self {
            port,
            guard: Arc::new(ConnectionGuard {
                target: ConnectionId::Destination(port),
                platform,
            }),
        }
    }

    pub async fn send(&self, msg: &MidiMessage) -> Result<(), Error> {
        let raw = RawMidiMessage::from(msg);
        self.guard.platform.send_midi(self.port, raw).await
    }

    pub async fn send_sysex(&self, sysex: &SysEx) -> Result<(), Error> {
        self.guard
            .platform
            .send_sysex(self.port, sysex.to_wire_bytes())
            .await
    }
}

#[derive(Debug)]
pub struct VirtualDestination {
    streams: ReceiveStreams,
    port: PortId,
    name: String,
}

impl VirtualDestination {
    pub(crate) fn new(
        message_rx: mpsc::Receiver<Timed<MidiMessage>>,
        sysex_rx: mpsc::Receiver<Timed<SysEx>>,
        error_rx: mpsc::Receiver<Timed<Error>>,
        id: VirtualPortId,
        port: PortId,
        name: String,
        platform: Arc<crate::platform::PlatformClient>,
    ) -> Self {
        Self {
            streams: ReceiveStreams {
                message_rx,
                sysex_rx,
                error_rx,
                guard: Arc::new(ConnectionGuard {
                    target: ConnectionId::VirtualDestination(id),
                    platform,
                }),
            },
            port,
            name,
        }
    }

    pub fn as_destination(&self) -> Destination {
        Destination {
            id: self.port,
            name: self.name.clone(),
            is_virtual: true,
        }
    }

    pub fn into_streams(self) -> Streams {
        self.streams.into_streams()
    }

    pub fn into_messages(self) -> MessageStream {
        self.streams.into_messages()
    }

    pub fn into_sysex(self) -> SysexStream {
        self.streams.into_sysex()
    }

    pub fn into_errors(self) -> ErrorStream {
        self.streams.into_errors()
    }

    pub fn into_events(self) -> EventStream {
        self.streams.into_events()
    }
}

#[derive(Debug, Clone)]
pub struct VirtualSource {
    pub(crate) id: VirtualPortId,
    pub(crate) port: PortId,
    pub(crate) name: String,
    pub(crate) guard: Arc<ConnectionGuard>,
}

impl VirtualSource {
    pub(crate) fn new(
        id: VirtualPortId,
        port: PortId,
        name: String,
        platform: Arc<crate::platform::PlatformClient>,
    ) -> Self {
        Self {
            id,
            port,
            name,
            guard: Arc::new(ConnectionGuard {
                target: ConnectionId::VirtualSource(id),
                platform,
            }),
        }
    }

    pub fn as_source(&self) -> Source {
        Source {
            id: self.port,
            name: self.name.clone(),
            is_virtual: true,
        }
    }

    pub async fn send(&self, msg: &MidiMessage) -> Result<(), Error> {
        let raw = RawMidiMessage::from(msg);
        self.guard.platform.send_virtual_midi(self.id, raw).await
    }

    pub async fn send_sysex(&self, sysex: &SysEx) -> Result<(), Error> {
        self.guard
            .platform
            .send_virtual_sysex(self.id, sysex.to_wire_bytes())
            .await
    }
}

#[derive(Debug)]
pub struct SourceChanges(pub(crate) mpsc::UnboundedReceiver<SourceChange>);

impl SourceChanges {
    pub async fn recv(&mut self) -> Option<SourceChange> {
        self.next().await
    }
}

impl Stream for SourceChanges {
    type Item = SourceChange;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.0).poll_next(cx)
    }
}

#[derive(Debug)]
pub struct DestinationChanges(pub(crate) mpsc::UnboundedReceiver<DestinationChange>);

impl DestinationChanges {
    pub async fn recv(&mut self) -> Option<DestinationChange> {
        self.next().await
    }
}

impl Stream for DestinationChanges {
    type Item = DestinationChange;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.0).poll_next(cx)
    }
}
