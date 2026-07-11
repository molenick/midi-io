use crate::midi::parse_error::ParseError;
use crate::midi::raw_message::RawMidiMessage;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CodecError {
    #[error("failed to parse MIDI message: {reason} (bytes: {bytes:02x?})")]
    Parse { reason: ParseError, bytes: Vec<u8> },
    #[error("unparseable MIDI message: {0:?}")]
    Unparseable(RawMidiMessage),
    #[error("sysex too long: {len} bytes (max {max})")]
    SysexTooLong { len: usize, max: usize },
}
