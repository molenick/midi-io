use midi_io::Client;
use midi_io::DestinationChange;
use midi_io::SourceChange;

#[tokio::main]
async fn main() -> Result<(), midi_io::Error> {
    let client = Client::new("monitor-ports").await?;

    let mut src_changes = client.source_changes();
    let mut dst_changes = client.destination_changes();

    eprintln!("Monitoring ports... (Ctrl-C to quit)");

    loop {
        tokio::select! {
            Some(change) = src_changes.recv() => {
                match change {
                    SourceChange::Added(port) => eprintln!("Source added: {}", port.name()),
                    SourceChange::Removed(port) => eprintln!("Source removed: {}", port.name()),
                }
            }
            Some(change) = dst_changes.recv() => {
                match change {
                    DestinationChange::Added(port) => eprintln!("Destination added: {}", port.name()),
                    DestinationChange::Removed(port) => eprintln!("Destination removed: {}", port.name()),
                }
            }
        }
    }
}
