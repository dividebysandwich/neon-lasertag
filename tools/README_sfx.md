# Sound effects

`tools/gen_sfx.py` synthesizes all sound effects in `assets/sounds/` (mono, 44.1 kHz Ogg
Vorbis) with numpy and encodes them with ffmpeg (needs libvorbis).

    python3 tools/gen_sfx.py                    # regenerate everything
    python3 tools/gen_sfx.py laser_fire deny    # only these

Each sound has a fixed random seed, so regenerating a subset gives the same audio as a full
run. Loudness is set per sound as an RMS level in the `SOUNDS` table at the bottom of the
script (fire/hits around -16 dBFS, footsteps -27 dBFS); peaks are softly limited below full
scale. Tweak the generator functions or levels there and rerun.
