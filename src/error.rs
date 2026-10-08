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
    #[error("IO thread initialization failed")]
    ThreadInit,
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
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PlatformError {
    #[cfg(target_os = "linux")]
    #[error(transparent)]
    Alsa(#[from] AlsaError),

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    #[error(transparent)]
    CoreMidi(#[from] CoreMidiError),

    #[cfg(target_arch = "wasm32")]
    #[error(transparent)]
    Web(#[from] WebError),
}

#[cfg(all(feature = "io", target_os = "linux"))]
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error(transparent)]
pub struct AlsaError(#[from] pub alsa::Error);

#[cfg(all(feature = "io", target_os = "linux"))]
impl Eq for AlsaError {}

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
#[error("{name}: {message}")]
pub struct WebError {
    pub name: String,
    pub message: String,
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

            let c1: Error = IoError::Platform(CoreMidiError::from(-1).into()).into();
            let c2: Error = IoError::Platform(CoreMidiError::from(-1).into()).into();
            assert_eq!(c1.clone(), c2);
        }
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
    }

    #[cfg(all(feature = "io", target_os = "linux"))]
    #[test]
    fn alsa_error_display_and_eq() {
        let inner = alsa::Error::new("snd_seq_open", libc::ENOENT);
        let e1: Error = IoError::Platform(AlsaError(inner).into()).into();
        let e2: Error = IoError::Platform(AlsaError(inner).into()).into();
        assert_eq!(e1.clone(), e2);
        assert_eq!(format!("{e1}"), format!("{inner}"));
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
