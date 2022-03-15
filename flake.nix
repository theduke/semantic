{
  description = "fabric";

  inputs = {
    # nixpkgs.url = github:NixOS/nixpkgs/nixos-unstable;
    flakeutils.url = "github:numtide/flake-utils";
    naersk.url = "github:nmattia/naersk";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = { self, nixpkgs, flakeutils, rust-overlay, naersk }:
    flakeutils.lib.eachDefaultSystem (system:
      let
        VERSION = "0.1";

        overlays = [
          rust-overlay.overlay
        ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        # Rust toolchain with wasm32 target.
        rustWasm = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" ];
          targets = [ "wasm32-unknown-unknown" ];
        };

        naersk-lib = naersk.lib."${system}";
        # Naersk with wasm32 target enabled Rust.
        naersk-lib-ui = pkgs.callPackage naersk.lib.${system} {
          rustc = rustWasm;
          cargo = rustWasm;
        };

        xtask = naersk-lib.buildPackage {
          pname = "xtask";
          version = "0.1";
          src = ./crates/xtask;
        };

        uiBuildInputs = with pkgs; [
          wasm-pack
          wasm-bindgen-cli
          sassc
          # provides wasm-opt for optimizations
          binaryen
        ];

        ui = pkgs.rustPlatform.buildRustPackage {
          pname = "semantic-ui";
          version = VERSION;
          src = self;

          CARGO_NET_GIT_FETCH_WITH_CLI = "true";

          cargoLock = { 
            lockFile = ./Cargo.lock; 
            outputHashes = {
              "brass-0.1.0" = "1r33089kgfr48727fcz9cm3l3vi69m1y9dm3svrng5dwzvcnv4hz";
              "factor_macros-0.1.0" = "sha256-cNPAqutOvi0cvGXAoO4Gk4NWnTlQMaNK+Vm4sAa22BU=";
            };
          };
          nativeBuildInputs = [ 
            rustWasm
            xtask
            # which is needed so wasm-pack can find the wasm-opt binary.
            pkgs.which 
          ] ++ uiBuildInputs;
          buildPhase = ''
            xtask build-ui --dev
            mkdir -p $out
            cp -r target/ui/* $out/
          '';
          checkPhase = "echo skipping check...";
          installPhase = "echo skipping install...";
        };

        runtimeDeps = with pkgs; [
          # lossless JPEG optimization with jpegtran
          mozjpeg
          # losless PNG optimization
          oxipng

          # for Typescript plugin runtime.
          deno
        ];


      in
      rec {
        packages.xtask = xtask;
        packages.semantic-ui-wasm = ui;

        packages.semantic = pkgs.rustPlatform.buildRustPackage {
          pname = "semantic";
          version = "0.1";
          src = self;
          targets = [ "crates/semantic" ];

          buildInputs = with pkgs; [
            pkgconfig
          ];

          nativeBuildInputs = [
            xtask
          ];

          CARGO_NET_GIT_FETCH_WITH_CLI = "true";

          cargoLock = { 
            lockFile = ./Cargo.lock; 
            outputHashes = {
              "brass-0.1.0" = "1r33089kgfr48727fcz9cm3l3vi69m1y9dm3svrng5dwzvcnv4hz";
              "factor_macros-0.1.0" = "sha256-cNPAqutOvi0cvGXAoO4Gk4NWnTlQMaNK+Vm4sAa22BU=";
              # "factor_macros-0.1" = "17l033diygcssg50cqrjp2rfmk1rccpz8369sfzjy8p229skpr1s";
              # "factordb.1.0" = "17l033diygcssg50cqrjp2rfmk1rccpz8369sfzjy8p229skpr1s";
            };
          };

          propagatedBuildInputs = with pkgs; [ ];
          runtimeDependencies = with pkgs; runtimeDeps;

          buildPhase = ''
            mkdir -p target/ui
            cp -r ${ui} target/ui
            xtask build-server --dev
            mkdir -p $out/bin
            cp target/release/semantic $out/bin/
          '';
          checkPhase = "echo skipping checks";
          installPhase = "echo skipping checks";
        };

        defaultPackage = packages.semantic;

        # `nix run`
        apps.semantic = flakeutils.lib.mkApp {
          drv = packages.semantic;
        };
        defaultApp = apps.semantic;

        packages.appimage = pkgs.stdenv.mkDerivation {
          pname = "semantic";
          version = VERSION;

          ARCH = "x86_64"; # required by appimagetool

          src = pkgs.buildEnv {
            name = "semantic";
            paths = [
              packages.semantic
              pkgs.oxipng
              pkgs.mozjpeg
              pkgs.appimagekit
            ];
          };

          builder = builtins.toFile "build.sh" ''
            source $stdenv/setup

            mkdir build
            cp -rL "$src/bin/oxipng" build
            cp -rL "$src/bin/jpegtran" build
            cp -rL "$src/bin/semantic" build
            cp ${./lib/appimage/semantic.desktop} build
            cp ${./lib/appimage/appicon.png} build

            mkdir $out
            appimagetool ./build $out/semantic.AppImage
          '';
        };

        devShell = pkgs.stdenv.mkDerivation {
          name = "semantics";
          src = self;
          buildInputs = with pkgs; [
            pkgconfig
            cargo-watch

          ] ++ uiBuildInputs ++ runtimeDeps;
          propagatedBuildInputs = with pkgs; [
            openssl
            gtk3
            glib
            gnumake

            # glib-networking
            # webkitgtk
          ] /* ++ (with gst_all_1; [
            gst_all_1.gstreamer
            gst_all_1.gst-plugins-base
            gst_all_1.gst-plugins-good
            gst_all_1.gst-plugins-bad
          ])*/;
          runtimeDependencies = runtimeDeps;
          buildPhase = "";
          installPhase = "";

          # Allow `cargo run` etc to find ssl lib.
          # LD_LIBRARY_PATH = "${pkgs.openssl.out}/lib:${pkgs.gtk3}/lib:${pkgs.webkitgtk}/lib:${pkgs.glib.out}/lib:${pkgs.stdenv.cc.cc.lib}/lib64:${pkgs.glib-networking}/lib";
          RUST_BACKTRACE = "1";
          # Use lld linker for speedup.
          RUSTFLAGS = "--cfg=web_sys_unstable_apis";
          RUST_LOG = "semantic=trace";
          CARGO_INCREMENTAL = "1";

          # Needed for https / ssl support
          GIO_MODULE_DIR = "${pkgs.glib-networking}/lib/gio/modules/";

          CARGO_NET_GIT_FETCH_WITH_CLI = "true";

          # Needed because font rendering in webviewis messed up with wayland 
          # backend.
          GDK_BACKENND = "x11";
        };

      }
    );
}
