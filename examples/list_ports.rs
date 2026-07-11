use midi_io::Client;

#[tokio::main]
async fn main() -> Result<(), midi_io::Error> {
    let client = Client::new("list-ports").await?;

    let sources = client.sources().await?;
    println!("Sources:");
    for source in sources {
        println!("  {}", source.name());
    }

    let destinations = client.destinations().await?;
    println!("Destinations:");
    for destination in destinations {
        println!("  {}", destination.name());
    }

    Ok(())
}
