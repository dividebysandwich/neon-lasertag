#!/bin/bash
# Packages the game as an AppImage: one file to download, mark executable and run.
#
#   tools/appimage.sh [--arch aarch64|x86_64] [--binary PATH] [--build] [--loader DIR|FILE]
#                     [--version NAME] [--out DIR]
#
# --arch     defaults to this machine's. aarch64 is the Steam Frame's: its AppImage starts in VR
#            with `--gl-vr`; the x86_64 one starts the flat-screen menu (packaging/linux/AppRun).
# --binary   the release build to package (default target[/aarch64-unknown-linux-gnu]/release/
#            neon-lasertag, under $CARGO_TARGET_DIR if set). It must be built without the `dev`
#            feature (no libbevy_dylib).
# --build    build it first: `cargo build --release --no-default-features --features glvr`,
#            cross-compiled for aarch64 on an x86-64 PC like tools/frame.sh.
# --loader   the OpenXR loader to ship (libopenxr_loader.so, or a folder holding it and
#            openxr-LICENSE.txt). Default target/openxr-ARCH, built by tools/openxr_loader.sh
#            when missing.
# --version  goes into the file name (default `git describe`).
# --out      default dist. The AppImage is OUT/lasertag-VERSION-ARCH.AppImage.
#
# Downloads appimagetool and the AppImage runtime into target/appimage-tools on first use. In
# GitHub Actions (GITHUB_REPOSITORY set) it embeds update information for the latest release,
# and appimagetool writes a .zsync next to the AppImage.

set -euo pipefail
cd "$(dirname "$0")/.."
APPIMAGETOOL_VERSION=${APPIMAGETOOL_VERSION:-1.9.1}
APPIMAGE_RUNTIME_VERSION=${APPIMAGE_RUNTIME_VERSION:-20251108}
host=$(uname -m)
arch=$host binary="" build=0 loader="" version="" out=dist
while [ $# -gt 0 ]; do
    case $1 in
        --arch) arch=$2; shift ;;
        --binary) binary=$2; shift ;;
        --build) build=1 ;;
        --loader) loader=$2; shift ;;
        --version) version=$2; shift ;;
        --out) out=$2; shift ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done
case $arch in
    x86_64) default_args="" ;;
    aarch64) default_args="--gl-vr" ;;
    *) echo "unsupported arch: $arch" >&2; exit 2 ;;
esac
version=${version:-$(git describe --tags --always --dirty 2> /dev/null || echo dev)}
version=${version//\//-}
cross=0
if [ "$arch" != "$host" ]; then cross=1; fi
if [ -z "$binary" ]; then
    binary=${CARGO_TARGET_DIR:-target}/release/neon-lasertag
    if [ $cross = 1 ]; then binary=${CARGO_TARGET_DIR:-target}/$arch-unknown-linux-gnu/release/neon-lasertag; fi
fi

if [ $build = 1 ]; then
    if [ $cross = 1 ]; then
        [ "$arch" = aarch64 ] || { echo "can only cross-compile for aarch64" >&2; exit 2; }
        SR=${LT_SYSROOT:-$PWD/target/sysroot-arm64}
        if [ ! -d "$SR/usr/lib/aarch64-linux-gnu" ]; then
            echo "making the arm64 sysroot in $SR"
            python3 tools/sysroot_arm64.py "$SR"
        fi
        export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
        export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS="-C link-arg=--sysroot=$SR -C link-arg=-L$SR/usr/lib/aarch64-linux-gnu -C link-arg=-L$SR/usr/lib/gcc/aarch64-linux-gnu/13"
        export CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++
        export CFLAGS_aarch64_unknown_linux_gnu="--sysroot=$SR" CXXFLAGS_aarch64_unknown_linux_gnu="--sysroot=$SR"
        export PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_SYSROOT_DIR=$SR
        export PKG_CONFIG_LIBDIR=$SR/usr/lib/aarch64-linux-gnu/pkgconfig:$SR/usr/share/pkgconfig
        cargo build --release --target "$arch-unknown-linux-gnu" -p neon-lasertag --no-default-features --features glvr
    else
        cargo build --release -p neon-lasertag --no-default-features --features glvr
    fi
fi
[ -x "$binary" ] || { echo "no build at $binary (build it, or pass --build or --binary)" >&2; exit 1; }
readelf_=readelf
command -v readelf > /dev/null || readelf_="objdump -p"
if $readelf_ -d "$binary" 2> /dev/null | grep -q libbevy_dylib; then
    echo "$binary is a dev build (bevy/dynamic_linking): build with --no-default-features" >&2
    exit 1
fi
if ! file -L "$binary" | grep -q "$( [ "$arch" = aarch64 ] && echo aarch64 || echo x86-64)"; then
    echo "$binary is not an $arch executable" >&2
    exit 1
fi

# The OpenXR loader.
if [ -z "$loader" ]; then
    loader=target/openxr-$arch
    [ -e "$loader/libopenxr_loader.so" ] || tools/openxr_loader.sh --arch "$arch" --out "$loader"
fi
if [ -d "$loader" ]; then
    loader_license=$loader/openxr-LICENSE.txt
    loader=$loader/libopenxr_loader.so
else
    loader_license=$(dirname "$loader")/openxr-LICENSE.txt
fi
[ -e "$loader" ] || { echo "no OpenXR loader at $loader" >&2; exit 1; }

# The AppDir.
app=target/appimage/$arch/AppDir
rm -rf "$app"
mkdir -p "$app/usr/bin" "$app/usr/share/lasertag" "$app/usr/share/applications" \
    "$app/usr/share/icons/hicolor/256x256/apps" "$app/usr/share/doc/lasertag"
strip=strip
if [ $cross = 1 ]; then strip=aarch64-linux-gnu-strip; fi
$strip -o "$app/usr/bin/neon-lasertag" "$binary"
cp -L "$loader" "$app/usr/bin/libopenxr_loader.so"
if [ -e "$loader_license" ]; then cp "$loader_license" "$app/usr/share/doc/lasertag/"; fi
cp README.md "$app/usr/share/doc/lasertag/"
cp -r assets "$app/usr/share/lasertag/"
sed "s|@DEFAULT_ARGS@|$default_args|" packaging/linux/AppRun > "$app/AppRun"
chmod +x "$app/AppRun"
cp packaging/linux/lasertag.desktop "$app/"
cp packaging/linux/lasertag.desktop "$app/usr/share/applications/"
cp packaging/linux/lasertag.png "$app/"
cp packaging/linux/lasertag.png "$app/usr/share/icons/hicolor/256x256/apps/"
ln -s lasertag.png "$app/.DirIcon"

# appimagetool runs here (host arch); the runtime is the target's.
tools=target/appimage-tools
mkdir -p "$tools"
tool=$tools/appimagetool-$APPIMAGETOOL_VERSION-$host.AppImage
runtime=$tools/runtime-$APPIMAGE_RUNTIME_VERSION-$arch
if [ ! -x "$tool" ]; then
    curl -fsSL --retry 5 --retry-delay 3 --retry-all-errors -o "$tool.part" "https://github.com/AppImage/appimagetool/releases/download/$APPIMAGETOOL_VERSION/appimagetool-$host.AppImage"
    chmod +x "$tool.part" && mv "$tool.part" "$tool"
fi
if [ ! -e "$runtime" ]; then
    curl -fsSL --retry 5 --retry-delay 3 --retry-all-errors -o "$runtime.part" "https://github.com/AppImage/type2-runtime/releases/download/$APPIMAGE_RUNTIME_VERSION/runtime-$arch"
    mv "$runtime.part" "$runtime"
fi
update=()
if [ -n "${GITHUB_REPOSITORY:-}" ]; then
    update=(-u "gh-releases-zsync|${GITHUB_REPOSITORY%/*}|${GITHUB_REPOSITORY#*/}|latest|lasertag-*-$arch.AppImage.zsync")
fi
mkdir -p "$out"
file=$out/lasertag-$version-$arch.AppImage
# No FUSE needed to run appimagetool itself (CI runners have none).
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=$arch "$tool" --no-appstream --runtime-file "$runtime" \
    "${update[@]}" "$app" "$file"
# appimagetool writes the .zsync in the working directory.
for z in lasertag-*-"$arch".AppImage.zsync; do
    if [ -e "$z" ]; then mv "$z" "$out/"; fi
done
ls -l "$file"
