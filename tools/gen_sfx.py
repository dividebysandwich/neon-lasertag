#!/usr/bin/env python3
"""Synthesizes the lasertag game's sound effects (assets/sounds/*.ogg).

- footstep_1..3: a soft rubber sole on a metal grating: a low thud, a tiny metallic click
  and a faint rattle of the grating a moment later (each variant a little different).
- laser_fire: the classic "pew": an FM tone diving from 2.4 kHz to 300 Hz over a low punch,
  a bright transient and a little noise tick, a short slap echo.
- laser_fire_red: Red's blasters: a lower, buzzier zap: a ring-modulated square diving from
  1.7 kHz to 160 Hz with a growl, over a low punch, a short slap echo.
- hit_player: being tagged: a buzzy saw/square zap falling with vibrato over a low
  power-down tone and a deep thump.
- player_hit: someone else hit: an electric crack and a buzzing zap falling over a thud.
- tagged_out: eliminated: a long power-down whine, electric crackle and a low boom.
- hit_target: a team target hit, heard across the arena: a deep boom, a metallic ping, a
  rising three-note bell chime played twice over a two-tone alarm, and a shimmering tail.
- grenade_bounce: the grenade bouncing: a hard clack and a short electronic chirp.
- grenade_throw: a grenade thrown: the latch let go, a whoosh and a rising chirp.
- grenade_arm: the grenade activating and jumping up: a rising charge-up whine ending in a pop.
- grenade_burst: one burst of the hovering grenade: three rapid tiny high zaps.
- respawn: teleporting in: a rising arpeggio over a rising filtered-noise sweep.
- deny: can't fire: a short double buzz.
- empty: out of shots: a dry mechanical double click with a faint dying whine.
- reload: refilling at a station: a rising electric charge hum with a ticking pulse.
- reload_done: the blaster full again: a bright two-note confirmation chirp.
- score_bleep: you hit an enemy or a team target: an arcade "ba-ding", two quick square-wave
  notes up a fifth.
- score_tag: you tagged an enemy out: a fast four-note square-wave arpeggio up to a held top.
- pickup: taking a grenade: a quick sparkly upward blip with a soft metallic latch.
- mega_win: the megatarget hit (Megatarget mode), heard by everyone: a sub-bass impact, a
  rising noise sweep into a big detuned-saw brass fanfare (a triad climbing an octave, then a
  held major chord with a fifth on top), bells over it, and a long shimmering tail.
- power_tick: the ticking of a player who may enter the megatarget's room: a sharp clock tick
  with a tiny high ping.
- power_up: both secondary targets hit: a rising filtered sweep resolving to a bright chord.
- turret_fire: a turret's shot: a hard, metallic, lower zap with a mechanical knock.
- turret_down: a turret switched off: a falling power-down whine with sputtering crackle.
- shield_zap: a player walking into a forcefield: a violent electric crackle and a buzzing
  discharge.
- mirror_ping: a bolt bouncing off a mirror: a bright glassy ping bending up, a tiny
  zap and a short shimmering ring.
- jump_pad: a jump pad throwing someone up: a soft low thump, a rising whoosh of filtered
  noise and a sine chirp sweeping up two octaves.
- gate_hum: a laser gate's buzz while it's on (a seamless one-second loop): a harsh
  electric hum with a fast flutter and a thin high whine.
- shield_hum: a forcefield's hum (a seamless two-second loop): a mains-like drone with
  slowly beating partials and faint crackle.

Each sound is scaled to an RMS loudness (dBFS) suited to the mix, peaks softly limited under
full scale. Mono, 44.1 kHz, Ogg Vorbis.

    python3 tools/gen_sfx.py [name ...]      (needs numpy and ffmpeg with libvorbis)

Without arguments every sound is written, else only those named (e.g. `laser_fire` or
`laser_fire.ogg`). Every sound has its own random seed, so the output is identical either way.
"""

import os
import subprocess
import sys

import numpy as np

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
OUT = os.path.join(ROOT, "assets", "sounds")
RATE = 44100
EXT = ".ogg"

rng = np.random.default_rng(0)


# ---------------------------------------------------------------- helpers

def seconds(n):
    return np.arange(int(n * RATE)) / RATE


def noise(t, lo, hi, order=2.0):
    """Band-limited noise of `t` seconds between `lo` and `hi` Hz, RMS 1."""
    n = int(t * RATE)
    f = np.fft.rfftfreq(n, 1.0 / RATE)
    f[0] = 1e-3
    mag = 1.0 / np.sqrt(1.0 + (lo / f) ** (2 * order)) / np.sqrt(1.0 + (f / hi) ** (2 * order))
    spec = mag * np.exp(2j * np.pi * rng.random(len(f)))
    spec[0] = 0.0
    x = np.fft.irfft(spec, n)
    return x / np.sqrt(np.mean(x * x))


def sweep_noise(t, hi_from, hi_to, lo=20.0):
    """Noise whose upper cutoff glides from `hi_from` to `hi_to` Hz (one-pole filters)."""
    x = rng.standard_normal(int(t * RATE))
    hi = np.geomspace(hi_from, hi_to, len(x))
    a = np.exp(-2 * np.pi * hi / RATE)
    y = np.empty_like(x)
    s1 = s2 = 0.0
    for i, (v, k) in enumerate(zip(x, a)):
        s1 = (1 - k) * v + k * s1
        s2 = (1 - k) * s1 + k * s2
        y[i] = s2
    y -= np.convolve(y, np.ones(int(RATE / lo)) / int(RATE / lo), "same")
    return y / np.sqrt(np.mean(y * y))


def env(t, attack, decay, hold=0.0):
    """Envelope: rises over `attack`, holds `hold` s, then decays with time constant `decay`."""
    x = seconds(t)
    rise = 1.0 - np.exp(-x / max(attack, 1e-5))
    fall = np.where(x < hold, 1.0, np.exp(-(x - hold) / decay))
    return rise * fall


def fades(x, fade_in=0.002, fade_out=0.02):
    n_in, n_out = int(fade_in * RATE), int(fade_out * RATE)
    x = x.copy()
    x[:n_in] *= np.linspace(0, 1, n_in)
    x[len(x) - n_out:] *= np.linspace(1, 0, n_out)
    return x


def at(total, start, x):
    """`x` placed `start` s into a silence of `total` s (cut at the end)."""
    out = np.zeros(int(total * RATE))
    i = int(start * RATE)
    out[i:i + len(x)] += x[:max(0, len(out) - i)]
    return out


def phase(f):
    """Phase (in cycles) of a tone whose frequency follows the array `f` (Hz)."""
    return np.cumsum(f) / RATE


def thump(t, hz, decay, drop=0.5):
    """A low thud: a sine falling in pitch by `drop` (fraction), decaying."""
    x = seconds(t)
    f = hz * (1 - drop * (1 - np.exp(-x / decay)))
    return np.sin(2 * np.pi * phase(f)) * env(t, 0.001, decay)


def click(t, lo, hi, decay):
    return noise(t, lo, hi, 3.0) * env(t, 0.0002, decay)


def ring(t, freqs, decays, gains=None):
    """Damped sine partials (metal ringing)."""
    x = seconds(t)
    gains = gains or [1.0] * len(freqs)
    return sum(g * np.sin(2 * np.pi * f * x + rng.random() * 6.28) * np.exp(-x / d)
               for f, d, g in zip(freqs, decays, gains))


def saw(f):
    p = phase(f)
    return 2.0 * (p - np.floor(p + 0.5))


def square(f, duty=0.5):
    p = phase(f) % 1.0
    return np.where(p < duty, 1.0, -1.0)


def lowpass(x, hi, order=2.0):
    f = np.fft.rfftfreq(len(x), 1.0 / RATE)
    return np.fft.irfft(np.fft.rfft(x) / np.sqrt(1.0 + (f / hi) ** (2 * order)), len(x))


def highpass(x, lo, order=2.0):
    f = np.fft.rfftfreq(len(x), 1.0 / RATE)
    f[0] = 1e-3
    return np.fft.irfft(np.fft.rfft(x) / np.sqrt(1.0 + (lo / f) ** (2 * order)), len(x))


def echoes(x, delays, gains, hi):
    """`x` with its echoes `delays` s later, `gains` as loud, dulled above `hi` Hz."""
    t = len(x) / RATE
    dull = lowpass(x, hi)
    out = x.copy()
    for d, g in zip(delays, gains):
        out += g * at(t, d, dull)
    return out


def norm(x):
    return x / np.max(np.abs(x))


def write(name, x, rms_db, loop=False):
    """Writes `x` at the RMS level `rms_db` dBFS, the peaks softly limited under full scale
    (`loop`: played round and round, so no fades at the ends)."""
    x = x - np.mean(x) if loop else fades(x - np.mean(x))
    target = 10 ** (rms_db / 20)
    limit = lambda g: 0.95 * np.tanh(x * g / 0.95)
    lo, hi = 0.0, 1e3 / np.max(np.abs(x))
    for _ in range(60):
        g = (lo + hi) / 2
        lo, hi = (g, hi) if np.sqrt(np.mean(limit(g) ** 2)) < target else (lo, g)
    x = limit((lo + hi) / 2)
    pcm = np.round(np.clip(x, -1, 1) * 32767).astype("<i2").tobytes()
    path = os.path.join(OUT, name + EXT)
    codec = ["-c:a", "libvorbis", "-q:a", "6"] if EXT == ".ogg" else ["-compression_level", "12"]
    subprocess.run(
        ["ffmpeg", "-y", "-loglevel", "error", "-f", "s16le", "-ar", str(RATE), "-ac", "1", "-i", "-",
         *codec, path],
        input=pcm, check=True)
    peak = 20 * np.log10(np.max(np.abs(x)))
    print(f"{path}  {len(x) / RATE:.3f} s  rms {rms_db:.1f} dBFS  peak {peak:.1f} dBFS")


# ---------------------------------------------------------------- sounds

def footstep(variant):
    T = 0.15
    v = variant - 1
    body = thump(T, 85 + 12 * v, 0.022, drop=0.35)
    scuff = noise(T, 120, 900) * env(T, 0.003, 0.018) * 0.25
    base = 2350 + 310 * v
    ping = ring(T, [base, base * 1.53, base * 2.21, base * 3.08],
                [0.012, 0.009, 0.006, 0.004], [1.0, 0.7, 0.45, 0.25])
    tick = click(T, 2500, 9000, 0.0015) * 0.6
    metal = (ping * 0.12 + tick * 0.15) * env(T, 0.0005, 0.03)
    rattle = at(T, 0.028 + 0.009 * v,
                ring(0.06, [base * 1.17, base * 1.9], [0.006, 0.004]) * 0.05
                + click(0.06, 3000, 8000, 0.001) * 0.05)
    return body + scuff + metal + rattle


def laser_fire():
    T = 0.25
    x = seconds(T)
    f = 300 + (2400 - 300) * np.exp(-x / 0.045)
    index = 1.6 * np.exp(-x / 0.05) + 0.25
    mod = np.sin(2 * np.pi * phase(f * 0.5))
    carrier = np.sin(2 * np.pi * phase(f) + index * mod)
    tone = (carrier + 0.25 * np.sin(4 * np.pi * phase(f))) * env(T, 0.0008, 0.07, hold=0.02)
    transient = click(T, 3000, 10000, 0.003) * 0.35
    tick = noise(T, 800, 5000) * env(T, 0.0005, 0.012) * 0.12
    # The low end: a short punch under the pew.
    punch = thump(T, 95, 0.045, drop=0.45) * 0.9 + lowpass(noise(T, 30, 250), 160) * env(T, 0.001, 0.03) * 0.3
    y = tone + transient + tick + punch
    return echoes(lowpass(y, 9000), [0.045], [0.18], 2500)


def laser_fire_red():
    T = 0.27
    x = seconds(T)
    f = 160 + (1700 - 160) * np.exp(-x / 0.05)
    buzz = square(f, 0.35) * (0.6 + 0.4 * np.sin(2 * np.pi * phase(f * 0.5)))
    growl = saw(f * 0.5) * (0.5 + 0.5 * np.sin(2 * np.pi * 55 * x))
    tone = lowpass(buzz * 0.7 + growl * 0.4, 5000) * env(T, 0.0008, 0.07, hold=0.025)
    transient = click(T, 2000, 8000, 0.003) * 0.3
    punch = thump(T, 85, 0.05, drop=0.45) * 1.0 + lowpass(noise(T, 30, 250), 160) * env(T, 0.001, 0.03) * 0.3
    return echoes(tone + transient + punch, [0.05], [0.18], 2200)


def hit_player():
    T = 0.6
    x = seconds(T)
    vib = 1 + 0.06 * np.sin(2 * np.pi * 19 * x)
    f = (150 + 750 * np.exp(-x / 0.16)) * vib
    zap = 0.6 * saw(f) + 0.4 * square(f * 1.005, 0.4)
    zap = lowpass(zap, 3800) * env(T, 0.002, 0.16, hold=0.06)
    fl = 220 * np.exp(-x / 0.35) + 45
    down = np.sin(2 * np.pi * phase(fl)) * env(T, 0.01, 0.25, hold=0.1) * 0.8
    sizzle = noise(T, 1500, 7000) * env(T, 0.001, 0.05) * 0.15
    body = thump(T, 70, 0.14, drop=0.4) * 1.1
    return zap + down + sizzle + body


def player_hit():
    T = 0.4
    x = seconds(T)
    crack = click(T, 2500, 11000, 0.003) * 0.9 + noise(T, 1500, 8000) * env(T, 0.0005, 0.02) * 0.3
    f = (220 + 1300 * np.exp(-x / 0.05)) * (1 + 0.08 * np.sin(2 * np.pi * 31 * x))
    zap = lowpass(0.6 * saw(f) + 0.4 * square(f * 1.01, 0.3), 4500) * env(T, 0.001, 0.07, hold=0.03)
    thud = thump(T, 120, 0.06, drop=0.5) + thump(T, 65, 0.1, drop=0.35) * 0.9
    return crack + 0.7 * zap + 0.8 * thud


def tagged_out():
    T = 1.2
    x = seconds(T)
    vib = 1 + (0.01 + 0.06 * x / T) * np.sin(2 * np.pi * (9 + 6 * x) * x)
    f = 1500 * np.exp(-x / 0.33) * vib + 45
    whine = np.sin(2 * np.pi * phase(f)) + 0.35 * lowpass(saw(f), 3000)
    whine *= env(T, 0.005, 0.35, hold=0.5)
    # crackle: sparse filtered impulses, thinning out
    imp = np.zeros(len(x))
    density = 900 * np.exp(-x / 0.35) + 30
    hits = rng.random(len(x)) < density / RATE
    imp[hits] = rng.standard_normal(hits.sum())
    crackle = highpass(lowpass(imp, 6500), 1200)
    crackle = crackle / np.max(np.abs(crackle)) * env(T, 0.001, 0.4, hold=0.15) * 0.5
    boom = at(T, 0.02, thump(1.0, 70, 0.32, drop=0.45)) * 1.1
    rumble = lowpass(noise(T, 25, 400), 180) * env(T, 0.005, 0.3) * 0.35
    zap = click(T, 2000, 9000, 0.01) * 0.4
    return 0.7 * whine + crackle + boom + rumble + zap


def bell(t, hz, decay):
    return ring(t, [hz, hz * 2.76, hz * 5.40, hz * 2.0],
                [decay, decay * 0.45, decay * 0.2, decay * 0.7],
                [1.0, 0.35, 0.15, 0.25])


def hit_target():
    T = 1.3
    notes = [(0.0, 1046.5), (0.07, 1318.5), (0.14, 1568.0), (0.42, 1318.5), (0.49, 1568.0), (0.56, 2093.0)]
    chime = sum(at(T, s, bell(T, hz, 0.22) * env(T, 0.001, 1.0)) * (1.0 + 0.15 * (i % 3))
                for i, (s, hz) in enumerate(notes))
    ping = (ring(T, [3520, 5280, 7390], [0.03, 0.02, 0.012], [1.0, 0.5, 0.3])
            + click(T, 4000, 12000, 0.002)) * 0.4
    x = seconds(T)
    # Two-tone alarm under the chime.
    alarm_f = np.where((x * 7.0) % 1.0 < 0.5, 660.0, 880.0)
    alarm = lowpass(square(alarm_f, 0.5), 2500) * env(T, 0.01, 0.35, hold=0.5) * 0.18
    boom = thump(T, 62, 0.28, drop=0.4) * 1.3 + lowpass(noise(T, 25, 300), 140) * env(T, 0.003, 0.2) * 0.4
    shimmer = noise(T, 6000, 12000) * (0.6 + 0.4 * np.sin(2 * np.pi * 23 * x))
    shimmer *= at(T, 0.14, env(T, 0.03, 0.3)) * 0.12
    sparkle = at(T, 0.56, bell(T, 3136, 0.25)) * 0.3
    return echoes(chime + ping + alarm + boom + shimmer + sparkle, [0.09, 0.18], [0.25, 0.1], 5000)


def grenade_bounce():
    T = 0.2
    clack = (click(T, 1500, 8000, 0.004) * 0.7
             + ring(T, [1150, 2930, 4410], [0.02, 0.012, 0.007], [0.5, 0.4, 0.25])
             + thump(T, 190, 0.015, drop=0.3) * 0.6)
    x = seconds(0.07)
    f = 1500 + 1700 * x / 0.07
    chirp = (np.sin(2 * np.pi * phase(f)) + 0.2 * square(f)) * env(0.07, 0.003, 0.03, hold=0.02)
    return clack + at(T, 0.03, lowpass(chirp, 7000) * 0.3)


def grenade_throw():
    T = 0.45
    x = seconds(T)
    latch = click(T, 1200, 6000, 0.004) * 0.6 + ring(T, [1900, 3100], [0.015, 0.01]) * 0.25
    whoosh = sweep_noise(T, 600, 4000) * np.sin(np.pi * np.clip(x / 0.38, 0, 1)) ** 2 * 0.5
    f = 500 + 1600 * np.clip((x - 0.03) / 0.25, 0, 1) ** 1.5
    chirp = at(T, 0.03, (np.sin(2 * np.pi * phase(f)) * env(T, 0.005, 0.09, hold=0.12))[:int((T - 0.03) * RATE)]) * 0.3
    return latch + highpass(whoosh, 300) + chirp


def grenade_arm():
    T = 0.6
    x = seconds(T)
    rise = np.clip(x / 0.5, 0, 1)
    f = 200 * (2500 / 200) ** rise
    trem = 0.75 + 0.25 * np.sin(2 * np.pi * phase(12 + 50 * rise))
    whine = (np.sin(2 * np.pi * phase(f)) + 0.3 * lowpass(saw(f * 0.5), 5000)) * trem
    whine *= (0.15 + 0.85 * rise ** 1.5) * np.where(x < 0.5, 1.0, np.exp(-(x - 0.5) / 0.008))
    pop = at(T, 0.5, thump(0.1, 140, 0.03, drop=0.5) * 1.2
             + click(0.1, 800, 9000, 0.006) * 0.8
             + noise(0.1, 300, 3000) * env(0.1, 0.001, 0.02) * 0.3)
    return 0.55 * whine + pop


def grenade_burst():
    T = 0.15
    out = np.zeros(int(T * RATE))
    for i, s in enumerate([0.0, 0.042, 0.088]):
        d = 0.04
        x = seconds(d)
        hz = 4200 - 300 * i
        f = 1300 + (hz - 1300) * np.exp(-x / 0.011)
        z = np.sin(2 * np.pi * phase(f) + 0.8 * np.sin(2 * np.pi * phase(f * 0.5)))
        z = z * env(d, 0.0005, 0.011) + click(d, 5000, 12000, 0.001) * 0.25
        out += at(T, s, z * (1.0 - 0.15 * i))
    return out


def respawn():
    T = 0.8
    notes = [523.3, 659.3, 784.0, 1046.5, 1318.5]
    arp = np.zeros(int(T * RATE))
    for i, hz in enumerate(notes):
        d = T - 0.07 * i
        x = seconds(d)
        f = hz * (1 + 0.006 * np.sin(2 * np.pi * 6 * x))
        p = phase(f)
        tri = 2 * np.abs(2 * (p - np.floor(p + 0.5))) - 1
        tone = (0.7 * tri + 0.3 * np.sin(2 * np.pi * 2 * p)) * env(d, 0.004, 0.16 + 0.03 * i)
        arp += at(T, 0.07 * i, tone)
    x = seconds(T)
    sweep = highpass(sweep_noise(T, 300, 10000), 250)
    sweep *= np.sin(np.pi * np.clip(x / 0.7, 0, 1)) ** 2 * (0.7 + 0.3 * np.sin(2 * np.pi * 31 * x))
    return echoes(0.6 * arp + 0.2 * sweep, [0.08, 0.16], [0.3, 0.12], 4000)


def deny():
    T = 0.2
    x = seconds(T)
    tone = square(np.full(len(x), 140.0)) + square(np.full(len(x), 147.0), 0.3)
    gate = ((x > 0.0) & (x < 0.075)) | ((x > 0.1) & (x < 0.175))
    gate = np.convolve(gate.astype(float), np.ones(88) / 88, "same")  # 2 ms soft edges
    return lowpass(tone, 2200) * gate


def empty():
    T = 0.22
    out = np.zeros(int(T * RATE))
    for start, g in ((0.0, 1.0), (0.055, 0.7)):
        c = click(0.05, 1800, 7000, 0.004) + 0.5 * ring(0.05, [2300, 3900], [0.01, 0.006])
        out += g * at(T, start, c)
    x = seconds(T)
    f = 900 * np.exp(-x * 9)
    whine = np.sin(2 * np.pi * phase(f)) * env(T, 0.003, 0.05) * 0.15
    return out + whine


def reload():
    T = 0.9
    x = seconds(T)
    f = 110 * (1 + 2.2 * (x / T) ** 1.4)
    hum = 0.6 * saw(f) + 0.4 * square(f * 2.01, 0.3)
    hum = lowpass(hum, 1800) * (0.4 + 0.6 * x / T)
    tick = (np.sin(2 * np.pi * 14 * x * (1 + x)) > 0.6).astype(float)
    tick = np.convolve(tick, np.ones(66) / 66, "same")
    zap = highpass(noise(T, 3000, 9000), 2500) * tick * 0.25
    return fades(hum * 0.7 + zap, 0.02, 0.08)


def reload_done():
    T = 0.4
    out = np.zeros(int(T * RATE))
    for i, hz in enumerate([1318.5, 1975.5]):
        d = T - 0.09 * i
        x = seconds(d)
        p = phase(np.full(len(x), hz))
        tone = (np.sin(2 * np.pi * p) + 0.3 * np.sin(4 * np.pi * p)) * env(d, 0.003, 0.09)
        out += at(T, 0.09 * i, tone)
    return echoes(out, [0.07], [0.25], 5000)


def pickup():
    T = 0.45
    x = seconds(T)
    f = 600 * np.exp(x * 5.5)
    blip = np.sin(2 * np.pi * phase(f)) * env(T, 0.002, 0.07)
    sparkle = highpass(noise(T, 4000, 12000), 3500) * env(T, 0.001, 0.12)
    sparkle *= (np.sin(2 * np.pi * 37 * x) > 0).astype(float)
    latch = at(T, 0.11, click(0.08, 900, 5000, 0.006) + 0.6 * ring(0.08, [1700, 2600], [0.03, 0.02]))
    return 0.7 * blip + 0.2 * sparkle + 0.5 * latch


def chip(notes, step, last, duty=0.25):
    """8-bit style notes: (Hz) each `step` s long, the last held `last` s."""
    T = step * (len(notes) - 1) + last + 0.12
    out = np.zeros(int(T * RATE))
    for i, hz in enumerate(notes):
        d = last if i == len(notes) - 1 else step
        x = seconds(d)
        f = hz * (1 + 0.01 * np.exp(-x / 0.01))
        tone = (square(np.full(len(x), 0.0) + f, duty) * 0.7 + np.sin(2 * np.pi * phase(f)) * 0.3)
        out += at(T, step * i, tone * env(d, 0.002, d * 0.8, hold=d * 0.4))
    return echoes(lowpass(out, 9000), [0.07], [0.2], 6000)


def score_bleep():
    return chip([1318.5, 1975.5], 0.055, 0.11)


def score_tag():
    return chip([1046.5, 1318.5, 1568.0, 2093.0], 0.05, 0.2)


def supersaw(hz, T, voices=7, spread=0.012):
    """Detuned saws around `hz` (a big analog brass/pad tone)."""
    x = seconds(T)
    out = np.zeros(len(x))
    for k in range(voices):
        d = 1.0 + spread * (k - (voices - 1) / 2) / ((voices - 1) / 2)
        f = np.full(len(x), hz * d) * (1 + 0.003 * np.sin(2 * np.pi * (4.5 + k * 0.3) * x))
        p = phase(f) + rng.random()
        out += 2.0 * (p - np.floor(p + 0.5))
    return out / voices


def brass(hz, T, attack, decay, hold, bright):
    """A supersaw note through a filter that opens with the note (brassy swell)."""
    tone = supersaw(hz, T) + 0.5 * supersaw(hz / 2, T, 3, 0.006)
    # Filter sweep approximated by mixing a dull and a bright copy along the envelope.
    e = env(T, attack, decay, hold)
    dull = lowpass(tone, hz * 2.0)
    shine = lowpass(tone, hz * bright)
    return (dull * (1 - e) + shine * e) * e


def mega_win():
    T = 4.6
    x = seconds(T)
    # Impact: a sub drop and a burst of noise.
    boom = thump(T, 55, 0.6, drop=0.55) * 1.6 + lowpass(noise(T, 20, 400), 180) * env(T, 0.002, 0.35) * 0.8
    crack = click(T, 1500, 12000, 0.015) * 0.6
    # A rising sweep into the fanfare.
    sweep = sweep_noise(0.55, 300, 9000) * env(0.55, 0.4, 0.05, hold=0.5)
    sweep = at(T, 0.0, sweep) * 0.35
    # Fanfare: C E G (rising), then C' held as a big chord (C E G C' G').
    fan = np.zeros(len(x))
    for start, hz, dur in [(0.30, 261.6, 0.18), (0.48, 329.6, 0.18), (0.66, 392.0, 0.18)]:
        fan += at(T, start, brass(hz, dur + 0.25, 0.01, 0.12, dur * 0.6, 6.0)) * 0.9
    chord = [261.6, 329.6, 392.0, 523.3, 784.0]
    for i, hz in enumerate(chord):
        fan += at(T, 0.86, brass(hz, T - 0.86, 0.04, 1.4, 0.9, 7.0)) * (0.85 if i < 4 else 0.55)
    # Sub pedal under the chord.
    pedal = at(T, 0.86, np.sin(2 * np.pi * 65.4 * seconds(T - 0.86)) * env(T - 0.86, 0.05, 1.6, 0.8)) * 0.7
    # Bells and sparkle.
    bells = sum(at(T, 0.86 + 0.09 * i, bell(T, hz, 0.9)) for i, hz in enumerate([1046.5, 1568.0, 2093.0, 3136.0])) * 0.22
    shimmer = noise(T, 5000, 14000) * (0.5 + 0.5 * np.sin(2 * np.pi * 17 * x))
    shimmer *= at(T, 0.86, env(T, 0.2, 1.4, hold=0.4)) * 0.1
    mix = boom + crack + sweep + fan + pedal + bells + shimmer
    return echoes(mix, [0.13, 0.27, 0.41], [0.35, 0.2, 0.1], 4000)


def power_tick():
    T = 0.12
    tick = click(T, 2500, 9000, 0.0025) * 1.2
    knock = ring(T, [1800, 2950], [0.012, 0.008], [1.0, 0.6]) * 0.8
    ping = np.sin(2 * np.pi * 3520 * seconds(T)) * env(T, 0.0005, 0.025) * 0.35
    return tick + knock + ping


def power_up():
    T = 1.1
    x = seconds(T)
    f = 180 * (1 + 5 * (1 - np.exp(-x / 0.25)))
    rise = lowpass(saw(f) * env(T, 0.02, 0.25, hold=0.45), 3500) * 0.5
    sweep = sweep_noise(T, 400, 10000) * env(T, 0.3, 0.2, hold=0.4) * 0.25
    chord = sum(at(T, 0.45, np.sin(2 * np.pi * phase(np.full(int((T - 0.45) * RATE), hz))) * env(T - 0.45, 0.01, 0.3))
                for hz in [1046.5, 1318.5, 1568.0, 2093.0]) * 0.3
    return echoes(rise + sweep + chord, [0.08], [0.25], 6000)


def turret_fire():
    T = 0.32
    x = seconds(T)
    f = 900 * np.exp(-x / 0.06) + 140
    zap = (square(f, 0.3) * 0.6 + saw(f * 1.5) * 0.4) * env(T, 0.001, 0.07)
    zap = lowpass(zap, 4500)
    knock = thump(T, 120, 0.05, drop=0.6) * 0.9 + click(T, 300, 3000, 0.006) * 0.6
    clank = ring(T, [930, 1410, 2350], [0.04, 0.03, 0.02], [0.6, 0.4, 0.3]) * 0.4
    return echoes(zap + knock + clank, [0.06], [0.2], 3000)


def turret_down():
    T = 1.2
    x = seconds(T)
    f = 1400 * np.exp(-x / 0.35) + 60
    whine = (saw(f) * 0.5 + np.sin(2 * np.pi * phase(f * 0.5)) * 0.5) * env(T, 0.005, 0.5, hold=0.3)
    whine = lowpass(whine, 3000)
    sputter = noise(T, 1500, 8000) * ((rng.random(len(x)) < 0.002).astype(float).cumsum() % 2)
    sputter = lowpass(sputter * env(T, 0.01, 0.4), 6000) * 0.25
    clunk = at(T, 0.85, thump(0.3, 90, 0.08) + click(0.3, 200, 2000, 0.01) * 0.5) * 0.9
    return whine + sputter + clunk


def shield_zap():
    T = 0.8
    x = seconds(T)
    crackle = noise(T, 1500, 14000) * (rng.random(len(x)) < 0.02).astype(float)
    crackle = lowpass(crackle, 9000) * env(T, 0.001, 0.25) * 3.0
    buzz_f = 110 + 30 * np.sin(2 * np.pi * 13 * x)
    buzz = (square(buzz_f, 0.5) * 0.6 + saw(buzz_f * 2.01) * 0.4) * env(T, 0.002, 0.3, hold=0.1)
    buzz = lowpass(buzz, 4000)
    snap = click(T, 800, 12000, 0.004) * 1.5 + thump(T, 70, 0.12) * 0.8
    return crackle + buzz * 0.7 + snap


def shield_hum():
    T = 2.0
    x = seconds(T)
    # Every partial a whole number of cycles in 2 s (multiples of 0.5 Hz): a seamless loop.
    drone = (np.sin(2 * np.pi * 60 * x) + 0.6 * np.sin(2 * np.pi * 120.5 * x)
             + 0.35 * np.sin(2 * np.pi * 180 * x) + 0.25 * np.sin(2 * np.pi * 240.5 * x)
             + 0.15 * np.sin(2 * np.pi * 420 * x))
    drone *= 0.8 + 0.2 * np.sin(2 * np.pi * 1.5 * x)
    # Noise from noise() is periodic over its length: the crackle loops too.
    hiss = noise(T, 2000, 9000) * (0.5 + 0.5 * np.sin(2 * np.pi * 3.0 * x)) ** 4 * 0.12
    return drone * 0.5 + hiss


def mirror_ping():
    T = 0.45
    x = seconds(T)
    f = 2600 * (1 + 0.25 * (1 - np.exp(-x / 0.03)))
    ping = np.sin(2 * np.pi * phase(f)) * env(T, 0.001, 0.12) * 0.6
    glass = ring(T, [3900, 5230, 7110], [0.09, 0.06, 0.04], [0.5, 0.35, 0.25]) * 0.5
    zap = lowpass(square(1800 * np.exp(-x / 0.02) + 400, 0.3) * env(T, 0.0005, 0.02), 6000) * 0.3
    return echoes(ping + glass + zap, [0.05], [0.25], 7000)


def jump_pad():
    T = 0.9
    x = seconds(T)
    thud = thump(T, 70, 0.12) * 0.9
    whoosh = sweep_noise(T, 300, 6000) * env(T, 0.05, 0.45, hold=0.2) * 0.6
    f = 220 * 4 ** np.clip(x / 0.5, 0, 1)
    chirp = np.sin(2 * np.pi * phase(f)) * env(T, 0.01, 0.35, hold=0.25) * 0.35
    return echoes(thud + whoosh + chirp, [0.07], [0.2], 5000)


def gate_hum():
    T = 1.0
    x = seconds(T)
    # Every partial a whole number of cycles in 1 s: a seamless loop.
    buzz = square(np.full(len(x), 100.0), 0.3) * 0.5 + saw(np.full(len(x), 200.0)) * 0.3
    buzz = lowpass(buzz, 2500) * (0.75 + 0.25 * np.sin(2 * np.pi * 25 * x))
    whine = np.sin(2 * np.pi * 3200 * x) * 0.05 * (0.6 + 0.4 * np.sin(2 * np.pi * 3 * x))
    return buzz + whine


SOUNDS = {
    # name: (generator, RMS dBFS, seed)
    "footstep_1": (lambda: footstep(1), -27.0, 11),
    "footstep_2": (lambda: footstep(2), -27.0, 12),
    "footstep_3": (lambda: footstep(3), -27.0, 13),
    "laser_fire": (laser_fire, -16.0, 21),
    "laser_fire_red": (laser_fire_red, -16.0, 22),
    "hit_player": (hit_player, -16.0, 31),
    "player_hit": (player_hit, -16.0, 35),
    "tagged_out": (tagged_out, -16.0, 41),
    "hit_target": (hit_target, -14.0, 51),
    "grenade_bounce": (grenade_bounce, -22.0, 61),
    "grenade_throw": (grenade_throw, -20.0, 66),
    "grenade_arm": (grenade_arm, -21.5, 71),
    "grenade_burst": (grenade_burst, -20.0, 81),
    "respawn": (respawn, -20.0, 91),
    "deny": (deny, -21.0, 101),
    "empty": (empty, -22.0, 111),
    "reload": (reload, -24.0, 121),
    "reload_done": (reload_done, -21.0, 131),
    "pickup": (pickup, -19.0, 141),
    "score_bleep": (score_bleep, -22.0, 151),
    "score_tag": (score_tag, -21.0, 161),
    "mega_win": (mega_win, -13.0, 171),
    "power_tick": (power_tick, -22.0, 181),
    "power_up": (power_up, -18.0, 191),
    "turret_fire": (turret_fire, -18.0, 201),
    "turret_down": (turret_down, -19.0, 211),
    "shield_zap": (shield_zap, -15.0, 221),
    "shield_hum": (shield_hum, -24.0, 231),
    "mirror_ping": (mirror_ping, -21.0, 241),
    "jump_pad": (jump_pad, -20.0, 251),
    "gate_hum": (gate_hum, -27.0, 261),
}

# Sounds played as loops.
LOOPS = {"shield_hum", "gate_hum"}


if __name__ == "__main__":
    wanted = [os.path.splitext(os.path.basename(a))[0] for a in sys.argv[1:]] or list(SOUNDS)
    unknown = [w for w in wanted if w not in SOUNDS]
    if unknown:
        sys.exit(f"unknown sound(s): {', '.join(unknown)}; known: {', '.join(SOUNDS)}")
    os.makedirs(OUT, exist_ok=True)
    for name in wanted:
        make, level, seed = SOUNDS[name]
        rng = np.random.default_rng(seed)
        write(name, make(), level, loop=name in LOOPS)
