#!/usr/bin/env python3
"""Precomputes the spectrum of the game's music for the spectrum displays (src/music.rs).

    python3 tools/gen_spectrum.py [assets/music/menu01.ogg ...]   (needs numpy and ffmpeg)

Without arguments every assets/music/*.ogg is analysed. Each X.ogg gets an X.spectrum beside it.

File format (little endian):

    offset  size  what
    0       4     magic b"LTSP"
    4       2     u16 version (1)
    6       2     u16 channels per frame C (7: the 6 bands, then the bass pulse)
    8       4     f32 frames per second
    12      4     u32 number of frames N
    16      4     f32 duration of the music (s)
    20      N*C   u8 values, frame by frame (frame i at time i / fps): value / 255 is 0..1

Bands (log spaced): 20-120, 120-350, 350-1000, 1000-2800, 2800-7000, 7000-16000 Hz. Each is the
band's power in dB, mapped to 0..1 between a floor and a ceiling taken from the band's own
level over the whole track and over the last few seconds (an auto gain, so every band moves
however loud it is), then lightly gamma'd for a lively display.

The bass pulse is an onset envelope of the lowest band: the rise of its level from frame to
frame (half-wave rectified), normalised by its recent loudest onsets, with an instant attack
and a ~180 ms decay. Meant for subtle pulsing in time with the beat.
"""

import glob
import os
import struct
import subprocess
import sys

import numpy as np

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
RATE = 44100
FPS = 50.0
WINDOW = 4096
BANDS = [(20, 120), (120, 350), (350, 1000), (1000, 2800), (2800, 7000), (7000, 16000)]
# Seconds of the rolling auto gain window.
LOCAL = 6.0


def decode(path):
    raw = subprocess.run(
        ["ffmpeg", "-v", "error", "-i", path, "-ac", "1", "-ar", str(RATE), "-f", "f32le", "-"],
        check=True,
        capture_output=True,
    ).stdout
    return np.frombuffer(raw, dtype=np.float32).astype(np.float64)


def band_db(x):
    hop = RATE / FPS
    n = int(len(x) / hop)
    pad = np.concatenate([np.zeros(WINDOW // 2), x, np.zeros(WINDOW)])
    win = np.hanning(WINDOW)
    freqs = np.fft.rfftfreq(WINDOW, 1.0 / RATE)
    masks = [(freqs >= lo) & (freqs < hi) for lo, hi in BANDS]
    out = np.zeros((n, len(BANDS)))
    for i in range(n):
        s = int(i * hop)
        spec = np.abs(np.fft.rfft(pad[s : s + WINDOW] * win)) ** 2
        for b, m in enumerate(masks):
            out[i, b] = spec[m].sum()
    return 10.0 * np.log10(out + 1e-10)


def rolling(v, size, q):
    """The q-th percentile of v over the last `size` frames, computed every 10 frames."""
    out = np.empty_like(v)
    step = 10
    keys = []
    for i in range(0, len(v) + step, step):
        lo = max(0, i - size)
        hi = max(lo + 1, min(len(v), i + 1))
        keys.append(np.percentile(v[lo:hi], q))
    keys = np.array(keys)
    xs = np.arange(len(keys)) * step
    out[:] = np.interp(np.arange(len(v)), xs, keys)
    return out


def normalise(db):
    size = int(LOCAL * FPS)
    out = np.zeros_like(db)
    for b in range(db.shape[1]):
        v = db[:, b]
        gfloor, gceil = np.percentile(v, 8), np.percentile(v, 99.5)
        lfloor, lceil = rolling(v, size, 10), rolling(v, size, 99)
        floor = 0.5 * gfloor + 0.5 * lfloor
        ceil = np.maximum(0.5 * gceil + 0.5 * lceil, floor + 15.0)
        out[:, b] = np.clip((v - floor) / (ceil - floor), 0.0, 1.0) ** 0.9
    return out


def bass_pulse(db0):
    flux = np.maximum(np.diff(db0, prepend=db0[0]), 0.0)
    # Light smoothing over two frames so a kick spread across frames counts once.
    flux = np.convolve(flux, [0.5, 0.5], mode="same")
    ref = np.maximum(rolling(flux, int(4 * FPS), 97), 1e-3)
    onset = np.clip(flux / ref, 0.0, 1.0)
    onset = np.where(onset > 0.25, onset, 0.0)
    decay = np.exp(-1.0 / (0.18 * FPS))
    env = np.zeros_like(onset)
    e = 0.0
    for i, o in enumerate(onset):
        e = max(o, e * decay)
        env[i] = e
    return env


def process(path):
    x = decode(path)
    duration = len(x) / RATE
    db = band_db(x)
    bands = normalise(db)
    bass = bass_pulse(db[:, 0])
    data = np.concatenate([bands, bass[:, None]], axis=1)
    q = np.round(np.clip(data, 0.0, 1.0) * 255.0).astype(np.uint8)
    out = os.path.splitext(path)[0] + ".spectrum"
    with open(out, "wb") as f:
        f.write(b"LTSP")
        f.write(struct.pack("<HHfIf", 1, data.shape[1], FPS, data.shape[0], duration))
        f.write(q.tobytes())
    means = ", ".join(f"{m:.2f}" for m in data.mean(axis=0))
    print(f"{os.path.relpath(out, ROOT)}: {data.shape[0]} frames at {FPS:g}/s, {duration:.1f} s; means {means}")


def main():
    paths = sys.argv[1:] or sorted(glob.glob(os.path.join(ROOT, "assets", "music", "*.ogg")))
    for p in paths:
        process(p)


if __name__ == "__main__":
    main()
