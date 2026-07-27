use super::*;
use crate::dbus_constants::{
    BLOCKED_APP_SIGNATURE, EVENT_FIELD_APP_ID, EVENT_FIELD_PID, EVENT_FIELD_POWER_TAG,
    EVENT_FIELD_TAG, EVENT_FIELD_TITLE, EVENT_POWER_HIBERNATE, EVENT_POWER_SHUTDOWN,
    EVENT_POWER_SUSPEND, EVENT_STRUCT_FIELD_COUNT, EVENT_STRUCT_SIGNATURE, EVENT_TAG_BLOCK,
    EVENT_TAG_FOCUS, EVENT_TAG_IDLE, EVENT_TAG_LOCKED, EVENT_TAG_LOGOUT, EVENT_TAG_POWER,
    EVENT_TAG_RESUME, EVENT_TAG_UNFOCUS,
};
use crate::valuetypes::*;
use chrono::Utc;
use zvariant::{DynamicType, LE, Structure, Value, to_bytes};

#[test]
fn effect_roundtrips_as_u8() {
    let effect = Effect::Block;
    let ctxt = zvariant::serialized::Context::new_dbus(LE, 0);
    let bytes = to_bytes(ctxt, &effect).expect("serialize Effect");
    assert_eq!(
        bytes.len(),
        1,
        "Effect should serialize as 1 byte (u8), got {}",
        bytes.len()
    );
    let (decoded, _): (Effect, _) = bytes.deserialize().expect("deserialize Effect");
    assert_eq!(decoded, Effect::Block);
}

#[test]
fn target_type_roundtrips_as_u8() {
    let tt = TargetType::Any;
    let ctxt = zvariant::serialized::Context::new_dbus(LE, 0);
    let bytes = to_bytes(ctxt, &tt).expect("serialize TargetType");
    assert_eq!(bytes.len(), 1, "TargetType should serialize as 1 byte (u8)");
    let (decoded, _): (TargetType, _) = bytes.deserialize().expect("deserialize TargetType");
    assert_eq!(decoded, TargetType::Any);
}

#[test]
fn policy_id_roundtrips_as_i64() {
    let id = PolicyId(42i64);
    let sig = id.signature();
    assert_eq!(sig.to_string(), "x");
    let ctxt = zvariant::serialized::Context::new_dbus(LE, 0);
    let bytes = to_bytes(ctxt, &id).expect("serialize PolicyId");
    assert_eq!(bytes.len(), 8, "PolicyId should serialize as 8 bytes (i64)");
}

#[test]
fn policy_dbus_roundtrips() {
    let policy = PolicyData {
        id: PolicyId(1),
        name: "Test".to_string(),
        effect: Effect::Block,
        target_type: TargetType::App,
        app_class: AppClass::new("firefox").unwrap(),
        category_name: String::new(),
        domain_pattern: DomainPattern::new("placeholder").unwrap(),
        priority: 100,
        time_limit_minutes: 0,
        schedule_json: "[]".to_string(),
        user_id: Uid(1000),
        created_by: Uid(1000),
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    let ctxt = zvariant::serialized::Context::new_dbus(LE, 0);
    let bytes = to_bytes(ctxt, &policy).expect("serialize PolicyData");
    let (decoded, _): (PolicyData, _) = bytes.deserialize().expect("deserialize PolicyData");
    assert_eq!(decoded.name, policy.name);
    assert_eq!(decoded.effect, policy.effect);
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

// ═════════════════════════════════════════════════════════════════════════════
// Cross-language D-Bus contract tests
//
// These tests pin D-Bus type signatures and binary encodings that the C++
// compositor plugin (wellbeing-lockdown) relies on. If any of these fail,
// the plugin will get InvalidArgs D-Bus errors ("Failed to enter a
// container" / "Failed to open a variant") because the wire format
// between Rust daemon and C++ plugin diverged.
//
// The C++ side mirrors these in test/dbus_serialization_test.cpp.
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn blocked_app_entry_dbus_signature_matches_cpp() {
    let entry = BlockedAppEntry {
        app_class: AppClass::new("firefox").unwrap(),
        policy_id: PolicyId(42),
        reason: BlockReason::AppTimeLimit,
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
        app_class: AppClass::new("firefox").unwrap(),
        policy_id: PolicyId(42),
        reason: BlockReason::AppBlock,
        blocked_since: 1_700_000_000_000,
    };
    let ctxt = zvariant::serialized::Context::new_dbus(LE, 0);
    let bytes = to_bytes(ctxt, &entry).expect("serialize BlockedAppEntry");
    let (decoded, _): (BlockedAppEntry, _) =
        bytes.deserialize().expect("deserialize BlockedAppEntry");
    assert_eq!(decoded.app_class, entry.app_class);
    assert_eq!(decoded.policy_id, entry.policy_id);
    assert_eq!(decoded.reason, entry.reason);
    assert_eq!(decoded.blocked_since, entry.blocked_since);
}

#[test]
fn blocked_app_entry_value_roundtrip() {
    // Pins the `Value`-derive field order for `BlockedAppEntry`.
    // If fields are added, removed, or reordered, this test fails.
    let entry = BlockedAppEntry {
        app_class: AppClass::new("firefox").unwrap(),
        policy_id: PolicyId(42),
        reason: BlockReason::AppTimeLimit,
        blocked_since: 1_700_000_000_000,
    };

    let val: Value<'_> = entry.clone().into();
    let decoded: BlockedAppEntry = val
        .try_into()
        .expect("BlockedAppEntry::try_from(Value<'_>)");
    assert_eq!(decoded.app_class, entry.app_class);
    assert_eq!(decoded.policy_id, entry.policy_id);
    assert_eq!(decoded.reason, entry.reason);
    assert_eq!(decoded.blocked_since, entry.blocked_since);
}

// ═════════════════════════════════════════════════════════════════════════════
// Unified event struct tests  (replaces old FocusChanged + ActivityChanged)
//
// The `Event` D-Bus signal carries a struct with signature `(ussuu)`:
//
//   field | type   | contents
//   ------+--------+-----------------------------------------------
//   0     | u32    | event tag (EVENT_TAG_FOCUS / …)
//   1     | string | app_class
//   2     | string | title
//   3     | u32    | pid
//   4     | u32    | power_tag
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn event_struct_raw_signature() {
    let s = Structure::from((
        EVENT_TAG_FOCUS,
        "code",
        "main.rs",
        9999u32, // pid
        0u32,    // power_tag (unused for Focus)
    ));
    assert_eq!(
        s.signature().to_string(),
        EVENT_STRUCT_SIGNATURE,
        "Event struct signature must match C++ sdbus::Struct encoding. \
         Update event.rs and the C++ compositor plugin if this fails."
    );
}

#[test]
fn event_struct_focus_encoding() {
    // Construct a Focus event struct matching what the C++ plugin emits.
    let val = Value::Structure(Structure::from((
        EVENT_TAG_FOCUS,
        "firefox",
        "Mozilla Firefox",
        12345u32,
        0u32,
    )));
    let ctxt = zvariant::serialized::Context::new_dbus(LE, 0);
    let bytes = to_bytes(ctxt, &val).expect("serialize Focus event");
    let (decoded, _): (Value, _) = bytes.deserialize().expect("deserialize Focus event");
    match decoded {
        Value::Structure(ref fields) => {
            let f = fields.fields();
            assert_eq!(
                f.len(),
                EVENT_STRUCT_FIELD_COUNT,
                "expected {EVENT_STRUCT_FIELD_COUNT} fields"
            );
            assert_eq!(
                f[EVENT_FIELD_TAG],
                Value::U32(EVENT_TAG_FOCUS),
                "field 0 = event tag"
            );
            assert_eq!(
                f[EVENT_FIELD_APP_ID],
                Value::Str("firefox".into()),
                "field 1 = app_class"
            );
            assert_eq!(
                f[EVENT_FIELD_TITLE],
                Value::Str("Mozilla Firefox".into()),
                "field 2 = title"
            );
            assert_eq!(f[EVENT_FIELD_PID], Value::U32(12345u32), "field 3 = pid");
            assert_eq!(
                f[EVENT_FIELD_POWER_TAG],
                Value::U32(0),
                "field 4 = power_tag (unused for Focus)"
            );
        }
        _ => panic!("expected Value::Structure variant"),
    }
}

#[test]
fn event_struct_unfocus_encoding() {
    let val = Value::Structure(Structure::from((EVENT_TAG_UNFOCUS, "", "", 0u32, 0u32)));
    let ctxt = zvariant::serialized::Context::new_dbus(LE, 0);
    let bytes = to_bytes(ctxt, &val).expect("serialize Unfocus event");
    let (decoded, _): (Value, _) = bytes.deserialize().expect("deserialize Unfocus event");
    match decoded {
        Value::Structure(ref fields) => {
            let f = fields.fields();
            assert_eq!(
                f[EVENT_FIELD_TAG],
                Value::U32(EVENT_TAG_UNFOCUS),
                "field 0 = event tag"
            );
        }
        _ => panic!("expected Value::Structure variant"),
    }
}

#[test]
fn event_struct_power_encoding() {
    let val = Value::Structure(Structure::from((
        EVENT_TAG_POWER,
        "",
        "",
        0u32,
        EVENT_POWER_HIBERNATE,
    )));
    let ctxt = zvariant::serialized::Context::new_dbus(LE, 0);
    let bytes = to_bytes(ctxt, &val).expect("serialize Power/Hibernate event");
    let (decoded, _): (Value, _) = bytes
        .deserialize()
        .expect("deserialize Power/Hibernate event");
    match decoded {
        Value::Structure(ref fields) => {
            let f = fields.fields();
            assert_eq!(
                f[EVENT_FIELD_TAG],
                Value::U32(EVENT_TAG_POWER),
                "field 0 = EVENT_TAG_POWER"
            );
            assert_eq!(
                f[EVENT_FIELD_POWER_TAG],
                Value::U32(EVENT_POWER_HIBERNATE),
                "field 4 = Hibernate"
            );
        }
        _ => panic!("expected Value::Structure variant"),
    }
}

#[test]
fn event_struct_all_tags_have_correct_field_count() {
    let tags = [
        (EVENT_TAG_FOCUS, "Focus"),
        (EVENT_TAG_UNFOCUS, "Unfocus"),
        (EVENT_TAG_BLOCK, "Block"),
        (EVENT_TAG_IDLE, "Idle"),
        (EVENT_TAG_RESUME, "Resume"),
        (EVENT_TAG_LOGOUT, "LogOut"),
        (EVENT_TAG_LOCKED, "Locked"),
        (EVENT_TAG_POWER, "PowerEvent"),
    ];
    let ctxt = zvariant::serialized::Context::new_dbus(LE, 0);
    for (tag, name) in &tags {
        let val = Value::Structure(Structure::from((tag, "", "", 0u32, 0u32)));
        let bytes = to_bytes(ctxt, &val).unwrap_or_else(|e| panic!("serialize {name} event: {e}"));
        let (decoded, _): (Value, _) = bytes
            .deserialize()
            .unwrap_or_else(|e| panic!("deserialize {name} event: {e}"));
        match decoded {
            Value::Structure(ref fields) => {
                assert_eq!(
                    fields.fields().len(),
                    EVENT_STRUCT_FIELD_COUNT,
                    "{name} event should have {EVENT_STRUCT_FIELD_COUNT} fields"
                );
            }
            _ => panic!("{name} event should be Value::Structure"),
        }
    }
}

#[test]
fn event_struct_encode_decode_roundtrip_all_power_kinds() {
    let power_kinds = [
        (EVENT_POWER_SUSPEND, "Suspend"),
        (EVENT_POWER_HIBERNATE, "Hibernate"),
        (EVENT_POWER_SHUTDOWN, "Shutdown"),
    ];
    let ctxt = zvariant::serialized::Context::new_dbus(LE, 0);
    for (power_tag, name) in &power_kinds {
        let val = Value::Structure(Structure::from((EVENT_TAG_POWER, "", "", 0u32, power_tag)));
        let bytes =
            to_bytes(ctxt, &val).unwrap_or_else(|e| panic!("serialize Power/{name} event: {e}"));
        let (decoded, _): (Value, _) = bytes
            .deserialize()
            .unwrap_or_else(|e| panic!("deserialize Power/{name} event: {e}"));
        match decoded {
            Value::Structure(ref fields) => {
                assert_eq!(
                    fields.fields()[EVENT_FIELD_POWER_TAG],
                    Value::U32(*power_tag),
                    "Power/{name} power_tag mismatch"
                );
            }
            _ => panic!("Power/{name} event should be Value::Structure"),
        }
    }
}
