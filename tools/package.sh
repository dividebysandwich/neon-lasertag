#!/bin/bash
# Packs release builds into the archives CI publishes (the AppImages: tools/appimage.sh).
#
#   tools/package.sh game TARGET [--version NAME] [--loader DIR] [--out DIR]
#   tools/package.sh servers TARGET [--version NAME] [--out DIR]
#
# game      lasertag-VERSION-TARGET.tar.gz (Linux) or .zip (macOS, Windows): the game, the
#           assets folder beside it (where the game looks for it) and README.md. --loader adds
#           the OpenXR loader (a folder from tools/openxr_loader.sh) beside the game on Linux:
#           the game's RUNPATH is $ORIGIN (build.rs), so it is found before the system's.
# servers   lt-server-VERSION-TARGET.tar.gz (the dedicated server) and
#           lt-master-VERSION-TARGET.tar.gz (the server list, with its deploy files).
#
# The builds are read from target/TARGET/release ($CARGO_TARGET_DIR if set), or
# target/release when that has none. --version defaults to `git describe`; --out to dist.
# Zips are made with 7z on Windows (Git Bash) and zip elsewhere.

set -euo pipefail
cd "$(dirname "$0")/.."
what=${1:?game or servers}
target=${2:?target triple}
shift 2
version="" loader="" out=dist
while [ $# -gt 0 ]; do
    case $1 in
        --version) version=$2; shift ;;
        --loader) loader=$2; shift ;;
        --out) out=$2; shift ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done
version=${version:-$(git describe --tags --always --dirty 2> /dev/null || echo dev)}
version=${version//\//-}
tdir=${CARGO_TARGET_DIR:-target}
bin=$tdir/$target/release
[ -d "$bin" ] || bin=$tdir/release
ext=""
case $target in *windows*) ext=.exe ;; esac
mkdir -p "$out"
out=$(cd "$out" && pwd)
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT

strip_() {
    case $target in
        *windows*) ;;
        *apple*) strip -x "$1" ;;
        *) strip "$1" ;;
    esac
}

archive() { # archive NAME: packs $stage/NAME into $out
    case $target in
        *linux*) tar -czf "$out/$1.tar.gz" -C "$stage" "$1"; echo "$out/$1.tar.gz" ;;
        *windows*) (cd "$stage" && 7z a -tzip "$out/$1.zip" "$1" > /dev/null); echo "$out/$1.zip" ;;
        *) (cd "$stage" && zip -qry "$out/$1.zip" "$1"); echo "$out/$1.zip" ;;
    esac
}

case $what in
    game)
        name=lasertag-$version-$target
        d=$stage/$name
        mkdir -p "$d"
        cp "$bin/lasertag$ext" "$d/"
        strip_ "$d/lasertag$ext"
        cp -r assets "$d/"
        cp README.md "$d/"
        if [ -n "$loader" ]; then
            cp -L "$loader/libopenxr_loader.so" "$d/"
            cp "$loader/openxr-LICENSE.txt" "$d/"
        fi
        archive "$name"
        ;;
    servers)
        name=lt-server-$version-$target
        mkdir -p "$stage/$name"
        cp "$bin/lt-server$ext" "$stage/$name/"
        strip_ "$stage/$name/lt-server$ext"
        archive "$name"
        name=lt-master-$version-$target
        mkdir -p "$stage/$name"
        cp "$bin/lt-master$ext" "$stage/$name/"
        strip_ "$stage/$name/lt-master$ext"
        cp -r crates/lt-master/deploy "$stage/$name/"
        archive "$name"
        ;;
    *) echo "unknown: $what (game or servers)" >&2; exit 2 ;;
esac
