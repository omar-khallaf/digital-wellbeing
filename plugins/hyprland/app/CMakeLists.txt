cmake_minimum_required(VERSION 3.27)
project(wellbeing-hyprland-plugin LANGUAGES CXX)

list(APPEND CMAKE_MODULE_PATH "${CMAKE_CURRENT_SOURCE_DIR}/../cmake")

# ── GCC guard ────────────────────────────────────────────────────────────────
# Hyprland loads the plugin into compositor address space. The hook
# function-pointer casts used by Event::bus() listeners are rejected by Clang's
# strict type-punning rules. GCC is the only supported compiler.
if(NOT CMAKE_CXX_COMPILER_ID STREQUAL "GNU")
    message(FATAL_ERROR "Hyprland plugins must be built with GCC (g++), not ${CMAKE_CXX_COMPILER_ID}.")
endif()

# ── Standards ────────────────────────────────────────────────────────────────
set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)

# ── IPO ──────────────────────────────────────────────────────────────────────
include(CheckIPOSupported)
check_ipo_supported(RESULT IPO_SUPPORTED OUTPUT error)
if(IPO_SUPPORTED)
    set(CMAKE_INTERPROCEDURAL_OPTIMIZATION TRUE)
endif()

# ── Sources ──────────────────────────────────────────────────────────────────
file(GLOB_RECURSE PLUGIN_SOURCES CONFIGURE_DEPENDS
    "${CMAKE_CURRENT_SOURCE_DIR}/src/*.cpp")

add_library(wellbeing-hyprland-plugin SHARED ${PLUGIN_SOURCES})

# Keep CMake's default `lib` prefix intentionally so the artifact is
# `libwellbeing-hyprland-plugin.so`, matching the nixpkgs `hyprlandPlugins`
# convention (`$out/lib/lib<name>.so`) which home-manager's
# `wayland.windowManager.hyprland.plugins` list auto-derives. Hyprland loads
# by absolute path so the prefix is irrelevant to the compositor.

target_link_options(wellbeing-hyprland-plugin PRIVATE $<$<CONFIG:Release>:-s>)

target_include_directories(wellbeing-hyprland-plugin PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/include)

# ── System dependencies (pkg-config) ─────────────────────────────────────────
find_package(PkgConfig REQUIRED)

pkg_check_modules(deps REQUIRED IMPORTED_TARGET hyprland)

pkg_check_modules(hl_headers QUIET
    libdrm
    hyprgraphics
    hyprcursor
    hyprutils
    hyprlang
    wayland-server
    xkbcommon
    aquamarine
    cairo
    libinput
    pixman-1
    egl
)

if(hl_headers_FOUND)
    target_include_directories(wellbeing-hyprland-plugin PRIVATE ${hl_headers_INCLUDE_DIRS})
    message(STATUS "wellbeing-hyprland-plugin: transitive Hyprland header include dirs: ${hl_headers_INCLUDE_DIRS}")
endif()

pkg_check_modules(libsystemd REQUIRED IMPORTED_TARGET libsystemd)

target_link_libraries(wellbeing-hyprland-plugin PRIVATE PkgConfig::deps PkgConfig::libsystemd rt)

# ── LockFreeSpscQueue: vendored header-only ─────────────────────────────────
add_library(spsc_queue INTERFACE)
target_include_directories(spsc_queue INTERFACE
    ${CMAKE_CURRENT_SOURCE_DIR}/third_party/include)
target_link_libraries(wellbeing-hyprland-plugin PRIVATE spsc_queue)

# ── Warning flags ────────────────────────────────────────────────────────────
# Flags from the project template, excluding any that break Hyprland headers.
target_compile_options(
    wellbeing-hyprland-plugin
    PRIVATE -Wall
            -Wextra
            -Wundef
            -Wshadow
            -Wpedantic
            -Wconversion
            -Wcast-align
            -Wwrite-strings
            -Wpointer-arith
            -Wstrict-aliasing
            -Werror=format-security
            -Wno-missing-field-initializers
)

# ── Tests ────────────────────────────────────────────────────────────────────
option(BUILD_TESTING "Build plugin tests" ON)
if(BUILD_TESTING)
    enable_testing()
    add_subdirectory(test)
endif()

# ── LSP / clangd ─────────────────────────────────────────────────────────────
include(ClangdHelpers)
generate_clangd_config("${CMAKE_CURRENT_SOURCE_DIR}")

# Expose the real compile DB (compile_commands.json) at the source root so
# clangd uses the actual -std=... / -I flags.
link_compile_commands(wellbeing-hyprland-plugin "${CMAKE_BINARY_DIR}" "${CMAKE_CURRENT_SOURCE_DIR}")

# Force CMake re-configure on next build so clangd sees source changes.
add_custom_target(force_reconfigure_clangd ALL
    COMMAND ${CMAKE_COMMAND} -E touch "${CMAKE_CURRENT_LIST_FILE}"
    COMMENT "Triggering CMake re-configure for clangd update..."
    VERBATIM
)
