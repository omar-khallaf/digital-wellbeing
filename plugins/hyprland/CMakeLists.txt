cmake_minimum_required(VERSION 3.10...4.1)
project(wellbeing-plugin-superbuild NONE)

list(APPEND CMAKE_MODULE_PATH "${CMAKE_CURRENT_SOURCE_DIR}/cmake")

include(ExternalProject)
include(ClangdHelpers)
include(TargetEnv)

# --- Hyprland headers --------------------------------------------------------
# Fetches Hyprland source, generates protocol headers, copies source headers
# into staging, and writes a minimal hyprland.pc (no Requires — the plugin
# resolves transitive symbols at runtime inside Hyprland's process).
include(Hyprland)

# --- PLUGIN (Hyprland .so, loaded into the compositor) ---
# The plugin is built as a shared object, not a runnable executable. The
# superbuild just wraps the `app/` project so it inherits the same cross-compile
# / staging / clangd tooling as the template.
externalproject_add(wellbeing-lockdown
    SOURCE_DIR        ${CMAKE_CURRENT_SOURCE_DIR}/app
    CMAKE_ARGS        ${COMMON_CMAKE_ARGS}
    BUILD_ALWAYS      TRUE
    DEPENDS           hyprland-headers
)
