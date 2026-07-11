/// A value outside the range representable in a MIDI message field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ValueError {
    #[error("{0} is not a 7-bit MIDI data value (0..=127)")]
    DataByte(u8),
    #[error("{0} is not a MIDI channel index (0..=15)")]
    ChannelIndex(u8),
    #[error("{0} is not a MIDI channel number (1..=16)")]
    ChannelNumber(u8),
    #[error("{0} is out of MIDI pitch-bend range")]
    PitchBend(i32),
    #[error("{0} is not a 14-bit MIDI song-position value (0..=16383)")]
    SongPosition(u16),
}
