// =============================================================================
// LockManager unit tests — pure state logic only
// (no OpenGL, no D-Bus, no compositor dependencies)
// =============================================================================

#include "lockdown.hpp"
#include <gtest/gtest.h>

using wellbeing::ActionType;
using wellbeing::AppId;
using wellbeing::BlockReason;

// ── Fixture ─────────────────────────────────────────────────────────────────

class LockManagerTest : public ::testing::Test {
  protected:
    void SetUp() override { lm = LockManager(); }

    LockManager lm;
    const AppId kAppId = AppId::from_unchecked("firefox");
    const AppId kOther = AppId::from_unchecked("other-app");
    const uint64_t kPolicy = 42;
    const BlockReason kReason = BlockReason::AppTimeLimit;
    const uint64_t kBlockedSince = 1700000000000ULL;
    const std::vector<ActionType> kActions = {ActionType::Extra, ActionType::Close};
    const std::vector<uint8_t> kSig = {0xde, 0xad, 0xbe, 0xef};
};

// ── Tests ───────────────────────────────────────────────────────────────────

TEST_F(LockManagerTest, InitiallyUnlocked) { EXPECT_FALSE(lm.isOverlayShown(kAppId)); }

TEST_F(LockManagerTest, ShowOverlayThenIsLocked) {
    lm.showOverlay(kAppId, kPolicy, kReason, kBlockedSince, kActions, kSig);
    EXPECT_TRUE(lm.isOverlayShown(kAppId));
    EXPECT_FALSE(lm.isOverlayShown(kOther));
}

TEST_F(LockManagerTest, ShowOverlayStoresToken) {
    lm.showOverlay(kAppId, kPolicy, kReason, kBlockedSince, kActions, kSig);
    EXPECT_TRUE(lm.isOverlayShown(kAppId));
    EXPECT_EQ(lm.activePolicyId(kAppId), kPolicy);
    EXPECT_EQ(lm.blockedSince(kAppId), kBlockedSince);
    EXPECT_EQ(lm.activeSignature(kAppId), kSig);
}

TEST_F(LockManagerTest, HideOverlayClearsState) {
    lm.showOverlay(kAppId, kPolicy, kReason, kBlockedSince, kActions, kSig);
    EXPECT_EQ(lm.hideOverlay(kAppId), LockManagerError::None);
    EXPECT_FALSE(lm.isOverlayShown(kAppId));
}

TEST_F(LockManagerTest, HideOverlayWrongAppIdNoEffect) {
    lm.showOverlay(kAppId, kPolicy, kReason, kBlockedSince, kActions, kSig);
    EXPECT_EQ(lm.hideOverlay(kOther), LockManagerError::AppIdMismatch);
    EXPECT_TRUE(lm.isOverlayShown(kAppId));
    EXPECT_EQ(lm.activePolicyId(kAppId), kPolicy);
}

TEST_F(LockManagerTest, IsTargetReturnsFalseByDefault) {
    // Without captured compositor window handles, isTarget returns false.
    lm.showOverlay(kAppId, kPolicy, kReason, kBlockedSince, kActions, kSig);
    EXPECT_FALSE(lm.isTarget(0));
    EXPECT_FALSE(lm.isTarget(12345));
}

TEST_F(LockManagerTest, ShowHideShowRoundtrip) {
    lm.showOverlay(kAppId, kPolicy, kReason, kBlockedSince, kActions, kSig);
    lm.hideOverlay(kAppId);

    const AppId appId2 = AppId::from_unchecked("code");
    const uint64_t policy2 = 99;
    const std::vector<uint8_t> sig2 = {0x01, 0x02, 0x03};
    lm.showOverlay(appId2, policy2, kReason, kBlockedSince, {ActionType::Close}, sig2);

    EXPECT_TRUE(lm.isOverlayShown(appId2));
    EXPECT_FALSE(lm.isOverlayShown(kAppId));
    EXPECT_EQ(lm.activePolicyId(appId2), policy2);
    EXPECT_EQ(lm.activeSignature(appId2), sig2);
}

TEST_F(LockManagerTest, MultipleAppsSimultaneously) {
    // Show overlay for two distinct apps.
    lm.showOverlay(kAppId, kPolicy, kReason, kBlockedSince, kActions, kSig);
    const AppId appId2 = AppId::from_unchecked("code");
    const uint64_t policy2 = 99;
    const std::vector<uint8_t> sig2 = {0x01, 0x02, 0x03};
    lm.showOverlay(appId2, policy2, kReason, kBlockedSince, {ActionType::Close}, sig2);

    EXPECT_TRUE(lm.isOverlayShown(kAppId));
    EXPECT_TRUE(lm.isOverlayShown(appId2));
    EXPECT_EQ(lm.activePolicyId(kAppId), kPolicy);
    EXPECT_EQ(lm.activePolicyId(appId2), policy2);

    // Hide one app; the other remains.
    lm.hideOverlay(kAppId);
    EXPECT_FALSE(lm.isOverlayShown(kAppId));
    EXPECT_TRUE(lm.isOverlayShown(appId2));
    EXPECT_EQ(lm.activePolicyId(appId2), policy2);
}

TEST_F(LockManagerTest, OverlayActionsListStored) {
    lm.showOverlay(kAppId, kPolicy, kReason, kBlockedSince, kActions, kSig);
    // The buttons built from actions should be available for hit-testing.
    // This tests that no crash occurs.
    lm.onMouseClick(0.0, 0.0); // no crash on empty callback (no focused app set)
}

TEST_F(LockManagerTest, CallbackInvokedOnButtonHit) {
    bool called = false;
    AppId calledAppId = AppId::from_unchecked("");
    ActionType calledAction = ActionType::Extra;

    lm.setUserActionCallback([&](const AppId &appId, ActionType action) -> void {
        called = true;
        calledAppId = appId;
        calledAction = action;
    });

    lm.showOverlay(kAppId, kPolicy, kReason, kBlockedSince, kActions, kSig);
    lm.setFocusedApp(kAppId); // gate: only focused app receives clicks

    // Hit the first button (Extra, actionId=0) at its center.
    // ButtonRect{200, 350, 140, 40, 0}
    EXPECT_TRUE(lm.onMouseClick(270.0, 370.0));
    EXPECT_TRUE(called);
    EXPECT_EQ(calledAppId, kAppId);
    EXPECT_EQ(calledAction, ActionType::Extra);
}

TEST_F(LockManagerTest, ClickOutsideButtonReturnsFalse) {
    bool called = false;
    lm.setUserActionCallback([&](const AppId &, ActionType) -> void { called = true; });

    lm.showOverlay(kAppId, kPolicy, kReason, kBlockedSince, kActions, kSig);
    lm.setFocusedApp(kAppId);

    // Click far outside all button rects.
    EXPECT_FALSE(lm.onMouseClick(0.0, 0.0));
    EXPECT_FALSE(called);
}

TEST_F(LockManagerTest, ClickWithoutFocusedAppReturnsFalse) {
    // Even with an active overlay, onMouseClick returns false when
    // no focused app is set (m_focusedApp gate).
    lm.showOverlay(kAppId, kPolicy, kReason, kBlockedSince, kActions, kSig);
    // Deliberately do NOT set focused app.
    EXPECT_FALSE(lm.onMouseClick(270.0, 370.0));
}

// =============================================================================
// Entry point
// =============================================================================

auto main(int argc, char **argv) -> int {
    ::testing::InitGoogleTest(&argc, argv);
    return RUN_ALL_TESTS();
}
