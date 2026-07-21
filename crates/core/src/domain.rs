//! Shared domain types used across daemon ↔ GUI D-Bus interface.
//! Flat structs with sentinel values for zvariant 5 compat (no Option<T>).
//! Convert to proper domain types with Options at handler boundaries.

use crate::valuetypes::*;
use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};
use zvariant::{OwnedValue, Type, Value};

/// Policy kind discriminant — maps to DB integer.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize_repr, Deserialize_repr, Type)]
pub enum PolicyKind {
    Block = 0,
    TimeLimit = 1,
    Notify = 2,
}

/// Full policy as exposed over D-Bus.
/// 0 / empty string = None for optional fields.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct Policy {
    pub id: PolicyId,
    pub name: String,
    pub kind: PolicyKind,
    /// Empty string = no app target.
    pub app_id: String,
    /// 0 = no category target.
    pub category_id: i64,
    /// 0 = no time limit (Block kind).
    pub time_limit_seconds: i64,
    pub extra_seconds: i64,
    /// 0 = no repeat notification.
    pub notification_repeat_interval_seconds: i64,
    pub schedule_json: String,
    pub active: bool,
    pub created_by: u32,
    pub owner_id: u32,
    pub created_at: String,
    pub updated_at: String,
}

/// Input for creating/updating a policy.

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct PolicyInput {
    pub name: String,
    pub kind: PolicyKind,
    pub app_id: String,
    pub category_id: i64,
    pub time_limit_seconds: i64,
    pub extra_seconds: i64,
    pub notification_repeat_interval_seconds: i64,
    pub schedule_json: String,
    pub active: bool,
    pub owner_id: u32,
}

/// One row of daily usage per app.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DailyUsageEntry {
    pub date: String,
    pub user_id: u32,
    pub app_id: String,
    pub total_seconds: i64,
    pub extended: bool,
}

/// Summary for a date range.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DailySummary {
    pub date: String,
    pub user_id: u32,
    pub entries: Vec<DailyUsageEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct Category {
    pub id: CategoryId,
    pub name: String,
    pub color: String,
    pub icon: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AppCategoryRow {
    pub app_id: String,
    pub user_id: u32,
    pub category_id: i64,
    pub display_name: String,
    pub icon_path: String,
    pub ignore: bool,
}

/// Current active window info.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ActiveWindowInfo {
    pub app_id: String,
    pub title: String,
    pub pid: u32,
}

/// Why an app was blocked. D-Bus serialized as uint32_t.
/// Maps to C++ `wellbeing::BlockReason` in the plugin.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum BlockReason {
    AppTimeLimit = 0,
    CategoryTimeLimit = 1,
    AppBlock = 2,
    CategoryBlock = 3,
}

/// Block state for a currently blocked app.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BlockStateInfo {
    pub uid: u32,
    pub app_id: String,
    pub blocked: bool,
    pub reason: BlockReason,
}

/// Entry in the daemon's ActiveBlocks property.
/// Consumed by the compositor plugin (reads on startup for crash recovery)
/// and GUI (reads for dashboard display).
///
/// D-Bus wire order matches `a(s(tutau))`:
///   s  = app_id
///   t  = policy_id
///   u  = reason
///   t  = blocked_since
///   au = available_actions
#[derive(Debug, Clone, Serialize, Deserialize, Type, Value)]
pub struct ActiveBlockEntry {
    pub app_id: String,
    pub policy_id: u64,
    pub reason: u32,
    pub blocked_since: u64,
    pub available_actions: Vec<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ActiveBlocks(pub Vec<ActiveBlockEntry>);

impl TryFrom<OwnedValue> for ActiveBlocks {
    type Error = zvariant::Error;
    fn try_from(value: OwnedValue) -> Result<Self, Self::Error> {
        Vec::<ActiveBlockEntry>::try_from(value).map(Self)
    }
}

/// Window info emitted by plugin.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct WindowInfo {
    pub app_id: String,
    pub title: String,
    pub pid: u32,
    pub overlay_shown: bool,
}

/// Plugin session state.
/// variant: 0=NoSession, 1=Desktop, 2=App
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SessionState {
    pub variant: u32,
    pub app_id: String,
    pub title: String,
    pub pid: u32,
    pub overlay_shown: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use zvariant::{DynamicType, LE, Value, to_bytes};

    #[test]
    fn policy_kind_roundtrips_as_u8() {
        let kind = PolicyKind::Block;
        let ctxt = zvariant::serialized::Context::new_dbus(LE, 0);
        let bytes = to_bytes(ctxt, &kind).expect("serialize PolicyKind");
        assert_eq!(
            bytes.len(),
            1,
            "PolicyKind should serialize as 1 byte (u8), got {}",
            bytes.len()
        );
        assert_eq!(
            bytes.deserialize::<PolicyKind>().unwrap().0,
            PolicyKind::Block
        );
    }

    #[test]
    fn policy_id_roundtrips_as_i64() {
        let id = PolicyId(42i64);
        let sig = id.signature();
        assert_eq!(sig.to_string(), "t");
        let ctxt = zvariant::serialized::Context::new_dbus(LE, 0);
        let bytes = to_bytes(ctxt, &id).expect("serialize PolicyId");
        assert_eq!(bytes.len(), 8, "PolicyId should serialize as 8 bytes (i64)");
    }

    #[test]
    fn policy_struct_roundtrips() {
        let policy = Policy {
            id: PolicyId(1),
            name: "Test".to_string(),
            kind: PolicyKind::Block,
            app_id: "firefox".to_string(),
            category_id: 0,
            time_limit_seconds: 0,
            extra_seconds: 0,
            notification_repeat_interval_seconds: 0,
            schedule_json: "{}".to_string(),
            active: true,
            created_by: 1000,
            owner_id: 1000,
            created_at: "2024-01-01".to_string(),
            updated_at: "2024-01-01".to_string(),
        };
        let ctxt = zvariant::serialized::Context::new_dbus(LE, 0);
        let bytes = to_bytes(ctxt, &policy).expect("serialize Policy");
        // Deserialize back
        let (decoded, _): (Policy, _) = bytes.deserialize().expect("deserialize Policy");
        assert_eq!(decoded.name, policy.name);
        assert_eq!(decoded.kind, policy.kind);
    }

    #[test]
    fn block_reason_roundtrips_as_u32() {
        let reason = BlockReason::AppTimeLimit;
        let ctxt = zvariant::serialized::Context::new_dbus(LE, 0);
        let bytes = to_bytes(ctxt, &reason).expect("serialize BlockReason");
        assert_eq!(
            bytes.len(),
            4,
            "BlockReason should serialize as 4 bytes (u32)"
        );
        let (decoded, _): (BlockReason, _) = bytes.deserialize().expect("deserialize BlockReason");
        assert_eq!(decoded, BlockReason::AppTimeLimit);
    }

    // ── Option<WindowInfo> variant wire format ─────────────────────────────
    //
    // The plugin sends CurrentSession / FocusChanged as a D-Bus variant:
    //   None  → Value::U32(1)   (FocusVariantTag::Desktop)
    //   Some  → Value::Structure(U32(2), app_id, title, pid, uid, overlay_shown)
    //
    // These tests verify the wire format round-trips correctly so the
    // pattern matching in daemon/src/platform/linux/manager.rs works.

    #[test]
    fn window_info_variant_desktop_none() {
        // None/Desktop encodes as U32(1) — FocusVariantTag::Desktop
        let val = Value::U32(1u32);
        let ctxt = zvariant::serialized::Context::new_dbus(LE, 0);
        let bytes = to_bytes(ctxt, &val).expect("serialize desktop variant");
        let (decoded, _): (Value, _) = bytes.deserialize().expect("deserialize desktop variant");
        assert_eq!(decoded, Value::U32(1u32));
    }

    #[test]
    fn window_info_variant_app_some() {
        // Some/App encodes as a struct starting with U32(2) — FocusVariantTag::App
        use zvariant::Structure;
        let val = Value::Structure(Structure::from((
            2u32,              // FocusVariantTag::App
            "firefox",         // app_id
            "Mozilla Firefox", // title
            12345u32,          // pid
            1000u32,           // uid
            false,             // overlay_shown
        )));
        let ctxt = zvariant::serialized::Context::new_dbus(LE, 0);
        let bytes = to_bytes(ctxt, &val).expect("serialize app variant");
        let (decoded, _): (Value, _) = bytes.deserialize().expect("deserialize app variant");
        match decoded {
            Value::Structure(ref fields) => {
                let f = fields.fields();
                assert_eq!(f.len(), 6, "expected 6 fields");
                assert_eq!(f[0], Value::U32(2u32), "tag should be App");
                assert_eq!(f[1], Value::Str("firefox".into()));
                assert_eq!(f[2], Value::Str("Mozilla Firefox".into()));
                assert_eq!(f[3], Value::U32(12345u32));
                assert_eq!(f[4], Value::U32(1000u32));
                assert_eq!(f[5], Value::Bool(false));
            }
            _ => panic!("expected Value::Structure variant"),
        }
    }

    #[test]
    fn window_info_variant_pattern_match() {
        // Verify the exact pattern used in manager.rs works.
        // Plugin sends the focus value as an OwnedValue variant.
        use zvariant::{OwnedValue, Structure};

        // Test 1: Desktop (U32(1)) — matches Value::U32(1)
        let desktop_val: OwnedValue = OwnedValue::from(1u32);
        let v: Value = desktop_val.into();
        match &v {
            Value::U32(1) => { /* desktop — correct */ }
            Value::Structure(_) => panic!("expected U32 for desktop"),
            _ => panic!("unexpected variant"),
        }

        // Test 2: App (Structure with tag 2)
        let app_val: OwnedValue = Value::Structure(Structure::from((
            2u32,                // FocusVariantTag::App
            "code",              // app_id
            "main.rs — VS Code", // title
            9999u32,             // pid
            1000u32,             // uid
            true,                // overlay_shown
        )))
        .try_into()
        .expect("convert Value to OwnedValue");
        let v: Value = app_val.into();
        match &v {
            Value::U32(1) => panic!("expected Structure for app"),
            Value::Structure(s) if s.fields().len() >= 6 => {
                let f = s.fields();
                // Verify pattern from manager.rs
                assert_eq!(f[0], Value::U32(2u32), "tag should be App");
                assert_eq!(f[1], Value::Str("code".into()));
                assert_eq!(f[2], Value::Str("main.rs — VS Code".into()));
                assert_eq!(f[3], Value::U32(9999u32));
                assert_eq!(f[4], Value::U32(1000u32));
                assert_eq!(f[5], Value::Bool(true));
            }
            _ => panic!("expected Structure variant"),
        }
    }
}
