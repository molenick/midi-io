#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]

pub(crate) mod error;
pub(crate) mod midi;

#[cfg(feature = "io")]
pub(crate) mod client;
#[cfg(feature = "io")]
pub(crate) mod connection;
#[cfg(feature = "io")]
pub(crate) mod name;
#[cfg(feature = "io")]
pub(crate) mod platform;
#[cfg(feature = "io")]
pub(crate) mod port;

pub use error::Error;
#[cfg(feature = "io")]
pub use io::*;
pub use midi::channel::Channel;
pub use midi::codec_error::CodecError;
pub use midi::data_byte::DataByte;
pub use midi::decode::decode;
pub use midi::decode::Decoded;
pub use midi::message::MidiMessage;
pub use midi::parse_error::ParseError;
pub use midi::pitch_bend::PitchBend;
pub use midi::raw_message::RawMidiMessage;
pub use midi::song_position::SongPosition;
pub use midi::sys_ex::SysEx;
pub use midi::sys_ex_error::SysExError;
pub use midi::value_error::ValueError;

#[cfg(feature = "io")]
mod io {
    pub use crate::client::Client;
    pub use crate::connection::DestinationChanges;
    pub use crate::connection::DestinationConnection;
    pub use crate::connection::ErrorStream;
    pub use crate::connection::EventStream;
    pub use crate::connection::MessageStream;
    pub use crate::connection::SourceChanges;
    pub use crate::connection::SourceConnection;
    pub use crate::connection::Streams;
    pub use crate::connection::SysexStream;
    pub use crate::connection::Timed;
    pub use crate::connection::VirtualDestination;
    pub use crate::connection::VirtualSource;
    pub use crate::error::IoError;
    pub use crate::error::NameError;
    pub use crate::error::PlatformError;
    pub use crate::port::Destination;
    pub use crate::port::DestinationChange;
    pub use crate::port::PortId;
    pub use crate::port::Source;
    pub use crate::port::SourceChange;
}
