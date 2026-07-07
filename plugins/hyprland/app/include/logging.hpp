#pragma once

#include <print>
#include <string>

namespace wellbeing {

// Plugin-local logging. Stderr until Hyprland logging infra is wired.
// `inline` so the header is includable from multiple TUs without ODR issues.
inline void logErr(const std::string &msg) { std::println(stderr, "[wellbeing-lockdown] ERROR: {}", msg); }

inline void logInfo(const std::string &msg) { std::println(stderr, "[wellbeing-lockdown] INFO: {}", msg); }

} // namespace wellbeing
