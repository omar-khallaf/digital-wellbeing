// =============================================================================
// LockManager unit tests — pure state logic only
// (no OpenGL, no D-Bus, no compositor dependencies)
// =============================================================================

#include "lockdown.hpp"
#include "messages.hpp"
#include "types.hpp"
#include <gtest/gtest.h>

using wellbeing::BlockCmd;
using wellbeing::BlockReason;
using wellbeing::UnblockCmd;
using wellbeing::SyncAllCmd;
using wellbeing::WindowClass;

// ── Fixture ─────────────────────────────────────────────────────────────────

class LockManagerTest : public ::testing::Test {
  protected:
    void SetUp() override {
        lm = std::make_unique<wellbeing::LockManager>();
    }

    std::unique_ptr<wellbeing::LockManager> lm;
    const WindowClass kFirefox = WindowClass::from_unchecked("firefox");
    const WindowClass kCode    = WindowClass::from_unchecked("code");
    const BlockReason kReason  = BlockReason::AppTimeLimit;
};

// ── Tests ───────────────────────────────────────────────────────────────────

TEST_F(LockManagerTest, InitiallyNothingBlocked) {
    EXPECT_FALSE(lm->isBlocked(kFirefox.value()));
    EXPECT_FALSE(lm->isBlocked(kCode.value()));
    EXPECT_TRUE(lm->allBlocked().empty());
}

TEST_F(LockManagerTest, BlockCmdThenIsBlocked) {
    lm->apply(BlockCmd{kFirefox.value(), kReason});
    EXPECT_TRUE(lm->isBlocked(kFirefox.value()));
    EXPECT_FALSE(lm->isBlocked(kCode.value()));
}

TEST_F(LockManagerTest, UnblockCmdClearsState) {
    lm->apply(BlockCmd{kFirefox.value(), kReason});
    ASSERT_TRUE(lm->isBlocked(kFirefox.value()));

    lm->apply(UnblockCmd{kFirefox.value()});
    EXPECT_FALSE(lm->isBlocked(kFirefox.value()));
}

TEST_F(LockManagerTest, UnblockCmdOnNonBlockedIsNoop) {
    lm->apply(UnblockCmd{kFirefox.value()}); // not blocked — no crash
    EXPECT_FALSE(lm->isBlocked(kFirefox.value()));
}

TEST_F(LockManagerTest, SyncAllCmdReplacesState) {
    lm->apply(BlockCmd{kFirefox.value(), kReason});
    ASSERT_TRUE(lm->isBlocked(kFirefox.value()));

    SyncAllCmd sync;
    sync.entries.push_back({kCode.value(), kReason});
    lm->apply(sync);

    EXPECT_FALSE(lm->isBlocked(kFirefox.value()));
    EXPECT_TRUE(lm->isBlocked(kCode.value()));
}

TEST_F(LockManagerTest, SyncAllCmdEmptyClearsAll) {
    lm->apply(BlockCmd{kFirefox.value(), kReason});
    lm->apply(BlockCmd{kCode.value(), kReason});

    SyncAllCmd empty{};
    lm->apply(empty);

    EXPECT_TRUE(lm->allBlocked().empty());
}

TEST_F(LockManagerTest, MultipleAppsSimultaneously) {
    lm->apply(BlockCmd{kFirefox.value(), kReason});
    lm->apply(BlockCmd{kCode.value(), BlockReason::CategoryBlock});

    EXPECT_TRUE(lm->isBlocked(kFirefox.value()));
    EXPECT_TRUE(lm->isBlocked(kCode.value()));

    lm->apply(UnblockCmd{kFirefox.value()});
    EXPECT_FALSE(lm->isBlocked(kFirefox.value()));
    EXPECT_TRUE(lm->isBlocked(kCode.value()));
}

TEST_F(LockManagerTest, BlockReasonReturnsCorrectReason) {
    lm->apply(BlockCmd{kFirefox.value(), BlockReason::AppTimeLimit});

    auto *reason = lm->blockReason(kFirefox.value());
    ASSERT_NE(reason, nullptr);
    EXPECT_EQ(*reason, BlockReason::AppTimeLimit);
}

TEST_F(LockManagerTest, BlockReasonNonBlockedReturnsNull) {
    EXPECT_EQ(lm->blockReason(kFirefox.value()), nullptr);
}

TEST_F(LockManagerTest, RepeatedBlockCmdUpdatesState) {
    lm->apply(BlockCmd{kFirefox.value(), BlockReason::AppTimeLimit});
    auto *r1 = lm->blockReason(kFirefox.value());
    ASSERT_NE(r1, nullptr);
    EXPECT_EQ(*r1, BlockReason::AppTimeLimit);

    // Re-issue block with different reason
    lm->apply(BlockCmd{kFirefox.value(), BlockReason::CategoryBlock});
    auto *r2 = lm->blockReason(kFirefox.value());
    ASSERT_NE(r2, nullptr);
    EXPECT_EQ(*r2, BlockReason::CategoryBlock);
}

// =============================================================================
// Entry point
// =============================================================================

auto main(int argc, char **argv) -> int {
    ::testing::InitGoogleTest(&argc, argv);
    return RUN_ALL_TESTS();
}
