use serde::{Deserialize, Serialize, de};
use zvariant::{Type, Value};

/// Application identifier (e.g. "firefox", "Code", "org.gnome.gedit").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type, Value)]
#[zvariant(signature = "s")]
pub struct AppClass(String);

impl AppClass {
    /// Validate and construct. Rejects empty strings.
    pub fn new(s: &str) -> Result<Self, Error> {
        if s.is_empty() {
            return Err(Error::EmptyAppClass);
        }
        Ok(Self(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for AppClass {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AppClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Window title string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type, Value)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Value)]
#[zvariant(signature = "u")]
pub struct Pid(pub u32);

/// Policy identifier (SQLite row id).
///
/// # Wire format
/// Explicit `x` (INT64) signature to match serde's i64 serialization.
/// Do NOT use `t` (UINT64) — that would disagree with serde and cause
/// "incorrect type" errors in the D-Bus Value deserialization path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type, Value)]
#[zvariant(signature = "x")]
pub struct PolicyId(pub i64);

/// Fixed category discriminant (no SQLite row id, no DB table).
///
/// Stored as `u8` in the database and serialized as `y` (BYTE) over D-Bus.
/// [`Display`] returns the human-readable category name for display/UI use.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Type, Value)]
#[zvariant(signature = "y")]
pub enum Category {
    Productivity = 0,
    Communication = 1,
    Entertainment = 2,
    Social = 3,
    Development = 4,
    Utilities = 5,
    Uncategorized = 6,
}

impl Category {
    /// All variants in discriminant order.
    pub const ALL: [Category; 7] = [
        Category::Productivity,
        Category::Communication,
        Category::Entertainment,
        Category::Social,
        Category::Development,
        Category::Utilities,
        Category::Uncategorized,
    ];

    /// Static display name.
    pub const fn name(self) -> &'static str {
        match self {
            Category::Productivity => "Productivity",
            Category::Communication => "Communication",
            Category::Entertainment => "Entertainment",
            Category::Social => "Social",
            Category::Development => "Development",
            Category::Utilities => "Utilities",
            Category::Uncategorized => "Uncategorized",
        }
    }

    /// Static hex color string.
    pub const fn color(self) -> &'static str {
        match self {
            Category::Productivity => "#4CAF50",
            Category::Communication => "#2196F3",
            Category::Entertainment => "#FF9800",
            Category::Social => "#E91E63",
            Category::Development => "#9C27B0",
            Category::Utilities => "#607D8B",
            Category::Uncategorized => "#9E9E9E",
        }
    }

    /// Static icon name.
    pub const fn icon(self) -> &'static str {
        match self {
            Category::Productivity => "terminal",
            Category::Communication => "chat",
            Category::Entertainment => "games",
            Category::Social => "globe",
            Category::Development => "code",
            Category::Utilities => "settings",
            Category::Uncategorized => "help",
        }
    }

    /// Parse a category name string into a [`Category`] variant.
    /// Unknown names map to [`Category::Uncategorized`].
    pub fn from_name(name: &str) -> Self {
        match name {
            "Productivity" => Category::Productivity,
            "Communication" => Category::Communication,
            "Entertainment" => Category::Entertainment,
            "Social" => Category::Social,
            "Development" => Category::Development,
            "Utilities" => Category::Utilities,
            _ => Category::Uncategorized,
        }
    }
}

impl From<Category> for u8 {
    fn from(c: Category) -> Self {
        c as u8
    }
}

impl From<Category> for i32 {
    fn from(c: Category) -> Self {
        c as i32
    }
}

impl TryFrom<u8> for Category {
    type Error = &'static str;

    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(Category::Productivity),
            1 => Ok(Category::Communication),
            2 => Ok(Category::Entertainment),
            3 => Ok(Category::Social),
            4 => Ok(Category::Development),
            5 => Ok(Category::Utilities),
            6 => Ok(Category::Uncategorized),
            _ => Err("unknown Category discriminant"),
        }
    }
}

impl TryFrom<i32> for Category {
    type Error = &'static str;

    fn try_from(v: i32) -> Result<Self, Self::Error> {
        if !(0..=6).contains(&v) {
            Err("unknown Category discriminant")
        } else {
            Category::try_from(v as u8)
        }
    }
}

impl std::fmt::Display for Category {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Default is [`Category::Uncategorized`].
impl Default for Category {
    fn default() -> Self {
        Category::Uncategorized
    }
}

impl serde::Serialize for Category {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(*self as u8)
    }
}

impl<'de> serde::Deserialize<'de> for Category {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let v = u8::deserialize(deserializer)?;
        Category::try_from(v).map_err(de::Error::custom)
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type, Value,
)]
#[zvariant(signature = "x")]
pub struct DurationSecs(pub i64);

impl DurationSecs {
    pub fn as_secs(&self) -> i64 {
        self.0
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type, Value,
)]
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type, Value)]
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

/// Domain pattern for domain-targeted policies.
///
/// Supports exact match (`"reddit.com"`), subdomain wildcard (`"*.reddit.com"`),
/// suffix match (`".reddit.com"`), and regex (`"/regex/"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type, Value)]
#[zvariant(signature = "s")]
pub struct DomainPattern(String);

impl DomainPattern {
    /// Validate and construct. Rejects empty strings.
    pub fn new(s: &str) -> Result<Self, Error> {
        if s.is_empty() {
            return Err(Error::InvalidArgument("DomainPattern cannot be empty"));
        }
        Ok(Self(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for DomainPattern {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for DomainPattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

use crate::error::Error;
