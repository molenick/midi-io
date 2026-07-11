use crate::midi::channel::Channel;
use crate::midi::data_byte::DataByte;
use crate::midi::pitch_bend::PitchBend;
use crate::midi::song_position::SongPosition;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MidiMessage {
    NoteOn {
        channel: Channel,
        key: DataByte,
        velocity: DataByte,
    },
    NoteOff {
        channel: Channel,
        key: DataByte,
        velocity: DataByte,
    },
    ControlChange {
        channel: Channel,
        controller: DataByte,
        value: DataByte,
    },
    ProgramChange {
        channel: Channel,
        program: DataByte,
    },
    PitchBend {
        channel: Channel,
        value: PitchBend,
    },
    PolyKeyPressure {
        channel: Channel,
        key: DataByte,
        pressure: DataByte,
    },
    ChannelPressure {
        channel: Channel,
        pressure: DataByte,
    },
    MtcQuarterFrame(DataByte),
    SongPositionPointer(SongPosition),
    SongSelect(DataByte),
    TuneRequest,
    TimingClock,
    Start,
    Continue,
    Stop,
    ActiveSensing,
    Reset,
}

impl std::fmt::Display for MidiMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MidiMessage::NoteOn {
                channel,
                key,
                velocity,
            } => write!(f, "NoteOn ch={} key={} vel={}", channel, key, velocity),
            MidiMessage::NoteOff {
                channel,
                key,
                velocity,
            } => write!(f, "NoteOff ch={} key={} vel={}", channel, key, velocity),
            MidiMessage::ControlChange {
                channel,
                controller,
                value,
            } => write!(f, "CC ch={} ctrl={} val={}", channel, controller, value),
            MidiMessage::ProgramChange { channel, program } => {
                write!(f, "PC ch={} prog={}", channel, program)
            }
            MidiMessage::PitchBend { channel, value } => {
                write!(f, "PitchBend ch={} val={}", channel, value)
            }
            MidiMessage::PolyKeyPressure {
                channel,
                key,
                pressure,
            } => write!(
                f,
                "PolyKeyPressure ch={} key={} pres={}",
                channel, key, pressure
            ),
            MidiMessage::ChannelPressure { channel, pressure } => {
                write!(f, "ChannelPressure ch={} pres={}", channel, pressure)
            }
            MidiMessage::MtcQuarterFrame(data) => {
                write!(f, "MtcQuarterFrame {:#04x}", data.get())
            }
            MidiMessage::SongPositionPointer(pos) => write!(f, "SongPosition {}", pos),
            MidiMessage::SongSelect(song) => write!(f, "SongSelect {}", song),
            MidiMessage::TuneRequest => write!(f, "TuneRequest"),
            MidiMessage::TimingClock => write!(f, "TimingClock"),
            MidiMessage::Start => write!(f, "Start"),
            MidiMessage::Continue => write!(f, "Continue"),
            MidiMessage::Stop => write!(f, "Stop"),
            MidiMessage::ActiveSensing => write!(f, "ActiveSensing"),
            MidiMessage::Reset => write!(f, "Reset"),
        }
    }
}
