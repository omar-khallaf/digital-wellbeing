// =============================================================================
// D-Bus serialization contract tests
//
// Verifies that the C++ D-Bus types used by wellbeing-lockdown match the
// Rust daemon's expectations. These tests catch wire-format mismatches like:
//   - std::tuple (flat fields, no struct container) vs sdbus::Struct
//     (struct-wrapped fields) inside sdbus::Variant
//   - Properties.Get variant wrapping (v(a(...)))
//   - FocusVariantTag values
//
// Run via: ctest --preset test-host  (or directly from build dir)
//
// The Rust side mirrors these in crates/core/src/domain.rs
//   (focus_changed_*_variant_matches_cpp_*, active_block_entry_*).
// =============================================================================

#include <cstdint>
#include <string>
#include <vector>

#include <gtest/gtest.h>
#include <sdbus-c++/sdbus-c++.h>

#include "types.hpp"

using namespace wellbeing;

// ═════════════════════════════════════════════════════════════════════════════
// FocusChanged signal variant encoding
//
// C++ emits:   sdbus::Variant{sdbus::Struct{tag, app_id, title, pid, uid}}
// Rust expects: zvariant variant containing struct(u32, string, string, u32, u32)
// D-Bus wire:   v(ussuu)
// ═════════════════════════════════════════════════════════════════════════════

TEST(DbusSerializationTest, FocusChangedAppVariantRoundtrip) {
    // Match the exact encoding in windowInfoToVariant() with the fix applied:
    // using sdbus::Struct (not std::tuple) inside sdbus::Variant.
    const auto expectedTag = static_cast<uint32_t>(FocusVariantTag::App);
    const std::string expectedAppId = "firefox";
    const std::string expectedTitle = "Mozilla Firefox";
    const uint32_t expectedPid = 12345;
    const uint32_t expectedUid = 1000;

    auto variant = sdbus::Variant{sdbus::Struct{
        expectedTag,
        expectedAppId,
        expectedTitle,
        expectedPid,
        expectedUid,
    }};

    EXPECT_STREQ(variant.peekValueType(), FOCUS_STRUCT_SIGNATURE)
        << "FocusChanged App variant content signature must match Rust handler";

    auto extracted = variant.get<sdbus::Struct<uint32_t, std::string, std::string, uint32_t, uint32_t>>();

    EXPECT_EQ(std::get<FOCUS_FIELD_TAG>(extracted), expectedTag);
    EXPECT_EQ(std::get<FOCUS_FIELD_APP_ID>(extracted), expectedAppId);
    EXPECT_EQ(std::get<FOCUS_FIELD_TITLE>(extracted), expectedTitle);
    EXPECT_EQ(std::get<FOCUS_FIELD_PID>(extracted), expectedPid);
    EXPECT_EQ(std::get<FOCUS_FIELD_UID>(extracted), expectedUid);
}

TEST(DbusSerializationTest, FocusChangedBlockedVariantRoundtrip) {
    // Blocked variant uses FocusVariantTag::Blocked (tag=2) with the same
    // struct layout as App (no overlay_bool).
    const auto expectedTag = static_cast<uint32_t>(FocusVariantTag::Blocked);
    const std::string expectedAppId = "firefox";
    const std::string expectedTitle = "Blocked Window";
    const uint32_t expectedPid = 12345;
    const uint32_t expectedUid = 1000;

    auto variant = sdbus::Variant{sdbus::Struct{
        expectedTag,
        expectedAppId,
        expectedTitle,
        expectedPid,
        expectedUid,
    }};

    EXPECT_STREQ(variant.peekValueType(), FOCUS_STRUCT_SIGNATURE)
        << "FocusChanged Blocked variant content signature must match Rust handler";

    auto extracted = variant.get<sdbus::Struct<uint32_t, std::string, std::string, uint32_t, uint32_t>>();
    EXPECT_EQ(std::get<FOCUS_FIELD_TAG>(extracted), expectedTag);
    EXPECT_EQ(std::get<FOCUS_FIELD_APP_ID>(extracted), expectedAppId);
    EXPECT_EQ(std::get<FOCUS_FIELD_TITLE>(extracted), expectedTitle);
    EXPECT_EQ(std::get<FOCUS_FIELD_PID>(extracted), expectedPid);
    EXPECT_EQ(std::get<FOCUS_FIELD_UID>(extracted), expectedUid);
}

TEST(DbusSerializationTest, FocusChangedDesktopVariantRoundtrip) {
    // Desktop = no focused window → variant(uint32(0))
    // Must match Rust handler checking Value::U32(FOCUS_TAG_DESKTOP).
    const auto desktopTag = static_cast<uint32_t>(FocusVariantTag::Desktop);

    auto variant = sdbus::Variant{desktopTag};

    EXPECT_STREQ(variant.peekValueType(), "u") << "Desktop variant must be uint32 to match Rust handler (Value::U32)";

    auto extracted = variant.get<uint32_t>();
    EXPECT_EQ(extracted, desktopTag);
}

TEST(DbusSerializationTest, FocusChangedVariantTagValues) {
    // Critical: the Rust handler in daemon/src/platform/linux/manager.rs
    // checks for Value::U32(0) = desktop, Value::U32(1) = app, Value::U32(2) = blocked.
    // These values live in FocusVariantTag enum (types.hpp).
    // Rust mirror: FOCUS_TAG_DESKTOP=0, FOCUS_TAG_APP=1, FOCUS_TAG_BLOCKED=2 (dbus_constants.rs).
    EXPECT_EQ(static_cast<uint32_t>(FocusVariantTag::Desktop), 0U)
        << "FocusVariantTag::Desktop must be 0 to match Rust FOCUS_TAG_DESKTOP";
    EXPECT_EQ(static_cast<uint32_t>(FocusVariantTag::App), 1U)
        << "FocusVariantTag::App must be 1 to match Rust FOCUS_TAG_APP";
    EXPECT_EQ(static_cast<uint32_t>(FocusVariantTag::Blocked), 2U)
        << "FocusVariantTag::Blocked must be 2 to match Rust FOCUS_TAG_BLOCKED";
}

// ═════════════════════════════════════════════════════════════════════════════
// BlockedApps property encoding
//
// C++ reads via Properties.Get:  variant containing a(stutau)
// Rust sends:                    v(a(stutau))  →  Vec<BlockedAppEntry>
// ═════════════════════════════════════════════════════════════════════════════

TEST(DbusSerializationTest, BlockedAppsPropertyRoundtrip) {
    // Verify that the tuple type used in readBlockedApps() round-trips through
    // an sdbus::Variant, mirroring how Properties.Get wraps the response.
    // Must match Rust BlockedAppEntry: (string, uint64, uint32, uint64) = (stut)
    // No actions vector — close button is handled locally.
    using BlockTuple = sdbus::Struct<std::string, uint64_t, uint32_t, uint64_t>;
    using BlockEntries = std::vector<BlockTuple>;

    BlockEntries original = {
        BlockTuple{"firefox", 42, 0, 1700000000000ULL},
        BlockTuple{"code", 99, 2, 1700000000001ULL},
    };

    auto variant = sdbus::Variant{original};

    // Properties.Get returns v(a(stut)) — the variant content is an array of structs.
    // BLOCKED_APP_SIGNATURE describes the inner struct "(stut)"; the full array
    // signature is "a" + BLOCKED_APP_SIGNATURE.
    std::string expectedSig = std::string("a") + BLOCKED_APP_SIGNATURE;
    EXPECT_STREQ(variant.peekValueType(), expectedSig.c_str())
        << "BlockedApps variant content signature must match Rust BlockedAppEntry array";

    auto extracted = variant.get<BlockEntries>();

    ASSERT_EQ(extracted.size(), original.size());
    EXPECT_EQ(std::get<0>(extracted[0]), std::get<0>(original[0]));
    EXPECT_EQ(std::get<1>(extracted[0]), std::get<1>(original[0]));
    EXPECT_EQ(std::get<2>(extracted[0]), std::get<2>(original[0]));
    EXPECT_EQ(std::get<3>(extracted[0]), std::get<3>(original[0]));
}

TEST(DbusSerializationTest, BlockedAppsEmptyArrayRoundtrip) {
    // Edge case: empty BlockedApps array.
    using BlockEntries = std::vector<sdbus::Struct<std::string, uint64_t, uint32_t, uint64_t>>;

    BlockEntries original;
    auto variant = sdbus::Variant{original};

    std::string expectedSig = std::string("a") + BLOCKED_APP_SIGNATURE;
    EXPECT_STREQ(variant.peekValueType(), expectedSig.c_str());

    auto extracted = variant.get<BlockEntries>();
    EXPECT_TRUE(extracted.empty());
}

// ═════════════════════════════════════════════════════════════════════════════
// ActivityChanged signal encoding
//
// C++ emits: static_cast<uint32_t>(FocusActivityTag)
// Rust expects: u32 where 0=Idle, 1=Resumed
// ═════════════════════════════════════════════════════════════════════════════

TEST(DbusSerializationTest, ActivityChangedIdleTagValue) {
    // Rust handler in manager.rs checks args.tag == ACTIVITY_TAG_IDLE (0).
    EXPECT_EQ(static_cast<uint32_t>(FocusActivityTag::Idle), 0U)
        << "FocusActivityTag::Idle must be 0 to match Rust ACTIVITY_TAG_IDLE";
}

TEST(DbusSerializationTest, ActivityChangedResumedTagValue) {
    // Rust handler in manager.rs treats any non-zero as Resumed.
    EXPECT_EQ(static_cast<uint32_t>(FocusActivityTag::Resumed), 1U)
        << "FocusActivityTag::Resumed must be 1 to match Rust ACTIVITY_TAG_RESUMED";
}

// ═════════════════════════════════════════════════════════════════════════════
// types.hpp constants — catch accidental changes to shared D-Bus constants
// ═════════════════════════════════════════════════════════════════════════════

TEST(DbusSerializationTest, DbUsConstantsMatchRust) {
    // These are mirrored in crates/core/src/dbus_constants.rs.
    EXPECT_STREQ(DAEMON_INTERFACE, "org.wellbeing.v1.Controller");
    EXPECT_STREQ(DAEMON_OBJECT_PATH, "/org/wellbeing/Controller");
    EXPECT_STREQ(MANAGER_INTERFACE, "org.wellbeing.v1.Manager");
    EXPECT_STREQ(MANAGER_OBJECT_PATH, "/org/wellbeing/Manager");
    EXPECT_STREQ(FOCUS_CHANGED_SIGNAL, "FocusChanged");
    EXPECT_STREQ(ACTIVITY_CHANGED_SIGNAL, "ActivityChanged");
}

auto main(int argc, char **argv) -> int {
    ::testing::InitGoogleTest(&argc, argv);
    return RUN_ALL_TESTS();
}
