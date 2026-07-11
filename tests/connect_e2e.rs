use midi_io::Client;
use midi_io::Error;
use midi_io::IoError;

#[tokio::test]
#[ignore = "platform integration tests"]
async fn second_source_connect_rejected_until_drop() {
    let client = Client::new("connect-e2e-src").await.unwrap();
    let virtual_source = client
        .create_virtual_source(&format!("connect-e2e-src-{}", std::process::id()))
        .await
        .unwrap();
    let source = virtual_source.as_source();

    let first = client.connect_source(&source).await.unwrap();
    let err = client.connect_source(&source).await.unwrap_err();
    assert!(matches!(err, Error::Io(IoError::AlreadyConnected)));

    drop(first);
    client.connect_source(&source).await.unwrap();
}

#[tokio::test]
#[ignore = "platform integration tests"]
async fn second_destination_connect_rejected_until_drop() {
    let client = Client::new("connect-e2e-dst").await.unwrap();
    let virtual_destination = client
        .create_virtual_destination(&format!("connect-e2e-dst-{}", std::process::id()))
        .await
        .unwrap();
    let destination = virtual_destination.as_destination();

    let first = client.connect_destination(&destination).await.unwrap();
    let err = client.connect_destination(&destination).await.unwrap_err();
    assert!(matches!(err, Error::Io(IoError::AlreadyConnected)));

    drop(first);
    client.connect_destination(&destination).await.unwrap();
}
