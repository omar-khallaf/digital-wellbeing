use chrono::{DateTime, Utc};

/// Abstraction over wall-clock time for deterministic testing.
pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> DateTime<Utc>;
}

/// Production clock: delegates to `Utc::now()`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> { todo!() }
}

/// Virtual clock for testing — returns the configured time.
#[derive(Debug, Clone)]
pub struct VirtualClock {
    now: DateTime<Utc>,
}

impl VirtualClock {
    pub fn new(now: DateTime<Utc>) -> Self { todo!() }
    pub fn advance(&mut self, delta: chrono::Duration) { todo!() }
    pub fn set_time(&mut self, now: DateTime<Utc>) { todo!() }
}

impl Clock for VirtualClock {
    fn now(&self) -> DateTime<Utc> { todo!() }
}
