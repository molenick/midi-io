use std::ops::Deref;

use crate::midi::decode::decode;
use crate::midi::decode::Decoded;
use crate::midi::message::MidiMessage;
use crate::CodecError;

/// An unparsed MIDI message.
#[derive(Copy, Clone)]
pub struct RawMidiMessage {
    bytes: [u8; 3],
    len: u8,
}

impl PartialEq for RawMidiMessage {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl Eq for RawMidiMessage {}

impl RawMidiMessage {
    fn new(bytes: [u8; 3], len: u8) -> Self {
        debug_assert!(len <= 3);
        Self { bytes, len }
    }

    #[cfg(test)]
    pub(crate) fn from_slice(bytes: &[u8]) -> Self {
        let len = bytes.len().min(3);
        let mut buf = [0u8; 3];
        buf[..len].copy_from_slice(&bytes[..len]);
        Self::new(buf, len as u8)
    }
}

impl Deref for RawMidiMessage {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}

impl std::fmt::Debug for RawMidiMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:02x?}", &**self)
    }
}

impl From<&MidiMessage> for RawMidiMessage {
    fn from(msg: &MidiMessage) -> Self {
        match msg {
            MidiMessage::NoteOff {
                channel,
                key,
                velocity,
            } => Self::new([0x80 | channel.index(), key.get(), velocity.get()], 3),
            MidiMessage::NoteOn {
                channel,
                key,
                velocity,
            } => Self::new([0x90 | channel.index(), key.get(), velocity.get()], 3),
            MidiMessage::PolyKeyPressure {
                channel,
                key,
                pressure,
            } => Self::new([0xA0 | channel.index(), key.get(), pressure.get()], 3),
            MidiMessage::ControlChange {
                channel,
                controller,
                value,
            } => Self::new([0xB0 | channel.index(), controller.get(), value.get()], 3),
            MidiMessage::ProgramChange { channel, program } => {
                Self::new([0xC0 | channel.index(), program.get(), 0], 2)
            }
            MidiMessage::ChannelPressure { channel, pressure } => {
                Self::new([0xD0 | channel.index(), pressure.get(), 0], 2)
            }
            MidiMessage::PitchBend { channel, value } => {
                Self::new([0xE0 | channel.index(), value.lsb(), value.msb()], 3)
            }
            MidiMessage::MtcQuarterFrame(data) => Self::new([0xF1, data.get(), 0], 2),
            MidiMessage::SongPositionPointer(pos) => Self::new([0xF2, pos.lsb(), pos.msb()], 3),
            MidiMessage::SongSelect(song) => Self::new([0xF3, song.get(), 0], 2),
            MidiMessage::TuneRequest => Self::new([0xF6, 0, 0], 1),
            MidiMessage::TimingClock => Self::new([0xF8, 0, 0], 1),
            MidiMessage::Start => Self::new([0xFA, 0, 0], 1),
            MidiMessage::Continue => Self::new([0xFB, 0, 0], 1),
            MidiMessage::Stop => Self::new([0xFC, 0, 0], 1),
            MidiMessage::ActiveSensing => Self::new([0xFE, 0, 0], 1),
            MidiMessage::Reset => Self::new([0xFF, 0, 0], 1),
        }
    }
}

impl From<MidiMessage> for RawMidiMessage {
    fn from(msg: MidiMessage) -> Self {
        Self::from(&msg)
    }
}

impl TryFrom<RawMidiMessage> for MidiMessage {
    type Error = CodecError;

    fn try_from(raw: RawMidiMessage) -> Result<Self, CodecError> {
        match decode(&raw) {
            Ok(Decoded::Message(msg)) => Ok(msg),
            _ => Err(CodecError::Unparseable(raw)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::midi::data_byte::DataByte;
    use crate::midi::pitch_bend::PitchBend;
    use crate::midi::pitch_bend::PITCH_BEND_CENTER;
    use crate::midi::song_position::SongPosition;
    use crate::Channel;

    fn roundtrip(msg: &MidiMessage) {
        let raw = RawMidiMessage::from(msg);
        let decoded = MidiMessage::try_from(raw).unwrap();
        assert_eq!(decoded, *msg, "roundtrip failed for {msg:?}");
    }

    #[test]
    fn channel_voice_roundtrip() {
        for ch in (0..16u8).map(|i| Channel::from_index(i).unwrap()) {
            for (key, vel) in [(0u8, 1u8), (63, 100), (127, 127)] {
                roundtrip(&MidiMessage::NoteOn {
                    channel: ch,
                    key: DataByte::try_from(key).unwrap(),
                    velocity: DataByte::try_from(vel).unwrap(),
                });
            }
            for (key, vel) in [(0u8, 0u8), (63, 100), (127, 127)] {
                roundtrip(&MidiMessage::NoteOff {
                    channel: ch,
                    key: DataByte::try_from(key).unwrap(),
                    velocity: DataByte::try_from(vel).unwrap(),
                });
            }

            for (ctl, val) in [(0u8, 0u8), (64, 64), (127, 127)] {
                roundtrip(&MidiMessage::ControlChange {
                    channel: ch,
                    controller: DataByte::try_from(ctl).unwrap(),
                    value: DataByte::try_from(val).unwrap(),
                });
            }

            for val in [0u8, 64, 127] {
                roundtrip(&MidiMessage::ProgramChange {
                    channel: ch,
                    program: DataByte::try_from(val).unwrap(),
                });
                roundtrip(&MidiMessage::ChannelPressure {
                    channel: ch,
                    pressure: DataByte::try_from(val).unwrap(),
                });
            }

            for (key, pres) in [(0u8, 0u8), (63, 64), (127, 127)] {
                roundtrip(&MidiMessage::PolyKeyPressure {
                    channel: ch,
                    key: DataByte::try_from(key).unwrap(),
                    pressure: DataByte::try_from(pres).unwrap(),
                });
            }
        }
    }

    #[test]
    fn note_on_velocity_zero_decodes_as_note_off() {
        let raw = RawMidiMessage::from(&MidiMessage::NoteOn {
            channel: Channel::Ch1,
            key: DataByte::try_from(60).unwrap(),
            velocity: DataByte::try_from(0).unwrap(),
        });
        assert_eq!(&*raw, &[0x90, 60, 0]);
        assert_eq!(
            MidiMessage::try_from(raw).unwrap(),
            MidiMessage::NoteOff {
                channel: Channel::Ch1,
                key: DataByte::try_from(60).unwrap(),
                velocity: DataByte::try_from(0).unwrap(),
            }
        );
    }

    #[test]
    fn pitch_bend_roundtrip_boundaries() {
        for value in [-PITCH_BEND_CENTER, -4096, -1, 0, 1, PITCH_BEND_CENTER - 1] {
            roundtrip(&MidiMessage::PitchBend {
                channel: Channel::Ch1,
                value: PitchBend::from_signed(value).unwrap(),
            });
        }
    }

    #[test]
    fn system_common_one_byte() {
        for msg in [
            MidiMessage::TimingClock,
            MidiMessage::Start,
            MidiMessage::Continue,
            MidiMessage::Stop,
            MidiMessage::ActiveSensing,
            MidiMessage::Reset,
            MidiMessage::TuneRequest,
        ] {
            roundtrip(&msg);
        }
    }

    #[test]
    fn system_common_two_byte() {
        for val in [0u8, 3, 7] {
            roundtrip(&MidiMessage::MtcQuarterFrame(
                DataByte::try_from(val).unwrap(),
            ));
        }

        for val in [0u8, 42, 127] {
            roundtrip(&MidiMessage::SongSelect(DataByte::try_from(val).unwrap()));
        }
    }

    #[test]
    fn system_common_three_byte() {
        for pos in [0u16, 8191, 16383] {
            roundtrip(&MidiMessage::SongPositionPointer(
                SongPosition::try_from(pos).unwrap(),
            ));
        }
    }

    #[test]
    fn encoding_produces_legal_data_bytes() {
        let msg = MidiMessage::NoteOn {
            channel: Channel::Ch1,
            key: DataByte::try_from(127).unwrap(),
            velocity: DataByte::try_from(127).unwrap(),
        };
        let raw = RawMidiMessage::from(&msg);
        assert_eq!(&*raw, &[0x90, 0x7F, 0x7F]);
        assert!(raw[1..].iter().all(|&b| b <= 0x7F));
    }

    #[test]
    fn try_from_raw_unparseable_returns_bytes() {
        let raw = RawMidiMessage::from_slice(&[0xF4, 0x05]);
        let err = MidiMessage::try_from(raw).unwrap_err();
        let CodecError::Unparseable(returned) = err else {
            panic!("expected Unparseable, got {err:?}");
        };
        assert_eq!(&*returned, &[0xF4, 0x05]);
    }

    #[test]
    fn from_slice_accepts_arbitrary_bytes() {
        let raw = RawMidiMessage::from_slice(&[0xF4, 0x90, 0xFF]);
        assert_eq!(&*raw, &[0xF4, 0x90, 0xFF]);
    }

    #[test]
    fn from_slice_truncates_to_three() {
        let raw = RawMidiMessage::from_slice(&[0x90, 60, 100, 200]);
        assert_eq!(&*raw, &[0x90, 60, 100]);
    }

    #[test]
    fn zero_value_deref_empty() {
        let raw = RawMidiMessage::from_slice(&[]);
        assert_eq!(raw.deref().len(), 0);
    }
}
