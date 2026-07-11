use std::time::Duration;

use midi_io::Client;
use midi_io::DestinationChange;
use midi_io::DestinationChanges;
use midi_io::SourceChange;
use midi_io::SourceChanges;

const EVENT_TIMEOUT: Duration = Duration::from_secs(10);

async fn expect_source_change(
    changes: &mut SourceChanges,
    what: &str,
    mut pred: impl FnMut(&SourceChange) -> bool,
) {
    tokio::time::timeout(EVENT_TIMEOUT, async {
        loop {
            let change = changes.recv().await.expect("source change stream closed");
            if pred(&change) {
                return;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

async fn expect_destination_change(
    changes: &mut DestinationChanges,
    what: &str,
    mut pred: impl FnMut(&DestinationChange) -> bool,
) {
    tokio::time::timeout(EVENT_TIMEOUT, async {
        loop {
            let change = changes
                .recv()
                .await
                .expect("destination change stream closed");
            if pred(&change) {
                return;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

#[tokio::test]
#[ignore = "platform integration tests"]
async fn virtual_ports_emit_added_and_removed() {
    let client = Client::new("hotplug-e2e").await.expect("client");
    let mut source_changes = client.source_changes();
    let mut destination_changes = client.destination_changes();

    let source_name = format!("hotplug-e2e-src-{}", std::process::id());
    let destination_name = format!("hotplug-e2e-dst-{}", std::process::id());

    let virtual_source = client
        .create_virtual_source(&source_name)
        .await
        .expect("create virtual source");
    let source_id = virtual_source.as_source().id();
    expect_source_change(
        &mut source_changes,
        "source Added",
        |c| matches!(c, SourceChange::Added(p) if p.id() == source_id),
    )
    .await;

    let virtual_destination = client
        .create_virtual_destination(&destination_name)
        .await
        .expect("create virtual destination");
    let destination_id = virtual_destination.as_destination().id();
    expect_destination_change(
        &mut destination_changes,
        "destination Added",
        |c| matches!(c, DestinationChange::Added(p) if p.id() == destination_id),
    )
    .await;

    drop(virtual_source);
    expect_source_change(
        &mut source_changes,
        "source Removed",
        |c| matches!(c, SourceChange::Removed(p) if p.id() == source_id),
    )
    .await;

    drop(virtual_destination);
    expect_destination_change(
        &mut destination_changes,
        "destination Removed",
        |c| matches!(c, DestinationChange::Removed(p) if p.id() == destination_id),
    )
    .await;
}
