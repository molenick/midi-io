use std::time::Duration;

use midi_io::Channel;
use midi_io::Client;
use midi_io::DataByte;
use midi_io::Decoded;
use midi_io::MidiMessage;

#[tokio::main]
async fn main() -> Result<(), midi_io::Error> {
    let client = Client::new("virtual-destination").await?;

    let destination = client.create_virtual_destination("midi-io sink").await?;
    eprintln!("Virtual destination created: midi-io sink");

    let conn = client
        .connect_destination(&destination.as_destination())
        .await?;

    let _sender = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        let note_on = MidiMessage::NoteOn {
            channel: Channel::Ch1,
            key: DataByte::try_from(60).unwrap(),
            velocity: DataByte::try_from(100).unwrap(),
        };
        let note_off = MidiMessage::NoteOff {
            channel: Channel::Ch1,
            key: DataByte::try_from(60).unwrap(),
            velocity: DataByte::try_from(0).unwrap(),
        };
        loop {
            ticker.tick().await;
            if conn.send(&note_on).await.is_err() {
                break;
            }
            let _ = conn.send(&note_off).await;
        }
    });

    eprintln!("Receiving on the virtual destination. Press Ctrl-C to exit.");
    let mut events = destination.into_events();
    while let Some(timed) = events.recv().await {
        match &timed.payload {
            Ok(Decoded::Message(m)) => println!("recv: {}", m),
            Ok(Decoded::SysEx(s)) => println!("recv: SysEx({} bytes)", s.bytes().len()),
            Err(e) => eprintln!("error: {}", e),
        }
    }

    Ok(())
}
