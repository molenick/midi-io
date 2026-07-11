use midi_io::decode;
use midi_io::Channel;
use midi_io::CodecError;
use midi_io::DataByte;
use midi_io::MidiMessage;
use midi_io::RawMidiMessage;

fn main() {
    let note = MidiMessage::NoteOn {
        channel: Channel::Ch1,
        key: DataByte::try_from(60).unwrap(),
        velocity: DataByte::try_from(100).unwrap(),
    };

    let raw = RawMidiMessage::from(&note);
    println!("Message: {}", note);
    println!("Raw bytes: {:02X?}", &*raw);

    match decode(&raw) {
        Ok(decoded) => println!("Decoded: {:?}", decoded),
        Err(e) => eprintln!("Failed to decode: {}", e),
    }

    if let Err(CodecError::Parse { reason, bytes }) = decode(&[0x90, 0xFF]) {
        println!("Parse error: {}", reason);
        println!("Rejected bytes: {:02X?}", bytes);
    }

    let note_off = MidiMessage::NoteOff {
        channel: Channel::Ch1,
        key: DataByte::try_from(60).unwrap(),
        velocity: DataByte::try_from(0).unwrap(),
    };

    let raw_off = RawMidiMessage::from(&note_off);
    assert_eq!(
        decode(&raw_off).ok(),
        Some(midi_io::Decoded::Message(note_off))
    );
    println!("Roundtrip successful");
}
