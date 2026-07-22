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
#include <tuple>
#include <vector>

#include <gtest/gtest.h>
#include <sdbus-c++/sdbus-c++.h>

#include "types.hpp"

using namespace wellbeing;

// ═════════════════════════════════════════════════════════════════════════════
// FocusChanged signal variant encoding
//
// C++ emits:   sdbus::Variant{sdbus::Struct{tag, app_id, title, pid, uid, overlay}}
// Rust expects: zvariant variant containing struct(u32, string, string, u32, u32, bool)
// D-Bus wire:   v(ussuub)
// ═════════════════════════════════════════════════════════════════════════════

TEST(DbusSerializationTest, FocusChangedAppVariantRoundtrip) {
    // Match the exact encoding in windowInfoToVariant() with the fix applied:
    // using sdbus::Struct (not std::tuple) inside sdbus::Variant.
    const auto expectedTag = static_cast<uint32_t>(FocusVariantTag::App);
    const std::string expectedAppId = "firefox";
    const std::string expectedTitle = "Mozilla Firefox";
    const uint32_t expectedPid = 12345;
    const uint32_t expectedUid = 1000;
    const bool expectedOverlay = true;

    auto variant = sdbus::Variant{sdbus::Struct{
        expectedTag,
        expectedAppId,
        expectedTitle,
        expectedPid,
        expectedUid,
        expectedOverlay,
    }};

    // Verify the D-Bus signature matches what Rust expects: FOCUS_STRUCT_SIGNATURE
    EXPECT_STREQ(variant.peekValueType(), FOCUS_STRUCT_SIGNATURE)
        << "FocusChanged App variant content signature must match Rust handler";

    // Extract back and verify all fields survive round-trip.
    auto extracted = variant.get<sdbus::Struct<uint32_t, std::string, std::string, uint32_t, uint32_t, bool>>();

    EXPECT_EQ(std::get<FOCUS_FIELD_TAG>(extracted), expectedTag);
    EXPECT_EQ(std::get<FOCUS_FIELD_APP_ID>(extracted), expectedAppId);
    EXPECT_EQ(std::get<FOCUS_FIELD_TITLE>(extracted), expectedTitle);
    EXPECT_EQ(std::get<FOCUS_FIELD_PID>(extracted), expectedPid);
    EXPECT_EQ(std::get<FOCUS_FIELD_UID>(extracted), expectedUid);
    EXPECT_EQ(std::get<FOCUS_FIELD_OVERLAY>(extracted), expectedOverlay);
}

TEST(DbusSerializationTest, FocusChangedDesktopVariantRoundtrip) {
    // Desktop = no focused window → variant(uint32(0))
    // Must match Rust handler checking Value::U32(FOCUS_TAG_DESKTOP).
    const auto desktopTag = static_cast<uint32_t>(FocusVariantTag::Desktop);

    auto variant = sdbus::Variant{desktopTag};

    EXPECT_STREQ(variant.peekValueType(), "u")
        << "Desktop variant must be uint32 to match Rust handler (Value::U32)";

    auto extracted = variant.get<uint32_t>();
    EXPECT_EQ(extracted, desktopTag);
}

TEST(DbusSerializationTest, FocusChangedVariantTagValues) {
    // Critical: the Rust handler in daemon/src/platform/linux/manager.rs
    // checks for Value::U32(0) = desktop, Value::U32(1) = app.
    // These values live in FocusVariantTag enum (types.hpp).
    // Rust mirror: FOCUS_TAG_DESKTOP=0, FOCUS_TAG_APP=1 (dbus_constants.rs).
    EXPECT_EQ(static_cast<uint32_t>(FocusVariantTag::Desktop), 0u)
        << "FocusVariantTag::Desktop must be 0 to match Rust FOCUS_TAG_DESKTOP";
    EXPECT_EQ(static_cast<uint32_t>(FocusVariantTag::App), 1u)
        << "FocusVariantTag::App must be 1 to match Rust FOCUS_TAG_APP";
}

// ═════════════════════════════════════════════════════════════════════════════
// ActiveBlocks property encoding
//
// C++ reads via Properties.Get:  variant containing a(stutau)
// Rust sends:                    v(a(stutau))  →  Vec<ActiveBlockEntry>
// ═════════════════════════════════════════════════════════════════════════════

TEST(DbusSerializationTest, ActiveBlocksPropertyRoundtrip) {
    // Verify that the tuple type used in readActiveBlocks() round-trips through
    // an sdbus::Variant, mirroring how Properties.Get wraps the response.
    using BlockTuple = std::tuple<std::string, uint64_t, uint32_t, uint64_t, std::vector<uint32_t>>;
    using BlockEntries = std::vector<BlockTuple>;

    BlockEntries original = {
        BlockTuple{"firefox", 42, 0, 1700000000000ULL, {0, 1}},
        BlockTuple{"code",    99, 2, 1700000000001ULL, {1}},
    };

    // Serialize: wrap in variant (as Properties.Get does).
    auto variant = sdbus::Variant{original};

    // Verify the variant's content signature: ACTIVE_BLOCK_SIGNATURE
    EXPECT_STREQ(variant.peekValueType(), ACTIVE_BLOCK_SIGNATURE)
        << "ActiveBlocks variant content signature must match Rust ActiveBlockEntry";

    // Deserialize back.
    auto extracted = variant.get<BlockEntries>();

    ASSERT_EQ(extracted.size(), original.size());
    EXPECT_EQ(extracted[0], original[0]);
    EXPECT_EQ(extracted[1], original[1]);
}

TEST(DbusSerializationTest, ActiveBlocksEmptyArrayRoundtrip) {
    // Edge case: empty ActiveBlocks array.
    using BlockEntries = std::vector<std::tuple<std::string, uint64_t, uint32_t, uint64_t, std::vector<uint32_t>>>;

    BlockEntries original;
    auto variant = sdbus::Variant{original};

    EXPECT_STREQ(variant.peekValueType(), ACTIVE_BLOCK_SIGNATURE);

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
    EXPECT_EQ(static_cast<uint32_t>(FocusActivityTag::Idle), 0u)
        << "FocusActivityTag::Idle must be 0 to match Rust ACTIVITY_TAG_IDLE";
}

TEST(DbusSerializationTest, ActivityChangedResumedTagValue) {
    // Rust handler in manager.rs treats any non-zero as Resumed.
    EXPECT_EQ(static_cast<uint32_t>(FocusActivityTag::Resumed), 1u)
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
    EXPECT_STREQ(USER_ACTION_SIGNAL, "UserAction");
    EXPECT_STREQ(CURRENT_FOCUS_PROPERTY, "CurrentFocus");
}

// ═════════════════════════════════════════════════════════════════════════════
// Entry point
// ═════════════════════════════════════════════════════════════════════════════

auto main(int argc, char **argv) -> int {
    ::testing::InitGoogleTest(&argc, argv);
    return RUN_ALL_TESTS();
}
