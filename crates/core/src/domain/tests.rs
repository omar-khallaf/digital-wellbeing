use super::*;
use crate::dbus_constants::{
    ACTIVITY_TAG_IDLE, ACTIVITY_TAG_RESUMED, BLOCKED_APP_SIGNATURE, FOCUS_FIELD_APP_ID,
    FOCUS_FIELD_PID, FOCUS_FIELD_TAG, FOCUS_FIELD_TITLE, FOCUS_FIELD_UID, FOCUS_STRUCT_FIELD_COUNT,
    FOCUS_STRUCT_SIGNATURE, FOCUS_TAG_APP, FOCUS_TAG_DESKTOP,
};
use crate::valuetypes::*;
use chrono::Utc;
use zvariant::{DynamicType, LE, OwnedValue, Value, to_bytes};

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
fn policy_dbus_roundtrips() {
    let policy = PolicyData {
        id: PolicyId(1),
        name: "Test".to_string(),
        action: PolicyKind::Block,
        app_id: "firefox".to_string(),
        category_id: 0,
        time_limit_minutes: 0,
        notification_repeat_interval_minutes: 0,
        schedule_json: "{}".to_string(),
        active: true,
        created_by: 1000,
        owner_id: 1000,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    let ctxt = zvariant::serialized::Context::new_dbus(LE, 0);
    let bytes = to_bytes(ctxt, &policy).expect("serialize PolicyData");
    let (decoded, _): (PolicyData, _) = bytes.deserialize().expect("deserialize PolicyData");
    assert_eq!(decoded.name, policy.name);
    assert_eq!(decoded.action, policy.action);
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

#[test]
fn window_info_variant_desktop_none() {
    let val = Value::U32(FOCUS_TAG_DESKTOP);
    let ctxt = zvariant::serialized::Context::new_dbus(LE, 0);
    let bytes = to_bytes(ctxt, &val).expect("serialize desktop variant");
    let (decoded, _): (Value, _) = bytes.deserialize().expect("deserialize desktop variant");
    assert_eq!(decoded, Value::U32(FOCUS_TAG_DESKTOP));
}

#[test]
fn window_info_variant_app_some() {
    use zvariant::Structure;
    let val = Value::Structure(Structure::from((
        FOCUS_TAG_APP,
        "firefox",
        "Mozilla Firefox",
        12345u32,
        1000u32,
    )));
    let ctxt = zvariant::serialized::Context::new_dbus(LE, 0);
    let bytes = to_bytes(ctxt, &val).expect("serialize app variant");
    let (decoded, _): (Value, _) = bytes.deserialize().expect("deserialize app variant");
    match decoded {
        Value::Structure(ref fields) => {
            let f = fields.fields();
            assert_eq!(
                f.len(),
                FOCUS_STRUCT_FIELD_COUNT,
                "expected {FOCUS_STRUCT_FIELD_COUNT} fields"
            );
            assert_eq!(
                f[FOCUS_FIELD_TAG],
                Value::U32(FOCUS_TAG_APP),
                "tag should be App"
            );
            assert_eq!(f[FOCUS_FIELD_APP_ID], Value::Str("firefox".into()));
            assert_eq!(f[FOCUS_FIELD_TITLE], Value::Str("Mozilla Firefox".into()));
            assert_eq!(f[FOCUS_FIELD_PID], Value::U32(12345u32));
            assert_eq!(f[FOCUS_FIELD_UID], Value::U32(1000u32));
        }
        _ => panic!("expected Value::Structure variant"),
    }
}

// Cross-language D-Bus contract tests
//
// These tests pin D-Bus type signatures and binary encodings that the C++
// compositor plugin (wellbeing-lockdown) relies on. If any of these fail,
// the plugin will get InvalidArgs D-Bus errors ("Failed to enter a
// container" / "Failed to open a variant") because the wire format
// between Rust daemon and C++ plugin diverged.
//
// The C++ side mirrors these in test/dbus_serialization_test.cpp.

#[test]
fn blocked_app_entry_dbus_signature_matches_cpp() {
    let entry = BlockedAppEntry {
        app_id: "firefox".into(),
        policy_id: 42,
        reason: 0,
        blocked_since: 1_700_000_000_000,
    };
    assert_eq!(
        entry.signature().to_string(),
        BLOCKED_APP_SIGNATURE,
        "BlockedAppEntry D-Bus signature changed. Update C++ readBlockedApps tuple type."
    );
}

#[test]
fn blocked_app_entry_binary_roundtrip() {
    let entry = BlockedAppEntry {
        app_id: "firefox".into(),
        policy_id: 42,
        reason: 2,
        blocked_since: 1_700_000_000_000,
    };
    let ctxt = zvariant::serialized::Context::new_dbus(LE, 0);
    let bytes = to_bytes(ctxt, &entry).expect("serialize BlockedAppEntry");
    let (decoded, _): (BlockedAppEntry, _) =
        bytes.deserialize().expect("deserialize BlockedAppEntry");
    assert_eq!(decoded.app_id, entry.app_id);
    assert_eq!(decoded.policy_id, entry.policy_id);
    assert_eq!(decoded.reason, entry.reason);
    assert_eq!(decoded.blocked_since, entry.blocked_since);
}

#[test]
fn focus_changed_desktop_variant_matches_cpp_encoding() {
    // C++ emits: sdbus::Variant{uint32_t(FocusVariantTag::Desktop)}   → U32(FOCUS_TAG_DESKTOP)
    // Rust handler in manager.rs checks Value::U32(FOCUS_TAG_DESKTOP) for unfocused.
    let val = Value::U32(FOCUS_TAG_DESKTOP);
    let ctxt = zvariant::serialized::Context::new_dbus(LE, 0);
    let bytes = to_bytes(ctxt, &val).expect("serialize desktop variant");
    let (decoded, _): (Value, _) = bytes.deserialize().expect("deserialize desktop variant");
    assert_eq!(
        decoded,
        Value::U32(FOCUS_TAG_DESKTOP),
        "Desktop variant must be U32({FOCUS_TAG_DESKTOP}) to match C++ FocusVariantTag::Desktop={FOCUS_TAG_DESKTOP}"
    );
}

#[test]
fn focus_changed_app_variant_matches_cpp_struct_encoding() {
    // C++ emits: sdbus::Variant{sdbus::Struct{uint32_t(App), str, str, uint32, uint32}}
    // D-Bus wire: variant containing struct(u32, string, string, u32, u32) = v(ussuu)
    // Rust handler in manager.rs destructures this as:
    //   f[FOCUS_FIELD_TAG]     → Value::U32(FOCUS_TAG_APP)
    //   f[FOCUS_FIELD_APP_ID]  → Value::Str(app_id)
    //   f[FOCUS_FIELD_TITLE]   → Value::Str(title)
    //   f[FOCUS_FIELD_PID]     → Value::U32(pid)
    //   f[FOCUS_FIELD_UID]     → Value::U32(uid)
    use zvariant::Structure;

    let app_val: OwnedValue = Value::Structure(Structure::from((
        FOCUS_TAG_APP,
        "code",
        "main.rs",
        9999u32,
        1000u32,
    )))
    .try_into()
    .expect("convert Value to OwnedValue");
    let v: Value = app_val.into();
    match &v {
        Value::U32(_) => panic!("expected Structure for app, got U32"),
        Value::Structure(s) => {
            let f = s.fields();
            assert_eq!(
                f.len(),
                FOCUS_STRUCT_FIELD_COUNT,
                "C++ struct has {FOCUS_STRUCT_FIELD_COUNT} fields"
            );
            assert_eq!(
                f[FOCUS_FIELD_TAG],
                Value::U32(FOCUS_TAG_APP),
                "field 0 = App tag ({FOCUS_TAG_APP})"
            );
            assert_eq!(
                f[FOCUS_FIELD_APP_ID],
                Value::Str("code".into()),
                "field 1 = app_id"
            );
            assert_eq!(
                f[FOCUS_FIELD_TITLE],
                Value::Str("main.rs".into()),
                "field 2 = title"
            );
            assert_eq!(f[FOCUS_FIELD_PID], Value::U32(9999u32), "field 3 = pid");
            assert_eq!(f[FOCUS_FIELD_UID], Value::U32(1000u32), "field 4 = uid");
        }
        _ => panic!("unexpected variant type"),
    }
}

#[test]
fn focus_changed_app_variant_raw_signature() {
    use zvariant::Structure;
    let s = Structure::from((FOCUS_TAG_APP, "term", "Terminal", 7777u32, 1000u32));
    assert_eq!(
        s.signature().to_string(),
        FOCUS_STRUCT_SIGNATURE,
        "FocusChanged app variant inner struct signature must match C++ sdbus::Struct encoding"
    );
}

#[test]
fn activity_changed_idle_tag_matches_cpp_encoding() {
    // C++ emits: static_cast<uint32_t>(FocusActivityTag::Idle)  → u32(ACTIVITY_TAG_IDLE)
    // Rust handler in manager.rs checks args.tag == ACTIVITY_TAG_IDLE → PlatformEvent::Idle
    let val = Value::U32(ACTIVITY_TAG_IDLE);
    let ctxt = zvariant::serialized::Context::new_dbus(LE, 0);
    let bytes = to_bytes(ctxt, &val).expect("serialize idle tag");
    let (decoded, _): (Value, _) = bytes.deserialize().expect("deserialize idle tag");
    assert_eq!(
        decoded,
        Value::U32(ACTIVITY_TAG_IDLE),
        "ActivityChanged idle tag must be U32({ACTIVITY_TAG_IDLE}) to match C++ FocusActivityTag::Idle={ACTIVITY_TAG_IDLE}"
    );
}

#[test]
fn activity_changed_resumed_tag_matches_cpp_encoding() {
    // C++ emits: static_cast<uint32_t>(FocusActivityTag::Resumed)  → u32(ACTIVITY_TAG_RESUMED)
    // Rust handler in manager.rs checks args.tag != ACTIVITY_TAG_IDLE → PlatformEvent::Resumed
    let val = Value::U32(ACTIVITY_TAG_RESUMED);
    let ctxt = zvariant::serialized::Context::new_dbus(LE, 0);
    let bytes = to_bytes(ctxt, &val).expect("serialize resumed tag");
    let (decoded, _): (Value, _) = bytes.deserialize().expect("deserialize resumed tag");
    assert_eq!(
        decoded,
        Value::U32(ACTIVITY_TAG_RESUMED),
        "ActivityChanged resumed tag must be U32({ACTIVITY_TAG_RESUMED}) to match C++ FocusActivityTag::Resumed={ACTIVITY_TAG_RESUMED}"
    );
}

#[test]
fn window_info_variant_pattern_match() {
    use zvariant::Structure;

    let desktop_val: OwnedValue = OwnedValue::from(FOCUS_TAG_DESKTOP);
    let v: Value = desktop_val.into();
    match &v {
        Value::U32(FOCUS_TAG_DESKTOP) => {}
        Value::Structure(_) => panic!("expected U32 for desktop"),
        _ => panic!("unexpected variant"),
    }

    let app_val: OwnedValue = Value::Structure(Structure::from((
        FOCUS_TAG_APP,
        "code",
        "main.rs — VS Code",
        9999u32,
        1000u32,
    )))
    .try_into()
    .expect("convert Value to OwnedValue");
    let v: Value = app_val.into();
    match &v {
        Value::U32(FOCUS_TAG_DESKTOP) => panic!("expected Structure for app"),
        Value::Structure(s) if s.fields().len() >= FOCUS_STRUCT_FIELD_COUNT => {
            let f = s.fields();
            assert_eq!(
                f[FOCUS_FIELD_TAG],
                Value::U32(FOCUS_TAG_APP),
                "tag should be App"
            );
            assert_eq!(f[FOCUS_FIELD_APP_ID], Value::Str("code".into()));
            assert_eq!(f[FOCUS_FIELD_TITLE], Value::Str("main.rs — VS Code".into()));
            assert_eq!(f[FOCUS_FIELD_PID], Value::U32(9999u32));
            assert_eq!(f[FOCUS_FIELD_UID], Value::U32(1000u32));
        }
        _ => panic!("expected Value::Structure variant"),
    }
}
