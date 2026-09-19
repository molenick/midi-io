#![cfg(any(target_os = "macos", target_os = "ios"))]

use midi_io::Client;
use midi_io::Error;
use midi_io::IoError;

fn unused_id() -> u32 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    0x4000_0000 | (std::process::id() << 12) ^ nanos
}

#[tokio::test]
#[ignore = "platform integration tests"]
async fn a_virtual_destination_takes_the_unique_id_it_is_given() {
    let client = Client::new("virtual-id-e2e").await.unwrap();
    let wanted = unused_id();
    let name = format!("virtual-id-e2e-{}", std::process::id());
    let destination = client
        .create_virtual_destination_with_id(&name, wanted)
        .await
        .unwrap();
    assert_eq!(
        destination.as_destination().id().to_bits(),
        u64::from(wanted)
    );

    let observer = Client::new("virtual-id-e2e-observer").await.unwrap();
    let listed: Vec<u64> = observer
        .destinations()
        .await
        .unwrap()
        .into_iter()
        .filter(|d| d.name() == name)
        .map(|d| d.id().to_bits())
        .collect();
    assert_eq!(listed, [u64::from(wanted)]);
}

#[tokio::test]
#[ignore = "platform integration tests"]
async fn the_same_unique_id_is_given_again_after_the_port_is_dropped() {
    let client = Client::new("virtual-id-e2e-again").await.unwrap();
    let wanted = unused_id();
    let name = format!("virtual-id-e2e-again-{}", std::process::id());
    let first = client
        .create_virtual_destination_with_id(&name, wanted)
        .await
        .unwrap();
    drop(first);
    let second = client
        .create_virtual_destination_with_id(&name, wanted)
        .await
        .unwrap();
    assert_eq!(second.as_destination().id().to_bits(), u64::from(wanted));
}

#[tokio::test]
#[ignore = "platform integration tests"]
async fn a_unique_id_held_by_another_endpoint_is_refused() {
    let client = Client::new("virtual-id-e2e-taken").await.unwrap();
    let wanted = unused_id();
    let held = client
        .create_virtual_destination_with_id(
            &format!("virtual-id-e2e-held-{}", std::process::id()),
            wanted,
        )
        .await
        .unwrap();
    let err = client
        .create_virtual_destination_with_id(
            &format!("virtual-id-e2e-refused-{}", std::process::id()),
            wanted,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Io(IoError::UniqueIdTaken)), "{err:?}");
    assert_eq!(held.as_destination().id().to_bits(), u64::from(wanted));
}
