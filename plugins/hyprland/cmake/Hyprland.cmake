# Fetches Hyprland source and installs only the headers + a minimal hyprland.pc.
#
# The plugin .so is loaded into Hyprland's process at runtime, so ALL
# transitive libs (libdrm, wayland-server, xkbcommon, aquamarine, etc.) are
# already resolved by the compositor.  We only need the Hyprland header files
# at build time — no link-time dependency on the transitives.
#
# Build‑time tools (cmake, pkg‑config, wayland‑scanner, hyprwayland‑scanner,
# wayland‑protocols, hyprland‑protocols, Python3, OpenGL/EGL, glslang,
# aquamarine, hyprlang, …) are expected from the host environment / flake.nix.
# The .pc files for those are found via the standard pkg‑config path.
#
# ── Usage ────────────────────────────────────────────────────────────────────
#   include(Hyprland)
#   ExternalProject_Add_StepDependencies(... configure hyprland-headers)
# or add DEPENDS hyprland-headers to the consuming ExternalProject.

include(ExternalProject)

set(HYPRLAND_VERSION "v0.55.2" CACHE STRING
    "Hyprland git tag / branch to fetch headers from")

# ── Generate hyprland.pc at configure time ────────────────────────────────────
# Written into the build tree, then copied to staging during install.
# No Requires — the plugin .so is loaded into Hyprland's process and resolves
# all transitive symbols (libdrm, wayland-server, xkbcommon, aquamarine, …) at
# runtime inside the compositor.
set(_HYPRLAND_PC ${CMAKE_BINARY_DIR}/hyprland-deps/hyprland.pc)

file(WRITE ${_HYPRLAND_PC}
    "prefix=${CMAKE_STAGING_PREFIX}/include\n")
file(APPEND ${_HYPRLAND_PC}
    "includedir=\${prefix}\n")
file(APPEND ${_HYPRLAND_PC}
    "\n")
file(APPEND ${_HYPRLAND_PC}
    "Name: hyprland\n")
file(APPEND ${_HYPRLAND_PC}
    "Description: Hyprland headers (fetched by superbuild)\n")
file(APPEND ${_HYPRLAND_PC}
    "URL: https://github.com/hyprwm/Hyprland\n")
file(APPEND ${_HYPRLAND_PC}
    "Version: ${HYPRLAND_VERSION}\n")
file(APPEND ${_HYPRLAND_PC}
    "Cflags: -I\${includedir} -I\${includedir}/hyprland -I\${includedir}/hyprland/protocols\n")

# ── ExternalProject: hyprland-headers ────────────────────────────────────────
# Configures Hyprland's cmake (NO_XWAYLAND to drop xcb deps), builds only the
# generate-protocol-headers target, then copies headers + hyprland.pc to staging.
externalproject_add(hyprland-headers
    GIT_REPOSITORY  https://github.com/hyprwm/Hyprland.git
    GIT_TAG         ${HYPRLAND_VERSION}
    GIT_SHALLOW     TRUE
    GIT_PROGRESS    TRUE
    PREFIX          ${CMAKE_BINARY_DIR}/hyprland-deps

    CMAKE_ARGS
        -DCMAKE_INSTALL_PREFIX:PATH=${CMAKE_STAGING_PREFIX}
        -DCMAKE_C_COMPILER:STRING=${CMAKE_C_COMPILER}
        -DCMAKE_CXX_COMPILER:STRING=${CMAKE_CXX_COMPILER}
        -DCMAKE_BUILD_TYPE:STRING=Release
        -DNO_XWAYLAND:BOOL=ON
        -DNO_SYSTEMD:BOOL=ON
        -DNO_HYPRPM:BOOL=ON
        -DNO_UWSM:BOOL=ON
        -DBUILD_TESTING:BOOL=OFF
        -DWITH_TESTS:BOOL=OFF
        -DBUILT_WITH_NIX:BOOL=ON
        -DCMAKE_DISABLE_PRECOMPILE_HEADERS:BOOL=ON

    # Only generate protocol headers — no need to compile Hyprland itself.
    BUILD_COMMAND
        cmake --build <BINARY_DIR> --target generate-protocol-headers

    # ── Install ──────────────────────────────────────────────────────────────
    INSTALL_COMMAND
        # Create output directories
        COMMAND ${CMAKE_COMMAND} -E make_directory
            ${CMAKE_STAGING_PREFIX}/include/hyprland/protocols
        COMMAND ${CMAKE_COMMAND} -E make_directory
            ${CMAKE_STAGING_PREFIX}/lib/pkgconfig

        # Copy source headers. Flatten src/ into include/hyprland/ so the
        # plugin includes them as <hyprland/plugins/...>, <hyprland/render/...>,
        # <hyprland/event/...> etc. (no extra src/ component in the path).
        COMMAND ${CMAKE_COMMAND} -E copy_directory
            <SOURCE_DIR>/src/
            ${CMAKE_STAGING_PREFIX}/include/hyprland/

        # Copy generated protocol headers
        COMMAND ${CMAKE_COMMAND} -E copy_directory
            <SOURCE_DIR>/protocols/
            ${CMAKE_STAGING_PREFIX}/include/hyprland/protocols/

        # Copy pre-generated hyprland.pc
        COMMAND ${CMAKE_COMMAND} -E copy
            ${_HYPRLAND_PC}
            ${CMAKE_STAGING_PREFIX}/lib/pkgconfig/hyprland.pc

    LOG_CONFIGURE  ON
    LOG_BUILD      ON
    LOG_INSTALL    ON
)

# ── Stage transitive Hyprland dependency .pc files ────────────────────────────
# Stages the transitive Hyprland dependency .pc files into CMAKE_STAGING_PREFIX
# so the (isolated) plugin subproject can resolve them through pkg-config WITHOUT
# ever reading the host PKG_CONFIG_PATH.
#
# This preserves the superbuild's hermetic / cross-compilation intent: subprojects
# are built with PKG_CONFIG_USE_CMAKE_PREFIX_PATH=ON and CMAKE_PREFIX_PATH=staging,
# so pkg-config is constrained to staging/lib/pkgconfig only. The .pc files are
# generated here at SUPERBUILD configure time (where the full host pkg-config path
# from flake.nix is still available). Each is written FLAT with only `Cflags` and NO
# `Requires`, and the Cflags already contain the FULL transitive include closure
# (pkg-config resolves it against the host path now). The isolated plugin
# subproject therefore gets every include dir it needs with no host PKG_CONFIG_PATH
# leakage and no Requires closure to chase.
#
# The plugin adds only these INCLUDE dirs — it never links these libraries, since
# every symbol is resolved at runtime inside Hyprland's process. For a
# cross-compile target you would stage the target-built deps instead.
function(stage_host_hyprland_deps)
    find_package(PkgConfig REQUIRED)

    # pkg-config module names for the Hyprland dependency headers the plugin needs
    # when it includes <hyprland/render/OpenGL.hpp> (and friends).
    set(_deps
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

    set(_staged_pc_dir "${CMAKE_STAGING_PREFIX}/lib/pkgconfig")
    file(MAKE_DIRECTORY "${_staged_pc_dir}")

    foreach(_d ${_deps})
        execute_process(COMMAND ${PKG_CONFIG_EXECUTABLE} --exists ${_d}
                        RESULT_VARIABLE _rc)
        if(_rc EQUAL 0)
            # Resolve the FULL transitive include closure now (host PKG_CONFIG_PATH
            # is available at superbuild configure time), bake it into a flat .pc.
            execute_process(COMMAND ${PKG_CONFIG_EXECUTABLE} --cflags-only-I ${_d}
                            OUTPUT_VARIABLE _cflags OUTPUT_STRIP_TRAILING_WHITESPACE)
            execute_process(COMMAND ${PKG_CONFIG_EXECUTABLE} --modversion ${_d}
                            OUTPUT_VARIABLE _ver OUTPUT_STRIP_TRAILING_WHITESPACE)
            if(_ver STREQUAL "")
                set(_ver "0")
            endif()
            set(_pc "${_staged_pc_dir}/${_d}.pc")
            file(WRITE  ${_pc} "Name: ${_d}\n")
            file(APPEND ${_pc} "Description: Staged Hyprland dependency headers (superbuild)\n")
            file(APPEND ${_pc} "Version: ${_ver}\n")
            file(APPEND ${_pc} "Cflags: ${_cflags}\n")
            message(STATUS "HostDeps: staged ${_d}.pc (flat, include closure baked in)")
        else()
            message(STATUS "HostDeps: skip (not found): ${_d}")
        endif()
    endforeach()
endfunction()

stage_host_hyprland_deps()
