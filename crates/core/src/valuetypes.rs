use serde::{Deserialize, Serialize};
use zvariant::Type;

/// Application identifier (e.g. "firefox", "Code", "org.gnome.gedit").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[zvariant(signature = "s")]
pub struct AppId(String);

impl AppId {
    /// Validate and construct. Rejects empty strings.
    pub fn new(s: &str) -> Result<Self, Error> {
        if s.is_empty() {
            return Err(Error::EmptyAppId);
        }
        Ok(Self(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for AppId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AppId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Window title string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[zvariant(signature = "s")]
pub struct WindowTitle(String);

impl WindowTitle {
    /// Construct with character-aware trimming to a maximum of 256 characters.
    ///
    /// Uses `.chars().take(256)` so multi-byte / multi-code-point
    /// characters are never truncated mid-sequence. Titles exceeding
    /// 256 characters (not bytes) are silently truncated at the char boundary.
    /// An additional 1024-byte CHECK constraint in the database acts as
    /// a safety ceiling for unusually wide characters.
    pub fn new(s: &str) -> Self {
        Self(s.chars().take(256).collect::<String>())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Process ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[zvariant(signature = "u")]
pub struct Pid(pub u32);

/// Policy identifier (SQLite row id).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[zvariant(signature = "t")]
pub struct PolicyId(pub i64);

/// Category identifier (SQLite row id).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[zvariant(signature = "t")]
pub struct CategoryId(pub i64);

/// Duration in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[zvariant(signature = "t")]
pub struct DurationSecs(pub i64);

impl DurationSecs {
    pub fn as_secs(&self) -> i64 {
        self.0
    }
}

/// Unix user ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[zvariant(signature = "u")]
pub struct Uid(pub u32);

/// Inclusive date range for usage queries.
///
/// `start` and `end` are calendar dates (no time component). The range is
/// validated at construction time so `start > end` is impossible at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DateRange {
    pub start: chrono::NaiveDate,
    pub end: chrono::NaiveDate,
}

impl DateRange {
    /// Last N days including today.
    pub fn last_n_days(n: u32) -> Self {
        let today = chrono::Utc::now().date_naive();
        let start = today - chrono::Days::new((n - 1) as u64);
        Self { start, end: today }
    }

    /// Validate that start <= end.
    pub fn validate(self) -> Result<(), Error> {
        if self.start > self.end {
            return Err(Error::InvalidArgument("DateRange start must be <= end"));
        }
        Ok(())
    }

    /// Preset ranges: 7, 30, 90 days.
    pub fn presets() -> [Self; 3] {
        [
            Self::last_n_days(7),
            Self::last_n_days(30),
            Self::last_n_days(90),
        ]
    }

    /// Format as `%Y-%m-%d` for D-Bus / SQL queries.
    pub fn start_str(&self) -> String {
        self.start.format("%Y-%m-%d").to_string()
    }
    pub fn end_str(&self) -> String {
        self.end.format("%Y-%m-%d").to_string()
    }
}

/// Opaque plugin instance identifier (unique D-Bus bus name, e.g. ":1.123").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[zvariant(signature = "s")]
pub struct PluginInstanceId(String);

impl PluginInstanceId {
    /// Build from the plugin's unique D-Bus bus name (`header.sender()`).
    pub fn new(bus_name: &str) -> Self {
        Self(bus_name.to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for PluginInstanceId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Overlay action button shown on block overlay.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum OverlayAction {
    Extra = 0,
    Close = 1,
}

use crate::error::Error;
