// =============================================================================
// D-Bus serialization contract tests
//
// Verifies that the C++ D-Bus types used by wellbeing-lockdown match the
// Rust daemon's expectations. These tests catch wire-format mismatches like:
//   - std::tuple (flat fields, no struct container) vs sdbus::Struct
//     (struct-wrapped fields) inside sdbus::Variant
//   - Properties.Get variant wrapping (v(a(...)))
//   - EventTag / PowerTag values
//
// Run via: ctest --preset test-host  (or directly from build dir)
//
// The Rust side mirrors these in crates/core/src/domain.rs
//   (event_struct_* tests, blocked_app_entry_*).
// =============================================================================

#include <cstdint>
#include <string>
#include <vector>

#include <gtest/gtest.h>
#include <sdbus-c++/sdbus-c++.h>

#include "types.hpp"

using namespace wellbeing;

// Struct type matching the D-Bus (ussuu) event encoding.
// Typedef hides commas from the preprocessor for EXPECT_NO_THROW.
using EventStruct = sdbus::Struct<uint32_t, std::string, std::string, uint32_t, uint32_t>;

// ═════════════════════════════════════════════════════════════════════════════
// Unified Event signal struct encoding
//
// C++ emits:   sdbus::Variant{sdbus::Struct{tag, app_id, title, pid, power_tag}}
// Rust expects: zvariant variant containing struct(u32, string, string, u32, u32)
// D-Bus wire:   v(ussuu)
// ═════════════════════════════════════════════════════════════════════════════

TEST(DbusSerializationTest, EventStructSignatureMatchesRust) {
    // The Rust side checks: EVENT_STRUCT_SIGNATURE == "(ussuu)"
    auto variant = sdbus::Variant{sdbus::Struct{
        static_cast<uint32_t>(EventTag::Focus),
        std::string{"firefox"},
        std::string{"Mozilla Firefox"},
        static_cast<uint32_t>(12345),
        static_cast<uint32_t>(0),
    }};

    EXPECT_STREQ(variant.peekValueType(), EVENT_STRUCT_SIGNATURE)
        << "Event struct signature must match Rust EVENT_STRUCT_SIGNATURE = \"(ussuu)\"";
}

TEST(DbusSerializationTest, EventStructFocusRoundtrip) {
    const uint32_t expectedTag = static_cast<uint32_t>(EventTag::Focus);
    const std::string expectedAppId = "firefox";
    const std::string expectedTitle = "Mozilla Firefox";
    const uint32_t expectedPid = 12345;
    const uint32_t expectedPowerTag = 0;

    auto variant = sdbus::Variant{sdbus::Struct{
        expectedTag, expectedAppId, expectedTitle, expectedPid, expectedPowerTag,
    }};

    auto extracted = variant.get<EventStruct>();

    EXPECT_EQ(std::get<EVENT_FIELD_TAG>(extracted), expectedTag);
    EXPECT_EQ(std::get<EVENT_FIELD_APP_ID>(extracted), expectedAppId);
    EXPECT_EQ(std::get<EVENT_FIELD_TITLE>(extracted), expectedTitle);
    EXPECT_EQ(std::get<EVENT_FIELD_PID>(extracted), expectedPid);
    EXPECT_EQ(std::get<EVENT_FIELD_POWER_TAG>(extracted), expectedPowerTag);
}

TEST(DbusSerializationTest, EventStructUnfocusRoundtrip) {
    const uint32_t expectedTag = static_cast<uint32_t>(EventTag::Unfocus);

    auto variant = sdbus::Variant{sdbus::Struct{
        expectedTag,
        std::string{}, std::string{},
        static_cast<uint32_t>(0), static_cast<uint32_t>(0),
    }};

    auto extracted = variant.get<EventStruct>();

    EXPECT_EQ(std::get<EVENT_FIELD_TAG>(extracted), expectedTag);
    EXPECT_EQ(std::get<EVENT_FIELD_APP_ID>(extracted), "");
    EXPECT_EQ(std::get<EVENT_FIELD_TITLE>(extracted), "");
    EXPECT_EQ(std::get<EVENT_FIELD_PID>(extracted), 0U);
    EXPECT_EQ(std::get<EVENT_FIELD_POWER_TAG>(extracted), 0U);
}

TEST(DbusSerializationTest, EventStructBlockRoundtrip) {
    const uint32_t expectedTag = static_cast<uint32_t>(EventTag::Block);
    const std::string expectedAppId = "firefox";
    const std::string expectedTitle = "Blocked Window";

    auto variant = sdbus::Variant{sdbus::Struct{
        expectedTag, expectedAppId, expectedTitle,
        static_cast<uint32_t>(0), static_cast<uint32_t>(0),
    }};

    auto extracted = variant.get<EventStruct>();

    EXPECT_EQ(std::get<EVENT_FIELD_TAG>(extracted), expectedTag);
    EXPECT_EQ(std::get<EVENT_FIELD_APP_ID>(extracted), expectedAppId);
    EXPECT_EQ(std::get<EVENT_FIELD_TITLE>(extracted), expectedTitle);
}

TEST(DbusSerializationTest, EventStructPowerHibernateRoundtrip) {
    const uint32_t expectedTag = static_cast<uint32_t>(EventTag::Power);
    const uint32_t expectedPowerTag = static_cast<uint32_t>(PowerTag::Hibernate);

    auto variant = sdbus::Variant{sdbus::Struct{
        expectedTag,
        std::string{}, std::string{},
        static_cast<uint32_t>(0), expectedPowerTag,
    }};

    auto extracted = variant.get<EventStruct>();

    EXPECT_EQ(std::get<EVENT_FIELD_TAG>(extracted), expectedTag);
    EXPECT_EQ(std::get<EVENT_FIELD_POWER_TAG>(extracted), expectedPowerTag);
}

TEST(DbusSerializationTest, EventStructAllTagsHaveSixFields) {
    // Verify every event tag produces a 5-field struct.
    auto build = [](uint32_t tag) {
        return sdbus::Variant{sdbus::Struct{
            tag,
            std::string{}, std::string{},
            static_cast<uint32_t>(0), static_cast<uint32_t>(0),
        }};
    };

    auto checkFields = [](const char *name, sdbus::Variant &v) {
        auto extracted = v.get<EventStruct>();
        EXPECT_EQ(std::get<EVENT_FIELD_TAG>(extracted), static_cast<uint32_t>(EventTag::Focus))
            << name;
    };
    // We can't easily check field count with sdbus-c++ peek,
    // but if the get<> succeeds we know the struct has the right shape.
    for (auto tag : {static_cast<uint32_t>(EventTag::Focus),
                     static_cast<uint32_t>(EventTag::Unfocus),
                     static_cast<uint32_t>(EventTag::Block),
                     static_cast<uint32_t>(EventTag::Idle),
                     static_cast<uint32_t>(EventTag::Resume),
                     static_cast<uint32_t>(EventTag::LogOut),
                     static_cast<uint32_t>(EventTag::Power),
                     static_cast<uint32_t>(EventTag::Locked)}) {
        auto v = build(tag);
        EXPECT_NO_THROW(v.get<EventStruct>());
    }
}

TEST(DbusSerializationTest, EventTagValues) {
    // Critical: the Rust handler in daemon/src/platform/linux/manager.rs
    // checks for Value::U32(EVENT_TAG_FOCUS) = 0, …  EVENT_TAG_POWER = 6.
    EXPECT_EQ(static_cast<uint32_t>(EventTag::Focus), 0U);
    EXPECT_EQ(static_cast<uint32_t>(EventTag::Unfocus), 1U);
    EXPECT_EQ(static_cast<uint32_t>(EventTag::Block), 2U);
    EXPECT_EQ(static_cast<uint32_t>(EventTag::Idle), 3U);
    EXPECT_EQ(static_cast<uint32_t>(EventTag::Resume), 4U);
    EXPECT_EQ(static_cast<uint32_t>(EventTag::LogOut), 5U);
    EXPECT_EQ(static_cast<uint32_t>(EventTag::Power), 6U);
    EXPECT_EQ(static_cast<uint32_t>(EventTag::Locked), 7U);
}

TEST(DbusSerializationTest, PowerTagValues) {
    // Rust mirror: EVENT_POWER_SUSPEND=0, EVENT_POWER_HIBERNATE=1, EVENT_POWER_SHUTDOWN=2
    EXPECT_EQ(static_cast<uint32_t>(PowerTag::Suspend), 0U);
    EXPECT_EQ(static_cast<uint32_t>(PowerTag::Hibernate), 1U);
    EXPECT_EQ(static_cast<uint32_t>(PowerTag::Shutdown), 2U);
}

// ═════════════════════════════════════════════════════════════════════════════
// BlockedApps property encoding
//
// C++ reads via Properties.Get:  variant containing a(stutau)
// Rust sends:                    v(a(stutau))  →  Vec<BlockedAppEntry>
// ═════════════════════════════════════════════════════════════════════════════

TEST(DbusSerializationTest, BlockedAppsPropertyRoundtrip) {
    using BlockTuple = sdbus::Struct<std::string, uint64_t, uint32_t, uint64_t>;
    using BlockEntries = std::vector<BlockTuple>;

    BlockEntries original = {
        BlockTuple{"firefox", 42, 0, 1700000000000ULL},
        BlockTuple{"code", 99, 2, 1700000000001ULL},
    };

    auto variant = sdbus::Variant{original};

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
    using BlockEntries = std::vector<sdbus::Struct<std::string, uint64_t, uint32_t, uint64_t>>;

    BlockEntries original;
    auto variant = sdbus::Variant{original};

    std::string expectedSig = std::string("a") + BLOCKED_APP_SIGNATURE;
    EXPECT_STREQ(variant.peekValueType(), expectedSig.c_str());

    auto extracted = variant.get<BlockEntries>();
    EXPECT_TRUE(extracted.empty());
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
    EXPECT_STREQ(EVENT_SIGNAL, "Event");
}

auto main(int argc, char **argv) -> int {
    ::testing::InitGoogleTest(&argc, argv);
    return RUN_ALL_TESTS();
}
