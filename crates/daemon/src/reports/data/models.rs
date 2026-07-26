//! Diesel Queryable structs for reports.

/// One hour bucket (0-23) with total focus milliseconds.
#[derive(Debug, Clone)]
pub struct HourlyUsageRow {
    pub hour: u8,
    pub total_millis: i64,
}
