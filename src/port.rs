#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VirtualPortId(pub(crate) u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PortId(pub(crate) u64);

/// A source is a sender of MIDI messages.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Source {
    pub(crate) id: PortId,
    pub(crate) name: String,
    pub(crate) is_virtual: bool,
}

impl Source {
    pub fn id(&self) -> PortId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn is_virtual(&self) -> bool {
        self.is_virtual
    }
}

/// Indicates when available sources have changed
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SourceChange {
    Added(Source),
    Removed(Source),
}

/// A destination is a receiver of MIDI messages.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Destination {
    pub(crate) id: PortId,
    pub(crate) name: String,
    pub(crate) is_virtual: bool,
}

impl Destination {
    pub fn id(&self) -> PortId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn is_virtual(&self) -> bool {
        self.is_virtual
    }
}

/// Indicates when available destinations have changed
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DestinationChange {
    Added(Destination),
    Removed(Destination),
}
