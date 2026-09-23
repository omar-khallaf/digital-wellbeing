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
                # Hyprland itself — provides hyprland.pc consumed by
                # app/CMakeLists.txt (pkg_check_modules(deps hyprland))
                hyprland

                # Hyprland's own sub-libraries (.pc) — declared by hyprland.pc.in
                aquamarine
                hyprcursor
                hyprgraphics
                hyprlang
                hyprutils
                hyprwire # wire-protocol lib; required by Hyprland >= 0.55 cmake configure

                # X11/XCB closure required by hyprland.pc Requires
                libxcb-util
                libxcb-wm
                libxcb-keysyms
                libxcb-errors
                libxcb-image
                libxcb-render-util
                libxcb-cursor

                # Protocol data files (pkg_get_variable in Hyprland's cmake)
                wayland-protocols
                hyprland-protocols

                # Graphics / display
                libglvnd # OpenGL | GLES | EGL
                mesa # GL/GLES drivers
                glslang # Hyprland ShaderLoader.hpp includes glslang headers
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
          cargoLock = {
            lockFile = ./Cargo.lock;
            outputHashes = {
              "collections-0.1.0" = "sha256-TQknUZJvQ4/c++NymuKY1Et/x7iDmjtobAGFHDYICRg=";
              "derive_refineable-0.1.0" = "sha256-TQknUZJvQ4/c++NymuKY1Et/x7iDmjtobAGFHDYICRg=";
              "gpui-0.2.2" = "sha256-TQknUZJvQ4/c++NymuKY1Et/x7iDmjtobAGFHDYICRg=";
              "gpui-component-0.5.2" = "sha256-7nLq2YnLzoyMi2kuAT5C2CvarisJsueKe/4595CYNTw=";
              "gpui-component-assets-0.5.1" = "sha256-7nLq2YnLzoyMi2kuAT5C2CvarisJsueKe/4595CYNTw=";
              "gpui-component-macros-0.5.1" = "sha256-7nLq2YnLzoyMi2kuAT5C2CvarisJsueKe/4595CYNTw=";
              "gpui_linux-0.1.0" = "sha256-TQknUZJvQ4/c++NymuKY1Et/x7iDmjtobAGFHDYICRg=";
              "gpui_macos-0.1.0" = "sha256-TQknUZJvQ4/c++NymuKY1Et/x7iDmjtobAGFHDYICRg=";
              "gpui_macros-0.1.0" = "sha256-TQknUZJvQ4/c++NymuKY1Et/x7iDmjtobAGFHDYICRg=";
              "gpui_platform-0.1.0" = "sha256-TQknUZJvQ4/c++NymuKY1Et/x7iDmjtobAGFHDYICRg=";
              "gpui_shared_string-0.1.0" = "sha256-TQknUZJvQ4/c++NymuKY1Et/x7iDmjtobAGFHDYICRg=";
              "gpui_util-0.1.0" = "sha256-TQknUZJvQ4/c++NymuKY1Et/x7iDmjtobAGFHDYICRg=";
              "gpui_web-0.1.0" = "sha256-TQknUZJvQ4/c++NymuKY1Et/x7iDmjtobAGFHDYICRg=";
              "gpui_wgpu-0.1.0" = "sha256-TQknUZJvQ4/c++NymuKY1Et/x7iDmjtobAGFHDYICRg=";
              "gpui_windows-0.1.0" = "sha256-TQknUZJvQ4/c++NymuKY1Et/x7iDmjtobAGFHDYICRg=";
              "http_client-0.1.0" = "sha256-TQknUZJvQ4/c++NymuKY1Et/x7iDmjtobAGFHDYICRg=";
              "media-0.1.0" = "sha256-TQknUZJvQ4/c++NymuKY1Et/x7iDmjtobAGFHDYICRg=";
              "perf-0.1.0" = "sha256-TQknUZJvQ4/c++NymuKY1Et/x7iDmjtobAGFHDYICRg=";
              "refineable-0.1.0" = "sha256-TQknUZJvQ4/c++NymuKY1Et/x7iDmjtobAGFHDYICRg=";
              "scheduler-0.1.0" = "sha256-TQknUZJvQ4/c++NymuKY1Et/x7iDmjtobAGFHDYICRg=";
              "sum_tree-0.1.0" = "sha256-TQknUZJvQ4/c++NymuKY1Et/x7iDmjtobAGFHDYICRg=";
              "util_macros-0.1.0" = "sha256-TQknUZJvQ4/c++NymuKY1Et/x7iDmjtobAGFHDYICRg=";
              "wasm_thread-0.3.3" = "sha256-+lRLCIk0S6Y5ORYjDKsYYHia2FtoSoh+rWkQh7mnPBE=";
              "xim-ctext-0.3.0" = "sha256-pRT4Sz1JU9ros47/7pmIW9kosWOGMOItcnNd+VrvnpE=";
              "xim-parser-0.2.1" = "sha256-pRT4Sz1JU9ros47/7pmIW9kosWOGMOItcnNd+VrvnpE=";
              "zed-font-kit-0.14.1-zed" = "sha256-KXygi0olNQi5yM8eaJVykNDtbPMDjT+cWPBF8UrtXR4=";
              "zed-reqwest-0.12.15-zed" = "sha256-p4SiUrOrbTlk/3bBrzN/mq/t+1Gzy2ot4nso6w6S+F8=";
              "zed-scap-0.0.8-zed" = "sha256-BihiQHlal/eRsktyf0GI3aSWsUCW7WcICMsC2Xvb7kw=";
              "zed-xim-0.4.0-zed" = "sha256-pRT4Sz1JU9ros47/7pmIW9kosWOGMOItcnNd+VrvnpE=";
              "zlog-0.1.0" = "sha256-TQknUZJvQ4/c++NymuKY1Et/x7iDmjtobAGFHDYICRg=";
              "ztracing-0.1.0" = "sha256-TQknUZJvQ4/c++NymuKY1Et/x7iDmjtobAGFHDYICRg=";
              "ztracing_macro-0.1.0" = "sha256-TQknUZJvQ4/c++NymuKY1Et/x7iDmjtobAGFHDYICRg=";
            };
          };
        in
        rec {
          wellbeing-daemon = pkgs.rustPlatform.buildRustPackage {
            pname = "wellbeing-daemon";
            version = "0.1.0";
            src = ./.;
            inherit cargoLock;
            cargoBuildFlags = [
              "-p"
              "wellbeing-daemon"
            ];
            nativeBuildInputs = with pkgs; [ pkg-config ];
            buildInputs = with pkgs; [
              systemd
              wayland
            ];
            doCheck = false;
            postInstall = ''
              mkdir -p $out/lib/systemd/system
              cp deploy/systemd/digital-wellbeing-daemon.service $out/lib/systemd/system/digital-wellbeing-daemon.service
            '';
            meta = with pkgs.lib; {
              description = "Digital Wellbeing system daemon";
              license = licenses.mit;
              platforms = platforms.linux;
            };
          };

          wellbeing-gui = pkgs.rustPlatform.buildRustPackage {
            pname = "wellbeing-gui";
            version = "0.1.0";
            src = ./.;
            inherit cargoLock;
            cargoBuildFlags = [
              "-p"
              "wellbeing-gui"
            ];
            nativeBuildInputs = with pkgs; [
              pkg-config
              makeWrapper
            ];
            buildInputs = with pkgs; [
              wayland
              libxkbcommon
              libGL
              fontconfig
              freetype
              cairo
              pango
              libinput
              vulkan-loader
              libX11
              libXcursor
              libXi
              libXrandr
              expat
            ];
            doCheck = false;
            postInstall = ''
              wrapProgram $out/bin/wellbeing-gui \
                --prefix LD_LIBRARY_PATH : "${
                  pkgs.lib.makeLibraryPath [
                    pkgs.wayland
                    pkgs.libxkbcommon
                    pkgs.libGL
                    pkgs.fontconfig
                    pkgs.freetype
                    pkgs.vulkan-loader
                    pkgs.libX11
                    pkgs.libXcursor
                    pkgs.libXi
                    pkgs.libXrandr
                    pkgs.expat
                  ]
                }"
            '';
            meta = with pkgs.lib; {
              description = "Digital Wellbeing desktop GUI";
              license = licenses.mit;
              platforms = platforms.linux;
            };
          };

          wellbeing-hyprland-plugin =
            let
              preset = if system == "aarch64-linux" then "linux-aarch64" else "linux-amd64";
            in
            pkgs.stdenv.mkDerivation {
              pname = "wellbeing-hyprland-plugin";
              version = "0.1.0";
              src = ./plugins/hyprland;
              nativeBuildInputs = with pkgs; [
                cmake
                ninja
                gcc
                pkg-config
              ];
              buildInputs = with pkgs; [
                hyprland
                aquamarine
                hyprcursor
                hyprgraphics
                hyprlang
                hyprutils
                wayland
                wayland-protocols
                hyprland-protocols
                libdrm
                libxkbcommon
                cairo
                pango
                pixman
                libinput
                mesa
                libglvnd
                glslang
                libxcb-util
                libxcb-wm
                libxcb-keysyms
                libxcb-errors
                libxcb-image
                libxcb-render-util
                libxcb-cursor
                systemd
              ];
              cmakeFlags = [ "-DBUILD_TESTING=OFF" ];
              configurePhase = ''
                cmake --preset ${preset} -DBUILD_TESTING=OFF
              '';
              buildPhase = ''
                cmake --build --preset ${preset}
              '';
              installPhase = ''
                mkdir -p $out/lib/hyprland-plugins
                cp build/${preset}/app/wellbeing-lockdown.so $out/lib/hyprland-plugins/wellbeing-lockdown.so
              '';
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

              dbusSystemService = mkOption {
                type = types.path;
                default = ./deploy/system-services/org.wellbeing.v1.Controller.service;
                description = "D-Bus system-service activation file for the daemon.";
              };
            };
          };

          config = mkIf cfg.enable (
            let
              dbusPackage = pkgs.runCommand "wellbeing-dbus-files" { } ''
                mkdir -p $out/share/dbus-1/system.d $out/share/dbus-1/system-services
                cp "${cfg.dbusPolicyDir}/org.wellbeing.v1.Controller.conf" $out/share/dbus-1/system.d/
                cp "${cfg.dbusPolicyDir}/org.wellbeing.v1.Manager.conf" $out/share/dbus-1/system.d/
                substitute "${cfg.dbusSystemService}" \
                  $out/share/dbus-1/system-services/org.wellbeing.v1.Controller.service \
                  --replace-fail /usr/bin/wellbeing-daemon ${cfg.package}/bin/wellbeing-daemon
              '';
            in
            {
              services.dbus.packages = [ dbusPackage ];

              systemd.services.digital-wellbeing-daemon = {
                description = "Digital Wellbeing Daemon";
                after = [ "dbus.service" ];
                requires = [ "dbus.service" ];
                wantedBy = [ "multi-user.target" ];
                serviceConfig = {
                  Type = "dbus";
                  BusName = "org.wellbeing.v1.Controller";
                  ExecStart = "${cfg.package}/bin/wellbeing-daemon";
                  Restart = "on-failure";
                  RestartSec = 3;
                  User = "root";
                  Group = "root";
                  NoNewPrivileges = true;
                  ProtectSystem = "strict";
                  ReadWritePaths = "/var/lib/digital-wellbeing";
                  PrivateTmp = true;
                  PrivateDevices = true;
                  ProtectHome = true;
                  ProtectKernelTunables = true;
                  ProtectKernelModules = true;
                  ProtectControlGroups = true;
                  StateDirectory = "digital-wellbeing";
                  StateDirectoryMode = "0700";
                };
              };

              environment.systemPackages = [ cfg.package ];
            }
          );
        };
    in
    devOutputs
    // {
      inherit packages nixosModules;
    };
}
