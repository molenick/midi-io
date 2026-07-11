#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ParseError {
    #[error("empty message")]
    Empty,
    #[error("truncated message")]
    Truncated,
    #[error("unexpected trailing bytes after message")]
    TrailingData,
    #[error("standalone SysexEnd marker")]
    StandaloneSysexEnd,
    #[error("data byte with high bit set")]
    DataByteOutOfRange,
    #[error("channel out of range")]
    ChannelOutOfRange,
    #[error("sysex body byte with high bit set")]
    SysexDataOutOfRange,
    #[error("empty sysex body")]
    EmptySysex,
    #[error("unterminated sysex")]
    UnterminatedSysex,
    #[error("unrecognized status byte")]
    UnrecognizedStatus,
    #[error("orphaned data bytes ({len} total)")]
    OrphanedData { len: usize },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_error_display() {
        assert_eq!(format!("{}", ParseError::Empty), "empty message");
        assert_eq!(format!("{}", ParseError::Truncated), "truncated message");
        assert_eq!(
            format!("{}", ParseError::TrailingData),
            "unexpected trailing bytes after message"
        );
        assert_eq!(
            format!("{}", ParseError::StandaloneSysexEnd),
            "standalone SysexEnd marker"
        );
        assert_eq!(
            format!("{}", ParseError::DataByteOutOfRange),
            "data byte with high bit set"
        );
        assert_eq!(
            format!("{}", ParseError::ChannelOutOfRange),
            "channel out of range"
        );
        assert_eq!(
            format!("{}", ParseError::SysexDataOutOfRange),
            "sysex body byte with high bit set"
        );
        assert_eq!(format!("{}", ParseError::EmptySysex), "empty sysex body");
        assert_eq!(
            format!("{}", ParseError::UnterminatedSysex),
            "unterminated sysex"
        );
        assert_eq!(
            format!("{}", ParseError::UnrecognizedStatus),
            "unrecognized status byte"
        );
        assert_eq!(
            format!("{}", ParseError::OrphanedData { len: 3 }),
            "orphaned data bytes (3 total)"
        );
    }
}
