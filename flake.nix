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

          baseDeps = {
            # Base dependencies needed for normal workspace development.
            packages =
              (with pkgs; [
                rustToolchain
                pkg-config
                openssl
                openssl.dev
                cargo-nextest
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

          devShells.base = baseShell;
          devShells.default = commonShell;
          devShells.ui = commonShell;
        };
    };
}
