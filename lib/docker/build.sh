#!/usr/bin/env bash

set -euxo pipefail

function install_rust_build_deps() {
  echo "Installing Rust build dependencies..."

  apt-get update
  apt-get install -y curl sassc binaryen git

  echo "Installing Rust toolchain..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- --default-toolchain stable -y
  source $HOME/.cargo/env
  rustup target add wasm32-unknown-unknown

  echo "Installing wasm-pack..."
  curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh
}

function install_mozjpeg() {
  echo "Building mozjpeg..."

  # Prevent tzdata dialog
  ln -fs /usr/share/zoneinfo/America/New_York /etc/localtime

  apt-get update -y
  apt-get install -y build-essential nasm cmake ninja-build curl libpng-dev libpng-tools zlib1g-dev pkg-config

  mozjpeg_version=4.0.3

  mkdir /tmp/build
  cd /tmp/build

  curl -fLO "https://github.com/mozilla/mozjpeg/archive/v${mozjpeg_version}.tar.gz"
  tar xf "v${mozjpeg_version}.tar.gz"

  export CFLAGS='-pipe -flto -no-pie'
  export LDFLAGS='-flto -no-pie -static -static-libgcc'

  # This unsets CMAKE_SHARED_LIBRARY_LINK_C_FLAGS inside CMakeLists.txt,
  # which is necessary to build a static binary. It can't be unset from the CLI because it
  # is set as part of the compiler detection phase.
  sed -E -i.bk '/^cmake_minimum_required/a\
  unset(CMAKE_SHARED_LIBRARY_LINK_C_FLAGS)' "mozjpeg-$mozjpeg_version"/CMakeLists.txt

  mkdir build
  cd build

  cmake -G"Ninja" -DENABLE_SHARED=OFF -DCMAKE_INSTALL_PREFIX=/usr/local -DCMAKE_FIND_LIBRARY_SUFFIXES=.a -DCMAKE_LINK_SEARCH_END_STATIC=1 -DCMAKE_LINK_SEARCH_START_STATIC=1 "../mozjpeg-$mozjpeg_version/"

  ninja

  strip jpegtran-static
  mv jpegtran-static /usr/bin/jpegtran
  jpegtran -version
  rm -r /tmp/build
}

function install_runtime_deps() {
  install_mozjpeg

  echo "Installing deno..."
  apt-get install -y unzip
  curl -fsSL https://deno.land/x/install/install.sh | DENO_INSTALL=/usr sh
}

function build_semantic() {
  source $HOME/.cargo/env

  cd /host
  export CARGO_NET_GIT_FETCH_WITH_CLI="true"
  export CARGO_TARGET_DIR=/host/target/docker
  # cargo xtask build-ui
  cargo xtask build-server
}

function build_portable() {
  install_rust_build_deps
  install_runtime_deps

  build_semantic

  test -d target/portable/bin && rm -r target/portable/bin

  mkdir -p target/portable/bin
  cp target/docker/release/semantic target/portable/bin/
  cp /usr/bin/deno target/portable/bin/
  cp /usr/bin/jpegtran target/portable/bin/

  chmod -R 755 target/portable/bin
}

build_portable
