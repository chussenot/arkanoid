#!/usr/bin/env python3
"""BRYX's score: 32 level tracks and a menu theme, written out as ABC.

    python3 audio/compose.py --check                       # the score lint only
    python3 audio/compose.py --render /tmp/bryx-render     # lint, then rewrite audio/cues/music_*.abc

The music lives here as *musical decisions*: three themes, the ways they
are transformed and developed into phrases, a harmony per mode, a form per
act, and one recipe per level that picks a mode, a tempo, a texture and
which themes speak. This script turns those into ABC text for the vendored
Electric Pulse engine. The ABC files it writes are committed and are what
build.rs renders, so the game never runs Python. docs/design/music.md is
the listener's guide to the same material.

Everything is centered on E, because BRYX's brick sounds play the E minor
pentatonic over the music: the mode changes the color (Dorian brighter,
Phrygian darker, harmonic minor dramatic), never the key, so a rally always
harmonizes with the track under it.

The engine retriggers every sixteenth-note step, so a "held" note is a
stream of repeated sixteenths: rhythm is heard through pitch changes, and
a note on a strong beat is heard against the chord for as long as it
lasts. That is why `harmonize` and `check_score` exist.

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


def pc(degree: int, mode: str) -> int:
    """The pitch class of a degree, in semitones above E."""
    return MODES[mode][degree % 7]


def abc_pitch(m: int) -> str:
    octave, pclass = divmod(m, 12)
    name = NAMES[pclass]
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


def pitches(line):
    return [p for p, _ in line if p is not None]


# ------------------------------------------------------ transformations ---

def shift(line, d):
    return [(p if p is None else p + d, s) for p, s in line]


def invert(line):
    """Mirror every interval around the first note."""
    first = pitches(line)[0]
    return [(p if p is None else 2 * first - p, s) for p, s in line]


def retro(line):
    return list(reversed(line))


def augment(line):
    return [(p, s * 2) for p, s in line]


def diminish(line):
    return [(p, max(1, s // 2)) for p, s in line]


def staccato(line):
    """Every note one step, the rest of its length silence."""
    out = []
    for p, s in line:
        out.append((p, 1))
        if s > 1:
            out.append((R, s - 1))
    return out


def to_steps(line):
    """A line as one (pitch, starts a note) pair per step."""
    out = []
    for p, s in line:
        out.append((p, True))
        out += [(p, False)] * (s - 1)
    return out


def from_steps(st):
    out = []
    for p, start in st:
        if start or not out:
            out.append([p, 1])
        else:
            out[-1][1] += 1
    return [(p, s) for p, s in out]


def displace(line, n):
    """Push the line late by n steps, wrapping its tail to the start."""
    st = to_steps(line)
    st = st[-n:] + st[:-n]
    st[0] = (st[0][0], True)
    return from_steps(st)


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


def cut(line, a, b):
    """Steps a..b of a line, notes cut at both ends."""
    out, pos = [], 0
    for p, s in line:
        lo, hi = max(pos, a), min(pos + s, b)
        if lo < hi:
            out.append((p, hi - lo))
        pos += s
    return fit(out, b - a)


def bars_of(line):
    return [cut(line, 16 * i, 16 * i + 16) for i in range((steps(line) + 15) // 16)]


# ------------------------------------------------------- the variants ---
# name -> (motif, development, sequence step). A motif of one bar is
# developed into a four-bar phrase as a "sentence" (stated, sequenced,
# broken into fragments, closed) or a "climb" (sequenced up or down every
# bar, a ratchet); a two-bar motif as a "period" (stated, then answered
# with a close); a four-bar motif is a phrase already.

THEMES = {
    "ascent": (ASCENT, "period", 0),
    "ascent_hi": (shift(ASCENT, 7), "period", 0),     # an octave up
    "ascent_5th": (shift(ASCENT, 4), "period", 0),    # a diatonic fifth up
    "ascent_inv": (invert(ASCENT), "period", 0),
    "ascent_retro": (retro(ASCENT), "period", 0),
    "ascent_aug": (augment(ASCENT), "whole", 0),
    "ascent_head": (ASCENT[:7], "sentence", 2),
    "ascent_dim": (diminish(ASCENT), "sentence", 2),
    "ascent_stac": (staccato(ASCENT), "period", 0),
    "spark": (SPARK, "sentence", 2),
    "spark_fall": (SPARK, "sentence", -2),
    "spark_inv": (invert(SPARK), "sentence", 2),
    "spark_retro": (retro(SPARK), "sentence", -1),
    "spark_aug": (augment(SPARK), "period", 0),
    "spark_seq": (SPARK, "climb", 1),
    "spark_shift": (displace(SPARK, 2), "sentence", 2),
    "wall": (shift(WALL, 7), "climb", 1),
    "wall_hi": (shift(WALL, 11), "climb", 1),         # a fifth above the Wall
    "wall_seq": (shift(WALL, 7), "climb", 2),
    "wall_aug": (augment(shift(WALL, 7)), "period", 0),
    "wall_inv": (invert(shift(WALL, 7)), "climb", -1),
    "wall_inv_aug": (augment(invert(shift(WALL, 7))), "period", 0),
}


def home_of(line) -> int:
    """The tonic a line belongs to: the E nearest its first note, but never
    above E5, so a high theme still cadences in the lead's register."""
    return min(7 * round(pitches(line)[0] / 7), 7)


def close_tail(last: int, home: int):
    """Eight steps that walk by step to the second degree above `home`
    (F sharp, or F in Phrygian): a half cadence, left open for the next
    downbeat to resolve. F sharp is a tone of every chord that ends a
    progression here (V, v, bVII, bII), so the close always fits."""
    target = home + 1
    if last >= target:
        path = list(range(min(max(last, target + 1), target + 2), target - 1, -1))
    else:
        path = list(range(max(last, target - 2), target + 1))
    return [(p, 2) for p in path[:-1]] + [(path[-1], 8 - 2 * (len(path) - 1))]


def approach(mode: str, home: int):
    """The loop's cadence: eight steps that lead by step on to the tonic,
    which the next bar (the loop's first) sounds. Harmonic minor rises
    through its leading tone; Phrygian falls through its F natural."""
    if mode == "harmonic":
        return [(home + 2, 2), (home + 1, 2), (home - 1, 4)]
    return [(home + 3, 2), (home + 2, 2), (home + 1, 4)]


def cadence_bar(bar, home):
    head = fit(bar, 8)
    held = pitches(head)
    return head + close_tail(held[-1] if held else home + 3, home)


def lead_register(motif):
    """Lift a motif by octaves until no note falls below A3, so an
    inverted theme never sinks into the bass."""
    k = 0
    while min(pitches(motif)) + k < -4:
        k += 7
    return shift(motif, k)


def phrase(name: str, lead: bool = True):
    """A theme developed into a four-bar phrase that ends on a half
    cadence."""
    motif, kind, step = THEMES[name]
    if lead:
        motif = lead_register(motif)
    home = home_of(motif)
    bars = bars_of(motif)
    if len(bars) >= 4:
        body = bars[:4]
    elif len(bars) == 2:
        body = [bars[0], bars[1], bars[0], bars[1]]
    elif kind == "climb":
        body = [shift(bars[0], step * i) for i in range(4)]
    else:  # sentence: idea, idea sequenced, its head twice higher, close
        m = bars[0]
        head = fit(m, 8)
        body = [m, shift(m, step), shift(head, 2 * step) * 2, m]
    body[3] = cadence_bar(body[3], home)
    return body, home


def climax(body):
    """The recap's change: the first bar that can climb a third without
    passing C sharp 6 does, so the last statement reaches higher than any
    before it."""
    for i in (1, 2):
        if max(pitches(body[i]), default=0) + 2 <= 12:
            return body[:i] + [shift(body[i], 2)] + body[i + 1:]
    return body


# ------------------------------------------------------------- harmony ---
# A progression is four bars, written as scale-degree roots. "3|6" splits
# a bar into two half-bar chords; "3/5" is chord 3 over bass degree 5.
# Each mode has its own, chosen to keep its color chord and to end on the
# chord that leads home (V, v, bVII or bII), so the loop's seam cadences.
# The B section has its own progression, starting away from the tonic.

PROGS = {
    # name: (A progression, B progression)
    # Dorian
    "lift": ("0 3 0 3|6", "3 2 3 6"),          # i IV i IV-bVII | IV III IV bVII
    "sway": ("0 0 3 6", "2 3 2|3 6"),          # i i IV bVII: a pedal that leans
    "fall_dorian": ("0 6 3/5 4", "3 2 6 4"),   # i bVII IV6 v: the bass falls E D C# B
    "circle": ("0 3 2 6", "2 6 3 4"),          # i iv III bVII
    "climb": ("0 2 3 4", "3 2 3|2 4"),         # i III iv v
    # Aeolian
    "epic": ("0 5 3 6", "5 2 3 6"),            # i bVI iv bVII: the mixed cadence home
    "drive": ("0 6 5 6", "5 5 6 6"),           # i bVII bVI bVII; B two bars a chord
    "fall": ("0 6 5 4", "5 3 6 4"),            # i bVII bVI v
    # Phrygian
    "dark": ("0 6 0 1", "5 1 5 6|1"),          # i bvii i bII: the Phrygian cadence at the seam
    "pedal": ("0 0 5 1", "1 1 5 6|1"),         # i i bVI bII
    # Harmonic minor
    "heroic": ("0 5 3|1 4", "3 5 3|1 4"),      # i VI iv-ii° V
    "cadence": ("0 3 5|3 4", "5 5 3 4"),       # i iv VI-iv V
}


def parse_prog(text: str):
    """One list per bar of (root, bass, steps) chords."""
    bars = []
    for token in text.split():
        halves = token.split("|")
        bar = []
        for half in halves:
            root, _, bass = half.partition("/")
            bar.append((int(root), int(bass) if bass else int(root), 16 // len(halves)))
        bars.append(bar)
    return bars


def chord_pcs(root: int, mode: str):
    return {pc(root + k, mode) for k in (0, 2, 4)}


def is_color(p: int, root: int, mode: str) -> bool:
    """A seventh or ninth above the chord that is not a half step from any
    of its tones: heard as color, not as a wrong note."""
    above = (pc(p, mode) - pc(root, mode)) % 12
    if above not in (2, 10):
        return False
    return all((pc(p, mode) - t) % 12 not in (1, 11) for t in chord_pcs(root, mode))


def heard_right(played, k, chord_at, mode: str) -> bool:
    """Whether note k of a melody (a list of [pitch, onset, steps], rests
    left out) sits right against the chord under it: a chord tone, a
    color tone, an appoggiatura that steps straight on to a chord tone,
    or a short neighbor or passing note reached by step from one."""
    p, t, s = played[k]
    root = chord_at(t)
    if pc(p, mode) in chord_pcs(root, mode) or is_color(p, root, mode):
        return True
    nxt = played[k + 1] if k + 1 < len(played) else None
    if (nxt and s <= 4 and nxt[1] - (t + s) <= 2 and abs(nxt[0] - p) == 1
            and pc(nxt[0], mode) in chord_pcs(chord_at(nxt[1]), mode)):
        return True
    prv = played[k - 1] if k else None
    return bool(prv and s <= 2 and prv[1] + prv[2] == t and abs(prv[0] - p) == 1
                and pc(prv[0], mode) in chord_pcs(chord_at(prv[1]), mode))


def harmonize(line, chord_at, mode: str):
    """Fit a melody to the harmony under it. A note that starts on beat 1
    or 3, or that holds four steps from a beat, must be heard right
    (`heard_right`); otherwise it moves one degree to the nearest chord
    tone, toward where the line goes next. The themes keep their contour
    and their neighbor notes; the accidents go."""
    notes, t = [], 0
    for p, s in line:
        notes.append([p, t, s])
        t += s
    played = [n for n in notes if n[0] is not None]
    for k, n in enumerate(played):
        p, t, s = n
        if not (t % 8 == 0 or (t % 4 == 0 and s >= 4)) or heard_right(played, k, chord_at, mode):
            continue
        nxt = played[k + 1] if k + 1 < len(played) else None
        tones = chord_pcs(chord_at(t), mode)
        up = nxt is not None and nxt[0] > p
        for cand in ((p + 1, p - 1) if up else (p - 1, p + 1)):
            if pc(cand, mode) in tones:
                n[0] = cand
                break
    return [(p, s) for p, _, s in notes]


# -------------------------------------------------------------- parts ---

def bass_bar(style: str, root: int, bass: int):
    r = bass - 7
    f = root - 7 + 4  # the chord's fifth, kept above the bass note
    while f <= r:
        f += 7
    while f > r + 7:
        f -= 7
    o = bass
    return {
        "pulse": [(r, 2)] * 8,
        "octave": [(r, 2), (o, 2)] * 4,
        "sync": [(r, 3), (r, 1), (R, 2), (r, 2), (f, 3), (r, 1), (o, 2), (r, 2)],
        "walk": [(r, 4), (f, 4), (o, 4), (f, 2), (r + 2, 2)],
        "drive": [(r, 1)] * 16,
        "gallop": [(r, 2), (r, 1), (r, 1)] * 4,
        "drone": [(r, 16)],
        # Thinned, for the sections where the floor drops out: one thump a
        # bar, then the rebuild, and ticks on 1 and 3 for an entrance.
        "thump": [(r, 2), (R, 14)],
        "rebuild": [(r, 2), (R, 6), (r, 2), (R, 2), (r, 2), (r, 2)],
        "tick": [(r, 2), (R, 6), (r, 2), (R, 6)],
        "wall": shift(WALL, r),
        "half": [(r, 6), (r, 2), (f, 4), (o, 4)],
    }[style]


# B's bass, by A's: a different gait for the contrasting section.
BASS_B = {"drive": "octave", "gallop": "sync", "pulse": "octave", "octave": "sync",
          "sync": "octave", "walk": "half", "half": "walk", "drone": "half", "wall": "wall"}
# Styles that push the next chord a sixteenth early (an anticipation).
PUSH = {"pulse", "octave", "sync", "drive", "gallop"}


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


def chordwise(bar_chords, make):
    """A bar built chord by chord: each chord's share of `make(chord)`."""
    out, t = [], 0
    for chord in bar_chords:
        n = chord[2]
        out += cut(make(chord), t, t + n)
        t += n
    return out


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
# Fills add snare hits to the pattern's own. A roll opens a gap in the hat.
FILLS = {
    "roll": "........x.x.xxxx",
    "light": ".............x.x",
    "pickup": "..............xx",
}
LIGHT_STYLES = {"half", "sparse"}  # a roll would swamp them: they get the light fill


def merge(a: str, b: str) -> str:
    return "".join("x" if "x" in (x, y) else "." for x, y in zip(a, b))


def hits(pattern: str):
    return [(0 if ch == "x" else R, 1) for ch in pattern]


# ---------------------------------------------------------------- form ---
# One form per act, all 16 bars, so every track costs the same in the
# binary. Sections: intro (a bass, an arpeggio and a hat), A (the theme),
# B (the contrast: its own chords, bass and the pad), A2 (the recap: a
# climax and the counter-line), break (the floor drops out: pad, a pedal
# bass, a fragment of the theme), turn (a breath, then the cadence home).

FORMS = {
    1: [("intro", 2), ("A", 4), ("B", 4), ("A2", 4), ("turn", 2)],
    2: [("A", 4), ("B", 4), ("break", 2), ("A2", 4), ("turn", 2)],
    3: [("intro", 2), ("A", 4), ("B", 4), ("break", 2), ("A2", 4)],
    4: [("A", 4), ("B", 4), ("A2", 4), ("break", 2), ("turn", 2)],
}


def act_of(level: int) -> int:
    return 1 if level <= 8 else 2 if level <= 16 else 3 if level <= 24 else 4


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
#   mode, bpm, palette, fx, prog (a PROGS name), bass, arp, drums, swing
#   (50..75, 0 for straight), a: the A-section lead theme, b: the B-section
#   lead theme, counter: the line under the recap (a theme, "auto" for a
#   counter-line written by rule against the lead, "canon" for the lead
#   itself a beat later and an octave down, or None),
#   note: what the track is about (docs/design/music.md quotes these).
#   Optional: bass_b (B's bass style; BASS_B by default).

RECIPES = [
    # ---- Act I: Foundations. Dorian and Aeolian, 112-122, Spark leads.
    dict(level=1, name="Warm-up", mode="dorian", bpm=112, palette="pulse", fx="clean", prog="lift",
         bass="octave", arp="up", drums="half", swing=54, a="spark", b="ascent_head", counter=None,
         note="Spark in its plainest form over a Dorian i-IV sway; Ascent's head appears in the middle, as a promise."),
    dict(level=2, name="Cascade", mode="dorian", bpm=114, palette="glass", fx="space", prog="fall_dorian",
         bass="walk", arp="updown", drums="half", swing=0, a="spark_fall", b="spark_retro", counter="ascent_inv",
         note="Everything descends: Spark stepped down a degree at a time, then backwards; Ascent turned upside down underneath; a bass that walks down E, D, C sharp, B."),
    dict(level=3, name="Checkers", mode="aeolian", bpm=116, palette="pulse", fx="clean", prog="epic",
         bass="sync", arp="broken", drums="break", swing=0, a="spark", b="spark_inv", counter="auto",
         note="Call and answer: Spark, then Spark inverted; under the recap a second voice moves in thirds and sixths through the squares the lead leaves empty."),
    dict(level=4, name="Twin towers", mode="dorian", bpm=118, palette="glass", fx="clean", prog="climb",
         bass="octave", arp="eighths", drums="four", swing=0, a="ascent", b="spark", counter="ascent_5th",
         note="Ascent arrives in full, then doubled a fifth above: two towers of the same theme."),
    dict(level=5, name="Arrow", mode="aeolian", bpm=124, palette="pulse", fx="clean", prog="drive",
         bass="drive", arp="trance", drums="four", swing=0, a="ascent_dim", b="spark_seq", counter=None,
         note="Ascent at double speed, straight as an arrow over a driving sixteenth bass; Spark climbs a step a bar."),
    dict(level=6, name="Capped columns", mode="dorian", bpm=116, palette="soft", fx="clean", prog="sway",
         bass="pulse", arp="sparse", drums="march", swing=0, a="ascent_stac", b="spark", counter=None,
         note="Ascent clipped short, note by note, like columns with caps; a marching snare holds the line."),
    dict(level=7, name="Diamond", mode="dorian", bpm=120, palette="glass", fx="space", prog="circle",
         bass="walk", arp="spiral", drums="half", swing=54, a="ascent", b="ascent_inv", counter="spark",
         note="Symmetry: Ascent and its mirror image, facets of one theme."),
    dict(level=8, name="Scatter", mode="aeolian", bpm=122, palette="pulse", fx="clean", prog="epic",
         bass="sync", arp="sparse", drums="break", swing=56, a="spark_shift", b="ascent_head", counter=None,
         note="Spark knocked off the beat and lightly swung; Ascent's head returns in the middle, still a promise, as the first act ends."),
    # ---- Act II: Structures. Aeolian and harmonic minor, 120-128, Ascent leads.
    dict(level=9, name="Picture frame", mode="aeolian", bpm=120, palette="soft", fx="space", prog="heroic",
         bass="walk", arp="updown", drums="four", swing=0, a="ascent", b="spark_aug", counter="ascent_aug",
         note="Ascent framed by itself: stated, then, in the recap, held in long notes beneath its own return."),
    dict(level=10, name="Heart", mode="dorian", bpm=108, palette="soft", fx="space", prog="lift",
         bass="half", arp="eighths", drums="half", swing=60, a="ascent_aug", b="spark", counter="spark",
         note="The warm one: Ascent slowed to a sigh over a swung half-time beat."),
    dict(level=11, name="Double W", mode="aeolian", bpm=124, palette="pulse", fx="clean", prog="drive",
         bass="octave", arp="updown", drums="break", swing=0, a="spark_seq", b="ascent_inv", counter="auto",
         note="Zig-zag: Spark climbing a degree a bar, then Ascent upside down; the arpeggio never stops changing direction."),
    dict(level=12, name="Fortress", mode="phrygian", bpm=118, palette="buzz", fx="machine", prog="dark",
         bass="wall", arp="sparse", drums="march", swing=0, a="ascent", b="wall", counter=None,
         note="The Wall appears: its first full statement, in the bass, with Phrygian's F natural making it a fortress."),
    dict(level=13, name="Lattice", mode="aeolian", bpm=126, palette="glass", fx="clean", prog="circle",
         bass="pulse", arp="broken", drums="disco", swing=54, a="spark_shift", b="spark_inv", counter="spark_aug",
         note="Interlocking Sparks: one knocked off the beat, and in the recap Spark in long notes threaded underneath."),
    dict(level=14, name="Wave", mode="dorian", bpm=114, palette="soft", fx="space", prog="lift",
         bass="walk", arp="updown", drums="half", swing=62, a="ascent", b="spark_aug", counter="spark",
         note="Swing and glide: Ascent surfs a Dorian swell."),
    dict(level=15, name="Rocket", mode="aeolian", bpm=128, palette="pulse", fx="clean", prog="climb",
         bass="gallop", arp="trance", drums="gallop", swing=0, a="ascent_hi", b="ascent_dim", counter="auto",
         note="Launch: Ascent an octave up, then at double speed, over a galloping bass."),
    dict(level=16, name="Bridges", mode="harmonic", bpm=122, palette="glass", fx="space", prog="heroic",
         bass="wall", arp="eighths", drums="four", swing=0, a="ascent_aug", b="spark", counter="wall_hi",
         note="Ascent stretched across the Wall like a viaduct; harmonic minor's D sharp lifts the span."),
    # ---- Act III: Machines. Phrygian and Aeolian, 126-136, the Wall leads.
    dict(level=17, name="Spiral", mode="aeolian", bpm=128, palette="buzz", fx="machine", prog="circle",
         bass="pulse", arp="spiral", drums="disco", swing=0, a="wall_seq", b="spark_seq", counter="ascent_head",
         note="The machine act begins: the Wall turned into a rising sequence, an arpeggio that spirals up the octave."),
    dict(level=18, name="Pillars", mode="phrygian", bpm=126, palette="soft", fx="machine", prog="pedal",
         bass="drone", arp="sparse", drums="half", swing=0, a="wall_aug", b="ascent_aug", counter=None,
         note="Space between columns: a bass that keeps one note a bar, the Wall in long notes, Ascent held like a pillar."),
    dict(level=19, name="Crown", mode="harmonic", bpm=130, palette="pulse", fx="clean", prog="heroic",
         bass="octave", arp="up", drums="march", swing=0, a="ascent", b="ascent_hi", counter="spark",
         note="Regal: Ascent as a fanfare in harmonic minor, then crowned an octave higher."),
    dict(level=20, name="Hourglass", mode="aeolian", bpm=128, palette="glass", fx="space", prog="fall",
         bass="sync", arp="updown", drums="break", swing=0, a="ascent", b="ascent_retro", counter="auto",
         note="Time turned over: Ascent, then Ascent backwards, the sand running the other way."),
    dict(level=21, name="Shield", mode="phrygian", bpm=130, palette="buzz", fx="machine", prog="dark",
         bass="wall", arp="broken", drums="four", swing=0, a="spark", b="wall", counter="auto",
         note="Spark held up against the Wall: hope over the machine."),
    dict(level=22, name="Armored frame", mode="phrygian", bpm=132, palette="buzz", fx="machine", prog="dark",
         bass="wall", arp="trance", drums="drive", swing=0, a="wall_hi", b="wall_inv", counter="wall_inv_aug",
         note="All armor: the Wall high and ratcheting up, the Wall inverted and sinking, and in the recap the inverted Wall at half speed beneath the high one. The heaviest track of the act."),
    dict(level=23, name="Rain", mode="aeolian", bpm=124, palette="glass", fx="space", prog="fall",
         bass="half", arp="rain", drums="sparse", swing=58, a="spark_fall", b="ascent_inv", counter=None,
         note="Falling droplets: Spark stepping down over a sparse, swung beat; a breath before the last act."),
    dict(level=24, name="Funnel", mode="harmonic", bpm=134, palette="pulse", fx="clean", prog="cadence",
         bass="gallop", arp="trance", drums="gallop", swing=0, a="spark_seq", b="ascent_dim", counter="wall_hi",
         note="Narrowing: Spark ratchets up a step a bar, Ascent doubles its speed, and a V-to-i pours into the next act."),
    # ---- Act IV: Vault. Harmonic minor, 132-148, all three themes.
    dict(level=25, name="Two rooms", mode="dorian", bpm=132, palette="soft", fx="space", prog="lift",
         bass="octave", arp="eighths", drums="four", swing=0, a="ascent", b="wall", counter="spark",
         note="Two rooms, two moods: a bright Dorian Ascent, then the Wall next door."),
    dict(level=26, name="Ring", mode="aeolian", bpm=134, palette="glass", fx="space", prog="circle",
         bass="walk", arp="spiral", drums="disco", swing=55, a="spark", b="spark_shift", counter="canon",
         note="A round: Spark chasing itself a beat behind, around a circle of chords."),
    dict(level=27, name="Baffles", mode="phrygian", bpm=136, palette="buzz", fx="machine", prog="dark",
         bass="sync", arp="broken", drums="break", swing=0, a="spark_shift", b="wall_seq", counter="ascent_stac",
         note="Deflections: displaced Spark, a Wall sequence, Ascent bouncing in staccato."),
    dict(level=28, name="Chevron", mode="aeolian", bpm=138, palette="pulse", fx="clean", prog="fall",
         bass="octave", arp="updown", drums="four", swing=0, a="spark_fall", b="spark_seq", counter="ascent_inv",
         note="V-shape: Spark falls, then climbs back the same way."),
    dict(level=29, name="Split wall", mode="harmonic", bpm=138, palette="buzz", fx="machine", prog="cadence",
         bass="wall", arp="trance", drums="drive", swing=0, a="ascent_hi", b="wall_hi", counter="spark",
         note="The Wall split by Ascent breaking through it, an octave above."),
    dict(level=30, name="Bulwark", mode="phrygian", bpm=132, palette="buzz", fx="machine", prog="pedal",
         bass="half", arp="sparse", drums="half", swing=0, a="wall_aug", b="ascent_aug", counter="wall",
         note="Massive and slow-feeling at speed: half-time drums, the Wall in long notes, a bulwark."),
    dict(level=31, name="Minefield", mode="phrygian", bpm=140, palette="glass", fx="space", prog="dark",
         bass="pulse", arp="sparse", drums="sparse", swing=0, a="spark_shift", b="ascent_stac", counter=None,
         note="Tension before the end: sparse, staccato, every note placed carefully."),
    dict(level=32, name="Vault", mode="harmonic", bpm=144, palette="pulse", fx="space", prog="heroic",
         bass="wall", arp="trance", drums="drive", swing=0, a="ascent", b="spark", counter="spark",
         note="The finale: Ascent over the Wall, then Spark; in the recap all three at once, the Wall below, Ascent above, Spark between."),
]

MENU = dict(level=0, name="Menu", mode="dorian", bpm=92, palette="glass", fx="space", prog="lift",
            bass="half", arp="sparse", drums="sparse", swing=0, a="ascent_aug", b="spark_aug",
            counter="spark", note="The menu: Ascent at half speed, the first thing a player hears; Spark answers.")


# ------------------------------------------------------------- arrange ---

def counter_register(counter, lead):
    """Put a theme counter an octave or so under the lead, never into the
    bass: shift by octaves toward a median a seventh below the lead's,
    then up again if it would dip under C sharp 3."""
    lead_med = sorted(pitches(lead))[len(pitches(lead)) // 2]
    own = sorted(pitches(counter))
    k = 7 * round((lead_med - 7 - own[len(own) // 2]) / 7)
    while own[0] + k < -9:
        k += 7
    return shift(counter, k)


def auto_counter(lead_st, chord_at, mode, start, n):
    """A counter-line written by rule against the lead, half notes and the
    odd passing note: chord tones under the lead, a third, sixth or tenth
    from it where possible, moving by step, against the lead's direction
    when it leaps, never into parallel fifths or octaves."""
    choices, prev_c, prev_l, prev_iv = [], None, None, None
    for t in range(start, start + n, 8):
        held = [p for p, _ in lead_st[t:t + 8] if p is not None]
        l = lead_st[t][0] if lead_st[t][0] is not None else (held[0] if held else prev_l)
        tones = chord_pcs(chord_at(t), mode)
        best = None
        for c in range(-6, 3):  # F sharp 3 .. G4
            if pc(c, mode) not in tones or (l is not None and c > l - 2):
                continue
            iv = (l - c) % 7 if l is not None else 2
            cost = {2: 0, 5: 0, 0: 3, 4: 3}.get(iv, 8)
            if prev_c is not None:
                move = abs(c - prev_c)
                cost += 1 if move == 0 else 0 if move <= 2 else 1 if move <= 4 else 9
                if iv in (0, 4) and iv == prev_iv and c != prev_c:
                    cost += 12  # parallel fifths or octaves
                if l is not None and prev_l is not None and l != prev_l:
                    if (c - prev_c) * (l - prev_l) < 0:
                        cost -= 1  # contrary motion
                    elif iv in (0, 4) and (c - prev_c) * (l - prev_l) > 0:
                        cost += 4  # a perfect interval by similar motion
            if best is None or cost < best[0]:
                best = (cost, c, iv)
        if best is None:
            choices.append(None)
            continue
        _, c, iv = best
        choices.append(c)
        prev_c, prev_l, prev_iv = c, l, iv
    out = []
    for k, c in enumerate(choices):
        t = start + 8 * k
        nxt = choices[k + 1] if k + 1 < len(choices) else None
        lead_moves = any(start_ for _, start_ in lead_st[t + 4:t + 8])
        if c is not None and nxt is not None and abs(nxt - c) == 2 and not lead_moves:
            out += [(c, 4), ((c + nxt) // 2, 4)]  # a passing note while the lead holds
        else:
            out.append((c, 8))
    return out


def repair_counter(line, lead_st, chord_at, mode, start):
    """Voice-lead a theme counter against the lead it now sits under: a
    note that makes a tritone with the lead on a strong beat, or a fifth
    or octave straight after the same interval, moves by the smallest step
    that cures it, staying a chord tone on a strong beat and under the
    lead."""
    notes, t = [], start
    for p, s in line:
        notes.append([p, t, s])
        t += s
    prev = None
    for n in notes:
        p, t, s = n
        lp = lead_st[t][0]
        if p is None or lp is None:
            prev = None
            continue

        def bad(c):
            iv = (midi(lp, mode) - midi(c, mode)) % 12
            strong = t % 8 == 0
            if c > lp or (strong and pc(c, mode) not in chord_pcs(chord_at(t), mode)):
                return True
            if iv == 6 and (strong or lead_st[t][1]):
                return True
            return bool(prev and iv in (0, 7) and iv == prev[0] and lp != prev[1] and c != prev[2])

        if bad(p):
            for c in (p - 1, p + 1, p - 2, p + 2):
                if not bad(c):
                    n[0] = c
                    break
        prev = ((midi(lp, mode) - midi(n[0], mode)) % 12, lp, n[0])
    return [(p, s) for p, _, s in notes]


def compose(r: dict):
    """A track: its parts as lines of degrees, its chords per step, and its
    registers."""
    mode = r["mode"]
    prog_a, prog_b = (parse_prog(p) for p in PROGS[r["prog"]])
    form = FORMS[act_of(r["level"]) if r["level"] else 1]

    # Each bar: (section, index in section, section length, next section, chords).
    bars = []
    for k, (section, n) in enumerate(form):
        nxt = form[(k + 1) % len(form)][0]
        src = {"A": prog_a, "A2": prog_a, "B": prog_b,
               "intro": prog_a[0:2], "break": prog_a[0:2], "turn": prog_a[2:4]}[section]
        for i in range(n):
            bars.append((section, i, n, nxt, src[i % len(src)]))
    total = 16 * len(bars)
    chord_steps = []
    for *_, chords in bars:
        for root, bass, n in chords:
            chord_steps += [(root, bass)] * n

    def chord_at(t):
        return chord_steps[t % total][0]

    a_body, home = phrase(r["a"])
    b_body, _ = phrase(r["b"])
    a2_body = climax(a_body)

    parts = {k: [] for k in ("kick", "snare", "hat", "bass", "arp", "pad", "lead", "counter")}
    has_counter = bool(r["counter"])
    pad_prev = None
    for section, i, n, nxt, chords in bars:
        last = i == n - 1
        # Drums: what plays, and the fill.
        kick = snare = hat = True
        fill = None
        if section == "intro":
            kick, snare = i == 1, False
            fill = "pickup" if last else None
        elif section == "B":
            fill = "light" if i == 2 else ("roll" if nxt == "A2" else "light") if last else None
        elif section == "break":
            kick = snare = False
            hat = i == 1
            if last and nxt == "A2":
                snare, fill = True, "roll"
        elif section == "A2":
            fill = "pickup" if last and nxt != "turn" else None
        elif section == "turn":
            kick = i == 1
            fill = "pickup" if last else None
        k_pat, s_pat, h_pat = DRUMS[r["drums"]]
        if fill == "roll" and r["drums"] in LIGHT_STYLES:
            fill = "light"
        if not snare:
            s_pat = "." * 16
        if fill:
            s_pat = merge(s_pat, FILLS[fill])
            if fill == "roll":
                h_pat = h_pat[:12] + "...."
        parts["kick"] += hits(k_pat) if kick else [(R, 16)]
        parts["snare"] += hits(s_pat)
        parts["hat"] += hits(h_pat) if hat else [(R, 16)]

        # Bass, arpeggio, pad.
        style = r["bass"]
        if section == "B":
            style = r.get("bass_b", BASS_B[style])
        elif section == "break":
            style = "thump" if i == 0 else "rebuild"
        elif section == "intro" and i == 0:
            style = "tick"
        if section == "turn" and i == 0:
            parts["bass"] += [(R, 16)]
        else:
            parts["bass"] += chordwise(chords, lambda c: bass_bar(style, c[0], c[1]))
        arp_on = section not in ("break",) and not (section == "turn" and i == 0)
        arp_oct = 1 if section == "A2" and not has_counter else 0
        parts["arp"] += chordwise(chords, lambda c: arp_bar(r["arp"], c[0], arp_oct)) if arp_on else [(R, 16)]
        if section in ("B", "A2", "break", "turn"):
            for root, _, cn in chords:
                tones = [d for d in range(0, 10) if pc(d, mode) in chord_pcs(root, mode)]
                if pad_prev is None:
                    pick = min(tones, key=lambda d: abs(d - (root + 2) % 7))
                else:
                    pick = min(tones, key=lambda d: (abs(midi(d, mode) - midi(pad_prev, mode)),
                                                     d != (root + 2) % 7))
                pad_prev = pick
                parts["pad"].append((pick, cn))
        else:
            parts["pad"].append((R, 16))

        # Lead.
        if section == "intro":
            parts["lead"] += [(home, 4), (R, 12)] if i == 0 else [(R, 16)]
        elif section == "A":
            parts["lead"] += a_body[i]
        elif section == "B":
            parts["lead"] += b_body[i]
        elif section == "A2":
            parts["lead"] += a2_body[i]
        elif section == "break":
            parts["lead"] += augment(fit(a_body[0], 8)) if i == 0 else [(R, 16)]
        elif section == "turn":
            parts["lead"] += fit(a_body[0], 8) + [(R, 8)] if i == 0 else [(R, 8)] + approach(mode, home)

    # The loop's last bar leads home, whatever section it is.
    if form[-1][0] != "turn":
        lead_st = to_steps(parts["lead"])
        tail = to_steps(approach(mode, home))
        lead_st[total - 8:] = tail
        parts["lead"] = from_steps(lead_st)
    written = parts["lead"]
    parts["lead"] = harmonize(written, chord_at, mode)
    moved = sum(1 for (a, _), (b, _) in zip(written, parts["lead"]) if a != b)

    # Bass anticipations: the next chord's note a sixteenth early.
    bass_st = to_steps(parts["bass"])
    for t in range(16, total + 1, 16):
        here, there = t - 1, t % total
        if (r["bass"] in PUSH and bars[here // 16][0] != "B" and chord_steps[here][1] != chord_steps[there][1]
                and bass_st[here][0] is not None and bass_st[there][0] is not None):
            bass_st[here] = (bass_st[there][0], True)
            if t < total:
                bass_st[there] = (bass_st[there][0], True)
    parts["bass"] = from_steps(bass_st)

    # Counter: under the recap only.
    lead_st = to_steps(parts["lead"])
    a2_start = 16 * next(k for k, b in enumerate(bars) if b[0] == "A2")
    counter_line = None
    if r["counter"] == "auto":
        counter_line = auto_counter(lead_st, chord_at, mode, a2_start, 64)
    elif r["counter"] == "canon":
        a2 = from_steps(lead_st[a2_start:a2_start + 64])
        counter_line = harmonize(shift(displace(a2, 4), -7), lambda t: chord_at(a2_start + t), mode)
        counter_line = repair_counter(counter_line, lead_st, chord_at, mode, a2_start)
    elif r["counter"]:
        body, _ = phrase(r["counter"], lead=False)
        line = [x for bar in body for x in bar]
        if not r["counter"].endswith("_5th"):
            line = counter_register(line, [x for bar in a2_body for x in bar])
        counter_line = harmonize(line, lambda t: chord_at(a2_start + t), mode)
        if not r["counter"].endswith("_5th"):
            counter_line = repair_counter(counter_line, lead_st, chord_at, mode, a2_start)
    parts["counter"] = [(R, a2_start)] + (counter_line or [(R, 64)]) + [(R, total - a2_start - 64)]

    return dict(recipe=r, form=form, parts=parts, chord_steps=chord_steps, total=total, moved=moved)


def to_abc(track) -> str:
    r, parts = track["recipe"], track["parts"]
    mode = r["mode"]
    registers = {"bass": E4 - 12, "arp": E4, "pad": E4 - 12, "lead": E4, "counter": E4}
    pal = PALETTES[r["palette"]]
    form = " ".join(f"{s}({n})" for s, n in track["form"])
    head = [
        "X:1",
        f"T:BRYX {r['level']:02d} {r['name']}",
        "C:BRYX (audio/compose.py)",
        "M:4/4",
        "L:1/16",
        f"Q:1/4={r['bpm']}",
        "K:C",
        f"% {r['note']}",
        f"% mode E {r['mode']}, progression {r['prog']}, form {form}",
        f"% A={r['a']} B={r['b']} counter={r['counter']}",
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


def render_line(line, mode, base):
    out, count = [], 0
    for p, s in line:
        if p is None:
            tok = "z"
        elif p == -1000:
            tok = "E,,"  # the kick's thump: a low triangle E
        elif p == -2000:
            tok = "c"  # noise: pitch is irrelevant
        else:
            tok = abc_pitch(midi(p, mode, base))
        out.append(tok + (str(s) if s > 1 else ""))
        count += s
        if count % 16 == 0:
            out.append("|")  # bar lines for readability (the engine ignores them)
    return "| " + " ".join(out)


# --------------------------------------------------------------- lint ---

def check_track(track):
    """The numbers that tell a written line from an accident: how often the
    lead's strong beats miss the chord unresolved, and, for the counter,
    how many of the attacks in its span are its own or the lead's alone
    (rhythmic independence), how often it crosses above the lead, meets it
    in a tritone on an attack, or moves with it in parallel fifths or
    octaves."""
    r, parts, total = track["recipe"], track["parts"], track["total"]
    mode = r["mode"]
    chord = [c[0] for c in track["chord_steps"]]
    lead = to_steps(parts["lead"])
    counter = to_steps(parts["counter"])

    played, t = [], 0
    for p, n in parts["lead"]:
        if p is not None:
            played.append([p, t, n])
        t += n
    strong = sum(1 for _, t, _ in played if t % 8 == 0)
    miss = sum(1 for k, (_, t, _) in enumerate(played)
               if t % 8 == 0 and not heard_right(played, k, lambda u: chord[u % total], mode))

    c_on = [t for t in range(total) if counter[t][1] and counter[t][0] is not None]
    span = range(c_on[0], c_on[-1] + 1) if c_on else range(0)
    l_on = {t for t in span if lead[t][1] and lead[t][0] is not None}
    shared = len(l_on & set(c_on))
    union = len(l_on | set(c_on))
    cross = tritone = parallel = 0
    prev = None
    for t in c_on:
        lp = lead[t][0]
        if lp is None:
            prev = None
            continue
        lm, cm = midi(lp, mode), midi(counter[t][0], mode)
        if cm > lm:
            cross += 1
        if (lm - cm) % 12 == 6 and (t % 8 == 0 or lead[t][1]):
            tritone += 1
        iv = (lm - cm) % 12
        if prev and iv in (0, 7) and iv == prev[0] and lm != prev[1] and cm != prev[2]:
            parallel += 1
        prev = (iv, lm, cm)
    return dict(strong=strong, miss=miss, c_on=len(c_on), shared=shared, union=union,
                cross=cross, tritone=tritone, parallel=parallel)


def check_score(tracks) -> bool:
    """Print the lint for every track; False if any track fails it. The
    one deliberate exception: a "_5th" counter is a doubling in parallel
    fifths by design (Twin towers), so its parallels and shared attacks
    are expected, but it must still never meet the lead in a tritone."""
    ok = True
    print("track                 lead: misses moved   counter: on shared cross tritone parallel")
    for t in tracks:
        r = t["recipe"]
        c = check_track(t)
        doubling = bool(r["counter"]) and r["counter"].endswith("_5th")
        fails = []
        if c["miss"] > 0.1 * c["strong"]:
            fails.append("lead misses the chord")
        if c["c_on"]:
            if c["tritone"]:
                fails.append("tritone against the lead")
            if not doubling:
                if c["union"] - c["shared"] < 0.2 * c["union"]:
                    fails.append("counter moves in the lead's rhythm")
                if c["cross"] > 0.25 * c["c_on"]:
                    fails.append("counter above the lead")
                if c["parallel"] > 2:
                    fails.append("parallel fifths or octaves")
        ok = ok and not fails
        print(f"{r['level']:02d} {r['name']:<18} {c['miss']:>6}/{c['strong']:<3} {t['moved']:>4}/{len(pitches(t['parts']['lead'])):<4}  "
              f"{c['c_on']:>3} {c['shared']:>6} {c['cross']:>5} {c['tritone']:>7} {c['parallel']:>8}"
              + ("   FAIL: " + "; ".join(fails) if fails else ""))
    return ok


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
    """`--check` runs the lint. `--render PATH` names a renderer built like
    build.rs's (see docs/design/music.md, "Changing the music"): the lint
    runs, then every track is mastered with it and written. The renderer
    is required: an unmastered score would clip."""
    import sys
    import tempfile

    tracks = [compose(r) for r in RECIPES] + [compose(MENU)]
    ok = check_score(tracks)
    if "--render" not in sys.argv:
        if "--check" not in sys.argv:
            raise SystemExit("usage: compose.py --check | --render <path to a built audio/render.c>")
        raise SystemExit(0 if ok else 1)
    if not ok:
        raise SystemExit("the score lint failed: nothing written")
    render = sys.argv[sys.argv.index("--render") + 1]
    with tempfile.TemporaryDirectory() as tmp:
        for t in tracks:
            level = t["recipe"]["level"]
            name = f"music_{level:02d}" if level else "music_menu"
            (OUT / f"{name}.abc").write_text(master(to_abc(t), render, pathlib.Path(tmp)))
    print(f"wrote {len(tracks)} mastered tracks to {OUT}")


if __name__ == "__main__":
    main()
