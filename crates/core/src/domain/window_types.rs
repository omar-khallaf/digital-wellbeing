use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};
use zvariant::{Type, Value};

use crate::valuetypes::*;

/// Reason why an app was blocked.
///
/// Serializes as [`u8`] on the D-Bus wire (`#[repr(u8)]`).  Domain code
/// matches on this enum; the integer conversion lives only at the D-Bus
/// boundary via [`From`]/[`TryFrom`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize_repr, Deserialize_repr, Type, Value)]
#[zvariant(signature = "y")]
pub enum BlockReason {
    AppTimeLimit = 0,
    CategoryTimeLimit = 1,
    AppBlock = 2,
    CategoryBlock = 3,
}

impl From<BlockReason> for u8 {
    fn from(r: BlockReason) -> Self {
        r as u8
    }
}

impl TryFrom<u8> for BlockReason {
    type Error = &'static str;

    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(BlockReason::AppTimeLimit),
            1 => Ok(BlockReason::CategoryTimeLimit),
            2 => Ok(BlockReason::AppBlock),
            3 => Ok(BlockReason::CategoryBlock),
            _ => Err("unknown BlockReason discriminant"),
        }
    }
}

/// An entry in the blocked-apps map that is exposed over D-Bus.
///
/// All fields use validated domain types; the integer wire format is
/// handled by serde / zvariant at the D-Bus boundary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, Value)]
pub struct BlockedAppEntry {
    pub app_class: AppClass,
    pub policy_id: PolicyId,
    pub reason: BlockReason,
    pub blocked_since: u64,
}

/// Payload for the `AppBlocked` D-Bus signal.
///
/// The D-Bus wire signature is `(usby)` — matching the field order below.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, Value)]
pub struct AppBlockedSignal {
    pub uid: Uid,
    pub app_class: AppClass,
    pub blocked: bool,
    pub reason: BlockReason,
}

/// Payload for the `DomainBlocked` D-Bus signal.
///
/// The D-Bus wire signature is `(usby)` — matching the field order below.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, Value)]
pub struct DomainBlockedSignal {
    pub uid: Uid,
    pub domain: DomainPattern,
    pub blocked: bool,
    pub reason: BlockReason,
}

/// Payload for the `PolicyChanged` D-Bus signal.
///
/// The D-Bus wire signature is `(u)` — the affected user id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, Value)]
pub struct PolicyChangedSignal {
    pub uid: Uid,
}

/// Payload for the `UsageUpdated` D-Bus signal.
///
/// The D-Bus wire signature is `(u)` — the affected user id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, Value)]
pub struct UsageUpdatedSignal {
    pub uid: Uid,
}

#[cfg(test)]
mod tests {
    use super::*;
    use zvariant::{DynamicType, LE, Value, to_bytes};

    #[test]
    fn app_blocked_signal_wire_signature() {
        let sig = AppBlockedSignal {
            uid: Uid(1000),
            app_class: AppClass::new("firefox").unwrap(),
            blocked: true,
            reason: BlockReason::AppBlock,
        };
        assert_eq!(sig.signature().to_string(), "(usby)");
    }

    #[test]
    fn app_blocked_signal_value_roundtrip() {
        let sig = AppBlockedSignal {
            uid: Uid(1000),
            app_class: AppClass::new("firefox").unwrap(),
            blocked: true,
            reason: BlockReason::AppBlock,
        };
        let val: Value<'_> = sig.clone().into();
        let decoded: AppBlockedSignal = val.try_into().expect("AppBlockedSignal from Value");
        assert_eq!(decoded.uid, sig.uid);
        assert_eq!(decoded.app_class, sig.app_class);
        assert_eq!(decoded.reason, sig.reason);
        let ctxt = zvariant::serialized::Context::new_dbus(LE, 0);
        let bytes = to_bytes(ctxt, &sig).expect("serialize AppBlockedSignal");
        let (back, _): (AppBlockedSignal, _) = bytes.deserialize().expect("deserialize");
        assert_eq!(back.uid, sig.uid);
        assert_eq!(back.app_class, sig.app_class);
    }

    #[test]
    fn domain_blocked_signal_wire_signature() {
        let sig = DomainBlockedSignal {
            uid: Uid(1000),
            domain: DomainPattern::new("reddit.com").unwrap(),
            blocked: true,
            reason: BlockReason::CategoryBlock,
        };
        assert_eq!(sig.signature().to_string(), "(usby)");
    }

    #[test]
    fn domain_blocked_signal_value_roundtrip() {
        let sig = DomainBlockedSignal {
            uid: Uid(1000),
            domain: DomainPattern::new("reddit.com").unwrap(),
            blocked: true,
            reason: BlockReason::CategoryBlock,
        };
        let val: Value<'_> = sig.clone().into();
        let decoded: DomainBlockedSignal = val.try_into().expect("DomainBlockedSignal from Value");
        assert_eq!(decoded.uid, sig.uid);
        assert_eq!(decoded.domain, sig.domain);
        assert_eq!(decoded.reason, sig.reason);
    }

    #[test]
    fn policy_changed_signal_wire_signature() {
        let sig = PolicyChangedSignal { uid: Uid(1000) };
        assert_eq!(sig.signature().to_string(), "(u)");
    }

    #[test]
    fn policy_changed_signal_value_roundtrip() {
        let sig = PolicyChangedSignal { uid: Uid(1000) };
        let val: Value<'_> = sig.clone().into();
        let decoded: PolicyChangedSignal = val.try_into().expect("PolicyChangedSignal from Value");
        assert_eq!(decoded.uid, sig.uid);
    }

    #[test]
    fn usage_updated_signal_wire_signature() {
        let sig = UsageUpdatedSignal { uid: Uid(1000) };
        assert_eq!(sig.signature().to_string(), "(u)");
    }

    #[test]
    fn usage_updated_signal_value_roundtrip() {
        let sig = UsageUpdatedSignal { uid: Uid(1000) };
        let val: Value<'_> = sig.clone().into();
        let decoded: UsageUpdatedSignal = val.try_into().expect("UsageUpdatedSignal from Value");
        assert_eq!(decoded.uid, sig.uid);
    }
}
