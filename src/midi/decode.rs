use crate::midi::channel::channel_from_nibble;
use crate::midi::data_byte::DataByte;
use crate::midi::message::MidiMessage;
use crate::midi::pitch_bend::PitchBend;
use crate::midi::song_position::SongPosition;
use crate::midi::sys_ex::SysEx;
use crate::CodecError;
use crate::ParseError;
use crate::RawMidiMessage;
use crate::SysExError;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Decoded {
    Message(MidiMessage),
    SysEx(SysEx),
}

impl Decoded {
    pub fn to_wire_bytes(&self) -> Vec<u8> {
        match self {
            Decoded::Message(msg) => RawMidiMessage::from(msg).to_vec(),
            Decoded::SysEx(sysex) => sysex.to_wire_bytes(),
        }
    }
}

pub(crate) fn expected_len(status: u8) -> Option<usize> {
    match status {
        0xF6 | 0xF8 | 0xFA | 0xFB | 0xFC | 0xFE | 0xFF => Some(1),
        0xF1 | 0xF3 => Some(2),
        0xF2 => Some(3),
        0xF0 | 0xF4 | 0xF5 | 0xF7 | 0xF9 | 0xFD => None,
        _ => match status & 0xF0 {
            0x80 | 0x90 | 0xA0 | 0xB0 | 0xE0 => Some(3),
            0xC0 | 0xD0 => Some(2),
            _ => None,
        },
    }
}

pub(crate) fn sysex_decode_error(err: SysExError, bytes: Vec<u8>) -> CodecError {
    let reason = match err {
        SysExError::TooLong { len, max } => return CodecError::SysexTooLong { len, max },
        SysExError::MissingStart => ParseError::UnrecognizedStatus,
        SysExError::Unterminated => ParseError::UnterminatedSysex,
        SysExError::EmptyBody => ParseError::EmptySysex,
        SysExError::HighBit { .. } => ParseError::SysexDataOutOfRange,
    };
    CodecError::Parse { reason, bytes }
}

pub fn decode(bytes: &[u8]) -> Result<Decoded, CodecError> {
    let Some(&status) = bytes.first() else {
        return Err(CodecError::Parse {
            reason: ParseError::Empty,
            bytes: bytes.to_vec(),
        });
    };

    if status == 0xF0 {
        return SysEx::try_from(bytes)
            .map(Decoded::SysEx)
            .map_err(|e| sysex_decode_error(e, bytes.to_vec()));
    }
    if status == 0xF7 {
        return Err(CodecError::Parse {
            reason: ParseError::StandaloneSysexEnd,
            bytes: bytes.to_vec(),
        });
    }

    let Some(len) = expected_len(status) else {
        return Err(CodecError::Parse {
            reason: ParseError::UnrecognizedStatus,
            bytes: bytes.to_vec(),
        });
    };
    if bytes.len() < len {
        return Err(CodecError::Parse {
            reason: ParseError::Truncated,
            bytes: bytes.to_vec(),
        });
    }
    if bytes.len() > len {
        return Err(CodecError::Parse {
            reason: ParseError::TrailingData,
            bytes: bytes.to_vec(),
        });
    }
    let data = |i: usize| {
        DataByte::try_from(bytes[i]).map_err(|_| CodecError::Parse {
            reason: ParseError::DataByteOutOfRange,
            bytes: bytes.to_vec(),
        })
    };

    let message = match status {
        0xF1 => MidiMessage::MtcQuarterFrame(data(1)?),
        0xF2 => MidiMessage::SongPositionPointer(SongPosition::from_msb_lsb(data(2)?, data(1)?)),
        0xF3 => MidiMessage::SongSelect(data(1)?),
        0xF6 => MidiMessage::TuneRequest,
        0xF8 => MidiMessage::TimingClock,
        0xFA => MidiMessage::Start,
        0xFB => MidiMessage::Continue,
        0xFC => MidiMessage::Stop,
        0xFE => MidiMessage::ActiveSensing,
        0xFF => MidiMessage::Reset,
        _ => {
            let channel = channel_from_nibble(status);
            match status & 0xF0 {
                0x80 => MidiMessage::NoteOff {
                    channel,
                    key: data(1)?,
                    velocity: data(2)?,
                },
                0x90 => {
                    let key = data(1)?;
                    let velocity = data(2)?;
                    if velocity.get() == 0 {
                        MidiMessage::NoteOff {
                            channel,
                            key,
                            velocity,
                        }
                    } else {
                        MidiMessage::NoteOn {
                            channel,
                            key,
                            velocity,
                        }
                    }
                }
                0xA0 => MidiMessage::PolyKeyPressure {
                    channel,
                    key: data(1)?,
                    pressure: data(2)?,
                },
                0xB0 => MidiMessage::ControlChange {
                    channel,
                    controller: data(1)?,
                    value: data(2)?,
                },
                0xC0 => MidiMessage::ProgramChange {
                    channel,
                    program: data(1)?,
                },
                0xD0 => MidiMessage::ChannelPressure {
                    channel,
                    pressure: data(1)?,
                },
                0xE0 => MidiMessage::PitchBend {
                    channel,
                    value: PitchBend::from_msb_lsb(data(2)?, data(1)?),
                },
                _ => unreachable!("expected_len returns Some only for known statuses"),
            }
        }
    };
    Ok(Decoded::Message(message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::midi::channel::Channel;
    use crate::midi::conformance;
    use crate::midi::pitch_bend::PITCH_BEND_CENTER;
    use crate::midi::sys_ex::MAX_SYSEX_BYTES;

    fn sp(n: u16) -> SongPosition {
        SongPosition::try_from(n).unwrap()
    }

    fn msg(bytes: &[u8]) -> MidiMessage {
        match decode(bytes).unwrap() {
            Decoded::Message(m) => m,
            other => panic!("expected message, got {other:?}"),
        }
    }

    #[test]
    fn conformance_decode_matches_corpus() {
        for (bytes, expected) in conformance::all() {
            match decode(&bytes) {
                Ok(Decoded::Message(m)) => assert_eq!(m, expected, "decode {bytes:02X?}"),
                other => panic!("expected message for {bytes:02X?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn parse_note_on() {
        assert_eq!(
            msg(&[0x90, 60, 100]),
            MidiMessage::NoteOn {
                channel: Channel::Ch1,
                key: DataByte::try_from(60).unwrap(),
                velocity: DataByte::try_from(100).unwrap()
            }
        );
    }

    #[test]
    fn parse_note_off() {
        assert_eq!(
            msg(&[0x80, 60, 0]),
            MidiMessage::NoteOff {
                channel: Channel::Ch1,
                key: DataByte::try_from(60).unwrap(),
                velocity: DataByte::try_from(0).unwrap()
            }
        );
    }

    #[test]
    fn parse_note_on_channel() {
        assert_eq!(
            msg(&[0x93, 48, 80]),
            MidiMessage::NoteOn {
                channel: Channel::Ch4,
                key: DataByte::try_from(48).unwrap(),
                velocity: DataByte::try_from(80).unwrap()
            }
        );
    }

    #[test]
    fn parse_control_change() {
        assert_eq!(
            msg(&[0xB0, 7, 127]),
            MidiMessage::ControlChange {
                channel: Channel::Ch1,
                controller: DataByte::try_from(7).unwrap(),
                value: DataByte::try_from(127).unwrap()
            }
        );
    }

    #[test]
    fn parse_program_change() {
        assert_eq!(
            msg(&[0xC0, 42]),
            MidiMessage::ProgramChange {
                channel: Channel::Ch1,
                program: DataByte::try_from(42).unwrap()
            }
        );
    }

    #[test]
    fn parse_pitch_bend_roundtrip() {
        use crate::RawMidiMessage;
        for value in [-PITCH_BEND_CENTER, -1000, 0, 1000, PITCH_BEND_CENTER - 1] {
            let original = MidiMessage::PitchBend {
                channel: Channel::Ch1,
                value: PitchBend::from_signed(value).unwrap(),
            };
            let raw = RawMidiMessage::from(&original);
            assert_eq!(msg(&raw), original);
        }
    }

    #[test]
    fn parse_channel_pressure() {
        assert_eq!(
            msg(&[0xD1, 55]),
            MidiMessage::ChannelPressure {
                channel: Channel::Ch2,
                pressure: DataByte::try_from(55).unwrap()
            }
        );
    }

    #[test]
    fn parse_poly_key_pressure() {
        assert_eq!(
            msg(&[0xA2, 36, 90]),
            MidiMessage::PolyKeyPressure {
                channel: Channel::Ch3,
                key: DataByte::try_from(36).unwrap(),
                pressure: DataByte::try_from(90).unwrap()
            }
        );
    }

    #[test]
    fn parse_sysex() {
        assert_eq!(
            decode(&[0xF0, 0x41, 0x10, 0xF7]).unwrap(),
            Decoded::SysEx(SysEx::new(&[0x41, 0x10]).unwrap())
        );
    }

    #[test]
    fn parse_empty_sysex_is_err() {
        let err = decode(&[0xF0, 0xF7]).unwrap_err();
        let CodecError::Parse { reason, bytes } = err else {
            panic!("expected Parse, got {err:?}");
        };
        assert_eq!(reason, ParseError::EmptySysex);
        assert_eq!(bytes, vec![0xF0, 0xF7]);
    }

    #[test]
    fn parse_sysex_high_bit_body_is_err() {
        let bytes = [0xF0, 0x41, 0x80, 0x42, 0xF7];
        let err = decode(&bytes).unwrap_err();
        let CodecError::Parse {
            reason,
            bytes: returned,
        } = err
        else {
            panic!("expected Parse, got {err:?}");
        };
        assert_eq!(reason, ParseError::SysexDataOutOfRange);
        assert_eq!(returned, bytes.to_vec());
    }

    #[test]
    fn decoded_to_wire_bytes_roundtrips() {
        let message = Decoded::Message(MidiMessage::NoteOn {
            channel: Channel::Ch4,
            key: DataByte::try_from(60).unwrap(),
            velocity: DataByte::try_from(100).unwrap(),
        });
        assert_eq!(decode(&message.to_wire_bytes()), Ok(message));

        let sysex =
            Decoded::SysEx(SysEx::try_from([0xF0u8, 0x41, 0x10, 0x20, 0xF7].as_slice()).unwrap());
        assert_eq!(sysex.to_wire_bytes(), vec![0xF0, 0x41, 0x10, 0x20, 0xF7]);
        assert_eq!(decode(&sysex.to_wire_bytes()), Ok(sysex));
    }

    #[test]
    fn parse_timing_clock() {
        assert_eq!(msg(&[0xF8]), MidiMessage::TimingClock);
    }

    #[test]
    fn parse_undefined_realtime_f9_is_err() {
        assert!(decode(&[0xF9]).is_err());
    }

    #[test]
    fn parse_start() {
        assert_eq!(msg(&[0xFA]), MidiMessage::Start);
    }

    #[test]
    fn parse_truncated_note_on_error() {
        assert!(decode(&[0x90, 60]).is_err());
    }

    #[test]
    fn parse_empty_error() {
        assert!(decode(&[]).is_err());
    }

    #[test]
    fn parse_reset() {
        assert_eq!(msg(&[0xFF]), MidiMessage::Reset);
    }

    #[test]
    fn parse_sysex_unterminated_error() {
        assert!(decode(&[0xF0, 0x41]).is_err());
    }

    #[test]
    fn parse_sysex_multi_byte() {
        assert_eq!(
            decode(&[0xF0, 0x41, 0x10, 0x20, 0xF7]).unwrap(),
            Decoded::SysEx(SysEx::new(&[0x41, 0x10, 0x20]).unwrap())
        );
    }

    #[test]
    fn parse_poly_key_pressure_truncated_error() {
        assert!(decode(&[0xA3, 60]).is_err());
    }

    #[test]
    fn parse_unrecognized_status_is_err() {
        let err = decode(&[0xF4, 0x05]).unwrap_err();
        assert!(matches!(
            err,
            CodecError::Parse {
                reason: ParseError::UnrecognizedStatus,
                ..
            }
        ));
    }

    #[test]
    fn parse_data_byte_high_bit_is_err() {
        let err = decode(&[0x90, 0x80, 0x40]).unwrap_err();
        assert!(matches!(
            err,
            CodecError::Parse {
                reason: ParseError::DataByteOutOfRange,
                ..
            }
        ));
        let CodecError::Parse { bytes, .. } = err else {
            unreachable!()
        };
        assert_eq!(bytes, vec![0x90, 0x80, 0x40]);
    }

    #[test]
    fn parse_rejects_trailing_bytes() {
        let err = decode(&[0x90, 60, 100, 0x90]).unwrap_err();
        let CodecError::Parse { reason, bytes } = err else {
            panic!("expected Parse, got {err:?}");
        };
        assert_eq!(reason, ParseError::TrailingData);
        assert_eq!(bytes, vec![0x90, 60, 100, 0x90]);
    }

    #[test]
    fn parse_rejects_trailing_realtime_byte() {
        let err = decode(&[0xF8, 0xF8]).unwrap_err();
        assert!(matches!(
            err,
            CodecError::Parse {
                reason: ParseError::TrailingData,
                ..
            }
        ));
    }

    #[test]
    fn parse_rejects_oversized_sysex() {
        let mut bytes = vec![0xF0u8];
        bytes.extend(std::iter::repeat(0x01).take(MAX_SYSEX_BYTES + 1));
        bytes.push(0xF7);
        assert!(matches!(
            decode(&bytes),
            Err(CodecError::SysexTooLong { .. })
        ));
    }

    #[test]
    fn parse_accepts_max_sized_sysex() {
        let mut bytes = vec![0xF0u8];
        bytes.extend(std::iter::repeat(0x01).take(MAX_SYSEX_BYTES));
        bytes.push(0xF7);
        assert!(matches!(decode(&bytes), Ok(Decoded::SysEx(_))));
    }

    #[test]
    fn parse_mtc_quarter_frame() {
        assert_eq!(
            msg(&[0xF1, 0x35]),
            MidiMessage::MtcQuarterFrame(DataByte::try_from(0x35).unwrap())
        );
    }

    #[test]
    fn parse_song_position_pointer() {
        assert_eq!(
            msg(&[0xF2, 0x00, 0x02]),
            MidiMessage::SongPositionPointer(sp(256))
        );
    }

    #[test]
    fn parse_song_select() {
        assert_eq!(
            msg(&[0xF3, 42]),
            MidiMessage::SongSelect(DataByte::try_from(42).unwrap())
        );
    }

    #[test]
    fn parse_tune_request() {
        assert_eq!(msg(&[0xF6]), MidiMessage::TuneRequest);
    }

    #[test]
    fn parse_song_position_pointer_roundtrip() {
        use crate::RawMidiMessage;
        for pos in [0u16, 1000, 16383] {
            let original = MidiMessage::SongPositionPointer(sp(pos));
            let raw = RawMidiMessage::from(&original);
            assert_eq!(msg(&raw), original);
        }
    }
}
