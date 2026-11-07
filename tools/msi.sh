#!/bin/bash
# Packs the Windows release build into an installer: lasertag-VERSION-TARGET.msi.
#
#   tools/msi.sh [--target TRIPLE] [--version NAME] [--out DIR]
#
# Runs on Windows (Git Bash), with the WiX v5 CLI on PATH
# (`dotnet tool install --global wix --version 5.0.2`). The installer is
# packaging/windows/lasertag.wxs; its product version is the crate's (Cargo.toml), as MSI
# versions must be numeric. --version only goes into the file name (default `git describe`).
# --target defaults to x86_64-pc-windows-msvc; the build is read from target/TARGET/release
# ($CARGO_TARGET_DIR if set), or target/release when that has none. --out defaults to dist.

set -euo pipefail
cd "$(dirname "$0")/.."
target=x86_64-pc-windows-msvc version="" out=dist
while [ $# -gt 0 ]; do
    case $1 in
        --target) target=$2; shift ;;
        --version) version=$2; shift ;;
        --out) out=$2; shift ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done
version=${version:-$(git describe --tags --always --dirty 2> /dev/null || echo dev)}
version=${version//\//-}
product_version=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)
[ -n "$product_version" ] || { echo "no workspace version in Cargo.toml" >&2; exit 1; }
tdir=${CARGO_TARGET_DIR:-target}
bin=$tdir/$target/release
[ -d "$bin" ] || bin=$tdir/release
[ -f "$bin/neon-lasertag.exe" ] || { echo "no build: $bin/neon-lasertag.exe" >&2; exit 1; }
case $target in
    x86_64-*) arch=x64 ;;
    aarch64-*) arch=arm64 ;;
    *) echo "unsupported target: $target" >&2; exit 2 ;;
esac
mkdir -p "$out"
msi=$out/lasertag-$version-$target.msi
wix build packaging/windows/lasertag.wxs -arch "$arch" -d "Version=$product_version" \
    -bindpath "bin=$bin" -bindpath assets=assets -bindpath root=. -o "$msi"
rm -f "${msi%.msi}.wixpdb"
echo "$msi"
