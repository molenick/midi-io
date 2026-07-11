use midi_io::Channel;
use midi_io::Client;
use midi_io::DataByte;
use midi_io::MidiMessage;

#[tokio::main]
async fn main() -> Result<(), midi_io::Error> {
    let client = Client::new("send").await?;
    let destinations = client.destinations().await?;

    if destinations.is_empty() {
        eprintln!("No MIDI destinations found.");
        return Ok(());
    }

    let dest = destinations
        .iter()
        .find(|p| p.name().contains("IAC"))
        .or_else(|| destinations.first())
        .expect("non-empty, checked above");

    eprintln!("Sending to: {}", dest.name());

    let conn = client.connect_destination(dest).await?;

    let note_on = MidiMessage::NoteOn {
        channel: Channel::Ch1,
        key: DataByte::try_from(60).unwrap(),
        velocity: DataByte::try_from(100).unwrap(),
    };
    conn.send(&note_on).await?;

    let note_off = MidiMessage::NoteOff {
        channel: Channel::Ch1,
        key: DataByte::try_from(60).unwrap(),
        velocity: DataByte::try_from(0).unwrap(),
    };
    conn.send(&note_off).await?;

    conn.send_sysex(
        &midi_io::SysEx::try_from([0xF0u8, 0x41, 0x10, 0x42, 0xF7].as_slice()).unwrap(),
    )
    .await?;

    Ok(())
}
