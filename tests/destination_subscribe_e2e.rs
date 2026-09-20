#![cfg(target_os = "linux")]

use midi_io::Client;

fn connected_from(client: i32, port: i32) -> bool {
    let clients =
        std::fs::read_to_string("/proc/asound/seq/clients").expect("read /proc/asound/seq/clients");
    let mut seen_client = None;
    let mut here = false;
    for line in clients.lines() {
        if let Some(n) = line.strip_prefix("Client ").and_then(number_before_colon) {
            seen_client = Some(n);
            here = false;
        } else if let Some(n) = line
            .trim_start()
            .strip_prefix("Port ")
            .and_then(number_before_colon)
        {
            here = seen_client == Some(client) && n == port;
        } else if here && line.trim_start().starts_with("Connected From:") {
            return true;
        }
    }
    false
}

fn number_before_colon(rest: &str) -> Option<i32> {
    rest.split(':').next()?.trim().parse().ok()
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
