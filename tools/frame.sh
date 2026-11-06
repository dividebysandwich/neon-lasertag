#!/bin/bash
# Cross-compiles the aarch64 Linux release build on an x86-64 Linux PC, deploys it to a Steam
# Frame over ssh and launches it in VR there (ported from stealth-mission-89's tools/frame.sh).
#
#   tools/frame.sh [options] [ENV=VALUE ...] [-- game args]
#
# Options:
#   --no-build     deploy and launch the last build
#   --no-launch    build and deploy only
#   --time SECS    quit the game after SECS seconds (default: run until Ctrl-C)
#   --detach       leave the game running and return (stop it with `tools/frame.sh --stop`)
#   --stop         stop a running game and exit
# ENV=VALUE pairs are set for the game (e.g. LT_GL_FOVEATION=high LT_GL_MSAA=2).
# Game args default to `--gl-vr` (foveated, our own GL renderer); e.g. `-- --gl-vr --bots 9`.
#
# Ctrl-C stops the game on the Frame. The log is copied to target/frame-logs/, followed by
# SteamVR's measurement of the run (from vrcompositor.txt).
#
# Needs:
# - ssh access to the Frame as host `frame` (FRAME_HOST overrides it); FRAME_FEATURES the cargo
#   features (default `glvr`: both `--vr` and `--gl-vr`).
# - The aarch64-unknown-linux-gnu Rust target (`rustup target add`), aarch64-linux-gnu-gcc,
#   python3 and bsdtar. The first build makes an Ubuntu 24.04 arm64 sysroot (glibc 2.39 like
#   SteamOS) in target/sysroot-arm64 with tools/sysroot_arm64.py (LT_SYSROOT overrides it).
# - SteamVR running on the Frame. The game goes to ~/lasertag-dev there. Its OpenXR loader is
#   copied from ~/sm89-dev (a stealth-mission-89 dev install) or extracted from a
#   stealth-mission-89 aarch64 AppImage in ~/Downloads on the first deploy.

set -euo pipefail
cd "$(dirname "$0")/.."
HOST=${FRAME_HOST:-frame}
DIR=lasertag-dev
TARGET=aarch64-unknown-linux-gnu
SR=${LT_SYSROOT:-$PWD/target/sysroot-arm64}
BIN=target/$TARGET/release/neon-lasertag

build=1 launch=1 secs="" detach=0
envs=() args=()
while [ $# -gt 0 ]; do
    case $1 in
        --no-build) build=0 ;;
        --no-launch) launch=0 ;;
        --time) secs=$2; shift ;;
        --detach) detach=1 ;;
        --stop) stop=1 ;;
        --) shift; args=("$@"); break ;;
        *=*) envs+=("$1") ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done
[ ${#args[@]} -eq 0 ] && args=(--gl-vr)

remote() { ssh -o BatchMode=yes "$HOST" "$@"; }
stop_game() { remote 'pkill -INT -x neon-lasertag; sleep 2; pkill -KILL -x neon-lasertag' || true; }

if [ "${stop:-0}" = 1 ]; then
    stop_game
    exit 0
fi

if [ $build = 1 ]; then
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
    cargo build --release --target $TARGET -p neon-lasertag --no-default-features --features ${FRAME_FEATURES:-glvr}
    # The Frame gets a stripped copy; the unstripped one stays here for backtraces.
    aarch64-linux-gnu-strip -o "$BIN.stripped" "$BIN"
fi

echo "deploying to $HOST:$DIR"
stop_game
remote "mkdir -p $DIR && cd $DIR && if [ ! -e libopenxr_loader.so ]; then
    if [ -e ~/sm89-dev/libopenxr_loader.so ]; then cp -L ~/sm89-dev/libopenxr_loader.so .; else
        a=\$(ls -t ~/Downloads/stealth-mission-89-*-aarch64.AppImage 2>/dev/null | head -1)
        [ -n \"\$a\" ] || { echo 'no OpenXR loader: put libopenxr_loader.so in ~/$DIR on the Frame' >&2; exit 1; }
        rm -rf squashfs-root && \"\$a\" --appimage-extract >/dev/null && cp -L squashfs-root/usr/bin/libopenxr_loader.so . && rm -rf squashfs-root
    fi
fi"
rsync -az --delete --exclude '*.blend*' assets "$HOST:$DIR/"
rsync -az "$BIN.stripped" "$HOST:$DIR/neon-lasertag"
[ $launch = 1 ] || exit 0

# The game runs in the Frame's gamescope session, detached from this ssh connection.
log=run-$(date +%Y%m%d-%H%M%S).log
limit=${secs:+timeout -s INT -k 10 $secs}
remote "cd $DIR && DISPLAY=:0 XDG_RUNTIME_DIR=/run/user/\$(id -u) DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/\$(id -u)/bus \
    BEVY_ASSET_ROOT=\$PWD ${envs[*]} setsid nohup $limit ./neon-lasertag ${args[*]} > $log 2>&1 < /dev/null &"
echo "launched: ${envs[*]} ${args[*]} (log $DIR/$log)"
if [ $detach = 1 ]; then
    exit 0
fi

finish() {
    trap - INT TERM
    stop_game
    mkdir -p target/frame-logs
    if scp -q "$HOST:$DIR/$log" target/frame-logs/; then
        sed -i 's/\x1b\[[0-9;]*m//g' "target/frame-logs/$log"
        echo "log: target/frame-logs/$log"
    fi
    sleep 2
    remote 'grep -E "Game Info|Total\.\.\." ~/.local/share/Steam/logs/vrcompositor.txt | tail -2 | sed "s/^.*- //"' || true
    exit 0
}
trap finish INT TERM
# Follow the log until the game exits (or Ctrl-C).
remote "cd $DIR && tail -n +1 --pid=\$(pgrep -nx neon-lasertag || echo 1) -f $log" || true
finish
