use std::time::Duration;

use midi_io::Channel;
use midi_io::Client;
use midi_io::DataByte;
use midi_io::MidiMessage;
use midi_io::Streams;

#[tokio::main]
async fn main() -> Result<(), midi_io::Error> {
    let client = Client::new("loopback").await?;

    let source = client.create_virtual_source("midi-io loopback").await?;
    eprintln!("Virtual source created: midi-io loopback");

    let connection = client.connect_source(&source.as_source()).await?;

    let _listener = tokio::spawn(async move {
        let Streams {
            mut messages,
            mut sysex,
            mut errors,
        } = connection.into_streams();

        loop {
            tokio::select! {
                Some(timed) = messages.recv() => {
                    eprintln!("  msg: {}", timed.payload);
                }
                Some(timed) = sysex.recv() => {
                    eprintln!("  sysex: {} bytes", timed.payload.bytes().len());
                }
                Some(timed) = errors.recv() => {
                    eprintln!("  error: {}", timed.payload);
                }
            }
        }
    });

    eprintln!("Sending notes every 1 second. Press Ctrl-C to exit.");
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

    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    loop {
        ticker.tick().await;
        source.send(&note_on).await?;
        eprintln!("sent: NoteOn");
        source.send(&note_off).await?;
        eprintln!("sent: NoteOff");
    }
}
