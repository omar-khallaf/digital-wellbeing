//! Domain types for the blocking/enforcement feature.

use tokio::sync::oneshot;

/// Internal events for the blocking actor.
pub enum InternalEvent {
    /// Flush the event buffer. The optional oneshot sender is signaled
    /// after the flush completes, allowing callers (e.g. shutdown) to
    /// await completion instead of guessing with a sleep.
    Flush(Option<oneshot::Sender<()>>),
}
