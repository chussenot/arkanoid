#!/usr/bin/env python3
"""BRYX's score: 32 level tracks and a menu theme, written out as ABC.

    python3 audio/compose.py --render /tmp/bryx-render   # rewrites audio/cues/music_*.abc

The music lives here as *musical decisions*: three themes, the ways they
are transformed, and one recipe per level that picks a mode, a tempo, a
harmony, a texture and which themes speak. This script turns those into
ABC text for the vendored Electric Pulse engine. The ABC files it writes
are committed and are what build.rs renders, so the game never runs
Python. docs/design/music.md is the listener's guide to the same material.

Everything is centered on E, because BRYX's brick sounds play the E minor
pentatonic over the music: the mode changes the color (Dorian brighter,
Phrygian darker, harmonic minor dramatic), never the key, so a rally always
harmonizes with the track under it.

Run it with python3 >= 3.8 and nothing else installed.
"""

from __future__ import annotations

import pathlib

OUT = pathlib.Path(__file__).resolve().parent / "cues"

# --------------------------------------------------------------- pitch ---

MODES = {
    # Semitones above E for scale degrees 0..6.
    "dorian": [0, 2, 3, 5, 7, 9, 10],      # bright minor: C sharp
    "aeolian": [0, 2, 3, 5, 7, 8, 10],     # natural minor
    "phrygian": [0, 1, 3, 5, 7, 8, 10],    # dark: F natural
    "harmonic": [0, 2, 3, 5, 7, 8, 11],    # dramatic: D sharp
}
E4 = 64  # degree 0 of the melody register
NAMES = ["C", "^C", "D", "^D", "E", "F", "^F", "G", "^G", "A", "^A", "B"]


def midi(degree: int, mode: str, base: int = E4) -> int:
    octave, idx = divmod(degree, 7)
    return base + 12 * octave + MODES[mode][idx]


def abc_pitch(m: int) -> str:
    octave, pc = divmod(m, 12)
    name = NAMES[pc]
    acc, letter = (name[0], name[1]) if len(name) == 2 else ("", name)
    octave -= 1  # MIDI 60 = C4
    if octave >= 5:
        return acc + letter.lower() + "'" * (octave - 5)
    return acc + letter + "," * (4 - octave)


# ---------------------------------------------------------- the themes ---
# A line is a list of (degree or None for a rest, steps). One step is a
# sixteenth note; 16 steps make a bar.

R = None

# ASCENT, the player's theme. A leap up a fifth, on to the octave, a turn
# back, then an answer that climbs past where it started.
ASCENT = [
    (0, 2), (4, 2), (7, 4), (6, 2), (4, 2), (3, 2), (4, 2),
    (2, 2), (3, 2), (4, 4), (6, 2), (7, 2), (9, 4),
]

# THE WALL, the obstacle. A machine ostinato: a held note, its upper
# neighbor and its lower one. In Phrygian the upper neighbor is the dark
# F natural; in harmonic minor the lower one is the bright D sharp.
WALL = [(0, 3), (0, 1), (1, 2), (0, 2), (0, 3), (0, 1), (-1, 2), (0, 2)]

# SPARK, the hope. A syncopated bounce up the pentatonic and back down.
SPARK = [
    (4, 1), (R, 1), (6, 1), (7, 1), (R, 1), (9, 2), (R, 1),
    (10, 1), (9, 1), (7, 1), (R, 1), (6, 2), (R, 2),
]


def steps(line):
    return sum(s for _, s in line)


# ------------------------------------------------------ transformations ---

def shift(line, d):
    return [(p if p is None else p + d, s) for p, s in line]


def invert(line):
    """Mirror every interval around the first note."""
    first = next(p for p, _ in line if p is not None)
    return [(p if p is None else 2 * first - p, s) for p, s in line]


def retro(line):
    return list(reversed(line))


def augment(line):
    return [(p, s * 2) for p, s in line]


def diminish(line):
    out = []
    for p, s in line:
        if s >= 2:
            out.append((p, s // 2))
        else:
            out.append((p, 1))
    return out


def displace(line, n):
    """Push the line late by n steps, wrapping its tail to the start."""
    flat = []
    for p, s in line:
        flat += [(p, True)] + [(p, False)] * (s - 1)
    flat = flat[-n:] + flat[:-n]
    out = []
    for p, start in flat:
        if start or not out:
            out.append([p, 1])
        else:
            out[-1][1] += 1
    return [(p, s) for p, s in out]


def fit(line, total):
    """Cut or pad (with rest) a line to exactly `total` steps."""
    out, n = [], 0
    for p, s in line:
        if n >= total:
            break
        s = min(s, total - n)
        out.append((p, s))
        n += s
    if n < total:
        out.append((R, total - n))
    return out


def loop(line, total):
    out = []
    while steps(out) < total:
        out += line
    return fit(out, total)


def staccato(line):
    """Every note one step, the rest of its length silence."""
    out = []
    for p, s in line:
        out.append((p, 1))
        if s > 1:
            out.append((R, s - 1))
    return out


# Theme variants used across the score, by name.
def theme(name: str):
    t = {
        "ascent": ASCENT,
        "ascent_hi": shift(ASCENT, 7),       # an octave up (7 degrees)
        "ascent_5th": shift(ASCENT, 4),      # a diatonic fifth up
        "ascent_inv": invert(ASCENT),
        "ascent_retro": retro(ASCENT),
        "ascent_aug": augment(ASCENT),
        "ascent_head": ASCENT[:7] * 2,
        "ascent_dim": diminish(ASCENT) * 2,
        "ascent_stac": staccato(ASCENT),
        "spark": SPARK * 2,
        "spark_inv": invert(SPARK) * 2,
        "spark_retro": retro(SPARK) * 2,
        "spark_lo": shift(SPARK, -7) * 2,
        "spark_aug": augment(SPARK),
        "spark_seq": SPARK + shift(SPARK, 2),
        "spark_fall": SPARK + shift(SPARK, -2),
        "spark_shift": displace(SPARK * 2, 2),
        "wall": shift(WALL, 7) * 2,
        "wall_hi": shift(WALL, 14) * 2,
        "wall_seq": shift(WALL, 7) + shift(WALL, 9),
        "wall_aug": augment(shift(WALL, 7)),
        "wall_inv": invert(shift(WALL, 7)) * 2,
    }
    return t[name]


# ------------------------------------------------------------- harmony ---
# A progression is one root (scale degree) per bar of a 4-bar phrase.

PROGS = {
    "epic": [0, 5, 2, 6],       # i VI III VII
    "drive": [0, 6, 5, 6],      # i VII VI VII
    "dark": [0, 1, 0, 6],       # i II i VII (Phrygian's lean)
    "lift": [0, 3, 0, 3],       # i iv i iv (Dorian: IV)
    "cadence": [0, 3, 6, 4],    # i iv VII v
    "heroic": [0, 5, 3, 4],     # i VI iv v / V
    "climb": [0, 2, 3, 4],      # i III iv v
    "pedal": [0, 0, 5, 6],      # i i VI VII
    "fall": [0, 6, 5, 4],       # i VII VI v: the descending bass
    "circle": [0, 3, 6, 2],     # i iv VII III
}


def chord(root: int):
    return [root, root + 2, root + 4]


# -------------------------------------------------------------- parts ---

def bass_bar(style: str, root: int):
    r, f = root - 7, root - 7 + 4
    o = root
    return {
        "pulse": [(r, 2)] * 8,
        "octave": [(r, 2), (o, 2)] * 4,
        "sync": [(r, 3), (r, 1), (R, 2), (r, 2), (f, 3), (r, 1), (o, 2), (r, 2)],
        "walk": [(r, 4), (f, 4), (o, 4), (f, 2), (root - 7 + 2, 2)],
        "drive": [(r, 1)] * 16,
        "gallop": [(r, 2), (r, 1), (r, 1)] * 4,
        "drone": [(r, 16)],
        "wall": shift(WALL, root - 7),
        "half": [(r, 6), (r, 2), (f, 4), (o, 4)],
    }[style]


def arp_bar(style: str, root: int, octave: int = 0):
    c = [root + 7 * octave + d for d in (0, 2, 4, 7)]
    a, b, cc, d = c
    return {
        "up": [(a, 1), (b, 1), (cc, 1), (d, 1)] * 4,
        "updown": [(a, 1), (b, 1), (cc, 1), (d, 1), (cc, 1), (b, 1)] * 2 + [(a, 1), (b, 1), (cc, 1), (d, 1)],
        "broken": [(a, 1), (cc, 1), (b, 1), (d, 1)] * 4,
        "eighths": [(a, 2), (cc, 2), (d, 2), (cc, 2)] * 2,
        "sparse": [(a, 1), (R, 1), (cc, 1), (R, 1), (d, 1), (R, 3)] * 2,
        "trance": [(a, 1), (d, 1), (cc, 1), (d, 1)] * 4,
        "rain": [(d + 7, 1), (R, 1), (cc + 7, 1), (R, 2), (b + 7, 1), (R, 2)] * 2,
        "spiral": [(a, 1), (b, 1), (cc, 1), (d, 1), (b + 7, 1), (cc + 7, 1), (d + 7, 1), (a + 14, 1)] * 2,
    }[style]


DRUMS = {
    # kick, snare, hat as 16-step strings.
    "four": ("x...x...x...x...", "....x.......x...", "..x...x...x...x."),
    "half": ("x.......x.......", "........x.......", "x.x.x.x.x.x.x.x."),
    "break": ("x......x..x.....", "....x.......x...", "x.x.x.x.x.x.x.x."),
    "disco": ("x...x...x...x...", "....x.......x...", "..xx..xx..xx..xx"),
    "gallop": ("x..x....x..x....", "....x.......x...", "x.xxx.xxx.xxx.xx"),
    "march": ("x...x...x...x...", "....x..x....x.xx", "x...x...x...x..."),
    "sparse": ("x...........x...", "........x.......", "....x.......x..."),
    "drive": ("x.x.x.x.x.x.x.x.", "....x.......x...", "xxxxxxxxxxxxxxxx"),
}
FILL_SNARE = "....x...x.x.xxxx"


def drum_bar(pattern: str, part: int, fill: bool = False):
    line = DRUMS[pattern][part]
    if fill and part == 1:
        line = FILL_SNARE
    return [(0 if ch == "x" else R, 1) for ch in line]


# ---------------------------------------------------------- palettes ---
# Instruments per palette: name -> directive body. fx bus 0 = drums and
# bass, 1 = melody (echo), 2 = arpeggio (pump), 3 = the ladder filter.

DRUM_KIT = {
    "kick": "wave=triangle amp=96 attack=0 decay=70 sustain=0 release=40 gate=60 fx=0",
    "snare": "wave=noise amp=30 attack=0 decay=60 sustain=0 release=70 gate=50 fx=0",
    "hat": "wave=noise amp=11 attack=0 decay=12 sustain=0 release=14 gate=20 fx=0",
}

PALETTES = {
    "pulse": {
        "bass": "wave=pulse amp=46 duty=38 attack=1 decay=60 sustain=70 release=70 gate=80 fx=0",
        "arp": "wave=pulse amp=20 duty=14 attack=0 decay=30 sustain=40 release=50 gate=50 fx=2",
        "pad": "wave=triangle amp=12 attack=120 decay=200 sustain=80 release=300 gate=98 fx=1",
        "lead": "wave=pulse amp=30 duty=25 attack=4 decay=120 sustain=70 release=180 gate=88 vibrato=6 fx=1",
        "counter": "wave=triangle amp=22 attack=8 decay=120 sustain=70 release=200 gate=86 fx=1",
    },
    "glass": {
        "bass": "wave=triangle amp=58 attack=1 decay=80 sustain=70 release=90 gate=84 fx=0",
        "arp": "wave=pulse amp=17 duty=8 attack=0 decay=40 sustain=30 release=90 gate=40 fx=2",
        "pad": "wave=pulse amp=9 duty=46 attack=200 decay=240 sustain=84 release=420 gate=98 fx=1",
        "lead": "wave=triangle amp=36 attack=10 decay=140 sustain=76 release=260 gate=90 vibrato=9 glide=14 fx=1",
        "counter": "wave=pulse amp=15 duty=30 attack=6 decay=120 sustain=66 release=220 gate=84 fx=1",
    },
    "buzz": {
        "bass": "wave=square amp=36 attack=1 decay=50 sustain=74 release=60 gate=78 fx=3",
        "arp": "wave=pulse amp=18 duty=22 attack=0 decay=24 sustain=50 release=40 gate=46 fx=2",
        "pad": "wave=pulse amp=9 duty=40 attack=60 decay=200 sustain=80 release=240 gate=98 fx=3",
        "lead": "wave=square amp=24 attack=2 decay=90 sustain=74 release=140 gate=86 vibrato=5 fx=1",
        "counter": "wave=pulse amp=16 duty=12 attack=2 decay=100 sustain=60 release=160 gate=80 fx=1",
    },
    "soft": {
        "bass": "wave=pulse amp=42 duty=46 attack=2 decay=90 sustain=72 release=110 gate=86 fx=0",
        "arp": "wave=triangle amp=24 attack=0 decay=50 sustain=40 release=100 gate=50 fx=2",
        "pad": "wave=triangle amp=13 attack=240 decay=260 sustain=86 release=500 gate=98 fx=1",
        "lead": "wave=pulse amp=26 duty=40 attack=14 decay=160 sustain=78 release=300 gate=92 vibrato=11 glide=10 fx=1",
        "counter": "wave=triangle amp=20 attack=10 decay=140 sustain=70 release=260 gate=88 fx=1",
    },
}

FX = {
    # bus 0: drums and bass; 1: melody echo; 2: arpeggio pump; 3: ladder.
    "clean": [
        "0 delay_steps=0 delay_feedback=0 delay_mix=0 drive=12 lowpass=20 mix=100",
        "1 delay_steps=3 delay_feedback=34 delay_mix=26 drive=0 lowpass=30 mix=80",
        "2 delay_steps=2 delay_feedback=24 delay_mix=18 drive=0 lowpass=40 sidechain=34 sidechain_release=160 mix=80",
        "3 delay_steps=0 delay_feedback=0 delay_mix=0 drive=20 lowpass=20 mix=100",
    ],
    "space": [
        "0 delay_steps=0 delay_feedback=0 delay_mix=0 drive=8 lowpass=30 mix=100",
        "1 delay_steps=3 delay_feedback=48 delay_mix=38 drive=0 lowpass=46 mix=80",
        "2 delay_steps=4 delay_feedback=40 delay_mix=34 drive=0 lowpass=50 sidechain=40 sidechain_release=220 mix=76",
        "3 delay_steps=6 delay_feedback=30 delay_mix=30 drive=0 lowpass=60 mix=70",
    ],
    "machine": [
        "0 delay_steps=0 delay_feedback=0 delay_mix=0 drive=26 lowpass=24 mix=100",
        "1 delay_steps=2 delay_feedback=30 delay_mix=22 drive=12 lowpass=30 mix=80",
        "2 delay_steps=1 delay_feedback=20 delay_mix=14 drive=10 lowpass=36 sidechain=50 sidechain_release=140 mix=80",
        "3 delay_steps=0 delay_feedback=0 delay_mix=0 drive=30 lowpass=10 ladder=70 ladder_cutoff=34 ladder_resonance=58 mix=100",
    ],
}

# ------------------------------------------------------------- recipes ---
# One per campaign level, in order. Fields:
#   mode, bpm, palette, fx, prog, bass, arp, drums, swing,
#   a: the A-section lead theme, b: the B-section lead theme,
#   counter: a theme the counter voice carries under the last A,
#   note: what the track is about (docs/design/music.md quotes these).

RECIPES = [
    # ---- Act I: Foundations. Dorian and Aeolian, 112-122, Spark leads.
    dict(level=1, name="Warm-up", mode="dorian", bpm=112, palette="pulse", fx="clean", prog="lift",
         bass="octave", arp="up", drums="half", swing=0, a="spark", b="ascent_head", counter=None,
         note="Spark in its plainest form over a Dorian i-IV sway; Ascent's head appears once, as a promise."),
    dict(level=2, name="Cascade", mode="dorian", bpm=114, palette="glass", fx="space", prog="fall",
         bass="walk", arp="updown", drums="half", swing=0, a="spark_fall", b="spark_retro", counter="ascent_inv",
         note="Everything descends: Spark stepped down a degree a bar, then backwards; Ascent turned upside down underneath."),
    dict(level=3, name="Checkers", mode="aeolian", bpm=116, palette="pulse", fx="clean", prog="epic",
         bass="sync", arp="broken", drums="break", swing=0, a="spark", b="spark_inv", counter="spark_shift",
         note="Call and answer: Spark, then Spark inverted, with a counter-voice two steps behind like alternating squares."),
    dict(level=4, name="Twin towers", mode="dorian", bpm=118, palette="glass", fx="clean", prog="climb",
         bass="octave", arp="eighths", drums="four", swing=0, a="ascent", b="spark", counter="ascent_5th",
         note="Ascent arrives in full, then doubled a fifth above: two towers of the same theme."),
    dict(level=5, name="Arrow", mode="aeolian", bpm=124, palette="pulse", fx="clean", prog="drive",
         bass="drive", arp="trance", drums="four", swing=0, a="ascent_dim", b="spark_seq", counter=None,
         note="Ascent at double speed, straight as an arrow over a driving sixteenth bass."),
    dict(level=6, name="Capped columns", mode="dorian", bpm=116, palette="soft", fx="clean", prog="pedal",
         bass="pulse", arp="sparse", drums="march", swing=0, a="ascent_stac", b="spark", counter=None,
         note="Ascent clipped short, note by note, like columns with caps; a marching snare holds the line."),
    dict(level=7, name="Diamond", mode="dorian", bpm=120, palette="glass", fx="space", prog="circle",
         bass="walk", arp="spiral", drums="half", swing=0, a="ascent", b="ascent_inv", counter="spark",
         note="Symmetry: Ascent and its mirror image, facets of one theme."),
    dict(level=8, name="Scatter", mode="aeolian", bpm=122, palette="pulse", fx="clean", prog="epic",
         bass="sync", arp="sparse", drums="break", swing=56, a="spark_shift", b="ascent_head", counter=None,
         note="Spark knocked off the beat and lightly swung; the first act closes on Ascent's head, unresolved."),
    # ---- Act II: Structures. Aeolian and harmonic minor, 120-128, Ascent leads.
    dict(level=9, name="Picture frame", mode="aeolian", bpm=120, palette="soft", fx="space", prog="heroic",
         bass="walk", arp="updown", drums="four", swing=0, a="ascent", b="spark_aug", counter="ascent_aug",
         note="Ascent framed by itself: stated, then held in long notes beneath the B section."),
    dict(level=10, name="Heart", mode="dorian", bpm=108, palette="soft", fx="space", prog="lift",
         bass="half", arp="eighths", drums="half", swing=60, a="ascent_aug", b="spark", counter="spark_lo",
         note="The warm one: Ascent slowed to a sigh over a swung half-time beat."),
    dict(level=11, name="Double W", mode="aeolian", bpm=124, palette="pulse", fx="clean", prog="drive",
         bass="octave", arp="updown", drums="break", swing=0, a="spark_seq", b="ascent_inv", counter="spark_inv",
         note="Zig-zag: Spark climbing a degree, then Ascent upside down; the arpeggio never stops changing direction."),
    dict(level=12, name="Fortress", mode="phrygian", bpm=118, palette="buzz", fx="machine", prog="dark",
         bass="wall", arp="sparse", drums="march", swing=0, a="ascent", b="wall", counter=None,
         note="The Wall appears: its first full statement, in the bass, with Phrygian's F natural making it a fortress."),
    dict(level=13, name="Lattice", mode="aeolian", bpm=126, palette="glass", fx="clean", prog="circle",
         bass="pulse", arp="broken", drums="disco", swing=0, a="spark_shift", b="spark_inv", counter="spark",
         note="Interlocking Sparks, each displaced from the other, like a lattice."),
    dict(level=14, name="Wave", mode="dorian", bpm=114, palette="soft", fx="space", prog="lift",
         bass="walk", arp="updown", drums="half", swing=62, a="ascent", b="spark_aug", counter="spark",
         note="Swing and glide: Ascent surfs a Dorian swell."),
    dict(level=15, name="Rocket", mode="aeolian", bpm=128, palette="pulse", fx="clean", prog="climb",
         bass="gallop", arp="trance", drums="gallop", swing=0, a="ascent_hi", b="ascent_dim", counter="ascent",
         note="Launch: Ascent from a fifth up, then at double speed, over a galloping bass."),
    dict(level=16, name="Bridges", mode="harmonic", bpm=122, palette="glass", fx="space", prog="heroic",
         bass="wall", arp="eighths", drums="four", swing=0, a="ascent_aug", b="spark", counter="wall_hi",
         note="Ascent stretched across the Wall like a viaduct; harmonic minor's D sharp lifts the span."),
    # ---- Act III: Machines. Phrygian and Aeolian, 126-136, the Wall leads.
    dict(level=17, name="Spiral", mode="aeolian", bpm=128, palette="buzz", fx="machine", prog="circle",
         bass="pulse", arp="spiral", drums="disco", swing=0, a="wall_seq", b="spark_seq", counter="ascent_head",
         note="The machine act begins: the Wall turned into a rising sequence, an arpeggio that spirals up the octave."),
    dict(level=18, name="Pillars", mode="phrygian", bpm=126, palette="soft", fx="machine", prog="pedal",
         bass="drone", arp="sparse", drums="half", swing=0, a="wall_aug", b="ascent_aug", counter=None,
         note="Space between columns: a drone, the Wall in long notes, Ascent held like a pillar."),
    dict(level=19, name="Crown", mode="harmonic", bpm=130, palette="pulse", fx="clean", prog="heroic",
         bass="octave", arp="up", drums="march", swing=0, a="ascent", b="ascent_hi", counter="spark",
         note="Regal: Ascent as a fanfare in harmonic minor, then crowned a fifth higher."),
    dict(level=20, name="Hourglass", mode="aeolian", bpm=128, palette="glass", fx="space", prog="fall",
         bass="sync", arp="updown", drums="break", swing=0, a="ascent", b="ascent_retro", counter="spark_retro",
         note="Time turned over: Ascent, then Ascent backwards, the sand running the other way."),
    dict(level=21, name="Shield", mode="phrygian", bpm=130, palette="buzz", fx="machine", prog="dark",
         bass="wall", arp="broken", drums="four", swing=0, a="spark", b="wall", counter="spark_lo",
         note="Spark held up against the Wall: hope over the machine."),
    dict(level=22, name="Armored frame", mode="phrygian", bpm=132, palette="buzz", fx="machine", prog="dark",
         bass="wall", arp="trance", drums="drive", swing=0, a="wall_hi", b="wall_inv", counter="wall",
         note="All armor: the Wall high, the Wall inverted, the Wall below. The heaviest track of the act."),
    dict(level=23, name="Rain", mode="aeolian", bpm=124, palette="glass", fx="space", prog="fall",
         bass="half", arp="rain", drums="sparse", swing=58, a="spark_fall", b="ascent_inv", counter=None,
         note="Falling droplets: Spark stepping down over a sparse, swung beat; a breath before the last act."),
    dict(level=24, name="Funnel", mode="harmonic", bpm=134, palette="pulse", fx="clean", prog="cadence",
         bass="gallop", arp="trance", drums="gallop", swing=0, a="spark_seq", b="ascent_dim", counter="wall_hi",
         note="Narrowing: everything accelerates into a v-to-i cadence that pours into the next act."),
    # ---- Act IV: Vault. Harmonic minor, 132-148, all three themes.
    dict(level=25, name="Two rooms", mode="dorian", bpm=132, palette="soft", fx="space", prog="lift",
         bass="octave", arp="eighths", drums="four", swing=0, a="ascent", b="wall", counter="spark",
         note="Two rooms, two moods: a bright Dorian Ascent, then the Wall next door."),
    dict(level=26, name="Ring", mode="aeolian", bpm=134, palette="glass", fx="space", prog="circle",
         bass="walk", arp="spiral", drums="disco", swing=0, a="spark", b="spark_shift", counter="spark_shift",
         note="A round: Spark chasing itself, two steps behind, around a circle of chords."),
    dict(level=27, name="Baffles", mode="phrygian", bpm=136, palette="buzz", fx="machine", prog="dark",
         bass="sync", arp="broken", drums="break", swing=0, a="spark_shift", b="wall_seq", counter="ascent_stac",
         note="Deflections: displaced Spark, a Wall sequence, Ascent bouncing in staccato."),
    dict(level=28, name="Chevron", mode="aeolian", bpm=138, palette="pulse", fx="clean", prog="fall",
         bass="octave", arp="updown", drums="four", swing=0, a="spark_fall", b="spark_seq", counter="ascent_inv",
         note="V-shape: Spark falls, then climbs back the same way."),
    dict(level=29, name="Split wall", mode="harmonic", bpm=138, palette="buzz", fx="machine", prog="cadence",
         bass="wall", arp="trance", drums="drive", swing=0, a="ascent_hi", b="wall_hi", counter="spark",
         note="The Wall split by Ascent breaking through it, a fifth above."),
    dict(level=30, name="Bulwark", mode="phrygian", bpm=132, palette="buzz", fx="machine", prog="pedal",
         bass="half", arp="sparse", drums="half", swing=0, a="wall_aug", b="ascent_aug", counter="wall",
         note="Massive and slow-feeling at speed: half-time drums, the Wall in long notes, a bulwark."),
    dict(level=31, name="Minefield", mode="phrygian", bpm=140, palette="glass", fx="space", prog="dark",
         bass="pulse", arp="sparse", drums="sparse", swing=0, a="spark_shift", b="ascent_stac", counter=None,
         note="Tension before the end: sparse, staccato, every note placed carefully."),
    dict(level=32, name="Vault", mode="harmonic", bpm=144, palette="pulse", fx="space", prog="heroic",
         bass="wall", arp="trance", drums="drive", swing=0, a="ascent", b="spark", counter="ascent_5th",
         note="The finale: Ascent over the Wall, then Spark, then Ascent in fifths with itself, all three themes at once."),
]

# ------------------------------------------------------------- arrange ---

INTRO, SECTION = 2, 4  # bars
FORM = ["intro", "A", "B", "A2", "turn"]  # 2 + 4 + 4 + 4 + 2 = 16 bars


def compose(r: dict) -> str:
    mode, prog = r["mode"], PROGS[r["prog"]]
    bars = []  # (section, bar index within the section, chord root)
    for section in FORM:
        n = INTRO if section in ("intro", "turn") else SECTION
        for i in range(n):
            bars.append((section, i, prog[i % len(prog)]))

    parts = {k: [] for k in ("kick", "snare", "hat", "bass", "arp", "pad", "lead", "counter")}
    a_line = loop(theme(r["a"]), SECTION * 16)
    b_line = loop(theme(r["b"]), SECTION * 16)
    counter_line = loop(theme(r["counter"]), SECTION * 16) if r["counter"] else None

    for section, i, root in bars:
        last = (section in ("A", "B", "A2") and i == SECTION - 1) or (section == "turn" and i == 1)
        sparse_drums = section == "intro"
        for part, idx in (("kick", 0), ("snare", 1), ("hat", 2)):
            if sparse_drums and part == "snare":
                parts[part] += [(R, 16)]
            elif section == "turn" and i == 0 and part == "kick":
                parts[part] += [(R, 16)]  # the breath before the loop
            else:
                parts[part] += drum_bar(r["drums"], idx, fill=last)
        parts["bass"] += bass_bar(r["bass"], root)
        arp_oct = 1 if section in ("B", "A2") else 0
        parts["arp"] += arp_bar(r["arp"], root, arp_oct) if section != "turn" or i == 0 else [(R, 16)]
        parts["pad"] += [(root + 2, 16)] if section in ("B", "A2", "turn") else [(R, 16)]

        if section == "A":
            parts["lead"] += slice_line(a_line, i)
            parts["counter"] += [(R, 16)]
        elif section == "B":
            parts["lead"] += slice_line(b_line, i)
            parts["counter"] += [(R, 16)]
        elif section == "A2":
            parts["lead"] += slice_line(a_line, i)
            parts["counter"] += slice_line(counter_line, i) if counter_line else [(R, 16)]
        elif section == "turn":
            # The turn quotes the A theme's first bar, then rests on the
            # tonic so the loop's restart lands as a resolution.
            parts["lead"] += slice_line(a_line, 0) if i == 0 else [(0, 4), (R, 12)]
            parts["counter"] += [(R, 16)]
        else:
            parts["lead"] += [(R, 16)]
            parts["counter"] += [(R, 16)]

    # The counter sits an octave under the lead, except a "_5th" theme,
    # which shares the lead's register so it sounds a fifth *above* it:
    # parallel fifths, the score's power-chord moment.
    counter_base = E4 if r["counter"] and r["counter"].endswith("_5th") else E4 - 12
    registers = {"bass": E4 - 12, "arp": E4, "pad": E4 - 12, "lead": E4, "counter": counter_base}
    pal = PALETTES[r["palette"]]
    head = [
        "X:1",
        f"T:BRYX {r['level']:02d} {r['name']}",
        "C:BRYX (audio/compose.py)",
        "M:4/4",
        "L:1/16",
        f"Q:1/4={r['bpm']}",
        "K:C",
        f"% {r['note']}",
        f"% mode E {r['mode']}, progression {r['prog']}, A={r['a']} B={r['b']} counter={r['counter']}",
    ]
    if r["swing"]:
        head.append(f"%%swing {r['swing']}")
    for name, body in DRUM_KIT.items():
        head.append(f"%%instrument {name} {body}")
    for name in ("bass", "arp", "pad", "lead", "counter"):
        head.append(f"%%instrument {name} {pal[name]}")
    for bus in FX[r["fx"]]:
        head.append(f"%%effect {bus}")
    body = []
    for name in ("kick", "snare", "hat", "bass", "arp", "pad", "lead", "counter"):
        body.append(f"V:{name} instrument={name}")
        if name == "kick":
            line = [(None if p is None else -1000, s) for p, s in parts[name]]
        elif name in ("snare", "hat"):
            line = [(None if p is None else -2000, s) for p, s in parts[name]]
        else:
            line = parts[name]
        body.append(render_line(line, mode, registers.get(name, E4)))
    return "\n".join(head + body) + "\n"


def slice_line(line, bar):
    """Bar `bar` (16 steps) of a line, notes cut at the bar lines."""
    start, end = bar * 16, bar * 16 + 16
    out, pos = [], 0
    for p, s in line:
        a, b = max(pos, start), min(pos + s, end)
        if a < b:
            out.append((p, b - a))
        pos += s
    return fit(out, 16)


def render_line(line, mode, base):
    tokens, n = [], 0
    for p, s in line:
        if p is None:
            tok = "z"
        elif p == -1000:
            tok = "E,,"  # the kick's thump: a low triangle E
        elif p == -2000:
            tok = "c"  # noise: pitch is irrelevant
        else:
            tok = abc_pitch(midi(p, mode, base))
        tokens.append(tok + (str(s) if s > 1 else ""))
        n += s
    # Bar lines every 16 steps for readability (the engine ignores them).
    out, count = [], 0
    for tok, (p, s) in zip(tokens, line):
        out.append(tok)
        count += s
        if count % 16 == 0:
            out.append("|")
    return "| " + " ".join(out)


# ---------------------------------------------------------------- menu ---

MENU = dict(level=0, name="Menu", mode="dorian", bpm=92, palette="glass", fx="space", prog="lift",
            bass="half", arp="sparse", drums="sparse", swing=0, a="ascent_aug", b="spark_aug",
            counter="spark", note="The menu: Ascent at half speed, the first thing a player hears; Spark answers.")


# ------------------------------------------------------------- master ---

def master(text: str, render: str, tmp: pathlib.Path) -> str:
    """Scale every voice of a track together until it peaks at or under
    112 (of the engine's 128) with no clipped sample and its loudness is
    at most -19 dBFS RMS, the level of the score's quietest tracks: one
    level across all 33, so a level change never jumps in volume, and the
    balance between voices untouched."""
    import re
    import subprocess

    for _ in range(30):
        src, out = tmp / "track.abc", tmp / "track.u8"
        src.write_text(text)
        subprocess.run([render, str(src), str(out), "22050"], check=True)
        pcm = out.read_bytes()
        peak = max(abs(b - 128) for b in pcm)
        clip = sum(1 for b in pcm if b in (0, 255))
        rms = (sum((b - 128) ** 2 for b in pcm) / len(pcm)) ** 0.5 / 128
        if peak <= 112 and clip == 0 and rms <= 10 ** (-19 / 20):
            return text
        text = re.sub(r"amp=(\d+)", lambda m: f"amp={max(4, round(int(m.group(1)) * 0.95))}", text)
    raise SystemExit("could not master the track")


def main():
    """`--render PATH` names a renderer built like build.rs's (see
    docs/design/music.md, "Changing the music"); every track is mastered
    with it. Required: an unmastered score would clip."""
    import sys
    import tempfile

    if "--render" not in sys.argv:
        raise SystemExit("usage: compose.py --render <path to a built audio/render.c>")
    render = sys.argv[sys.argv.index("--render") + 1]
    tracks = [(f"music_{r['level']:02d}", compose(r)) for r in RECIPES]
    tracks.append(("music_menu", compose(MENU)))
    with tempfile.TemporaryDirectory() as tmp:
        for name, text in tracks:
            text = master(text, render, pathlib.Path(tmp))
            (OUT / f"{name}.abc").write_text(text)
    print(f"wrote {len(tracks)} mastered tracks to {OUT}")


if __name__ == "__main__":
    main()
