use midi_io::Client;
use midi_io::Decoded;

#[tokio::main]
async fn main() -> Result<(), midi_io::Error> {
    let client = Client::new("receive").await?;
    let sources = client.sources().await?;

    if sources.is_empty() {
        eprintln!("No MIDI sources found.");
        return Ok(());
    }

    let source = sources
        .iter()
        .find(|p| p.name().contains("IAC"))
        .or_else(|| sources.first())
        .expect("non-empty, checked above");

    eprintln!("Listening on: {}", source.name());

    let conn = client.connect_source(source).await?;
    let mut events = conn.into_events();

    while let Some(timed) = events.recv().await {
        match &timed.payload {
            Ok(Decoded::Message(m)) => println!("{} {}", timed.timestamp.elapsed().as_millis(), m),
            Ok(Decoded::SysEx(s)) => println!(
                "{} SysEx({} bytes)",
                timed.timestamp.elapsed().as_millis(),
                s.bytes().len()
            ),
            Err(e) => eprintln!("Error: {}", e),
        }
    }

    Ok(())
}
