//! In-memory event buffer for batched DB writes.
//!
//! [`EventBuffer`] is a bounded FIFO buffer that accumulates
//! timestamped [`PlatformEvent`]s and drains them in bulk. Flush
//! frequency is controlled externally (count threshold + minute-ticker).
//! When the buffer reaches its maximum capacity new pushes are silently
//! rejected (no panic, no eviction of old events).

use std::collections::VecDeque;

use chrono::{DateTime, Utc};

use crate::blocking::domain::TimedEvent;
use crate::platform::PlatformEvent;

/// Initial allocation hint for the internal deque.
const INITIAL_CAPACITY: usize = 10_000;

/// Hard capacity cap — at this many buffered events new pushes are
/// silently rejected. ~2 MB for a worst-case flush outage.
const MAX_CAPACITY: usize = 10_000;

/// Bounded FIFO buffer of [`TimedEvent`]s.
///
/// When full, [`push`](Self::push) silently rejects new events without
/// dropping existing ones. The capacity is fixed at construction time.
#[derive(Debug, Clone)]
pub(crate) struct EventBuffer {
    events: VecDeque<TimedEvent>,
    max_capacity: usize,
}

impl EventBuffer {
    /// Push a timestamped event into the buffer.
    ///
    /// If the buffer is at capacity the event is silently rejected — no
    /// panic, no eviction of older events.
    pub fn push(&mut self, event: PlatformEvent, timestamp: DateTime<Utc>) {
        if self.events.len() >= self.max_capacity {
            return;
        }
        self.events.push_back(TimedEvent { event, timestamp });
    }

    /// Create an `EventBuffer` with the given maximum capacity.
    ///
    /// The initial allocation hint is capped at [`INITIAL_CAPACITY`] or
    /// `max`, whichever is smaller.
    pub fn with_max_capacity(max: usize) -> Self {
        let cap = INITIAL_CAPACITY.min(max);
        Self {
            events: VecDeque::with_capacity(cap),
            max_capacity: max,
        }
    }

    /// Drain all buffered events in FIFO order.
    pub fn drain(&mut self) -> Vec<TimedEvent> {
        std::mem::take(&mut self.events).into()
    }

    /// Number of events currently buffered.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

impl Default for EventBuffer {
    fn default() -> Self {
        Self::with_max_capacity(MAX_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use wellbeing_core::{AppId, Uid, WindowTitle};

    fn app(s: &str) -> AppId {
        AppId::new(s).unwrap()
    }

    fn dt(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    #[test]
    fn test_fifo_order_preserved_on_drain() {
        // Given
        let mut buf = EventBuffer::default();

        // When
        buf.push(
            PlatformEvent::Focus {
                app_id: app("firefox"),
                title: WindowTitle::new("test"),
                pid: wellbeing_core::Pid(1),
                uid: Uid(1000),
            },
            dt(1_000_000),
        );
        buf.push(
            PlatformEvent::Focus {
                app_id: app("code"),
                title: WindowTitle::new("test"),
                pid: wellbeing_core::Pid(2),
                uid: Uid(1000),
            },
            dt(1_000_100),
        );
        buf.push(
            PlatformEvent::Focus {
                app_id: app("terminal"),
                title: WindowTitle::new("test"),
                pid: wellbeing_core::Pid(3),
                uid: Uid(1000),
            },
            dt(1_000_200),
        );

        // Then
        let drained = buf.drain();
        assert_eq!(drained.len(), 3);
        assert_eq!(drained[0].event.app_id().unwrap().as_ref(), "firefox");
        assert_eq!(drained[1].event.app_id().unwrap().as_ref(), "code");
        assert_eq!(drained[2].event.app_id().unwrap().as_ref(), "terminal");
        assert!(buf.is_empty());
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
        buf.push(
            PlatformEvent::Focus {
                app_id: app("firefox"),
                title: WindowTitle::new("test"),
                pid: wellbeing_core::Pid(1),
                uid: Uid(1000),
            },
            dt(1_000_000),
        );
        buf.push(
            PlatformEvent::Focus {
                app_id: app("code"),
                title: WindowTitle::new("test"),
                pid: wellbeing_core::Pid(2),
                uid: Uid(1000),
            },
            dt(1_000_100),
        );

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
        buf.push(
            PlatformEvent::Focus {
                app_id: app("firefox"),
                title: WindowTitle::new("test"),
                pid: wellbeing_core::Pid(1),
                uid: Uid(1000),
            },
            dt(1_000_000),
        );
        assert!(!buf.is_empty());

        // When
        buf.drain();

        // Then
        assert!(buf.is_empty());
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
    fn test_push_rejects_when_full() {
        // Given — a buffer with tiny capacity
        let mut buf = EventBuffer::with_max_capacity(3);

        // When — fill to capacity
        for i in 0..3 {
            buf.push(
                PlatformEvent::Focus {
                    app_id: app("firefox"),
                    title: WindowTitle::new("test"),
                    pid: wellbeing_core::Pid(i as u32),
                    uid: Uid(1000),
                },
                dt(i as i64),
            );
        }
        assert_eq!(buf.len(), 3);

        // When — push beyond capacity
        buf.push(
            PlatformEvent::Focus {
                app_id: app("overflow"),
                title: WindowTitle::new("test"),
                pid: wellbeing_core::Pid(99),
                uid: Uid(1000),
            },
            dt(99),
        );

        // Then — buffer still has exactly the first 3 events
        assert_eq!(buf.len(), 3);
        let drained = buf.drain();
        assert_eq!(drained.len(), 3);
        assert_eq!(drained[0].timestamp, dt(0));
        assert_eq!(drained[1].timestamp, dt(1));
        assert_eq!(drained[2].timestamp, dt(2));
    }

    #[test]
    fn test_default_can_hold_large_batch() {
        // Given — a default buffer (1M capacity)
        let mut buf = EventBuffer::default();

        // When — push well past INITIAL_CAPACITY
        for i in 0..INITIAL_CAPACITY + 5 {
            buf.push(
                PlatformEvent::Focus {
                    app_id: app("firefox"),
                    title: WindowTitle::new("test"),
                    pid: wellbeing_core::Pid(1),
                    uid: Uid(1000),
                },
                dt(i as i64),
            );
        }

        // Then — all events accepted (within 1M cap)
        assert_eq!(buf.len(), INITIAL_CAPACITY + 5);
        let drained = buf.drain();
        assert_eq!(drained.len(), INITIAL_CAPACITY + 5);
    }

    #[test]
    fn test_with_max_capacity_zero_rejects_all() {
        // Given
        let mut buf = EventBuffer::with_max_capacity(0);

        // When
        buf.push(
            PlatformEvent::Focus {
                app_id: app("firefox"),
                title: WindowTitle::new("test"),
                pid: wellbeing_core::Pid(1),
                uid: Uid(1000),
            },
            dt(1),
        );

        // Then
        assert!(buf.is_empty());
    }
}
