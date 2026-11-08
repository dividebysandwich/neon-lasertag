#!/bin/bash
# Derives sounds from others (assets/sounds/*.ogg, made by gen_sfx.py) with ffmpeg: pitched,
# filtered and with other sounds mixed in.
#
# The special weapons' fire sounds, from each team's blaster sound (`_red`: Red's):
# - ricochet_fire: lower, with a mirror ping's metallic ring trailing it.
# - dual_fire: higher, lighter and shorter.
# - charge_tap: narrower, with a wavering slice of the charge gun's hum after it.
#
# - dash: a jump pad's launch, lower and shorter, under a rush of air.
#
#     tools/gen_derived_sfx.sh
set -euo pipefail
cd "$(dirname "$0")/../assets/sounds"

# out base filter_graph [extra input]
make() {
    local out=$1 base=$2 graph=$3 extra=${4:-}
    local inputs=(-i "$base.ogg")
    [ -n "$extra" ] && inputs+=(-i "$extra.ogg")
    ffmpeg -v error -y "${inputs[@]}" -filter_complex "$graph" -map '[out]' \
        -ac 1 -ar 44100 -c:a libvorbis -q:a 5 "$out.ogg"
}

# (Every input to 44.1 kHz mono first: Red's blaster is 48 kHz stereo.)
norm='aresample=44100,aformat=channel_layouts=mono'

for team in "" _red; do
    base=laser_fire$team

    make ricochet_fire$team "$base" "
        [0:a]$norm,asetrate=44100*0.85,aresample=44100[b];
        [1:a]$norm,highpass=f=1500,volume=0.8,adelay=40[p];
        [b][p]amix=inputs=2:normalize=0,alimiter=limit=0.9[out]" mirror_ping

    make dual_fire$team "$base" "
        [0:a]$norm,asetrate=44100*1.2,aresample=44100,highpass=f=400,
            atrim=0:0.45,afade=t=out:st=0.22:d=0.23,volume=1.2[out]"

    make charge_tap$team "$base" "
        [0:a]$norm,highpass=f=300,lowpass=f=3500,volume=1.6[b];
        [1:a]$norm,atrim=0:0.4,afade=t=in:d=0.04,afade=t=out:st=0.15:d=0.25,
            vibrato=f=9:d=0.35,volume=0.45,adelay=40[h];
        [b][h]amix=inputs=2:normalize=0,alimiter=limit=0.9[out]" charge_loop
done

# (The rush: pink noise swelling and dying away, band-passed.)
ffmpeg -v error -y -i jump_pad.ogg -f lavfi -i "anoisesrc=color=pink:duration=0.4:seed=7" \
    -filter_complex "
        [0:a]$norm,asetrate=44100*0.75,aresample=44100,highpass=f=200,
            atrim=0:0.45,afade=t=out:st=0.2:d=0.25,volume=0.8[b];
        [1:a]$norm,bandpass=f=1400:width_type=o:w=1.5,
            afade=t=in:d=0.08,afade=t=out:st=0.12:d=0.28,volume=0.9[n];
        [b][n]amix=inputs=2:normalize=0,alimiter=limit=0.9[out]" \
    -map '[out]' -ac 1 -ar 44100 -c:a libvorbis -q:a 5 dash.ogg
