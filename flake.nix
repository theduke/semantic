{
  description = "fabric";

  inputs = {
    # nixpkgs.url = github:NixOS/nixpkgs/nixos-unstable;
    flakeutils.url = "github:numtide/flake-utils";
    # naersk.url = "github:nmattia/naersk";
  };

  outputs = { self, nixpkgs, flakeutils /*, naersk */ }: 
    flakeutils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages."${system}";
        # naersk-lib = naersk.lib."${system}";
      in rec {
        devShell = pkgs.stdenv.mkDerivation {
            name = "semantics";
            src = self;
            buildInputs = with pkgs; [
              pkgconfig
              sassc
              wasm-pack
              cargo-watch

              # lossless JPEG optimization with jpegtran
              mozjpeg
              # losless PNG optimization
              oxipng

              # for Typescript plugin runtime.
              deno
            ];
            propagatedBuildInputs = with pkgs; [
              openssl
              gtk3
              glib
              webkitgtk
              gnumake

              glib-networking
            ] ++ (with gst_all_1; [
              gst_all_1.gstreamer
              gst_all_1.gst-plugins-base
              gst_all_1.gst-plugins-good
              gst_all_1.gst-plugins-bad
            ]);
            buildPhase = "";
            installPhase = "";

            # Allow `cargo run` etc to find ssl lib.
            LD_LIBRARY_PATH = "${pkgs.openssl.out}/lib:${pkgs.gtk3}/lib:${pkgs.webkitgtk}/lib:${pkgs.glib.out}/lib:${pkgs.stdenv.cc.cc.lib}/lib64:${pkgs.glib-networking}/lib";
            RUST_BACKTRACE = "1";
            # Use lld linker for speedup.
            RUSTFLAGS = "-C link-arg=-fuse-ld=lld --cfg=web_sys_unstable_apis";
            RUST_LOG = "semantic=trace";

            # Needed for https / ssl support
            GIO_MODULE_DIR = "${pkgs.glib-networking}/lib/gio/modules/";

            # Needed because font rendering in webviewis messed up with wayland 
            # backend.
            GDK_BACKENND = "x11";
        };

      }
    );
}  
