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
