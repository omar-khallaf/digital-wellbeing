{
  description = "Digital Wellbeing — system daemon, GUI, and Hyprland plugin";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/26.05";
    flake-parts.url = "github:hercules-ci/flake-parts";
  };

  outputs =
    inputs@{ flake-parts, nixpkgs, ... }:
    let
      # ── Existing devShell via flake-parts ────────────────────────────────
      devOutputs = flake-parts.lib.mkFlake { inherit inputs; } {
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
            devShells.default = pkgs.mkShell rec {
              name = "digital-wellbeing-plugin";

              nativeBuildInputs = with pkgs; [
                expat
                fontconfig
                freetype
                freetype.dev
                libGL
                pkg-config
                libX11
                libXcursor
                libXi
                libXrandr
                wayland
                libxkbcommon

                # Build system
                cmake
                ninja
                pkg-config
                gcc # Hyprland plugins require GCC, not Clang
                git
                bash

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

              LD_LIBRARY_PATH = builtins.foldl' (
                a: b: "${a}:${b}/lib"
              ) "${pkgs.vulkan-loader}/lib" nativeBuildInputs;
            };
          };
      };

      # ── Custom packaging outputs ─────────────────────────────────────────
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];

      forAllSystems =
        fn:
        builtins.listToAttrs (
          map (s: {
            name = s;
            value = fn s;
          }) systems
        );

      packages = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
          src = ./.;
        in
        rec {
          wellbeing-daemon = pkgs.rustPlatform.buildRustPackage {
            pname = "wellbeing-daemon";
            version = "0.1.0";
            inherit src;

            cargoLock = {
              lockFile = "${src}/Cargo.lock";
              # The workspace has many git deps (gpui, gpui-component, font-kit, ...).
              # For local testing with network access this is fine; for offline
              # reproducible builds, pre-generate outputHashes via `nix-prefetch-cargo`
              # and drop this line.
              allowBuiltinFetchGit = true;
            };
            subPackages = [ "crates/daemon" ];

            nativeBuildInputs = with pkgs; [
              pkg-config
              cmake
            ];

            # libsqlite3-sys uses bundled=true; no system sqlite needed at runtime,
            # but cmake/pkg-config are needed to build the bundled amalgamation.
            buildInputs = with pkgs; [ ];

            meta = with pkgs.lib; {
              description = "Digital Wellbeing system daemon";
              license = licenses.mit;
              platforms = platforms.linux;
            };
          };

          wellbeing-gui = pkgs.rustPlatform.buildRustPackage {
            pname = "wellbeing-gui";
            version = "0.1.0";
            inherit src;

            cargoLock = {
              lockFile = "${src}/Cargo.lock";
              # The workspace has many git deps (gpui, gpui-component, font-kit, ...).
              # For local testing with network access this is fine; for offline
              # reproducible builds, pre-generate outputHashes via `nix-prefetch-cargo`
              # and drop this line.
              allowBuiltinFetchGit = true;
            };
            subPackages = [ "crates/gui" ];

            nativeBuildInputs = with pkgs; [
              pkg-config
              cmake
            ];

            # gpui / gpui_platform / gpui-component depend on X11 / Wayland / Font libs.
            # These are git dependencies from zed / longbridge, so this build needs
            # network access or a vendored Cargo registry.
            buildInputs = with pkgs; [
              wayland
              libxkbcommon
              libGL
              fontconfig
              freetype
              cairo
              pango
              libinput
              lua5_5
            ];

            meta = with pkgs.lib; {
              description = "Digital Wellbeing desktop GUI";
              license = licenses.mit;
              platforms = platforms.linux;
            };
          };

          wellbeing-hyprland-plugin = pkgs.stdenv.mkDerivation {
            pname = "wellbeing-hyprland-plugin";
            version = "0.1.0";
            inherit src;

            sourceRoot = "plugins/hyprland/app";

            nativeBuildInputs = with pkgs; [
              cmake
              ninja
              pkg-config
              gcc
              wayland-scanner
              glslang
            ];

            buildInputs = with pkgs; [
              hyprland
              wayland
              wayland-protocols
              hyprland-protocols
              libglvnd
              mesa
              libgbm
              libdrm
              libxkbcommon
              cairo
              pango
              pixman
              libxcursor
              libinput
              libuuid
              glib
              re2
              muparser
              lcms2
              lua5_5
              systemd
            ];

            meta = with pkgs.lib; {
              description = "Hyprland compositor plugin for Digital Wellbeing";
              license = licenses.mit;
              platforms = platforms.linux;
            };
          };

          default = wellbeing-daemon;
        }
      );

      nixosModules.digital-wellbeing =
        {
          config,
          lib,
          pkgs,
          ...
        }:
        with lib;
        let
          cfg = config.digital-wellbeing;
        in
        {
          options = {
            digital-wellbeing = {
              enable = mkEnableOption "Digital Wellbeing daemon";

              package = mkOption {
                type = types.package;
                default = packages.${pkgs.system}.wellbeing-daemon;
                description = "Daemon package to install.";
              };

              dbusPolicyDir = mkOption {
                type = types.path;
                default = ./deploy/dbus;
                description = "Directory containing org.wellbeing.v1.*.conf files.";
              };

              systemdService = mkOption {
                type = types.path;
                default = ./deploy/systemd/digital-wellbeing-daemon.service;
                description = "systemd unit file for the daemon.";
              };
            };
          };

          config = mkIf cfg.enable {
            environment.etc."dbus-1/system.d/org.wellbeing.v1.Controller.conf".source =
              "${cfg.dbusPolicyDir}/org.wellbeing.v1.Controller.conf";
            environment.etc."dbus-1/system.d/org.wellbeing.v1.Manager.conf".source =
              "${cfg.dbusPolicyDir}/org.wellbeing.v1.Manager.conf";

            systemd.packages = [ cfg.package ];

            environment.systemPackages = [ cfg.package ];
          };
        };
    in
    devOutputs
    // {
      inherit packages nixosModules;
    };
}
