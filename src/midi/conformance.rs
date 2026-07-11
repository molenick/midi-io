use crate::midi::channel::Channel;
use crate::midi::data_byte::DataByte;
use crate::midi::message::MidiMessage;
use crate::midi::pitch_bend::PitchBend;
use crate::midi::song_position::SongPosition;

pub(crate) fn channel_voice() -> Vec<(Vec<u8>, MidiMessage)> {
    vec![
        (
            vec![0x90, 60, 100],
            MidiMessage::NoteOn {
                channel: Channel::Ch1,
                key: DataByte::try_from(60).unwrap(),
                velocity: DataByte::try_from(100).unwrap(),
            },
        ),
        (
            vec![0x81, 62, 64],
            MidiMessage::NoteOff {
                channel: Channel::Ch2,
                key: DataByte::try_from(62).unwrap(),
                velocity: DataByte::try_from(64).unwrap(),
            },
        ),
        (
            vec![0xA3, 64, 30],
            MidiMessage::PolyKeyPressure {
                channel: Channel::Ch4,
                key: DataByte::try_from(64).unwrap(),
                pressure: DataByte::try_from(30).unwrap(),
            },
        ),
        (
            vec![0xB0, 7, 127],
            MidiMessage::ControlChange {
                channel: Channel::Ch1,
                controller: DataByte::try_from(7).unwrap(),
                value: DataByte::try_from(127).unwrap(),
            },
        ),
        (
            vec![0xC5, 42],
            MidiMessage::ProgramChange {
                channel: Channel::Ch6,
                program: DataByte::try_from(42).unwrap(),
            },
        ),
        (
            vec![0xD7, 55],
            MidiMessage::ChannelPressure {
                channel: Channel::Ch8,
                pressure: DataByte::try_from(55).unwrap(),
            },
        ),
        (
            vec![0xE0, 0x00, 0x40],
            MidiMessage::PitchBend {
                channel: Channel::Ch1,
                value: PitchBend::try_from(8192).unwrap(),
            },
        ),
        (
            vec![0xE2, 0x7F, 0x7F],
            MidiMessage::PitchBend {
                channel: Channel::Ch3,
                value: PitchBend::try_from(0x3FFF).unwrap(),
            },
        ),
    ]
}

pub(crate) fn system() -> Vec<(Vec<u8>, MidiMessage)> {
    vec![
        (
            vec![0xF1, 0x35],
            MidiMessage::MtcQuarterFrame(DataByte::try_from(0x35).unwrap()),
        ),
        (
            vec![0xF2, 0x68, 0x07],
            MidiMessage::SongPositionPointer(SongPosition::try_from(1000).unwrap()),
        ),
        (
            vec![0xF3, 0x12],
            MidiMessage::SongSelect(DataByte::try_from(0x12).unwrap()),
        ),
        (vec![0xF6], MidiMessage::TuneRequest),
        (vec![0xF8], MidiMessage::TimingClock),
        (vec![0xFA], MidiMessage::Start),
        (vec![0xFB], MidiMessage::Continue),
        (vec![0xFC], MidiMessage::Stop),
        (vec![0xFE], MidiMessage::ActiveSensing),
        (vec![0xFF], MidiMessage::Reset),
    ]
}

pub(crate) fn all() -> Vec<(Vec<u8>, MidiMessage)> {
    let mut v = channel_voice();
    v.extend(system());
    v
}

pub(crate) const SYSEX_FRAME: [u8; 5] = [0xF0, 0x01, 0x02, 0x03, 0xF7];
pub(crate) const SYSEX_BODY: [u8; 3] = [0x01, 0x02, 0x03];
