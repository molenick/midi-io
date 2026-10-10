#![cfg(target_os = "linux")]

use alsa::seq::Addr;
use alsa::seq::PortSubscribeIter;
use alsa::seq::QuerySubsType;
use alsa::seq::Seq;
use midi_io::Client;

fn connected_from(client: i32, port: i32) -> bool {
    let seq = Seq::open(None, None, false).unwrap();
    PortSubscribeIter::new(&seq, Addr { client, port }, QuerySubsType::WRITE)
        .next()
        .is_some()
}

async fn settles_to(client: i32, port: i32, wanted: bool) -> bool {
    for _ in 0..100 {
        if connected_from(client, port) == wanted {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    false
}

#[tokio::test]
#[ignore = "platform integration tests"]
async fn connecting_a_destination_subscribes_the_output_port() {
    let client = Client::new("dst-subscribe-e2e").await.unwrap();
    let virtual_destination = client
        .create_virtual_destination(&format!("dst-subscribe-e2e-{}", std::process::id()))
        .await
        .unwrap();
    let destination = virtual_destination.as_destination();
    let bits = destination.id().to_bits();
    let (seq_client, seq_port) = ((bits >> 32) as u32 as i32, bits as u32 as i32);

    assert!(
        !connected_from(seq_client, seq_port),
        "the destination must start with no write subscription"
    );

    let connection = client.connect_destination(&destination).await.unwrap();
    assert!(
        connected_from(seq_client, seq_port),
        "connecting must subscribe our output port to the destination"
    );

    drop(connection);
    assert!(
        settles_to(seq_client, seq_port, false).await,
        "disconnecting must drop the subscription"
    );
}
