#!/bin/bash
# Builds the OpenXR loader (libopenxr_loader.so) that the Linux packages ship beside the game,
# for systems without one (SteamOS, the Steam Frame). Khronos publishes no Linux build, so it is
# built from the OpenXR-SDK release (pregenerated sources), as stealth-mission-89's CI does.
# It is named libopenxr_loader.so, the name the openxr crate opens; the game's RUNPATH
# ($ORIGIN, build.rs) makes the dynamic loader look beside the executable first.
#
#   tools/openxr_loader.sh [--arch x86_64|aarch64] [--out DIR]
#
# --arch    defaults to this machine's. aarch64 on an x86-64 PC cross-compiles with
#           aarch64-linux-gnu-gcc against the sysroot of tools/frame.sh (target/sysroot-arm64,
#           LT_SYSROOT overrides it), with libstdc++ linked statically.
# --out     defaults to target/openxr-ARCH. It gets libopenxr_loader.so and openxr-LICENSE.txt.
# OPENXR_VERSION picks the OpenXR-SDK release (default below). Needs cmake, curl and a C++
# compiler; the configure step wants the Xlib headers (libx11-dev).

set -euo pipefail
cd "$(dirname "$0")/.."
VERSION=${OPENXR_VERSION:-1.1.63}
arch=$(uname -m) out=""
while [ $# -gt 0 ]; do
    case $1 in
        --arch) arch=$2; shift ;;
        --out) out=$2; shift ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done
out=${out:-target/openxr-$arch}
work=target/openxr-build/$arch
src=target/openxr-build/OpenXR-SDK-release-$VERSION

if [ ! -d "$src" ]; then
    mkdir -p target/openxr-build
    curl -fsSL --retry 5 --retry-delay 3 --retry-all-errors "https://github.com/KhronosGroup/OpenXR-SDK/archive/refs/tags/release-$VERSION.tar.gz" \
        | tar xz -C target/openxr-build
fi

cmake_args=(-DCMAKE_BUILD_TYPE=Release -DDYNAMIC_LOADER=ON -DBUILD_TESTS=OFF -DBUILD_API_LAYERS=OFF
    -DBUILD_WITH_SYSTEM_JSONCPP=OFF)
strip=strip
if [ "$arch" != "$(uname -m)" ]; then
    [ "$arch" = aarch64 ] || { echo "can only cross-compile for aarch64" >&2; exit 2; }
    sr=${LT_SYSROOT:-$PWD/target/sysroot-arm64}
    if [ ! -d "$sr/usr/lib/aarch64-linux-gnu" ]; then
        echo "making the arm64 sysroot in $sr"
        python3 tools/sysroot_arm64.py "$sr"
    fi
    # CMAKE_SYSROOT only takes effect from a toolchain file.
    libs="-L$sr/usr/lib/aarch64-linux-gnu -L$sr/usr/lib/gcc/aarch64-linux-gnu/13"
    mkdir -p "$(dirname "$work")"
    cat > "$work.toolchain.cmake" <<EOF
set(CMAKE_SYSTEM_NAME Linux)
set(CMAKE_SYSTEM_PROCESSOR aarch64)
set(CMAKE_C_COMPILER aarch64-linux-gnu-gcc)
set(CMAKE_CXX_COMPILER aarch64-linux-gnu-g++)
set(CMAKE_SYSROOT "$sr")
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
# Arch's cross gcc keeps its own headers under --sysroot; the sysroot's (Xlib) come after them.
set(CMAKE_C_FLAGS_INIT "-idirafter $sr/usr/include")
set(CMAKE_CXX_FLAGS_INIT "-idirafter $sr/usr/include")
set(CMAKE_EXE_LINKER_FLAGS_INIT "$libs")
set(CMAKE_SHARED_LINKER_FLAGS_INIT "-static-libstdc++ -static-libgcc $libs")
EOF
    cmake_args+=(-DCMAKE_TOOLCHAIN_FILE="$PWD/$work.toolchain.cmake")
    strip=aarch64-linux-gnu-strip
fi

cmake -S "$src" -B "$work" "${cmake_args[@]}" > "$work.configure.log" 2>&1 \
    || { cat "$work.configure.log"; exit 1; }
cmake --build "$work" --parallel
mkdir -p "$out"
cp -L "$work/src/loader/libopenxr_loader.so.1" "$out/libopenxr_loader.so"
$strip "$out/libopenxr_loader.so"
cp "$src/LICENSE" "$out/openxr-LICENSE.txt"
echo "$out/libopenxr_loader.so"
