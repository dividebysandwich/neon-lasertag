#!/bin/sh
# Rebuilds every model from its Blender Python build script, headless:
#   tools/models/build_all.sh [kit blaster grenade avatars reload]   (default: all)
# Each script writes assets-src/blend/<name>.blend and exports assets/models/<name>.glb;
# tools/models/check_glb.py then verifies the exports against the game's conventions.
set -e
cd "$(dirname "$0")/../.."
BLENDER=${BLENDER:-blender}
for f in ${@:-kit blaster grenade avatars reload}; do
    log=$("$BLENDER" -b --factory-startup --python-exit-code 1 --python "tools/models/$f.py" 2>&1) || {
        echo "$log"
        echo "$f: FAILED"
        exit 1
    }
    echo "$log" | grep -E "^(piece|blaster|grenade|avatar|reload_station) " || true
    echo "$f: exported assets/models/$f.glb"
done
python3 tools/models/check_glb.py
