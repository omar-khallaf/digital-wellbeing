use thiserror::Error;

/// Core domain errors. No formatted strings — raw metadata fields only.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum Error {
    #[error("empty app id")]
    EmptyAppId,

    #[error("invalid uid")]
    InvalidUid,

    #[error("access denied: caller uid {caller} cannot act on owner uid {owner}")]
    AccessDenied { caller: u32, owner: u32 },

    #[error("policy not found: {0}")]
    PolicyNotFound(i64),

    #[error("invalid policy kind")]
    InvalidPolicyKind,

    #[error("invalid policy: {reason}")]
    InvalidPolicy { reason: &'static str },

    #[error("category not found: {0}")]
    CategoryNotFound(i64),

    #[error("plugin not connected")]
    PluginNotConnected,

    #[error("dbus error: {0}")]
    Dbus(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;
