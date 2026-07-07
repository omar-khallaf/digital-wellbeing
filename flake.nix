{
  description = "Digital Wellbeing — Hyprland plugin superbuild";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/26.05";
    flake-parts.url = "github:hercules-ci/flake-parts";
  };

  outputs =
    inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      # Hyprland plugins are Linux-only.
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];

      perSystem =
        {
          config,
          self',
          inputs',
          pkgs,
          system,
          ...
        }:
        {
          # ── Development shell ────────────────────────────────────────────────
          # Enter with:  nix develop
          # Then build:  cmake -S plugins/hyprland -B build && cmake --build build
          devShells.default = pkgs.mkShell {
            name = "digital-wellbeing-plugin";

            nativeBuildInputs = with pkgs; [
              # Build system
              cmake
              ninja
              pkg-config
              gcc # Hyprland plugins require GCC, not Clang
              git

              # Code generation tools (needed by Hyprland's cmake configure)
              python3
              wayland-scanner
              hyprwayland-scanner # find_package(hyprwayland-scanner)
              glslang # find_package(glslang CONFIG)
            ];

            buildInputs = with pkgs; [
              # Hyprland's own sub-libraries (.pc) — declared by hyprland.pc.in
              aquamarine
              hyprcursor
              hyprgraphics
              hyprlang
              hyprutils
              hyprwire # wire-protocol lib; required by Hyprland >= 0.55 cmake configure

              # Protocol data files (pkg_get_variable in Hyprland's cmake)
              wayland-protocols
              hyprland-protocols

              # Graphics / display
              libglvnd # OpenGL | GLES | EGL
              mesa # GL/GLES drivers
              libgbm # gbm (gbm.pc ships in mesa-libgbm, not mesa)
              libdrm

              # Wayland core
              wayland # wayland-server, wayland-client, wayland-cursor
              libxkbcommon

              # Text rendering
              cairo
              pango # includes pangocairo
              pixman
              libxcursor

              # Input
              libinput

              # Utility libraries
              libuuid
              glib # gio-2.0
              re2
              muparser
              lcms2

              # Lua (pkg_search_module)
              lua5_5

              # D-Bus — needed by sdbus-c++ static link
              systemd # libsystemd
            ];

            # Ensure CMake's find_package can locate hyprwayland-scanner's config.
            # mkShell's setup-hooks propagate CMAKE_PREFIX_PATH automatically for
            # most packages; this makes the scanner location explicit if needed.
            # shellHook = ''
            #   export CMAKE_PREFIX_PATH="${pkgs.hyprwayland-scanner}:$CMAKE_PREFIX_PATH"
            # '';
          };
        };
    };
}
