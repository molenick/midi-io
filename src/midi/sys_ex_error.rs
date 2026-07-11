#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SysExError {
    #[error("sysex must begin with a 0xF0 start byte")]
    MissingStart,
    #[error("sysex must end with a 0xF7 end byte")]
    Unterminated,
    #[error("sysex has no body byte between 0xF0 and 0xF7")]
    EmptyBody,
    #[error("sysex body is {len} bytes, larger than the maximum of {max}")]
    TooLong { len: usize, max: usize },
    #[error("sysex body byte 0x{byte:02x} at index {index} is not a 7-bit value")]
    HighBit { index: usize, byte: u8 },
}
