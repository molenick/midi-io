use crate::CodecError;
use crate::SysExError;
use crate::ValueError;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    Codec(#[from] CodecError),

    #[error(transparent)]
    Value(#[from] ValueError),

    #[error(transparent)]
    SysEx(#[from] SysExError),

    #[cfg(feature = "io")]
    #[error(transparent)]
    Io(#[from] IoError),
}

#[cfg(feature = "io")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum IoError {
    #[error("port not found")]
    PortNotFound,
    #[error("port disconnected")]
    PortDisconnected,
    #[error("port already connected")]
    AlreadyConnected,

    #[error("invalid name: {0}")]
    InvalidName(#[from] NameError),

    #[error("platform backend thread terminated unexpectedly")]
    BackendThreadDied,
    #[error("command channel full - backend thread is not processing commands")]
    BackendCommandChannelFull,

    #[error(transparent)]
    Platform(#[from] PlatformError),

    #[error("inbound stream overflow - {dropped} message(s) dropped")]
    InboundOverflow { dropped: usize },
}

#[cfg(feature = "io")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PlatformError {
    #[error("client initialization failed: backend error code {0}")]
    ClientInit(i32),
    #[error("client initialization failed: IO thread initialization failed")]
    ThreadInit,
    #[error("connect failed: backend error code {0}")]
    Connect(i32),
    #[error("send failed: backend error code {0}")]
    Send(i32),
    #[error("send failed: MIDI encoder produced no event for valid input")]
    Encode,
    #[error("virtual port creation failed: backend error code {0}")]
    VirtualPortCreate(i32),
}

#[cfg(feature = "io")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum NameError {
    #[error("name contains NUL byte")]
    ContainsNul(#[from] std::ffi::NulError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ParseError;

    #[cfg(feature = "io")]
    fn nul_error() -> std::ffi::NulError {
        std::ffi::CString::new("a\0b").unwrap_err()
    }

    #[test]
    fn error_is_clone_and_partial_eq() {
        let s1: Error = CodecError::SysexTooLong { len: 100, max: 50 }.into();
        let s2: Error = CodecError::SysexTooLong { len: 100, max: 50 }.into();
        assert_eq!(s1.clone(), s2);

        let p1: Error = CodecError::Parse {
            reason: ParseError::Empty,
            bytes: vec![],
        }
        .into();
        let p2: Error = CodecError::Parse {
            reason: ParseError::Empty,
            bytes: vec![],
        }
        .into();
        assert_eq!(p1.clone(), p2);

        let u1: Error = CodecError::Unparseable(crate::RawMidiMessage::from_slice(&[0xF4])).into();
        let u2: Error = CodecError::Unparseable(crate::RawMidiMessage::from_slice(&[0xF4])).into();
        assert_eq!(u1.clone(), u2);

        #[cfg(all(feature = "io", any(target_os = "macos", target_os = "ios")))]
        {
            let e1: Error = IoError::InvalidName(NameError::ContainsNul(nul_error())).into();
            let e2: Error = IoError::InvalidName(NameError::ContainsNul(nul_error())).into();
            assert_eq!(e1.clone(), e2);

            let c1: Error = IoError::Platform(PlatformError::Send(-1)).into();
            let c2: Error = IoError::Platform(PlatformError::Send(-1)).into();
            assert_eq!(c1.clone(), c2);
        }
    }

    #[cfg(feature = "io")]
    #[test]
    fn name_error_display() {
        assert_eq!(
            format!("{}", NameError::ContainsNul(nul_error())),
            "name contains NUL byte"
        );
    }

    #[test]
    fn error_display() {
        let sysex: Error = CodecError::SysexTooLong { len: 100, max: 50 }.into();
        assert_eq!(format!("{sysex}"), "sysex too long: 100 bytes (max 50)");

        let parse: Error = CodecError::Parse {
            reason: ParseError::Empty,
            bytes: vec![],
        }
        .into();
        assert_eq!(
            format!("{parse}"),
            "failed to parse MIDI message: empty message (bytes: [])"
        );

        let unparseable: Error =
            CodecError::Unparseable(crate::RawMidiMessage::from_slice(&[0x90, 0x3c])).into();
        assert_eq!(
            format!("{unparseable}"),
            "unparseable MIDI message: [90, 3c]"
        );

        #[cfg(feature = "io")]
        {
            let invalid: Error = IoError::InvalidName(NameError::ContainsNul(nul_error())).into();
            assert_eq!(format!("{invalid}"), "invalid name: name contains NUL byte");

            let overflow: Error = IoError::InboundOverflow { dropped: 42 }.into();
            assert_eq!(
                format!("{overflow}"),
                "inbound stream overflow - 42 message(s) dropped"
            );
        }
    }

    #[test]
    fn unparseable_returns_offending_bytes() {
        let raw = crate::RawMidiMessage::from_slice(&[0xF4, 0x05]);
        let err = crate::MidiMessage::try_from(raw).unwrap_err();
        let CodecError::Unparseable(returned) = err else {
            panic!("expected Unparseable, got {err:?}");
        };
        assert_eq!(&*returned, &[0xF4, 0x05]);
    }
}
