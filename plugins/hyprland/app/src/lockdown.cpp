// =============================================================================
// LockManager — single-threaded blocked-apps state on the compositor thread.
//
// All methods are called from the compositor thread only.
// No mutexes needed.
// =============================================================================

#include "lockdown.hpp"

using wellbeing::BlockCmd;
using wellbeing::SyncAllCmd;
using wellbeing::UnblockCmd;

// ── Exhaustive command dispatch ─────────────────────────────────────────────

void wellbeing::LockManager::apply(const CompositorCommand &cmd) {
    std::visit(Overloaded{
                   [this](const BlockCmd &c) -> void { m_blocks[c.wclass] = c.reason; },
                   [this](const UnblockCmd &c) -> void { m_blocks.erase(c.wclass); },
                   [this](const SyncAllCmd &c) -> void {
                       m_blocks.clear();
                       for (const auto &e : c.entries) {
                           m_blocks[e.wclass] = e.reason;
                       }
                   },
               },
               cmd);
}
