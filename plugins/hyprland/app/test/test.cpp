// =============================================================================
// LockManager unit tests — pure state logic only
// (no OpenGL, no D-Bus, no compositor dependencies)
// =============================================================================

#include "lockdown.hpp"
#include <gtest/gtest.h>

using wellbeing::ActionType;
using wellbeing::AppClass;
using wellbeing::BlockReason;

// ── Fixture ─────────────────────────────────────────────────────────────────

class LockManagerTest : public ::testing::Test {
  protected:
    void SetUp() override { lm = LockManager(); }

    LockManager lm;
    const AppClass kAppClass = AppClass::from_unchecked("firefox");
    const AppClass kOther = AppClass::from_unchecked("other-app");
    const uint64_t kPolicy = 42;
    const BlockReason kReason = BlockReason::AppTimeLimit;
    const uint64_t kBlockedSince = 1700000000000ULL;
    const std::vector<ActionType> kActions = {ActionType::Close};
};

// ── Tests ───────────────────────────────────────────────────────────────────

TEST_F(LockManagerTest, InitiallyUnlocked) { EXPECT_FALSE(lm.isOverlayShown(kAppClass)); }

TEST_F(LockManagerTest, ShowOverlayThenIsLocked) {
    lm.showOverlay(kAppClass, kPolicy, kReason, kBlockedSince, kActions);
    EXPECT_TRUE(lm.isOverlayShown(kAppClass));
    EXPECT_FALSE(lm.isOverlayShown(kOther));
}

TEST_F(LockManagerTest, HideOverlayClearsState) {
    lm.showOverlay(kAppClass, kPolicy, kReason, kBlockedSince, kActions);
    EXPECT_EQ(lm.hideOverlay(kAppClass), LockManagerError::None);
    EXPECT_FALSE(lm.isOverlayShown(kAppClass));
}

TEST_F(LockManagerTest, HideOverlayWrongAppClassNoEffect) {
    lm.showOverlay(kAppClass, kPolicy, kReason, kBlockedSince, kActions);
    EXPECT_EQ(lm.hideOverlay(kOther), LockManagerError::AppClassMismatch);
    EXPECT_TRUE(lm.isOverlayShown(kAppClass));
}

TEST_F(LockManagerTest, IsTargetReturnsFalseByDefault) {
    // Without captured compositor window handles, isTarget returns false.
    lm.showOverlay(kAppClass, kPolicy, kReason, kBlockedSince, kActions);
    EXPECT_FALSE(lm.isTarget(0));
    EXPECT_FALSE(lm.isTarget(12345));
}

TEST_F(LockManagerTest, ShowHideShowRoundtrip) {
    lm.showOverlay(kAppClass, kPolicy, kReason, kBlockedSince, kActions);
    lm.hideOverlay(kAppClass);

    const AppClass appClass2 = AppClass::from_unchecked("code");
    const uint64_t policy2 = 99;
    lm.showOverlay(appClass2, policy2, kReason, kBlockedSince, {ActionType::Close});

    EXPECT_TRUE(lm.isOverlayShown(appClass2));
    EXPECT_FALSE(lm.isOverlayShown(kAppClass));
}

TEST_F(LockManagerTest, MultipleAppsSimultaneously) {
    lm.showOverlay(kAppClass, kPolicy, kReason, kBlockedSince, kActions);
    const AppClass appClass2 = AppClass::from_unchecked("code");
    const uint64_t policy2 = 99;
    lm.showOverlay(appClass2, policy2, kReason, kBlockedSince, {ActionType::Close});

    EXPECT_TRUE(lm.isOverlayShown(kAppClass));
    EXPECT_TRUE(lm.isOverlayShown(appClass2));

    // Hide one app; the other remains.
    lm.hideOverlay(kAppClass);
    EXPECT_FALSE(lm.isOverlayShown(kAppClass));
    EXPECT_TRUE(lm.isOverlayShown(appClass2));
}

TEST_F(LockManagerTest, OverlayActionsListStored) {
    lm.showOverlay(kAppClass, kPolicy, kReason, kBlockedSince, kActions);
    // The buttons built from actions should be available for hit-testing.
    lm.onMouseClick(0.0, 0.0); // no crash on empty callback (no focused app set)
}

TEST_F(LockManagerTest, ClickWithoutFocusedAppReturnsFalse) {
    lm.showOverlay(kAppClass, kPolicy, kReason, kBlockedSince, kActions);
    EXPECT_FALSE(lm.onMouseClick(270.0, 370.0));
}

TEST_F(LockManagerTest, GetFocusedAppInitiallyNone) { EXPECT_FALSE(lm.getFocusedApp().has_value()); }

TEST_F(LockManagerTest, SetFocusedAppThenGetReturnsIt) {
    lm.setFocusedApp(std::optional<AppClass>(kAppClass));
    EXPECT_TRUE(lm.getFocusedApp().has_value());
    EXPECT_EQ(*lm.getFocusedApp(), kAppClass);
}

TEST_F(LockManagerTest, SetFocusedAppNoneClears) {
    lm.setFocusedApp(std::optional<AppClass>(kAppClass));
    lm.setFocusedApp(std::nullopt);
    EXPECT_FALSE(lm.getFocusedApp().has_value());
}

// =============================================================================
// Entry point
// =============================================================================

auto main(int argc, char **argv) -> int {
    ::testing::InitGoogleTest(&argc, argv);
    return RUN_ALL_TESTS();
}
