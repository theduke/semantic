#!/bin/bash

set -euxo pipefail

# Prevent tzdata dialog
ln -fs /usr/share/zoneinfo/America/New_York /etc/localtime

apt-get update -y
apt-get install -y build-essential nasm cmake ninja-build curl libpng-dev libpng-tools zlib1g-dev

mozjpeg_version=4.0.3

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
# strip cjpeg-static
# sudo rm -f /usr/bin/mozcjpeg
# sudo cp cjpeg-static /usr/bin/
# sudo mv /usr/bin/cjpeg-static /usr/bin/mozcjpeg
# mozcjpeg -version
