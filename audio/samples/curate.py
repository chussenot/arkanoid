#!/usr/bin/env python3
"""Curate BRYX's orchestral sample subset from VSCO 2 Community Edition.

usage: audio/samples/curate.py VSCO_CHECKOUT [OUT_DIR]

VSCO 2 CE (https://github.com/sgossner/VSCO-2-CE, CC0 1.0, commit
VSCO_COMMIT below) is 3 GB of 44.1 kHz stereo WAV. BRYX ships ~30 MB of
it, as 16-bit mono FLAC, with one SFZ-subset map per instrument
(docs/design/orchestral-score.md, section 4.3). This script is how that
subset is made, so it can be made again: same input, same output.

For every instrument in INSTRUMENTS it:
  - picks the files (first round robin; the softest and loudest dynamic
    layers where two are kept);
  - checks each file's pitch against its name (VSCO names notes one
    octave below standard: its "A2" is MIDI 57) and stores the measured
    offset as `tune`, so the samples play in tune (short notes too, where
    their pitch can be measured);
  - trims the silence before the attack;
  - for sustained articulations, finds a steady loop and bakes a
    crossfade into it, so a plain loop is click-free;
  - cuts short articulations where they fall 55 dB below their peak;
  - levels each note on its own: the loudest layer of every key is
    brought to the same loudness (the loudest 50 ms of RMS), and a softer
    layer keeps VSCO's soft-to-loud ratio for that note, clamped to
    SOFT_RANGE_DB, so neither a quiet recording session nor a hot one
    makes one key jump out of a line;
  - scales the instrument as a whole to a -1 dBFS peak, dithers to 16 bit
    (fixed seed) and writes FLAC with ffmpeg;
  - writes INSTRUMENT.sfz: <region> lines with sample, pitch_keycenter,
    tune, lokey/hikey, lovel/hivel and loop_start/loop_end.

Requires numpy and ffmpeg. Never run at build time: its output is
committed, and build.rs reads that.
"""

import glob
import os
import re
import subprocess
import sys
import wave

import numpy as np

VSCO_COMMIT = "440300901dfe9275fd84e0b7763af1f8443ae62e"
RATE = 44100
NOTES = {"C": 0, "C#": 1, "D": 2, "D#": 3, "E": 4, "F": 5, "F#": 6, "G": 7, "G#": 8, "A": 9, "A#": 10, "B": 11}

# name: (glob under the checkout, kind, layers kept, longest seconds, family)
#   kind: "sustain" (looped), "short" (one shot, pitched), "hit" (one shot,
#   unpitched or fixed pitch)
INSTRUMENTS = {
    # Strings: sections, except the contrabass (VSCO has a solo one only).
    "violins_sus": ("Strings/Violin Section/susVib/*.wav", "sustain", 2, 3.2, "strings"),
    "violas_sus": ("Strings/Viola Section/susvib/*.wav", "sustain", 1, 3.2, "strings"),
    "celli_sus": ("Strings/Cello Section/susvib/*.wav", "sustain", 2, 3.2, "strings"),
    "basses_sus": ("Strings/Solo Contrabass/SusNV/*.wav", "sustain", 1, 3.2, "strings"),
    "violins_spic": ("Strings/Violin Section/Spic/*.wav", "short", 1, 0.9, "strings"),
    "violas_spic": ("Strings/Viola Section/spic/*.wav", "short", 1, 0.9, "strings"),
    "celli_spic": ("Strings/Cello Section/spic/*.wav", "short", 1, 0.9, "strings"),
    "basses_spic": ("Strings/Solo Contrabass/Spic/*.wav", "short", 1, 0.9, "strings"),
    "violins_trem": ("Strings/Violin Section/Trem/*.wav", "sustain", 1, 2.8, "strings"),
    "violas_trem": ("Strings/Viola Section/trem/*.wav", "sustain", 1, 2.8, "strings"),
    "celli_trem": ("Strings/Cello Section/trem/*.wav", "sustain", 1, 2.8, "strings"),
    "violins_pizz": ("Strings/Violin Section/Pizz/*.wav", "short", 1, 1.2, "strings"),
    "celli_pizz": ("Strings/Cello Section/pizzT/*.wav", "short", 1, 1.4, "strings"),
    # Brass: solo players; sections are stacked in the engine.
    "horn_sus": ("Brass/F Horn/sus/*.wav", "sustain", 2, 3.2, "brass"),
    "horn_stac": ("Brass/F Horn/stac/*.wav", "short", 1, 0.9, "brass"),
    "trumpet_sus": ("Brass/Trumpet/sus/*.wav", "sustain", 1, 3.2, "brass"),
    "trumpet_stac": ("Brass/Trumpet/stac/*.wav", "short", 1, 0.8, "brass"),
    "trombone_sus": ("Brass/Tenor Trombone/sus/*.wav", "sustain", 1, 3.2, "brass"),
    "trombone_stac": ("Brass/Tenor Trombone/stac/*.wav", "short", 1, 0.9, "brass"),
    "tuba_sus": ("Brass/Tuba/sus/*.wav", "sustain", 1, 3.2, "brass"),
    "tuba_stac": ("Brass/Tuba/stac/*.wav", "short", 1, 1.0, "brass"),
}

# Timpani: one drum per file, at the key VSCO's own Timpani.sfz gives it
# (checked against the measured pitch below).
TIMPANI = {1: 42, 2: 46, 3: 49, 4: 52, 5: 54}

# Unpitched percussion, one sample per key (GM-like keys where one fits).
KIT = [
    (35, "Percussion/BDrumNewhit_v2_rr1_Sum.wav", 1, 80, 3.0),
    (35, "Percussion/BDrumNewhit_v5_rr1_Sum.wav", 81, 127, 3.0),
    (49, "Percussion/susCymb1-hit_mp_rr1.wav", 1, 80, 5.0),
    (49, "Percussion/susCymb1-hit_fff_rr1.wav", 81, 127, 5.0),
    (52, "Percussion/susCymb1-cresc-Short_v1.wav", 1, 127, 20.0),
    (53, "Percussion/susCymb1-cresc-Median_v1.wav", 1, 127, 20.0),
    (54, "Percussion/susCymb1-cresc-Long_v1.wav", 1, 127, 20.0),
    (55, "Percussion/gongHit_f.wav", 1, 127, 7.0),
    (56, "Percussion/Anvil_Hit1_v2_Sum.wav", 1, 127, 1.5),
]


def load(path):
    w = wave.open(path)
    n, sw, ch, sr = w.getnframes(), w.getsampwidth(), w.getnchannels(), w.getframerate()
    raw = w.readframes(n)
    assert sr == RATE, (path, sr)
    if sw == 2:
        x = np.frombuffer(raw, "<i2").astype(np.float64) / 32768
    else:
        b = np.frombuffer(raw, np.uint8).reshape(-1, 3).astype(np.int32)
        x = b[:, 0] | (b[:, 1] << 8) | (b[:, 2] << 16)
        x = np.where(x >= 1 << 23, x - (1 << 24), x).astype(np.float64) / 8388608
    return x.reshape(-1, ch).mean(axis=1)


def yin(x, fmin=25.0, fmax=2500.0):
    """Fundamental frequency of a steady segment (YIN, threshold 0.1)."""
    x = x - x.mean()
    w = len(x) // 2
    maxlag = min(int(RATE / fmin), w - 1)
    minlag = int(RATE / fmax)
    e = np.cumsum(x ** 2)
    lags = np.arange(1, maxlag + 1)
    cross = np.array([np.dot(x[:w], x[lag : lag + w]) for lag in lags])
    d = e[w - 1] + (e[w - 1 + lags] - e[lags - 1]) - 2 * cross
    cm = d * lags / np.maximum(np.cumsum(d), 1e-12)
    cm = np.concatenate([[1.0], cm])
    cand = np.where(cm[minlag:] < 0.1)[0]
    lag = minlag + (cand[0] if len(cand) else int(np.argmin(cm[minlag:])))
    while lag + 1 < len(cm) and cm[lag + 1] < cm[lag]:
        lag += 1
    a, b, c = cm[lag - 1], cm[lag], cm[lag + 1]
    den = a - 2 * b + c
    return RATE / (lag + (0.5 * (a - c) / den if den else 0.0))


def name_midi(fn):
    m = re.search(r"_([A-G]#?)(-?\d)(?:_|\.)", fn)
    return None if m is None else 12 * (int(m.group(2)) + 2) + NOTES[m.group(1)]


def layer_rr(fn):
    v = re.search(r"_v(\d+)", fn)
    rr = re.search(r"_rr(\d+)", fn, re.I) or re.search(r"_v\d+_(\d+)(?:_|\.)", fn)
    return (int(v.group(1)) if v else 1), (int(rr.group(1)) if rr else 1)


def onset(x):
    """The attack's start: 2 ms before the first sample 50 dB under the peak."""
    peak = np.max(np.abs(x))
    i = int(np.argmax(np.abs(x) > peak * 10 ** (-50 / 20)))
    return max(0, i - int(0.002 * RATE))


def envelope_db(x, win=0.02):
    n = int(win * RATE)
    k = len(x) // n
    rms = np.sqrt(np.mean(x[: k * n].reshape(k, n) ** 2, axis=1) + 1e-20)
    return 20 * np.log10(rms / rms.max()), n


def cut_short(x, longest):
    db, n = envelope_db(x)
    below = np.where(db < -55)[0]
    peak_frame = int(np.argmax(db))
    after = below[below > peak_frame]
    end = min(len(x), (after[0] + 1) * n if len(after) else len(x), int(longest * RATE))
    y = x[:end].copy()
    fade = min(int(0.03 * RATE), end)
    y[-fade:] *= np.linspace(1, 0, fade) ** 2
    return y


def loop_sustain(x, longest, period):
    """Loop the steady middle of a sustain: loop_start after the attack,
    loop_end at the best-matching point ~1.5-2.2 s later, with a 180 ms
    equal-power crossfade baked in before loop_end."""
    start = int(0.9 * RATE)
    xf = int(0.18 * RATE)
    best, best_end = None, None
    lo, hi = start + int(1.5 * RATE), min(len(x) - 1, start + int((longest - 0.9) * RATE))
    if hi <= lo:
        raise ValueError("sample too short to loop")
    ref = x[start - xf : start]
    step = max(1, int(period)) if period else 64
    for end in range(lo, hi, max(1, step // 4)):
        seg = x[end - xf : end]
        score = np.sum((seg - ref) ** 2)
        if best is None or score < best:
            best, best_end = score, end
    end = best_end
    y = x[:end].copy()
    t = np.linspace(0, np.pi / 2, xf)
    y[end - xf : end] = x[end - xf : end] * np.cos(t) + x[start - xf : start] * np.sin(t)
    return y, start, end


def write_flac(y, path):
    rng = np.random.default_rng(0x5EED)
    tpdf = (rng.random(len(y)) - rng.random(len(y))) / 32768
    pcm = np.clip(np.round((y + tpdf) * 32767), -32768, 32767).astype("<i2")
    os.makedirs(os.path.dirname(path), exist_ok=True)
    subprocess.run(
        ["ffmpeg", "-loglevel", "error", "-y", "-f", "s16le", "-ar", str(RATE), "-ac", "1",
         "-i", "-", "-compression_level", "12", "-map_metadata", "-1", "-fflags", "+bitexact",
         "-flags:a", "+bitexact", path],
        input=pcm.tobytes(), check=True,
    )


def key_ranges(roots):
    roots = sorted(roots)
    out = {}
    for i, r in enumerate(roots):
        lo = roots[i - 1] + (r - roots[i - 1]) // 2 + 1 if i else r - 3
        hi = r + (roots[i + 1] - r) // 2 if i + 1 < len(roots) else r + 3
        out[r] = (max(0, lo), min(127, hi))
    return out


def measured_midi(x, at=0.6, span=0.25):
    seg = x[int(at * RATE) : int((at + span) * RATE)]
    if len(seg) < int(0.1 * RATE):
        return None, None
    f0 = yin(seg)
    return 69 + 12 * np.log2(f0 / 440), RATE / f0


def curate_pitched(src, out, name, pattern, kind, layers, longest, family):
    files = sorted(glob.glob(os.path.join(src, pattern)))
    by_note = {}
    if kind == "sustain":
        # Sustains are long enough to measure: group by the measured note,
        # so a file VSCO named an octave off (Cello trem "B2_v2" is a B1)
        # lands where it sounds. Any other disagreement stops the run.
        for f in files:
            fn = os.path.basename(f)
            nm = name_midi(fn)
            if nm is None or layer_rr(fn)[1] != 1:
                continue
            x = load(f)
            x = x[onset(x):]
            det, period = measured_midi(x)
            root = next((r for r in (nm, nm - 12, nm + 12) if abs(det - r) <= 0.6), None)
            if root is None:
                raise SystemExit(f"{f}: named MIDI {nm}, measured {det:.2f}")
            if root != nm:
                print(f"  relabel {fn}: sounds MIDI {root}, not {nm}")
            by_note.setdefault(root, []).append((layer_rr(fn)[0], f, x, det, period))
    else:
        # Short notes are too short to measure reliably everywhere: keep the
        # first round robin of each layer whose measured pitch agrees with
        # its name, or the first round robin if none can be measured.
        groups = {}
        for f in files:
            fn = os.path.basename(f)
            nm = name_midi(fn)
            if nm is None:
                continue
            layer, rr = layer_rr(fn)
            groups.setdefault((nm, layer), []).append((rr, f))
        for (nm, layer), cands in sorted(groups.items()):
            cands.sort()
            pick = None
            for rr, f in cands:
                x = load(f)
                x = x[onset(x):]
                det, _ = measured_midi(x, at=0.04, span=0.15)
                if det is not None and abs(det - nm) <= 0.6:
                    pick = (layer, f, x, det, None)
                    break
            if pick is None:
                f = cands[0][1]
                x = load(f)
                pick = (layer, f, x[onset(x):], None, None)
                print(f"  unverified pitch {os.path.basename(f)}: kept as named")
            by_note.setdefault(nm, []).append(pick)
    samples = []
    for root, fs in sorted(by_note.items()):
        fs.sort(key=lambda t: t[0])
        chosen = [fs[0], fs[-1]] if layers == 2 and len(fs) > 1 else [fs[-1]]
        for li, (layer, f, x, det, period) in enumerate(chosen):
            # A measured pitch tunes the sample to its key: sustains always,
            # short notes when their pitch could be measured (it is how
            # their name was checked); a pizzicato a few cents off its
            # neighbours beats against anything played with it.
            tune = int(round((root - det) * 100)) if det is not None else 0
            if kind == "sustain":
                y, ls, le = loop_sustain(x, longest, period)
            else:
                y, ls, le = cut_short(x, longest), None, None
            vel = ((1, 79) if li == 0 else (80, 127)) if len(chosen) == 2 else (1, 127)
            samples.append((y, f, root, tune, vel, ls, le))
    return finish(out, name, family, samples, pitched=True)


# How far a soft layer may sit below its note's loud layer, in dB. VSCO's
# own ratio is kept inside this range; outside it the recording, not the
# playing, set the level (timpani 1's soft hits sit 30 dB down).
SOFT_RANGE_DB = (-18.0, -6.0)


def loudness(y, win=0.05):
    """The loudest `win` seconds of RMS: a level that means the same thing
    for a short hit and a held note, unlike the peak."""
    n = max(1, int(RATE * win))
    p = np.convolve(y * y, np.ones(n) / n, mode="valid") if len(y) > n else [np.mean(y * y)]
    return float(np.sqrt(np.max(p)))


def note_gains(samples):
    """Per-sample gains that level every key's loudest layer to the same
    loudness and keep each softer layer's ratio to it, clamped."""
    gains = [1.0] * len(samples)
    by_key = {}
    for i, s in enumerate(samples):
        by_key.setdefault(s[2], []).append(i)
    for idx in by_key.values():
        loud = max(idx, key=lambda i: samples[i][4][1])
        ref = loudness(samples[loud][0])
        for i in idx:
            if i == loud:
                gains[i] = 1.0 / ref
                continue
            ratio = 20 * np.log10(max(loudness(samples[i][0]), 1e-9) / ref)
            ratio = min(max(ratio, SOFT_RANGE_DB[0]), SOFT_RANGE_DB[1])
            gains[i] = 10 ** (ratio / 20) / loudness(samples[i][0])
    return gains


def finish(out, name, family, samples, pitched):
    gains = note_gains(samples)
    peak = max(np.max(np.abs(s[0])) * g for s, g in zip(samples, gains))
    gains = [g * 10 ** (-1 / 20) / peak for g in gains]
    ranges = key_ranges({s[2] for s in samples}) if pitched else None
    lines = [
        f"// {name}: curated from VSCO 2 CE ({VSCO_COMMIT[:7]}), CC0 1.0, by audio/samples/curate.py.",
        "// Do not edit: rerun the script.",
    ]
    for (y, f, root, tune, vel, ls, le), gain in zip(samples, gains):
        v = f"_v{vel[0]}" if vel != (1, 127) else ""
        rel = f"{family}/{name}/{name}_{root}{v}.flac"
        write_flac(y * gain, os.path.join(out, rel))
        lo, hi = ranges[root] if pitched else (root, root)
        r = f"<region> sample={rel} lokey={lo} hikey={hi} pitch_keycenter={root}"
        if tune:
            r += f" tune={tune}"
        r += f" lovel={vel[0]} hivel={vel[1]}"
        if ls is not None:
            r += f" loop_mode=loop_continuous loop_start={ls} loop_end={le - 1}"
        r += f" // {os.path.relpath(f, SRC)}"
        lines.append(r)
    with open(os.path.join(out, f"{name}.sfz"), "w") as fh:
        fh.write("\n".join(lines) + "\n")
    size = sum(os.path.getsize(os.path.join(out, l.split("sample=")[1].split()[0])) for l in lines[2:])
    print(f"{name:16s} {len(samples):3d} samples {size / 1e6:6.2f} MB")
    return size


def curate_timpani(src, out):
    samples = []
    for drum, key in TIMPANI.items():
        hits = sorted(glob.glob(os.path.join(src, f"Percussion/Timpani/Timpani{drum}_Hit_v*_rr1_Sum.wav")))
        hits.sort(key=lambda f: layer_rr(os.path.basename(f))[0])
        chosen = [hits[0], hits[-1]] if len(hits) > 1 else hits
        for li, f in enumerate(chosen):
            x = load(f)
            y = cut_short(x[onset(x):], 4.0)
            vel = ((1, 79) if li == 0 else (80, 127)) if len(chosen) == 2 else (1, 127)
            samples.append((y, f, key, 0, vel, None, None))
    size = finish(out, "timpani", "percussion", samples, pitched=True)
    rolls = []
    for drum, key in TIMPANI.items():
        f = sorted(glob.glob(os.path.join(src, f"Percussion/Timpani/Rolls/Timpani{drum}_Roll_v*_rr1_Sum.wav")))[-1]
        x = load(f)
        rolls.append((cut_short(x[onset(x):], 6.0), f, key, 0, (1, 127), None, None))
    return size + finish(out, "timpani_roll", "percussion", rolls, pitched=True)


def curate_kit(src, out):
    samples = []
    for key, rel, lov, hiv, longest in KIT:
        f = os.path.join(src, rel)
        x = load(f)
        samples.append((cut_short(x[onset(x):], longest), f, key, 0, (lov, hiv), None, None))
    return finish(out, "kit", "percussion", samples, pitched=False)


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(2)
    SRC = sys.argv[1]
    OUT = sys.argv[2] if len(sys.argv) > 2 else os.path.dirname(os.path.abspath(__file__))
    total = 0
    for name, (pattern, kind, layers, longest, family) in INSTRUMENTS.items():
        total += curate_pitched(SRC, OUT, name, pattern, kind, layers, longest, family)
    total += curate_timpani(SRC, OUT)
    total += curate_kit(SRC, OUT)
    print(f"total {total / 1e6:.1f} MB")
