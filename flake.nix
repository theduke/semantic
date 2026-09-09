{
  description = "semantic development environment";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    flake-parts = {
      url = "github:hercules-ci/flake-parts";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    inputs@{
      flake-parts,
      rust-overlay,
      ...
    }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];

      perSystem =
        {
          pkgs,
          lib,
          system,
          ...
        }:
        let
          overlays = [ rust-overlay.overlays.default ];
          pkgs = import inputs.nixpkgs {
            inherit system overlays;
          };

          rustToolchain = pkgs.rust-bin.stable.latest.default.override {
            extensions = [
              "rust-src"
              "rust-analyzer"
              "clippy"
              "rustfmt"
            ];
            targets = [ "wasm32-unknown-unknown" ];
          };
          rustPlatform = pkgs.makeRustPlatform {
            cargo = rustToolchain;
            rustc = rustToolchain;
          };
          version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;
          source = lib.cleanSourceWith {
            src = ./.;
            filter =
              path: type:
              let
                base = baseNameOf path;
              in
              !lib.elem base [
                ".git"
                "docs"
                "node_modules"
                "target"
              ];
          };
          cargoDeps = rustPlatform.importCargoLock {
            lockFile = ./Cargo.lock;
            outputHashes = {
              "dioxus-attributes-0.1.0" = "sha256-RLV2EbE0/AoDNLWrt0BMqbnPkSb8jR1PHL/YbFjHFJM=";
              "logfs-0.2.0" = "sha256-PFtsk6KJupNudjhhmKjlH4GFi9bH+QuFZLN2dlDQyYk=";
            };
          };

          baseDeps = {
            # Base dependencies needed for normal workspace development.
            packages =
              (with pkgs; [
                bashInteractive
                rustToolchain
                pkg-config
                openssl
                openssl.dev
                cargo-nextest
                just
              ])
              ++ lib.optionals pkgs.stdenv.isLinux [ pkgs.fuse3 ];

            # Native libraries needed by non-UI crates that link common system deps.
            buildInputs =
              (with pkgs; [
                openssl
                openssl.dev
              ])
              ++ lib.optionals pkgs.stdenv.isLinux [ pkgs.fuse3 ];

            nativeBuildInputs = [ ];
          };

          uiDeps = {
            # UI-only CLI/tools for Dioxus and wasm builds.
            packages = with pkgs; [
              dioxus-cli
              wasm-bindgen-cli
              binaryen
              nodejs_22
            ];

            # UI-only native libraries for Dioxus desktop/webview builds.
            buildInputs =
              lib.optionals pkgs.stdenv.isLinux (
                with pkgs;
                [
                  glib
                  gtk3
                  libsoup_3
                  webkitgtk_4_1
                  xdotool
                  libx11
                  libxcursor
                  libxrandr
                  libxi
                  libxcb
                  libxkbcommon
                  wayland
                  gsettings-desktop-schemas
                  libGL
                  vulkan-loader
                  gst_all_1.gstreamer
                  gst_all_1.gst-plugins-base
                  gst_all_1.gst-plugins-good
                  gst_all_1.gst-plugins-bad
                ]
              )
              ++ lib.optionals pkgs.stdenv.isDarwin (
                with pkgs;
                [
                  apple-sdk_15
                  libiconv
                ]
              );

            nativeBuildInputs = with pkgs; [
              rustPlatform.bindgenHook
              python3
            ];
          };

          commonDeps = {
            packages = baseDeps.packages ++ uiDeps.packages;
            buildInputs = baseDeps.buildInputs ++ uiDeps.buildInputs;
            nativeBuildInputs = baseDeps.nativeBuildInputs ++ uiDeps.nativeBuildInputs;
          };

          semanticPackage = rustPlatform.buildRustPackage {
            pname = "semantic";
            inherit version cargoDeps;
            src = source;

            nativeBuildInputs = with pkgs; [
              binaryen
              dioxus-cli
              pkg-config
              wasm-bindgen-cli
            ];
            buildInputs = baseDeps.buildInputs;

            buildPhase = ''
              runHook preBuild
              export CARGO_TARGET_DIR="$PWD/target"
              dx build --web --release --package semantic_ui \
                --no-default-features --features web --debug-symbols=false --locked
              cargo build --release --package semantic_cli --bin semantic \
                --features embed-ui --locked
              runHook postBuild
            '';

            installPhase = ''
              runHook preInstall
              install -Dm755 target/release/semantic $out/bin/semantic
              runHook postInstall
            '';

            doCheck = false;
            dontCargoInstall = true;
            meta.mainProgram = "semantic";
          };

          uiLibraryPath = lib.makeLibraryPath (
            baseDeps.buildInputs
            ++ uiDeps.buildInputs
            ++ lib.optionals pkgs.stdenv.isLinux (
              with pkgs;
              [
                fontconfig
                freetype
              ]
            )
          );

          uiXdgDataDirs = lib.optionalString pkgs.stdenv.isLinux (
            lib.concatStringsSep ":" [
              "${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}"
              "${pkgs.gtk3}/share/gsettings-schemas/${pkgs.gtk3.name}"
            ]
          );

          baseShell = pkgs.mkShell {
            packages = baseDeps.packages;
            buildInputs = baseDeps.buildInputs;
            nativeBuildInputs = baseDeps.nativeBuildInputs;

            RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
            OPENSSL_DIR = "${pkgs.openssl.dev}";
            OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";

            shellHook = ''
              echo "semantic base dev shell"
              echo "Rust: $(rustc --version)"
            '';
          };

          commonShell = pkgs.mkShell {
            packages = commonDeps.packages;
            buildInputs = commonDeps.buildInputs;
            nativeBuildInputs = commonDeps.nativeBuildInputs;

            RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
            OPENSSL_DIR = "${pkgs.openssl.dev}";
            OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";
            LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
            CC_wasm32_unknown_unknown = "${pkgs.llvmPackages_18.clang}/bin/clang";
            AR_wasm32_unknown_unknown = "${pkgs.llvmPackages_18.bintools}/bin/llvm-ar";

            LD_LIBRARY_PATH = lib.optionalString pkgs.stdenv.isLinux uiLibraryPath;
            GDK_BACKEND = lib.optionalString pkgs.stdenv.isLinux "x11";
            WEBKIT_DISABLE_COMPOSITING_MODE = lib.optionalString pkgs.stdenv.isLinux "1";
            WEBKIT_ENABLE_WEBGPU = lib.optionalString pkgs.stdenv.isLinux "0";
            GTK_USE_PORTAL = lib.optionalString pkgs.stdenv.isLinux "0";
            XDG_DATA_DIRS = uiXdgDataDirs;

            shellHook = ''
              echo "semantic dev shell"
              echo "Rust: $(rustc --version)"
              echo "dx:   $(dx --version 2>/dev/null || true)"
              echo "Includes base and UI dependencies."
            '';
          };
        in
        {
          _module.args.pkgs = pkgs;

          formatter = pkgs.nixfmt-rfc-style;

          packages = {
            semantic = semanticPackage;
            default = semanticPackage;
          };

          apps.default = {
            type = "app";
            program = "${semanticPackage}/bin/semantic";
          };

          devShells.base = baseShell;
          devShells.default = commonShell;
          devShells.ui = commonShell;
        };
    };
}
