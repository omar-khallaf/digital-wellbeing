//! In-memory event buffer for batched DB writes.
//!
//! [`EventBuffer`] is a bounded FIFO buffer that accumulates
//! [`BufferedEvent`]s and drains them in bulk. It replaces the
//! individual `Unfocused` INSERT path in `close_interval` with
//! a deferred batch write.
//!
//! ## Overflow semantics
//!
//! When the buffer reaches [`CAPACITY`], the oldest event is
//! dropped to make room for the newest — the buffer always
//! retains the most recent `CAPACITY` events.

use std::collections::VecDeque;

use chrono::{DateTime, Utc};
use tracing::warn;
use wellbeing_core::{AppId, Uid, WindowTitle};

/// Maximum number of buffered events before the oldest is dropped.
const CAPACITY: usize = 10_000;

/// A single event waiting to be flushed to the database.
#[derive(Debug, Clone)]
pub struct BufferedEvent {
    pub uid: Uid,
    pub app_id: AppId,
    /// Discriminator matching `EVENT_WINDOW_FOCUSED = 0` /
    /// `EVENT_UNFOCUSED = 1` constants in `data/persistence.rs`.
    pub event_type: i32,
    pub timestamp: DateTime<Utc>,
    pub title: Option<WindowTitle>,
}

/// Bounded FIFO buffer of [`BufferedEvent`]s.
///
/// Pushing beyond [`CAPACITY`] drops the oldest event and
/// returns it as `Some(dropped)`.
#[derive(Debug, Clone)]
pub(crate) struct EventBuffer {
    events: VecDeque<BufferedEvent>,
}

impl EventBuffer {
    /// Push an event into the buffer.
    ///
    /// Returns `Some(dropped)` if the buffer was at capacity
    /// and the oldest event was evicted.
    pub fn push(&mut self, event: BufferedEvent) -> Option<BufferedEvent> {
        let dropped = if self.events.len() >= CAPACITY {
            let old = self.events.pop_front();
            warn!(
                dropped_app = old.as_ref().map(|e| e.app_id.as_ref()).unwrap_or("?"),
                dropped_uid = old.as_ref().map(|e| e.uid.0).unwrap_or(0),
                "event buffer overflow, oldest dropped",
            );
            old
        } else {
            None
        };
        self.events.push_back(event);
        dropped
    }

    /// Drain all buffered events in FIFO order.
    ///
    /// The buffer is cleared after this call.
    pub fn drain(&mut self) -> Vec<BufferedEvent> {
        std::mem::take(&mut self.events).into_iter().collect()
    }

    /// Number of events currently buffered.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether the buffer is empty.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

impl Default for EventBuffer {
    fn default() -> Self {
        Self {
            events: VecDeque::with_capacity(CAPACITY),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn app(s: &str) -> AppId {
        AppId::new(s).unwrap()
    }

    fn dt(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    fn event(uid: u32, app_name: &str, event_type: i32, ts: i64) -> BufferedEvent {
        BufferedEvent {
            uid: Uid(uid),
            app_id: app(app_name),
            event_type,
            timestamp: dt(ts),
            title: None,
        }
    }

    #[test]
    fn test_fifo_order_preserved_on_drain() {
        // Given
        let mut buf = EventBuffer::default();

        // When
        buf.push(event(1000, "firefox", 0, 1_000_000));
        buf.push(event(1000, "code", 0, 1_000_100));
        buf.push(event(1000, "terminal", 1, 1_000_200));

        // Then
        let drained = buf.drain();
        assert_eq!(drained.len(), 3);
        assert_eq!(drained[0].app_id.as_ref(), "firefox");
        assert_eq!(drained[1].app_id.as_ref(), "code");
        assert_eq!(drained[2].app_id.as_ref(), "terminal");
        assert!(buf.is_empty());
    }

    #[test]
    fn test_overflow_drops_oldest_event() {
        // Given
        let mut buf = EventBuffer::default();

        // When — fill to capacity
        for i in 0..CAPACITY {
            buf.push(event(1000, "firefox", 0, i as i64));
        }
        // Push one more — should drop the oldest (ts=0)
        let dropped = buf.push(event(1000, "code", 0, CAPACITY as i64));

        // Then
        assert!(dropped.is_some());
        assert_eq!(dropped.unwrap().timestamp, dt(0));

        let drained = buf.drain();
        assert_eq!(drained.len(), CAPACITY);
        assert_eq!(drained[0].timestamp, dt(1));
        assert_eq!(drained[CAPACITY - 1].timestamp, dt(CAPACITY as i64));
    }

    #[test]
    fn test_drain_on_empty_buffer_returns_empty_vec() {
        // Given
        let mut buf = EventBuffer::default();

        // When
        let drained = buf.drain();

        // Then
        assert!(drained.is_empty());
        assert_eq!(buf.len(), 0);
    }

    #[test]
    fn test_len_tracks_event_count_accurately() {
        // Given
        let mut buf = EventBuffer::default();
        assert_eq!(buf.len(), 0);

        // When
        buf.push(event(1000, "firefox", 0, 1_000_000));
        buf.push(event(1000, "code", 0, 1_000_100));

        // Then
        assert_eq!(buf.len(), 2);

        // When — drain
        buf.drain();

        // Then
        assert_eq!(buf.len(), 0);
    }

    #[test]
    fn test_is_empty_after_drain() {
        // Given
        let mut buf = EventBuffer::default();
        buf.push(event(1000, "firefox", 0, 1_000_000));
        assert!(!buf.is_empty());

        // When
        buf.drain();

        // Then
        assert!(buf.is_empty());
    }

    #[test]
    fn test_push_within_capacity_returns_none() {
        // Given
        let mut buf = EventBuffer::default();

        // When
        let result = buf.push(event(1000, "firefox", 0, 1_000_000));

        // Then
        assert!(result.is_none());
        assert_eq!(buf.len(), 1);
    }

    #[test]
    fn test_default_creates_empty_buffer() {
        // Given / When
        let buf = EventBuffer::default();

        // Then
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
    }

    #[test]
    fn test_multiple_overflow_cycles() {
        // Given
        let mut buf = EventBuffer::default();

        // When — push CAPACITY + 5 events
        for i in 0..CAPACITY + 5 {
            buf.push(event(1000, "firefox", 0, i as i64));
        }

        // Then
        assert_eq!(buf.len(), CAPACITY);
        let drained = buf.drain();
        assert_eq!(drained.len(), CAPACITY);
        assert_eq!(drained[0].timestamp, dt(5));
        assert_eq!(drained[CAPACITY - 1].timestamp, dt(CAPACITY as i64 + 4));
    }
}
