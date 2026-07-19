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
    pub fn new(s: &str) -> Self {
        Self(s.to_string())
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
#[zvariant(signature = "u")]
pub struct PolicyId(pub i64);

/// Category identifier (SQLite row id).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[zvariant(signature = "u")]
pub struct CategoryId(pub i64);

/// Duration in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[zvariant(signature = "u")]
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

/// Opaque plugin instance identifier (e.g. "<uid>@<session>").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[zvariant(signature = "s")]
pub struct PluginInstanceId(String);

impl PluginInstanceId {
    /// Build from uid and session identifier.
    pub fn new(uid: u32, session: &str) -> Self {
        Self(format!("{}@{}", uid, session))
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
