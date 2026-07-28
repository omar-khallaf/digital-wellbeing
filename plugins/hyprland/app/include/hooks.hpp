#pragma once

#include <string>

namespace wellbeing {

void registerHooks();
auto focusedWindowHasIdleInhibitor() -> bool;
/// Wclass of the currently focused window (empty if no window focused).
/// Safe to call from any compositor-thread context (no lock needed).
auto focusedWindowClass() -> std::string;
/// Title of the currently focused window (empty if none).
auto focusedWindowTitle() -> std::string;
} // namespace wellbeing
