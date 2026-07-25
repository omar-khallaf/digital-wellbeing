//! Plugin registration helpers — event forwarding loop extraction.
//!
//! The register_plugin handler spawns a background task that forwards
//! Plugin-originated signals (FocusChanged, ActivityChanged) from the
//! per-plugin mpsc channel into the daemon's unified PlatformEvent bus.

use crate::platform::PlatformEvent;

/// Spawn a background task that forwards events from a per-plugin receiver
/// into the daemon-wide event bus. Runs until the plugin disconnects or
/// the receiver is dropped.
pub fn spawn_event_forwarder(
    handle: tokio::runtime::Handle,
    ev_rx: tokio::sync::mpsc::Receiver<PlatformEvent>,
    ev_tx: tokio::sync::mpsc::UnboundedSender<PlatformEvent>,
) {
    handle.spawn(async move {
        use futures::StreamExt;
        let mut stream = tokio_stream::wrappers::ReceiverStream::new(ev_rx);
        while let Some(event) = stream.next().await {
            if ev_tx.send(event).is_err() {
                break;
            }
        }
    });
}
