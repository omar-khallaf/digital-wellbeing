use serde::{Deserialize, Serialize};
use zvariant::Type;

/// Application identifier (e.g. "firefox", "Code", "org.gnome.gedit").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[zvariant(signature = "s")]
pub struct AppId(String);

impl AppId {
    pub fn new(s: &str) -> Result<Self, Error> { todo!() }
    pub fn as_str(&self) -> &str { todo!() }
}

impl AsRef<str> for AppId {
    fn as_ref(&self) -> &str { todo!() }
}

impl std::fmt::Display for AppId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { todo!() }
}

/// Window title string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[zvariant(signature = "s")]
pub struct WindowTitle(String);

impl WindowTitle {
    pub fn new(s: &str) -> Self { todo!() }
    pub fn as_str(&self) -> &str { todo!() }
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
    pub fn as_secs(&self) -> i64 { todo!() }
}

use crate::error::Error;
