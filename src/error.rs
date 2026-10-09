use crate::CodecError;
use crate::SysExError;
use crate::ValueError;

#[derive(Debug, thiserror::Error)]
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
#[derive(Debug, thiserror::Error)]
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
    #[error("IO thread spawn failed: {0}")]
    ThreadSpawn(#[from] std::io::Error),
    #[error("backend not ready")]
    NotReady,
    #[error("command channel full - backend thread is not processing commands")]
    BackendCommandChannelFull,

    #[error(transparent)]
    Platform(#[from] PlatformError),

    #[error("MIDI encoder produced no event")]
    Encode,

    #[error("inbound stream overflow - {dropped} message(s) dropped")]
    InboundOverflow { dropped: usize },

    #[error("unsupported on this platform")]
    Unsupported,

    #[error("another endpoint already holds that unique ID")]
    UniqueIdTaken,

    #[error("MIDI access denied")]
    PermissionDenied,
}

#[cfg(feature = "io")]
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PlatformError {
    #[cfg(target_os = "linux")]
    #[error(transparent)]
    Alsa(#[from] alsa::Error),

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    #[error(transparent)]
    CoreMidi(#[from] CoreMidiError),

    #[cfg(target_arch = "wasm32")]
    #[error(transparent)]
    Web(#[from] WebError),
}

#[cfg(all(feature = "io", any(target_os = "macos", target_os = "ios")))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CoreMidiError {
    #[error("kMIDIInvalidClient")]
    InvalidClient,
    #[error("kMIDIInvalidPort")]
    InvalidPort,
    #[error("kMIDIWrongEndpointType")]
    WrongEndpointType,
    #[error("kMIDINoConnection")]
    NoConnection,
    #[error("kMIDIUnknownEndpoint")]
    UnknownEndpoint,
    #[error("kMIDIUnknownProperty")]
    UnknownProperty,
    #[error("kMIDIWrongPropertyType")]
    WrongPropertyType,
    #[error("kMIDINoCurrentSetup")]
    NoCurrentSetup,
    #[error("kMIDIMessageSendErr")]
    MessageSendErr,
    #[error("kMIDIServerStartErr")]
    ServerStartErr,
    #[error("kMIDISetupFormatErr")]
    SetupFormatErr,
    #[error("kMIDIWrongThread")]
    WrongThread,
    #[error("kMIDIObjectNotFound")]
    ObjectNotFound,
    #[error("kMIDIIDNotUnique")]
    IdNotUnique,
    #[error("kMIDINotPermitted")]
    NotPermitted,
    #[error("kMIDIUnknownError")]
    UnknownError,
    #[error("OSStatus {0}")]
    Other(coremidi_sys::OSStatus),
}

#[cfg(all(feature = "io", any(target_os = "macos", target_os = "ios")))]
impl From<coremidi_sys::OSStatus> for CoreMidiError {
    fn from(status: coremidi_sys::OSStatus) -> Self {
        match status {
            coremidi_sys::kMIDIInvalidClient => Self::InvalidClient,
            coremidi_sys::kMIDIInvalidPort => Self::InvalidPort,
            coremidi_sys::kMIDIWrongEndpointType => Self::WrongEndpointType,
            coremidi_sys::kMIDINoConnection => Self::NoConnection,
            coremidi_sys::kMIDIUnknownEndpoint => Self::UnknownEndpoint,
            coremidi_sys::kMIDIUnknownProperty => Self::UnknownProperty,
            coremidi_sys::kMIDIWrongPropertyType => Self::WrongPropertyType,
            coremidi_sys::kMIDINoCurrentSetup => Self::NoCurrentSetup,
            coremidi_sys::kMIDIMessageSendErr => Self::MessageSendErr,
            coremidi_sys::kMIDIServerStartErr => Self::ServerStartErr,
            coremidi_sys::kMIDISetupFormatErr => Self::SetupFormatErr,
            coremidi_sys::kMIDIWrongThread => Self::WrongThread,
            coremidi_sys::kMIDIObjectNotFound => Self::ObjectNotFound,
            coremidi_sys::kMIDIIDNotUnique => Self::IdNotUnique,
            coremidi_sys::kMIDINotPermitted => Self::NotPermitted,
            coremidi_sys::kMIDIUnknownError => Self::UnknownError,
            other => Self::Other(other),
        }
    }
}

#[cfg(all(feature = "io", target_arch = "wasm32"))]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum WebError {
    #[error("AbortError: {0}")]
    Abort(String),
    #[error("InvalidAccessError: {0}")]
    InvalidAccess(String),
    #[error("InvalidStateError: {0}")]
    InvalidState(String),
    #[error("NotAllowedError: {0}")]
    NotAllowed(String),
    #[error("NotSupportedError: {0}")]
    NotSupported(String),
    #[error("SecurityError: {0}")]
    Security(String),
    #[error("TypeError: {0}")]
    Type(String),
    #[error("{name}: {message}")]
    Other { name: String, message: String },
}

#[cfg(all(feature = "io", target_arch = "wasm32"))]
impl From<web_sys::DomException> for WebError {
    fn from(exception: web_sys::DomException) -> Self {
        let message = exception.message();
        match exception.name().as_str() {
            "AbortError" => Self::Abort(message),
            "InvalidAccessError" => Self::InvalidAccess(message),
            "InvalidStateError" => Self::InvalidState(message),
            "NotAllowedError" => Self::NotAllowed(message),
            "NotSupportedError" => Self::NotSupported(message),
            "SecurityError" => Self::Security(message),
            "TypeError" => Self::Type(message),
            _ => Self::Other {
                name: exception.name(),
                message,
            },
        }
    }
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

    #[cfg(all(feature = "io", any(target_os = "macos", target_os = "ios")))]
    #[test]
    fn coremidi_error_from_status() {
        let known = CoreMidiError::from(coremidi_sys::kMIDINoConnection);
        assert_eq!(known, CoreMidiError::NoConnection);
        assert_eq!(format!("{known}"), "kMIDINoConnection");
        let other = CoreMidiError::from(-50);
        assert_eq!(other, CoreMidiError::Other(-50));
        assert_eq!(format!("{other}"), "OSStatus -50");

        let err: Error = IoError::Platform(known.into()).into();
        assert!(matches!(
            err,
            Error::Io(IoError::Platform(PlatformError::CoreMidi(
                CoreMidiError::NoConnection
            )))
        ));
    }

    #[cfg(all(feature = "io", target_os = "linux"))]
    #[test]
    fn alsa_error_is_forwarded() {
        let inner = alsa::Error::new("snd_seq_open", libc::ENOENT);
        let err: Error = IoError::Platform(inner.into()).into();
        assert_eq!(format!("{err}"), format!("{inner}"));
        assert!(matches!(
            err,
            Error::Io(IoError::Platform(PlatformError::Alsa(e))) if e == inner
        ));
    }

    #[cfg(feature = "io")]
    #[test]
    fn thread_spawn_forwards_io_error() {
        let inner = std::io::Error::new(std::io::ErrorKind::WouldBlock, "no threads");
        let err: Error = IoError::ThreadSpawn(inner).into();
        assert_eq!(format!("{err}"), "IO thread spawn failed: no threads");
        assert!(matches!(
            err,
            Error::Io(IoError::ThreadSpawn(e)) if e.kind() == std::io::ErrorKind::WouldBlock
        ));
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
