use std::sync::Arc;

use crate::name::Name;
use crate::platform::PlatformClient;
use crate::Destination;
use crate::DestinationChanges;
use crate::DestinationConnection;
use crate::Error;
use crate::IoError;
use crate::Source;
use crate::SourceChanges;
use crate::SourceConnection;
use crate::VirtualDestination;
use crate::VirtualSource;

/// MIDI client for sending and receiving MIDI messages
#[derive(Debug, Clone)]
pub struct Client {
    pub(crate) inner: Arc<PlatformClient>,
}

impl Client {
    /// Create a new midi client.
    ///
    /// On macOS and iOS, CoreMIDI permanently routes hotplug notifications to
    /// whichever thread created the first MIDI client in the process. Create this
    /// client before anything else in the process uses CoreMIDI, or
    /// [`source_changes`](Client::source_changes) and
    /// [`destination_changes`](Client::destination_changes) may never receive events.
    pub async fn new(name: &str) -> Result<Self, Error> {
        let name = Name::try_from(name).map_err(IoError::from)?;
        let (platform, ready_rx) = PlatformClient::new(name)?;
        ready_rx.await.map_err(|_| IoError::BackendThreadDied)??;
        Ok(Self {
            inner: Arc::new(platform),
        })
    }

    /// Enumerate the available sources from the platform.
    pub async fn sources(&self) -> Result<Vec<Source>, Error> {
        self.inner.sources().await
    }

    /// Enumerate the available destinations from the platform.
    pub async fn destinations(&self) -> Result<Vec<Destination>, Error> {
        self.inner.destinations().await
    }

    /// Subscribe to a stream of source change events.
    #[must_use = "the source changes stream must be polled to receive source change events"]
    pub fn source_changes(&self) -> SourceChanges {
        SourceChanges(self.inner.source_changes_rx())
    }

    /// Subscribe to a stream of destination change events.
    #[must_use = "the destination changes stream must be polled to receive destination change events"]
    pub fn destination_changes(&self) -> DestinationChanges {
        DestinationChanges(self.inner.destination_changes_rx())
    }

    /// Connect to a known source. Use the `sources` method to query currently available sources.
    pub async fn connect_source(&self, port: &Source) -> Result<SourceConnection, Error> {
        let (msg_rx, sx_rx, err_rx) = self.inner.connect_source(port).await?;
        Ok(SourceConnection::new(
            msg_rx,
            sx_rx,
            err_rx,
            port.id,
            Arc::clone(&self.inner),
        ))
    }

    /// Connect to a known destination. Use the `destinations` method to query currently available destinations.
    pub async fn connect_destination(
        &self,
        port: &Destination,
    ) -> Result<DestinationConnection, Error> {
        self.inner.connect_destination(port).await?;
        Ok(DestinationConnection::new(port.id, Arc::clone(&self.inner)))
    }

    /// Creates a virtual source that can be used to send midi to destinations
    pub async fn create_virtual_source(&self, name: &str) -> Result<VirtualSource, Error> {
        let validated = Name::try_from(name).map_err(IoError::from)?;
        let id = self.inner.alloc_virtual_id();
        let port = self.inner.create_virtual_source(id, validated).await?;
        Ok(VirtualSource::new(
            id,
            port,
            name.to_string(),
            Arc::clone(&self.inner),
        ))
    }

    /// Creates a virtual destination that can be used to receive midi from sources
    pub async fn create_virtual_destination(
        &self,
        name: &str,
    ) -> Result<VirtualDestination, Error> {
        let validated = Name::try_from(name).map_err(IoError::from)?;
        let id = self.inner.alloc_virtual_id();
        let (port, (msg_rx, sx_rx, err_rx)) =
            self.inner.create_virtual_destination(id, validated).await?;
        Ok(VirtualDestination::new(
            msg_rx,
            sx_rx,
            err_rx,
            id,
            port,
            name.to_string(),
            Arc::clone(&self.inner),
        ))
    }
}
