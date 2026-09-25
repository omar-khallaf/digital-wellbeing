use chrono::{DateTime, Utc};

/// Abstraction over wall-clock time for deterministic testing.
pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> DateTime<Utc>;
}

/// Production clock: delegates to `Utc::now()`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Virtual clock for testing — returns the configured time.
#[derive(Debug, Clone)]
pub struct VirtualClock {
    now: DateTime<Utc>,
}

impl VirtualClock {
    pub fn new(now: DateTime<Utc>) -> Self {
        Self { now }
    }

    pub fn advance(&mut self, delta: chrono::Duration) {
        self.now += delta;
    }

    pub fn set_time(&mut self, now: DateTime<Utc>) {
        self.now = now;
    }
}

impl Clock for VirtualClock {
    fn now(&self) -> DateTime<Utc> {
        self.now
    }
}

/// Milliseconds in one UTC day.
pub const DAY_MS: i64 = 86_400_000;

/// Split `[start_ms, end_ms)` across UTC day boundaries.
///
/// Returns one `(day_start_ms, duration_ms)` pair per covered day, where
/// `day_start_ms` is the UTC midnight (epoch millis) opening that day.
/// Returns an empty vec when `end_ms <= start_ms`.
pub fn split_by_utc_day(start_ms: i64, end_ms: i64) -> Vec<(i64, i64)> {
    if end_ms <= start_ms {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut cursor = start_ms;
    while cursor < end_ms {
        let day_start = cursor - cursor.rem_euclid(DAY_MS);
        let next_midnight = day_start + DAY_MS;
        let seg_end = end_ms.min(next_midnight);
        out.push((day_start, seg_end - cursor));
        cursor = seg_end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_when_end_not_after_start() {
        assert!(split_by_utc_day(100, 100).is_empty());
        assert!(split_by_utc_day(200, 100).is_empty());
    }

    #[test]
    fn single_day_span() {
        let segs = split_by_utc_day(1_000, 2_000);
        assert_eq!(segs, vec![(0, 1_000)]);
    }

    #[test]
    fn multi_day_span_middle_days_full() {
        let start = DAY_MS - 1_000;
        let end = 2 * DAY_MS + 5_000;
        let segs = split_by_utc_day(start, end);
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0], (0, 1_000));
        assert_eq!(segs[1], (DAY_MS, DAY_MS));
        assert_eq!(segs[2], (2 * DAY_MS, 5_000));
    }
}
