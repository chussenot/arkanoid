#!/usr/bin/env python3
"""BRYX's studio composer: the Phase 3 reference cues.

usage: audio/compose_studio.py [OUT_DIR]   (default audio/reference/cues)

Writes the six reference cues of docs/design/orchestral-score.md (Phase 3)
as `.score` files for the studio (audio/engine/studio.c):

    act1_warmup     Act I, sparse        level 01 Warm-up    E dorian 112
    act2_frame      Act II, melodic      level 09 Picture frame  E aeolian 120
    act3_armored    Act III, mechanical  level 22 Armored frame  E phrygian 132
    act4_vault      Act IV, monumental   level 32 Vault      E harmonic 144-150
    menu            the menu             E dorian 92
    act_clear       the act stinger      an act's last level cleared

Each cue is a real track of the score (`compose.py`'s RECIPES and MENU),
re-orchestrated, not rewritten: the same mode, tempo, form, progressions
and themes, imported from `compose.py` so the two cannot drift. A cue that
passes the listening gate becomes that track in Phase 4.

How the cues are written, and why:
- **Contrast before size.** Each act owns a palette and a dynamic
  ceiling (DYN): Act I is restrained electronic tension, Act II evolving
  orchestral movement, Act III mechanical precision, Act IV scale. Act
  IV's tutti is the only one, and it is preceded by a real pianissimo.
- **Melody goes to articulated voices.** VSCO has no legato transitions,
  so a theme is carried by tongued brass, pizzicato, spiccato or bells,
  and doubled only where the doubling is the colour (horns with violins
  an octave up). Sustained strings carry harmony, never an exposed line.
- **Swells are orchestration, not a sample getting louder.** A crescendo
  adds sections, moves from sustain to tremolo, opens a filter or rolls
  the timpani, with a gain ramp under it rather than instead of it.
- **The orchestra breathes; the machine does not.** Orchestral notes get
  a few milliseconds of deterministic timing and velocity spread
  (`human`); synths and electronic percussion stay on the grid.
- **The rally stays in key.** Harmony is each mode's own progression
  (compose.PROGS), and every strong-beat melody note is fitted to its
  chord by compose.harmonize, as in the chiptune score.

Deterministic: the same script writes the same bytes. Never run at build
time: its output is committed, and build.rs reads that.
"""

import hashlib
import os
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import compose as C  # noqa: E402  (the themes, modes and harmony)

OUT = HERE / "reference" / "cues"

# Velocities per dynamic. The samplers switch to their f layer at 80, so mf
# stays on the p layer (louder) and f crosses to the f layer.
DYN = {"ppp": 26, "pp": 36, "p": 50, "mp": 64, "mf": 76, "f": 92, "ff": 108, "fff": 120}

# Playable range per sampled instrument (from its map); a note outside is
# folded by octaves into it rather than lost.
RANGE = {
    "strings_high": (55, 89), "strings_mid": (48, 84), "strings_low": (36, 76),
    "strings_bass": (28, 55), "brass_horns": (36, 77), "brass_trumpets": (55, 84),
    "brass_trombones": (34, 67), "brass_low": (26, 55), "timpani": (39, 57),
    "timpani_roll": (39, 57),
}
for base in list(RANGE):
    for art in ("_spic", "_trem", "_pizz", "_stac"):
        RANGE[base + art] = RANGE[base]

# Instruments whose timing and velocity breathe (the orchestra).
ORCHESTRA = ("strings", "brass", "timpani", "percussion")


def human(inst, key, spread_beats):
    """A deterministic nudge: (beats, velocity) for note `key`."""
    if not inst.startswith(ORCHESTRA):
        return 0.0, 0
    h = hashlib.blake2b(f"{inst}:{key}".encode(), digest_size=4).digest()
    a = (h[0] / 255 - 0.5) * 2 * spread_beats
    v = int(round((h[1] / 255 - 0.5) * 8))
    return a, v


def fold(inst, m):
    lo, hi = RANGE.get(inst, (0, 127))
    while m < lo:
        m += 12
    while m > hi:
        m -= 12
    return m


class Score:
    def __init__(self, name, title, mode, bpm, bars, loop=True, tail=0.0, comment="",
                 libs=("desk", "orchestra", "electronic")):
        self.name, self.mode, self.bars = name, mode, bars
        self.head = [f"# {line}" for line in comment.strip().splitlines()]
        self.head += [
            "score 1", f"title {title}", f"# mode E {mode}", f"tempo 0 {bpm}",
            f"length {bars * 4}",
        ]
        self.head.append("loop on" if loop else f"tail {tail}")
        self.head += ["seed 1"] + [f"include {{INCLUDES}}/{lib}.inc" for lib in libs]
        self.extra, self.body = [], []
        self.swing = 0.0
        self.counter = 0
        self.head_ramps = []

    def trim(self, db):
        """The cue's overall level: where its act sits against the others
        (the master's gain lane, so the desk's compressor and limiter stay
        as the desk sets them)."""
        self.head_ramps = [f"ramp master.gain 0 0.001 {db:g} {db:g} lin"]

    def tempo(self, bar, bpm):
        self.head.append(f"tempo {bar * 4:g} {bpm:g}")

    def define(self, line):
        self.extra.append(line)

    def comment(self, text):
        self.body.append(f"\n# {text}")

    def at(self, bar, step=0.0):
        """Beats at a bar and a sixteenth step, swung."""
        beat = bar * 4 + step * 0.25
        if self.swing and int(step) % 2 == 1 and step == int(step):
            beat += (self.swing / 100 - 0.5) * 0.5
        return beat

    def note(self, beat, beats, inst, midi, vel, spread=0.012):
        self.counter += 1
        dt, dv = human(inst, self.counter, spread)
        beat = max(0.0, beat + dt)
        vel = max(1, min(127, vel + dv))
        midi = fold(inst, midi)
        self.body.append(f"note {beat:.4f} {beats:.4f} {inst} {midi} {vel}")

    def ramp(self, target, b0, b1, a, b, shape="smooth"):
        self.body.append(f"ramp {target} {b0:g} {b1:g} {a:g} {b:g} {shape}")

    def write(self, out):
        out.mkdir(parents=True, exist_ok=True)
        # Includes resolve against the score's own directory.
        libs = os.path.relpath(HERE / "instruments", out.resolve())
        head = [line.replace("{INCLUDES}", libs) for line in self.head]
        text = "\n".join(head + self.extra + self.head_ramps + self.body) + "\n"
        (out / f"{self.name}.score").write_text(text)
        return len(self.body)


# ------------------------------------------------------------ material ---

def m(deg, mode, base=C.E4):
    return C.midi(deg, mode, base)


def chords(prog_text):
    """Per bar: list of (root, bass, steps)."""
    return C.parse_prog(prog_text)


def section_chords(track, name):
    a, b = C.PROGS[track["prog"]]
    A, B = chords(a), chords(b)
    return {
        "intro": [A[0], A[1]], "A": A, "A2": A, "B": B,
        "break": [A[0], A[3]], "turn": [A[2], A[3]],
    }[name]


def chord_at_fn(bars):
    """Step -> chord root, for compose.harmonize."""
    flat = []
    for bar in bars:
        for root, _, s in bar:
            flat += [root] * s
    return lambda t: flat[min(t, len(flat) - 1)]


def theme_bars(name, bars, mode, recap=False):
    """A theme as a four-bar phrase fitted to its chords."""
    body, home = C.phrase(name)
    if recap:
        body = C.climax(body)
    line = [n for bar in body for n in bar]
    line = C.harmonize(line, chord_at_fn(bars), mode)
    return C.bars_of(line), home


def play_line(sc, bar0, bars, inst, vel, base=C.E4, legato=0.9, accent=0, octave=0, spread=0.012,
              min_steps=0):
    """A line (bars of (degree, steps)) as notes. `legato` scales each
    note's length (tongued brass 0.85-0.92, detached 0.6); `accent` adds
    velocity on the downbeats."""
    for i, bar in enumerate(bars):
        step = 0
        for deg, s in bar:
            if deg is not None and s >= min_steps:
                v = vel + (accent if step % 16 == 0 else 0)
                sc.note(sc.at(bar0 + i, step), max(0.06, s * 0.25 * legato), inst,
                        m(deg, sc.mode, base) + 12 * octave, v, spread)
            step += s


def triad(root, mode, base):
    return [m(root + k, mode, base) for k in (0, 2, 4)]


def voice(prev, tones):
    """The voicing of `tones` (pitch classes) nearest `prev`."""
    out = []
    for p in prev:
        best = min((t + 12 * o for t in tones for o in range(-2, 3)), key=lambda c: abs(c - p))
        out.append(best)
    return sorted(out)


def pad(sc, bar0, bars, inst, vel, start=(52, 55, 59), legato=1.0, split=False):
    """Held chords, voice-led. `split` restarts each half-bar chord."""
    prev = list(start)
    t = bar0 * 4
    for bar in bars:
        for root, _, s in bar:
            tones = {x % 12 for x in triad(root, sc.mode, 52)}
            prev = voice(prev, tones)
            for p in prev:
                sc.note(t, s * 0.25 * legato, inst, p, vel)
            t += s * 0.25
    return prev


def bass_line(sc, bar0, bars, inst, vel, pattern, base=40, legato=0.8, spread=0.0):
    """`pattern`: steps per note and an octave map, e.g. [(4,0),(4,1)]."""
    t = bar0 * 4
    for bar in bars:
        for _root, bass, s in bar:
            pos = 0
            k = 0
            while pos < s:
                ln, octv = pattern[k % len(pattern)]
                ln = min(ln, s - pos)
                sc.note(t + pos * 0.25, ln * 0.25 * legato, inst,
                        m(bass, sc.mode, base) + 12 * octv, vel + (8 if pos == 0 else 0), spread)
                pos += ln
                k += 1
            t += s * 0.25


def arp(sc, bar0, bars, inst, vel, shape, base=C.E4, step=1, legato=0.8, accent=10):
    """Chord tones in a fixed shape (indices into root, 3rd, 5th, octave,
    10th), one per `step` sixteenths: the machine's sequence."""
    t = bar0 * 4
    for bar in bars:
        for root, _, s in bar:
            tones = [m(root + d, sc.mode, base) for d in (0, 2, 4, 7, 9)]
            for k in range(0, s, step):
                idx = shape[(k // step) % len(shape)]
                if idx is None:
                    continue
                sc.note(t + k * 0.25, step * 0.25 * legato, inst, tones[idx],
                        vel + (accent if k % 4 == 0 else 0), 0.0)
            t += s * 0.25


def wall(sc, bar0, bars, inst, vel, base=40, legato=0.7, accent=14, spread=0.006, shape=None):
    """The Wall's ostinato on each chord's root: the machine figure."""
    figure = shape or C.WALL
    t = bar0 * 4
    for bar in bars:
        for root, _, s in bar:
            pos = 0
            while pos < s:
                for deg, ln in figure:
                    if pos >= s:
                        break
                    ln = min(ln, s - pos)
                    sc.note(t + pos * 0.25, ln * 0.25 * legato, inst, m(root + deg, sc.mode, base),
                            vel + (accent if pos % 8 == 0 else 0), spread)
                    pos += ln
            t += s * 0.25


def hits(sc, bar0, nbars, inst, midi, vel, pattern, accent=0, length=0.2, spread=0.0):
    """`pattern`: 16 characters per bar, 'x' a hit, 'X' accented, '.' rest."""
    for b in range(nbars):
        for k, ch in enumerate(pattern):
            if ch in "xX":
                sc.note(sc.at(bar0 + b, k), length, inst, midi, vel + (accent if ch == "X" else 0), spread)


def approach_line(mode, home):
    return C.approach(mode, home)


# ---------------------------------------------------------------- cues ---

def track(level):
    if level == 0:
        return C.MENU
    return next(t for t in C.RECIPES if t["level"] == level)


def act1_warmup():
    t = track(1)
    sc = Score("act1_warmup", "Act I reference: Warm-up", t["mode"], t["bpm"], 16, comment="""
Phase 3 reference cue, Act I (Foundations): level 01, Warm-up. E dorian,
112 BPM, swing 54. Form: intro 2, A 4 (Spark), B 4 (Ascent's head, a
promise), A' 4, turn 2. Restrained electronic tension: a drone, a soft
pulse, a bell arpeggio; strings in tremolo and pizzicato; one soft horn.
Dynamic ceiling mf. Written by audio/compose_studio.py: do not edit.""")
    sc.trim(-0.5)
    sc.swing = t["swing"]
    mode = t["mode"]
    form = [("intro", 2), ("A", 4), ("B", 4), ("A2", 4), ("turn", 2)]
    bar = 0
    sc.comment("The floor under everything: a breathing drone, air at the edges.")
    sc.note(0, 64, "electronic_drone", 28, DYN["mp"])
    sc.note(0, 8, "electronic_air", 69, DYN["p"])
    sc.note(56, 8, "electronic_air", 69, DYN["p"])
    for name, n in form:
        bars = section_chords(t, name)
        sc.comment(f"{name}: bars {bar + 1}-{bar + n}")
        if name == "intro":
            arp(sc, bar, bars, "synth_glass", DYN["p"], [0, None, 2, None, 3, None, 2, None], base=76, step=2)
            pad(sc, bar, bars, "strings_mid_trem", DYN["pp"], start=(52, 59, 67))
            sc.ramp("strings_mid_trem.gain", bar * 4, bar * 4 + 8, -14, 0)
            hits(sc, bar + 1, 1, "electronic_pulse", 28, DYN["mp"], "x.......x.......")
        elif name in ("A", "A2"):
            recap = name == "A2"
            lead, _ = theme_bars(t["a"], bars, mode, recap)
            # Spark: pizzicato violins, a bell doubling an octave up.
            play_line(sc, bar, lead, "strings_high_pizz", DYN["f"] if recap else DYN["mf"], legato=1.0)
            play_line(sc, bar, lead, "synth_glass", DYN["p"], octave=1, legato=0.7)
            pad(sc, bar, bars, "strings_mid_trem" if not recap else "strings_high_trem",
                DYN["pp"], start=(55, 59, 64))
            pad(sc, bar, bars, "strings_low", DYN["pp"], start=(40, 47))
            hits(sc, bar, n, "electronic_pulse", 28, DYN["mp"], "x.......x.......")
            if recap:
                # The recap adds a walking pizzicato bass and the bell
                # arpeggio, never the brass: Act I stays small.
                bass_line(sc, bar, bars, "strings_low_pizz", DYN["mf"], [(4, 0), (4, 1)], base=40, legato=1.0)
                arp(sc, bar, bars, "synth_glass", DYN["pp"], [0, 2, 3, 2], base=76, step=2)
                hits(sc, bar, n, "electronic_metal", 76, DYN["p"], "....x.......x...")
        elif name == "B":
            lead, _ = theme_bars(t["b"], bars, mode)
            # Ascent's head, a promise: one soft horn, tongued.
            play_line(sc, bar, lead, "brass_horns", DYN["mp"], base=C.E4 - 12, legato=0.88)
            pad(sc, bar, bars, "strings_mid", DYN["p"], start=(52, 57, 64))
            pad(sc, bar, bars, "synth_pad", DYN["p"], start=(52, 59, 64))
            bass_line(sc, bar, bars, "strings_bass", DYN["p"], [(16, 0)], base=40, legato=1.0)
            sc.note(bar * 4, 1, "timpani", 40, DYN["p"])
            arp(sc, bar, bars, "synth_glass", DYN["pp"], [3, None, None, None, 2, None, None, None], base=76, step=2)
            hits(sc, bar, n, "electronic_pulse", 28, DYN["p"], "x...............")
        elif name == "turn":
            # A breath: the drone, the head of Spark on the bell, then the
            # lead walks on to E as the loop restarts.
            head = C.fit(C.THEMES[t["a"]][0], 8)
            play_line(sc, bar, [head + [(None, 8)]], "synth_glass", DYN["mp"], legato=0.7)
            ap = approach_line(mode, 7)
            play_line(sc, bar + 1, [[(None, 8)] + ap], "strings_high_pizz", DYN["mp"], legato=1.0)
            pad(sc, bar, bars, "strings_mid_trem", DYN["pp"], start=(55, 59, 64))
            sc.note(bar * 4 + 4, 4, "timpani_roll", 40, DYN["p"])
            sc.ramp("timpani_roll.gain", bar * 4 + 4, bar * 4 + 8, -20, -6)
        bar += n
    return sc


def act2_frame():
    t = track(9)
    sc = Score("act2_frame", "Act II reference: Picture frame", t["mode"], t["bpm"], 16, comment="""
Phase 3 reference cue, Act II (Structures): level 09, Picture frame. E
aeolian, 120 BPM. Form: A 4 (Ascent), B 4 (Spark, augmented), break 2,
A' 4 (Ascent over itself in long notes), turn 2. Evolving orchestral
movement: the full string section, horns carrying Ascent, the first low
brass, a driving sequence and a moving bass. Dynamic ceiling f.
Written by audio/compose_studio.py: do not edit.""")
    sc.trim(-1.5)
    mode = t["mode"]
    form = [("A", 4), ("B", 4), ("break", 2), ("A2", 4), ("turn", 2)]
    bar = 0
    for name, n in form:
        bars = section_chords(t, name)
        b0 = bar * 4
        sc.comment(f"{name}: bars {bar + 1}-{bar + n}")
        if name in ("A", "A2"):
            recap = name == "A2"
            lead, _ = theme_bars(t["a"], bars, mode, recap)
            # Ascent: horns, tongued, with violins an octave up.
            play_line(sc, bar, lead, "brass_horns", DYN["ff"] if recap else DYN["f"],
                      base=C.E4 - 12, legato=0.9, accent=6)
            play_line(sc, bar, lead, "strings_high", DYN["f"] if recap else DYN["mf"], legato=0.95)
            # Moving bass: the synth on eighths, celli spiccato with it.
            bass_line(sc, bar, bars, "synth_bass", DYN["mf"], [(2, 0), (2, 1)], base=28, legato=0.7)
            bass_line(sc, bar, bars, "strings_low_spic", DYN["mp"], [(2, 0), (2, 1)], base=40, legato=0.6,
                      spread=0.008)
            pad(sc, bar, bars, "strings_mid", DYN["p"], start=(52, 55, 59))
            arp(sc, bar, bars, "synth_sequence", DYN["p"], [0, 3, 2, 3, 1, 3, 2, 4], base=64)
            sc.ramp("synth_sequence.cutoff", b0, b0 + 16, -0.8 if not recap else 0.0, 0.4)
            hits(sc, bar, n, "electronic_pulse", 28, DYN["mp"], "x...x...x...x...")
            if recap:
                # The frame: Ascent in long notes beneath its own return.
                counter, _ = theme_bars(t["counter"], bars, mode)
                play_line(sc, bar, counter, "strings_low", DYN["mf"], base=C.E4 - 24, legato=1.0)
                pad(sc, bar, bars, "brass_trombones", DYN["mp"], start=(47, 52, 55))
                for k in range(n):
                    sc.note((bar + k) * 4, 1, "timpani", 40, DYN["mf"])
                sc.note((bar + n - 1) * 4, 4, "percussion", 52, DYN["mf"])  # a cymbal swell to the turn
        elif name == "B":
            lead, _ = theme_bars(t["b"], bars, mode)
            # Spark, stretched: violins detached, the bell above them.
            play_line(sc, bar, lead, "strings_high", DYN["mf"], legato=0.7)
            play_line(sc, bar, lead, "synth_glass", DYN["p"], octave=1, legato=0.6)
            # The first low brass: trombones and tuba, entering soft and
            # growing by section, not by sample.
            pad(sc, bar, bars, "brass_trombones", DYN["p"], start=(47, 52, 55))
            bass_line(sc, bar, bars, "brass_low", DYN["mp"], [(8, 0)], base=40, legato=0.9)
            sc.ramp("brass_trombones.gain", b0, b0 + 16, -10, 0)
            pad(sc, bar, bars, "strings_mid_trem", DYN["p"], start=(55, 60, 64))
            bass_line(sc, bar, bars, "synth_bass", DYN["mf"], [(4, 0), (2, 1), (2, 0)], base=28, legato=0.7)
            arp(sc, bar, bars, "synth_sequence", DYN["p"], [0, None, 3, None, 2, None, 4, None], base=64)
            hits(sc, bar, n, "electronic_pulse", 28, DYN["mf"], "x.......x.......")
            sc.note(b0 + 12, 4, "timpani_roll", 40, DYN["mf"])
            sc.ramp("timpani_roll.gain", b0 + 12, b0 + 16, -18, 0)
        elif name == "break":
            # The floor drops out: pad, the theme's head in long notes on
            # a solo horn line under tremolo, one bass thump a bar.
            head = C.augment(C.fit(C.THEMES[t["a"]][0], 8))
            play_line(sc, bar, [C.fit(head, 16), [(None, 16)]], "brass_horns", DYN["p"], base=C.E4 - 12,
                      legato=0.92)
            pad(sc, bar, bars, "synth_pad", DYN["p"], start=(52, 55, 59))
            pad(sc, bar, bars, "strings_high_trem", DYN["ppp"], start=(64, 67, 71))
            for k in range(n):
                sc.note((bar + k) * 4, 1.5, "strings_low_pizz", 40, DYN["mp"])
                sc.note((bar + k) * 4, 0.5, "synth_bass", 28, DYN["p"])
            # then the bass rebuilds under a hat
            hits(sc, bar + 1, 1, "electronic_metal", 76, DYN["p"], "..x...x...x...x.")
        elif name == "turn":
            head = C.fit(C.THEMES[t["a"]][0], 8)
            play_line(sc, bar, [head + [(None, 8)]], "brass_horns", DYN["mf"], base=C.E4 - 12, legato=0.88)
            play_line(sc, bar + 1, [[(None, 8)] + approach_line(mode, 7)], "strings_high", DYN["mf"], legato=0.8)
            pad(sc, bar, bars, "strings_mid", DYN["mp"], start=(52, 55, 59))
            bass_line(sc, bar + 1, bars[1:], "synth_bass", DYN["f"], [(2, 0), (2, 1)], base=28, legato=0.7)
            arp(sc, bar + 1, bars[1:], "synth_sequence", DYN["mp"], [0, 3, 2, 3], base=64)
        bar += n
    return sc



def chops(sc, bar0, bars, inst, vel, base=40, pattern="..x...x...x...x.", spread=0.004):
    """Short notes on each chord's fifth, on the off-beats: gears that
    turn against the bass instead of doubling it."""
    t = bar0 * 4
    for bar in bars:
        for root, _, s in bar:
            for k in range(s):
                if pattern[k % 16] in "xX":
                    sc.note(t + k * 0.25, 0.18, inst, m(root + 4, sc.mode, base), vel, spread)
            t += s * 0.25


def act3_armored():
    t = track(22)
    sc = Score("act3_armored", "Act III reference: Armored frame", t["mode"], t["bpm"], 16, comment="""
Phase 3 reference cue, Act III (Machines): level 22, Armored frame. E
phrygian, 132 BPM. Form: intro 2, A 4 (the Wall high, ratcheting up), B 4
(the Wall inverted, sinking), break 2, A' 4 (the inverted Wall at half
speed beneath the high one). Mechanical precision, written by register so
nothing muddies: the saw bass alone owns the Wall down low, celli chop
off-beats against it, the high Wall is bright spiccato, the sequence
carries the motion, low brass only punctuates. Act IV keeps the scale.
Written by audio/compose_studio.py: do not edit.""")
    sc.trim(-1.5)
    mode = t["mode"]
    # A pad that breathes with the pulse, kept above the bass's octave.
    sc.define("bus pump gain=-2 comp=-28:6:0.002:0.14 key=electronic_pulse send=hall:-14")
    sc.define("inst pad_pumped synth bus=pump gain=-14 send=long:-16 wave=saw stack=5 detune=12 spread=0.8 "
              "filter=lp cutoff=1600 res=0.15 keytrack=0.3 attack=0.3 decay=0.5 sustain=0.9 release=0.8")
    form = [("intro", 2), ("A", 4), ("B", 4), ("break", 2), ("A2", 4)]
    bar = 0
    for name, n in form:
        bars = section_chords(t, name)
        b0 = bar * 4
        sc.comment(f"{name}: bars {bar + 1}-{bar + n}")
        # Under everything: the Wall in the saw bass alone, on the grid.
        wall(sc, bar, bars, "synth_bass", DYN["f"], base=28, legato=0.6, spread=0.0)
        if name == "intro":
            # The machine starting: the sequence with its filter shut,
            # opening; metal; the pulse arrives in the second bar.
            arp(sc, bar, bars, "synth_sequence", DYN["mf"], [0, 3, 1, 3, 2, 3, 1, 4], base=64)
            sc.ramp("synth_sequence.cutoff", b0, b0 + 8, -1.6, 0)
            sc.ramp("synth_bass.cutoff", b0, b0 + 8, -1.2, 0)
            hits(sc, bar, n, "electronic_metal", 76, DYN["mp"], "x.x.x.x.x.x.x.x.")
            hits(sc, bar + 1, 1, "electronic_pulse", 28, DYN["f"], "x...x...x...x...")
        if name in ("A", "B", "A2"):
            arp(sc, bar, bars, "synth_sequence", DYN["mf"] if name != "B" else DYN["mp"],
                [0, 3, 1, 3, 2, 3, 1, 4], base=64)
            chops(sc, bar, bars, "strings_low_spic", DYN["mf"], base=40)
            hits(sc, bar, n, "electronic_pulse", 28, DYN["f"], "x...x...x...x...")
            hits(sc, bar, n, "electronic_metal", 76, DYN["mp"], "..x...x...x...x.")
            pad(sc, bar, bars, "pad_pumped", DYN["mp"], start=(55, 59, 64))
            # Timpani mark each two-bar phrase; low brass stabs close the
            # four-bar phrase, the only low brass in A and B.
            for k in (0, 2):
                sc.note((bar + k) * 4, 0.8, "timpani", m(bars[k][0][0], mode, 40), DYN["f"])
            last = bars[n - 1][0][0]
            for off in (2.5, 3.0, 3.5):
                sc.note((bar + n - 1) * 4 + off, 0.25, "brass_trombones_stac", m(last, mode, 52), DYN["f"])
        if name == "A":
            lead, _ = theme_bars(t["a"], bars, mode)
            # The high Wall, bright and short: violas and violins spiccato.
            play_line(sc, bar, lead, "strings_mid_spic", DYN["f"], legato=0.5, spread=0.004)
            play_line(sc, bar, lead, "strings_high_spic", DYN["mf"], octave=1, legato=0.5, spread=0.004)
            sc.ramp("synth_sequence.cutoff", b0, b0 + 16, 0, 0.8)
        elif name == "B":
            lead, _ = theme_bars(t["b"], bars, mode)
            # The Wall inverted and sinking, on the horns in their middle:
            # a voice the machine cannot cover.
            play_line(sc, bar, lead, "brass_horns", DYN["f"], legato=0.8, accent=6)
            sc.ramp("synth_sequence.cutoff", b0, b0 + 16, 0.4, -0.6)
        elif name == "break":
            # Down to the machine alone: bass, sequence, metal; the
            # sequence's filter closes and reopens.
            arp(sc, bar, bars, "synth_sequence", DYN["mf"], [0, 3, 1, 3, 2, 3, 1, 4], base=64)
            sc.ramp("synth_sequence.cutoff", b0, b0 + 8, -1.8, 0.6)
            sc.ramp("synth_bass.cutoff", b0, b0 + 8, -1.6, 0.4)
            hits(sc, bar, n, "electronic_metal", 76, DYN["mp"], "x.x.x.x.x.x.x.x.")
            hits(sc, bar + 1, 1, "electronic_pulse", 28, DYN["mf"], "x...x...x...x...")
        elif name == "A2":
            lead, _ = theme_bars(t["a"], bars, mode, recap=True)
            play_line(sc, bar, lead, "strings_mid_spic", DYN["f"], legato=0.5, spread=0.004)
            play_line(sc, bar, lead, "strings_high_spic", DYN["f"], octave=1, legato=0.5, spread=0.004)
            # Beneath it the inverted Wall at half speed: trombones, held
            # in their middle, above the bass's octave.
            counter, _ = theme_bars(t["counter"], bars, mode)
            play_line(sc, bar, counter, "brass_trombones", DYN["mf"], base=C.E4 - 12, legato=0.85)
            sc.ramp("synth_sequence.cutoff", b0, b0 + 16, 0.4, 1.2)
        bar += n
    return sc


def act4_vault():
    t = track(32)
    sc = Score("act4_vault", "Act IV reference: Vault", t["mode"], t["bpm"], 16, comment="""
Phase 3 reference cue, Act IV (Vault): level 32, Vault, the finale. E
harmonic minor, 144 BPM pushing to 150. Form: A 4 (Ascent over the Wall,
the full brass statement held back since Act I), B 4 (Spark), A' 4 (all
three: the Wall below, Ascent above, Spark answering between its
phrases), break 2 (a real pianissimo), turn 2 (the timpani rebuild into
the restart). The score's only tutti, written to be heard: one melody at
a time, one owner per register, size from doubling and register rather
than from the number of parts. Written by audio/compose_studio.py: do
not edit.""")
    sc.trim(ACT_TRIM[4])
    mode = t["mode"]
    for k, bpm in [(8, 146), (9, 147), (10, 148), (11, 150), (12, 144)]:
        sc.tempo(k, bpm)
    form = [("A", 4), ("B", 4), ("A2", 4), ("break", 2), ("turn", 2)]
    bar = 0
    for name, n in form:
        bars = section_chords(t, name)
        b0 = bar * 4
        sc.comment(f"{name}: bars {bar + 1}-{bar + n}")
        if name in ("A", "B", "A2"):
            # The ground: the Wall in the saw bass, the celli an octave up
            # for its edge; the basses join only for the recap.
            wall(sc, bar, bars, "synth_bass", DYN["f"], base=28, legato=0.6, spread=0.0)
            wall(sc, bar, bars, "strings_low_spic", DYN["f"] if name == "A2" else DYN["mf"],
                 base=40, legato=0.55, spread=0.004)
            hits(sc, bar, n, "electronic_pulse", 28, DYN["f"], "x...x...x...x...")
            for k in (0, 2):
                sc.note((bar + k) * 4, 0.8, "timpani", m(bars[k][0][0], mode, 40),
                        DYN["ff"] if name == "A2" else DYN["f"])
        if name == "A":
            lead, _ = theme_bars(t["a"], bars, mode)
            # The full brass statement: trumpets, horns an octave below,
            # trombones holding the harmony. Nothing else above the bass.
            play_line(sc, bar, lead, "brass_trumpets", DYN["f"], legato=0.88, accent=6)
            play_line(sc, bar, lead, "brass_horns", DYN["ff"], base=C.E4 - 12, legato=0.9, accent=6)
            pad(sc, bar, bars, "brass_trombones", DYN["mf"], start=(47, 52, 55))
            sc.note(b0 + 12, 4, "percussion", 53, DYN["f"])  # cymbal swell into B
        elif name == "B":
            lead, _ = theme_bars(t["b"], bars, mode)
            # Spark: violins spiccato with the bell; the sequence is the
            # machine's motion; tremolo strings hold the harmony.
            play_line(sc, bar, lead, "strings_high_spic", DYN["f"], legato=0.6, spread=0.004)
            play_line(sc, bar, lead, "synth_glass", DYN["mp"], octave=1, legato=0.6)
            arp(sc, bar, bars, "synth_sequence", DYN["mp"], [0, 3, 2, 4, 1, 3, 2, 3], base=64)
            pad(sc, bar, bars, "strings_mid_trem", DYN["mf"], start=(55, 59, 64))
            sc.note(b0 + 12, 4, "timpani_roll", 40, DYN["f"])
            sc.ramp("timpani_roll.gain", b0 + 12, b0 + 16, -16, 0)
            sc.note(b0 + 12, 4, "percussion", 54, DYN["ff"])  # the big swell into A'
        elif name == "A2":
            lead, _ = theme_bars(t["a"], bars, mode, recap=True)
            # All three, readable: Ascent above on the brass, the Wall
            # below (the basses join), and Spark as the bell answering in
            # the second half of every bar, where Ascent holds.
            play_line(sc, bar, lead, "brass_trumpets", DYN["ff"], legato=0.88, accent=6)
            play_line(sc, bar, lead, "brass_horns", DYN["ff"], base=C.E4 - 12, legato=0.9, accent=6)
            pad(sc, bar, bars, "brass_trombones", DYN["f"], start=(47, 52, 55))
            wall(sc, bar, bars, "strings_bass_spic", DYN["f"], base=40, legato=0.55, spread=0.004)
            spark, _ = theme_bars(t["counter"], bars, mode)
            answers = [C.fit([(None, 8)] + C.cut(b, 8, 16), 16) for b in spark]
            play_line(sc, bar, answers, "synth_glass", DYN["mf"], octave=1, legato=0.6)
            sc.note(b0, 6, "percussion", 55, DYN["ff"])  # the gong on the arrival
            sc.note(b0, 1, "percussion", 49, DYN["ff"])
        elif name == "break":
            # The real pianissimo: tremolo strings, the drone, air, and
            # Ascent's head far away on one horn.
            pad(sc, bar, bars, "strings_high_trem", DYN["ppp"], start=(64, 67, 71))
            pad(sc, bar, bars, "strings_low", DYN["ppp"], start=(40, 47))
            sc.note(b0, 8, "electronic_drone", 28, DYN["pp"])
            sc.note(b0, 8, "electronic_air", 69, DYN["pp"])
            head = C.fit(C.THEMES[t["a"]][0], 8)
            play_line(sc, bar, [head + [(None, 8)]], "brass_horns", DYN["pp"], base=C.E4 - 12, legato=0.9)
        elif name == "turn":
            # The rebuild: a timpani roll from nothing, the Wall returning
            # in the bass, the brass walking on to E for the restart.
            sc.note(b0, 8, "timpani_roll", 40, DYN["ff"])
            sc.ramp("timpani_roll.gain", b0, b0 + 8, -30, 0)
            wall(sc, bar + 1, bars[1:], "synth_bass", DYN["f"], base=28, legato=0.6, spread=0.0)
            pad(sc, bar, bars, "strings_mid_trem", DYN["mf"], start=(55, 59, 64))
            sc.ramp("strings_mid_trem.gain", b0, b0 + 8, -18, 0)
            play_line(sc, bar + 1, [[(None, 8)] + approach_line(mode, 7)], "brass_trumpets", DYN["f"], legato=0.85)
            play_line(sc, bar + 1, [[(None, 8)] + approach_line(mode, 7)], "brass_horns", DYN["f"],
                      base=C.E4 - 12, legato=0.85)
            sc.note(b0 + 4, 4, "percussion", 54, DYN["f"])
        bar += n
    return sc


def menu():
    t = track(0)
    sc = Score("menu", "Menu reference", t["mode"], t["bpm"], 16, comment="""
Phase 3 reference cue: the menu. E dorian, 92 BPM. Form: intro 2, A 4
(Ascent at half speed, the first thing a player hears), B 4 (Spark,
stretched), A' 4 (Ascent again with Spark answering), turn 2. An
invitation, not a fanfare: horns and a soft synth lead share Ascent,
strings hold the harmony, a quiet sequence keeps time.
Written by audio/compose_studio.py: do not edit.""")
    sc.trim(-1.0)
    mode = t["mode"]
    form = [("intro", 2), ("A", 4), ("B", 4), ("A2", 4), ("turn", 2)]
    bar = 0
    sc.note(0, 64, "electronic_drone", 28, DYN["mp"])
    for name, n in form:
        bars = section_chords(t, name)
        b0 = bar * 4
        sc.comment(f"{name}: bars {bar + 1}-{bar + n}")
        if name == "intro":
            sc.note(b0, 8, "electronic_air", 69, DYN["mp"])
            arp(sc, bar, bars, "synth_glass", DYN["p"], [0, 2, 3, 2], base=76, step=2)
            pad(sc, bar, bars, "synth_pad", DYN["mp"], start=(52, 59, 64))
        elif name in ("A", "A2"):
            recap = name == "A2"
            lead, _ = theme_bars(t["a"], bars, mode, recap)
            # Ascent, augmented: horns tongued, a soft synth lead above
            # them carrying the connection the samples cannot.
            play_line(sc, bar, lead, "brass_horns", DYN["mf"] if recap else DYN["mp"], base=C.E4 - 12,
                      legato=0.9)
            play_line(sc, bar, lead, "synth_lead", DYN["p"], legato=1.0)
            pad(sc, bar, bars, "strings_mid", DYN["p"], start=(52, 57, 64))
            pad(sc, bar, bars, "strings_low", DYN["p"], start=(40, 47))
            arp(sc, bar, bars, "synth_sequence", DYN["p"], [0, None, 3, None, 2, None, 3, None], base=64)
            if recap:
                spark, _ = theme_bars(t["counter"], bars, mode)
                play_line(sc, bar, spark, "synth_glass", DYN["mp"], octave=1, legato=0.6)
                bass_line(sc, bar, bars, "synth_bass", DYN["mp"], [(4, 0), (4, 0), (4, 1), (4, 0)], base=28)
                sc.note(b0, 1, "timpani", 40, DYN["mp"])
        elif name == "B":
            lead, _ = theme_bars(t["b"], bars, mode)
            play_line(sc, bar, lead, "strings_high_pizz", DYN["mf"], legato=1.0)
            play_line(sc, bar, lead, "synth_glass", DYN["p"], octave=1, legato=0.6)
            bass_line(sc, bar, bars, "strings_low_pizz", DYN["mp"], [(4, 0), (4, 1)], base=40, legato=1.0)
            pad(sc, bar, bars, "synth_pad", DYN["mp"], start=(52, 59, 64))
            pad(sc, bar, bars, "strings_high_trem", DYN["pp"], start=(64, 67, 71))
        elif name == "turn":
            head = C.fit(C.THEMES["ascent"][0], 8)
            play_line(sc, bar, [head + [(None, 8)]], "brass_horns", DYN["mp"], base=C.E4 - 12, legato=0.9)
            play_line(sc, bar + 1, [[(None, 8)] + approach_line(mode, 7)], "synth_lead", DYN["mp"], legato=1.0)
            pad(sc, bar, bars, "synth_pad", DYN["mp"], start=(52, 59, 64))
            sc.note(b0, 8, "electronic_air", 69, DYN["p"])
        bar += n
    return sc



def act_clear():
    sc = Score("act_clear", "Act clear", "aeolian", 160, 3, loop=False, tail=0.3, comment="""
Phase 3 reference cue: the act stinger (an act's last level cleared),
the chip score's act fanfare re-orchestrated: Spark bounding
up, Ascent's head over a bass walking up to E, a held E minor. Its
orchestra previews Act II's (strings and horns). Three bars, one idea
each, few parts, so it reads at a glance over gameplay. Written by
audio/compose_studio.py: do not edit.""")
    mode = "aeolian"
    spark = [(4, 1), (None, 1), (6, 1), (7, 1), (None, 1), (9, 2), (None, 1),
             (10, 1), (9, 1), (7, 1), (None, 1), (6, 2), (None, 2)]
    head = [(7, 2), (4, 2), (7, 2), (6, 2), (4, 2), (3, 2), (4, 2), (6, 2)]
    sc.comment("Bar 1: Spark bounding up, violins spiccato and the bell, a pizzicato bass.")
    play_line(sc, 0, [spark], "strings_high_spic", DYN["f"], legato=0.7, spread=0.004)
    play_line(sc, 0, [spark], "synth_glass", DYN["mf"], octave=1, legato=0.6)
    for k, deg in enumerate([0, 0, 2, 2]):
        sc.note(k, 0.9, "strings_low_pizz", m(deg, mode, 40), DYN["f"])
    sc.comment("Bar 2: Ascent's head on the horns, the celli walking A B C D up to E.")
    play_line(sc, 1, [head], "brass_horns", DYN["f"], base=C.E4 - 12, legato=0.88, accent=6)
    for k, deg in enumerate([3, 4, 5, 6]):
        sc.note(4 + k, 0.95, "strings_low", m(deg, mode, 40), DYN["f"])
    sc.note(4, 4, "timpani_roll", 40, DYN["f"])
    sc.ramp("timpani_roll.gain", 4, 8, -18, 0)
    sc.comment("Bar 3: the held E minor: high E over G and B in the strings, the horns' triad, the bass.")
    for inst, p in [("strings_high", 76), ("strings_high", 71), ("strings_mid", 67), ("strings_low", 52),
                    ("strings_bass", 40), ("brass_horns", 64), ("brass_horns", 59), ("brass_horns", 55),
                    ("synth_bass", 28)]:
        sc.note(8, 4, inst, p, DYN["f"])
    sc.note(8, 1, "timpani", 40, DYN["ff"])
    sc.note(8, 4, "percussion", 49, DYN["mf"])
    # The game holds the next act's music until the stinger ends, and the
    # level-end sweep keeps its three bars (4.5 s): the held chord fades
    # over the last beat and the short tail, instead of ringing on.
    sc.ramp("master.gain", 11, 12.8, 0, -40)
    return sc


# ------------------------------------------------------------ the score ---
# The other 28 tracks: compose.py's own parts (lead, counter, bass,
# arpeggio, pad, drums, as lines of degrees with a dynamic per step),
# orchestrated by act with the rules the reference cues proved. The notes
# and their rhythm are the score's; the studio decides who plays them.

# Each act's level against the others (dB on the master). Act IV stays
# 2.4 dB above anything in Act III and no more: in the band the effects
# live in (300 Hz-3 kHz) its brass already sits 2 dB over the old score,
# and gameplay sounds come first (docs/design/audio.md, "Balance").
ACT_TRIM = {1: -0.5, 2: -1.5, 3: -1.5, 4: 0.5}
LEVEL_NAMES = ["ppp", "pp", "p", "mp", "mf", "f", "ff", "fff"]
# Each act's dynamic shift against compose.py's (intro mp, A mf, B mf,
# break p, A' f, turn mp) and its ceiling.
ACT_SHIFT = {1: -1, 2: 0, 3: 0, 4: 1}
ACT_CEILING = {1: "mf", 2: "f", 3: "f", 4: "ff"}
UNPITCHED = ("percussion", "electronic_metal", "electronic_air", "electronic_pulse")

# Who carries a theme, by act and by its family: (instrument, octave,
# level offset, length factor). Melody goes to articulated voices.
LEAD = {
    1: {"spark": [("strings_high_pizz", 0, 1, 1.0), ("synth_glass", 1, -1, 0.7)],
        "ascent": [("brass_horns", -1, 0, 0.88), ("synth_glass", 0, -2, 0.7)],
        "wall": [("strings_mid_spic", 0, 1, 0.5), ("synth_glass", 1, -2, 0.6)]},
    2: {"spark": [("strings_high", 0, 1, 0.7), ("synth_glass", 1, -1, 0.6)],
        "ascent": [("brass_horns", -1, 1, 0.9), ("strings_high", 0, 0, 0.95)],
        "wall": [("strings_mid_spic", 0, 1, 0.5), ("strings_high_spic", 1, 0, 0.5)]},
    3: {"spark": [("strings_high_spic", 0, 1, 0.6), ("synth_glass", 1, -1, 0.6)],
        "ascent": [("brass_horns", 0, 1, 0.8)],
        "wall": [("strings_mid_spic", 0, 1, 0.5), ("strings_high_spic", 1, 0, 0.5)]},
    4: {"spark": [("strings_high_spic", 0, 1, 0.6), ("synth_glass", 1, -1, 0.6)],
        "ascent": [("brass_trumpets", 0, 1, 0.88), ("brass_horns", -1, 1, 0.9)],
        "wall": [("strings_mid_spic", 0, 1, 0.5), ("strings_high_spic", 1, 0, 0.5)]},
}
# A short variant (staccato, diminished) on its short instruments.
SHORT = {"brass_horns": "brass_horns_stac", "brass_trumpets": "brass_trumpets_stac",
         "strings_high": "strings_high_spic"}
# The counter-line under the recap.
COUNTER = {
    1: {"spark": ("synth_glass", 1, -1, 0.6), "ascent": ("strings_low", 0, -1, 0.95),
        "wall": ("strings_low", 0, -1, 0.95), "auto": ("strings_mid", 0, -1, 0.95)},
    2: {"spark": ("synth_glass", 1, -1, 0.6), "ascent": ("strings_low", 0, -1, 0.95),
        "wall": ("strings_low", 0, -1, 0.95), "auto": ("strings_mid", 0, -1, 0.95)},
    3: {"spark": ("strings_mid_spic", 0, -1, 0.5), "ascent": ("brass_trombones", 0, -1, 0.85),
        "wall": ("brass_trombones", 0, -1, 0.85), "auto": ("brass_trombones", 0, -1, 0.85)},
    4: {"spark": ("synth_glass", 1, -1, 0.6), "ascent": ("brass_trombones", 0, -1, 0.85),
        "wall": ("brass_trombones", 0, -1, 0.85), "auto": ("brass_trombones", 0, -1, 0.85),
        "canon": ("synth_glass", 1, -1, 0.6)},
}
ARP = {"pulse": "synth_sequence", "buzz": "synth_sequence", "soft": "synth_sequence", "glass": "synth_glass"}


def lvl(name, shift):
    i = LEVEL_NAMES.index(name) + shift
    return LEVEL_NAMES[max(0, min(len(LEVEL_NAMES) - 1, i))]


def capped(name, act, shift=0):
    v = lvl(name, ACT_SHIFT[act] + shift)
    cap = ACT_CEILING[act]
    if LEVEL_NAMES.index(v) > LEVEL_NAMES.index(cap) + max(0, shift):
        v = lvl(cap, max(0, shift))
    return DYN[v]


def events(line):
    """(step, degree or chord, steps) for each sounding note of a line."""
    t = 0
    for p, s in line:
        if p is not None:
            yield t, p, s
        t += s


def family(theme):
    return theme.split("_")[0] if theme not in ("auto", "canon") else theme


def arrange(level):
    r = track(level)
    tr = C.compose(r)
    act = C.act_of(level)
    mode = r["mode"]
    total = tr["total"]
    nbars = total // 16
    sc = Score(f"music_{level:02d}", f"BRYX {level:02d} {r['name']}", mode, r["bpm"], nbars, comment=f"""
Level {level:02d}, {r['name']}: {r['note']}
E {mode}, {r['bpm']} BPM; Act {act}; A {r['a']}, B {r['b']}, counter {r['counter']}.
compose.py's parts, orchestrated for Act {act} by audio/compose_studio.py:
do not edit.""")
    sc.swing = r["swing"]
    sc.trim(ACT_TRIM[act])
    bars = tr["bars"]
    section_at = lambda t: bars[min(t // 16, nbars - 1)][0]
    dyn = tr["dyn"]
    parts = tr["parts"]
    beat = lambda t: sc.at(t // 16, t % 16)
    start = {}
    for k, (section, i, *_rest) in enumerate(bars):
        if i == 0:
            start[section] = k

    def emit(inst, t, deg, steps, base, octave, level, legato, spread=0.012):
        degs = deg if isinstance(deg, tuple) else (deg,)
        for d in degs:
            sc.note(beat(t), max(0.06, steps * 0.25 * legato), inst,
                    m(d, mode, base) + 12 * octave, level, spread)

    # Act III's pad breathes with the pulse.
    if act == 3:
        sc.define("bus pump gain=-2 comp=-28:6:0.002:0.14 key=electronic_pulse send=hall:-14")
        sc.define("inst pad_pumped synth bus=pump gain=-14 send=long:-16 wave=saw stack=5 detune=12 spread=0.8 "
                  "filter=lp cutoff=1600 res=0.15 keytrack=0.3 attack=0.3 decay=0.5 sustain=0.9 release=0.8")

    # Lead.
    for t, deg, steps in events(parts["lead"]):
        section = section_at(t)
        theme = r["b"] if section == "B" else r["a"]
        voices = LEAD[act][family(theme)]
        short = theme.endswith(("_stac", "_dim"))
        long_ = theme.endswith("_aug")
        if section == "intro" or (section == "break" and act == 1):
            voices = [("synth_glass", 0, 0, 0.7)]
        elif section == "break":
            voices = [("brass_horns", -1, -1, 0.92)]
        accent = 1 if t in tr["acc"]["lead"] else 0
        for inst, octv, off, legato in voices:
            if short:
                inst, legato = SHORT.get(inst, inst), 0.5
            emit(inst, t, deg, steps, C.E4, octv, capped(dyn["lead"][t], act, off + accent), legato)
        if long_ and act in (1, 2) and section in ("A", "B", "A2"):
            # Long notes: a soft synth lead carries the connection.
            emit("synth_lead", t, deg, steps, C.E4, 0, capped(dyn["lead"][t], act, -2), 1.0)

    # Counter, under the recap.
    if r["counter"]:
        kind = r["counter"] if r["counter"] in ("auto", "canon") else family(r["counter"])
        inst, octv, off, legato = COUNTER[act].get(kind, COUNTER[act]["auto"])
        for t, deg, steps in events(parts["counter"]):
            emit(inst, t, deg, steps, C.E4, octv, capped(dyn["counter"][t], act, off), legato)

    # Bass.
    for t, deg, steps in events(parts["bass"]):
        section = section_at(t)
        level = dyn["bass"][t]
        if act == 1:
            emit("strings_low_pizz", t, deg, steps, C.E4 - 12, 0, capped(level, act, -1), 1.0)
            continue
        emit("synth_bass", t, deg, steps, C.E4 - 12, -1, capped(level, act, -1 if act == 2 else 0), 0.7, 0.0)
        if act in (2, 4) and t % 4 == 0 and section != "break":
            emit("strings_low_spic", t, deg, min(steps, 2), C.E4 - 12, 0, capped(level, act, -2), 0.6, 0.006)
    if act == 3:
        # Celli chop off-beats against the saw: gears, not a doubling.
        for k, (section, *_rest, chords) in enumerate(bars):
            if section in ("A", "B", "A2"):
                chops(sc, k, [chords], "strings_low_spic", capped("mf", act, -1), base=40)

    # Arpeggio: the palette's machine; thinned to eighths in Act I.
    arp_inst = ARP[r["palette"]]
    for t, deg, steps in events(parts["arp"]):
        if act == 1 and t % 2:
            continue
        off = -2 if (act == 1 or r["palette"] == "soft") else -1
        emit(arp_inst, t, deg, steps, C.E4, 1 if arp_inst == "synth_glass" else 0,
             capped(dyn["arp"][t], act, off), 0.8, 0.0)

    # Each act's floor, as its reference cue has it: Act I's drone and low
    # strings holding the root; Act II's string pad under A.
    if act == 1:
        sc.note(0, nbars * 4, "electronic_drone", 28, DYN["mp"])
        for k, (section, *_rest, chords) in enumerate(bars):
            if section != "intro":
                pad(sc, k, [chords], "strings_low", DYN["pp"], start=(40,))
    if act == 2:
        for k, (section, *_rest, chords) in enumerate(bars):
            if section == "A":
                pad(sc, k, [chords], "strings_mid", DYN["p"], start=(52, 55, 59))

    # Pad: the harmony, by act.
    for t, chord, steps in events(parts["pad"]):
        section = section_at(t)
        inst = {1: "strings_mid_trem", 2: "strings_mid", 3: "pad_pumped",
                4: "brass_trombones" if section == "A2" else "strings_mid_trem"}[act]
        emit(inst, t, chord, steps, C.E4 - 12, 0, capped(dyn["pad"][t], act, -2 if act < 3 else -1), 1.0)

    # Drums: the pulse is the kick; metal from Act III; toms are timpani.
    for t, _, _ in events(parts["kick"]):
        sc.note(beat(t), 0.2, "electronic_pulse", 28, capped(dyn["kick"][t], act, -1 if act < 3 else 0), 0.0)
    if act >= 3:
        for t, _, _ in events(parts["snare"]):
            sc.note(beat(t), 0.15, "electronic_metal", 76, capped(dyn["snare"][t], act, -1), 0.0)
        for t, _, _ in events(parts["hat"]):
            sc.note(beat(t), 0.08, "electronic_metal", 88, capped(dyn["hat"][t], act, -3), 0.0)
    for t, deg, steps in events(parts["tom"]):
        sc.note(beat(t), 0.5, "timpani", fold("timpani", m(deg, mode, C.E4 - 12)), capped(dyn["tom"][t], act, 0))
    # Timpani mark the phrases; a cymbal swells into the recap; Act IV's
    # recap lands on the gong.
    for k, (section, i, *_rest, chords) in enumerate(bars):
        root = chords[0][0]
        if act >= 3 and section in ("A", "B", "A2") and i in (0, 2):
            sc.note(k * 4, 0.8, "timpani", m(root, mode, 40), capped("f" if section == "A2" else "mf", act))
        elif act == 2 and section == "A2":
            sc.note(k * 4, 1, "timpani", m(root, mode, 40), capped("mf", act))
    if "A2" in start and act >= 2:
        sc.note((start["A2"] - 1) * 4, 4, "percussion", 53, capped("mf", act))
    if act == 4 and "A2" in start:
        sc.note(start["A2"] * 4, 6, "percussion", 55, capped("f", act))
    if act == 3 and "B" in start:
        last_b = start["B"] + 3
        root = bars[last_b][-1][0][0]
        for off in (2.5, 3.0, 3.5):
            sc.note(last_b * 4 + off, 0.25, "brass_trombones_stac", m(root, mode, 52), capped("f", act))

    # The recipe's touches.
    b0 = start.get("B", 0) * 4
    if r.get("squelch") or r["fx"] == "machine":
        sc.ramp("synth_bass.cutoff", 0, 0.001, 0, 0, "lin")
        sc.ramp("synth_bass.cutoff", b0, b0 + 16, 0, 1.4)
        sc.ramp("synth_bass.cutoff", b0 + 16, b0 + 17, 1.4, 0)
    room = r.get("room")
    if room:
        hall = 2.0 if room[0] >= 10 else -6.0
        sc.ramp("hall.gain", 0, 0.001, hall, hall, "lin")
        if "break" in start:
            k = start["break"] * 4
            sc.ramp("hall.gain", k, k + 4, hall, hall + 3)
            sc.ramp("hall.gain", k + 6, k + 8, hall + 3, hall)
    if r.get("room_b"):
        sc.ramp("long.gain", b0, b0 + 2, 0, 5)
        sc.ramp("long.gain", b0 + 14, b0 + 16, 5, 0)
    if r.get("throws"):
        for k in range(4):
            t = b0 + 4 * k + 3
            sc.ramp("echo.gain", t, t + 0.05, -3, 6, "lin")
            sc.ramp("echo.gain", t + 0.9, t + 1.0, 6, -3, "lin")
    for bpm_at in tempo_map(r, tr, start):
        sc.head.append(f"tempo {bpm_at[0]:g} {bpm_at[1]:g}")
    return sc


def tempo_map(r, tr, start):
    """compose.py's tempo arcs and push, as tempo steps a beat apart."""
    out = []
    def line(b0, b1, frm, to):
        n = int(b1 - b0)
        for k in range(n + 1):
            out.append((b0 + k, round(frm + (to - frm) * k / max(1, n), 2)))
    form = dict(tr["form"])
    for arc in r.get("arc", ()):
        section, frm, to = arc[:3]
        first, last = (arc[3], arc[4]) if len(arc) == 5 else (0, form[section] - 1)
        b0 = (start[section] + first) * 4
        b1 = (start[section] + last + 1) * 4 - 1
        line(b0, b1, frm, to)
        out.append((b1 + 1, r["bpm"]))
    if r.get("push") and "break" in start:
        b, k = start["B"] * 4, start["break"] * 4
        line(b, b + 15, r["bpm"], r["push"])
        line(k, k + 7, r["push"], r["bpm"])
        out.append((k + 8, r["bpm"]))
    return sorted(set(out))


def lint(sc, mode):
    """The rules every track keeps: each pitched note in its mode (the
    rally's key), and a texture that can be heard."""
    scale = {(4 + s) % 12 for s in C.MODES[mode]} if mode else set(range(12))
    steps = sc.bars * 16
    parts = [set() for _ in range(steps)]
    for line in sc.body:
        if not line.startswith("note "):
            continue
        _, b, d, inst, pitch, _v = line.split()
        if not inst.startswith(UNPITCHED) and int(pitch) % 12 not in scale:
            raise SystemExit(f"{sc.name}: {inst} {pitch} is outside E {mode}")
        for k in range(int(float(b) * 4), min(steps, int((float(b) + max(float(d), 0.25)) * 4 + 0.999))):
            parts[k].add(inst)
    avg = sum(len(p) for p in parts) / steps
    peak = max(len(p) for p in parts)
    if avg > 7.0 or peak > 12:
        raise SystemExit(f"{sc.name}: {avg:.1f} parts on average, {peak} at once: too dense to hear")
    return avg, peak


def retitle(sc, level):
    """A track is titled after its level, `BRYX NN Name`: the menu's Music
    row names it the same way (src/menu.rs checks the two agree)."""
    title = f"title BRYX {level:02d} {track(level)['name']}"
    sc.head = [title if line.startswith("title ") else line for line in sc.head]


def score():
    """The whole score: the approved reference cues where they exist, the
    arranger everywhere else."""
    fixed = {1: act1_warmup, 9: act2_frame, 22: act3_armored, 32: act4_vault}
    out = []
    for level in range(1, C.LEVEL_TRACKS + 1 if hasattr(C, "LEVEL_TRACKS") else 33):
        sc = fixed[level]() if level in fixed else arrange(level)
        sc.name = f"music_{level:02d}"
        retitle(sc, level)
        out.append((sc, track(level)["mode"]))
    menu_sc = menu()
    menu_sc.name = "music_menu"
    retitle(menu_sc, 0)
    out.append((menu_sc, "dorian"))
    out.append((act_clear(), None))
    for make in STINGERS:
        out.append((make(), None))
    return out


# ----------------------------------------------------------- stingers ---
# The stingers' own notes (as the chip score had them), re-
# orchestrated. Notes are (MIDI or a chord tuple or None, sixteenths).
# Level clear and life lost are timed by the game (the music waits for
# them, the level-end sweep keeps level clear's bar): their tails fade
# inside their slot.

def put(sc, inst, notes, vel, beat0=0.0, legato=0.9, transpose=0, spread=0.008):
    t = beat0
    for p, steps in notes:
        if p is not None:
            for q in (p if isinstance(p, tuple) else (p,)):
                sc.note(t, max(0.06, steps * 0.25 * legato), inst, q + transpose, vel, spread)
        t += steps * 0.25


def level_clear():
    sc = Score("level_clear", "Level clear", "aeolian", 160, 2, loop=False, tail=0.15, comment="""
The level stinger: Spark bounding up, then the E minor triad climbing to
a held high E (the chip stinger's notes). Light, as it is heard
32 times a run: spiccato and the bell, horns on the climb, the strings'
held chord. Two bars at 160 (3.0 s), the level-end sweep's meter.
Written by audio/compose_studio.py: do not edit.""")
    spark = [(71, 1), (None, 1), (74, 1), (76, 1), (None, 1), (79, 2), (None, 1),
             (81, 1), (79, 1), (76, 1), (None, 1), (74, 2), (None, 2)]
    put(sc, "strings_high_spic", spark, DYN["mf"], legato=0.7)
    put(sc, "synth_glass", spark, DYN["mp"], transpose=12, legato=0.6)
    put(sc, "strings_low_pizz", [(40, 4), (40, 4), (43, 4), (43, 4)], DYN["mf"], legato=1.0)
    climb = [(76, 2), (79, 2), (83, 2), (88, 10)]
    put(sc, "strings_high", climb, DYN["f"], beat0=4, legato=1.0)
    put(sc, "brass_horns", [(64, 2), (67, 2), (71, 2), ((64, 67, 71), 10)], DYN["mf"], beat0=4, legato=0.95)
    put(sc, "strings_mid", [(71, 2), (76, 2), (79, 2), ((79, 83), 10)], DYN["mf"], beat0=4, legato=1.0)
    put(sc, "strings_low", [(48, 4), (50, 4), (52, 8)], DYN["mf"], beat0=4, legato=1.0)
    sc.note(6, 2, "timpani", 40, DYN["mf"])
    sc.note(6, 0.4, "synth_glass", 88, DYN["mf"])
    sc.ramp("master.gain", 7, 8.4, 0, -40)
    return sc


def life_lost():
    sc = Score("life_lost", "Life lost", "aeolian", 150, 1, loop=False, tail=0.25, comment="""
A life lost: Ascent's opening leaps turned upside down and falling,
fading as they fall, stopping on B, unresolved (the chip stinger's notes). One bar at 150 (1.6 s): horns over the celli.
Written by audio/compose_studio.py: do not edit.""")
    put(sc, "brass_horns", [(76, 2), (69, 2), (64, 4), (66, 2), (69, 2), (59, 4)], DYN["f"], legato=0.9)
    put(sc, "strings_low", [(52, 4), (45, 4), (48, 4), (47, 4)], DYN["mf"], legato=1.0)
    sc.note(0, 0.5, "strings_low_pizz", 40, DYN["f"])
    sc.ramp("master.gain", 0.5, 4, 0, -9)
    sc.ramp("master.gain", 4, 4.6, -9, -40)
    return sc


def game_over():
    sc = Score("game_over", "Game over", "phrygian", 112, 3, loop=False, tail=0.5, comment="""
Game over: Ascent backwards and slowed, over a Phrygian F in the bass, a
pad of thirds above, sinking to a low E as the tempo slows, in a large
room (the chip stinger's notes). A horn, the strings' thirds, the
low strings and tuba; the long hall. Written by audio/compose_studio.py:
do not edit.""")
    for k, bpm in enumerate([112, 105, 98, 91], start=4):
        sc.tempo(k / 4, bpm)
    sc.tempo(2, 84)
    sc.define("ramp long.gain 0 0.001 4 4 lin")
    # The last bar: the low E held half of it, the hall the rest.
    put(sc, "brass_horns", [(71, 4), (69, 4), (71, 4), (74, 4), (76, 4), (71, 4), (64, 8)], DYN["mf"], legato=0.92)
    put(sc, "strings_mid", [((64, 67), 8), ((62, 66), 8), ((60, 64), 8), ((59, 63), 8), ((59, 64), 8)],
        DYN["p"], legato=1.0)
    put(sc, "strings_low", [(48, 8), (45, 8), (41, 8), (47, 8), (40, 8)], DYN["mp"], legato=1.0)
    put(sc, "brass_low", [(36, 8), (33, 8), (29, 8), (35, 8), (28, 8)], DYN["p"], legato=0.95)
    put(sc, "brass_horns", [(None, 32), (52, 8)], DYN["p"], legato=1.0)
    sc.note(8, 4, "timpani_roll", 40, DYN["p"])
    sc.ramp("timpani_roll.gain", 8, 12, -20, -8)
    sc.note(8, 4, "electronic_drone", 28, DYN["mp"])
    return sc


def victory():
    sc = Score("victory", "Victory", "harmonic", 132, 4, loop=False, tail=0.5, comment="""
Victory: Ascent in harmonic minor, Spark answering above, over a swept
kick, broadening into a full E major chord: the score's one major chord
(the chip stinger's notes). Trumpets and horns on Ascent, spiccato
and the bell on Spark, the Wall's bass and the pulse below.
Written by audio/compose_studio.py: do not edit.""")
    for k, bpm in enumerate([126, 120, 114, 108]):
        sc.tempo(8 + k, bpm)
    ascent = [(64, 2), (71, 2), (76, 4), (75, 2), (71, 2), (69, 2), (71, 2),
              (67, 2), (69, 2), (71, 4), (75, 2), (76, 2), (79, 4),
              (81, 2), (79, 2), (78, 2), (76, 2), (75, 4), (78, 4)]
    put(sc, "brass_trumpets", ascent, DYN["f"], legato=0.88)
    put(sc, "brass_horns", ascent, DYN["f"], transpose=-12, legato=0.9)
    spark = [(71, 1), (None, 1), (74, 1), (76, 1), (None, 1), (79, 2), (None, 1),
             (81, 1), (79, 1), (76, 1), (None, 1), (75, 2), (None, 2),
             (84, 2), (83, 2), (81, 2), (79, 2), (78, 4), (83, 4)]
    put(sc, "strings_high_spic", spark, DYN["f"], beat0=4, legato=0.7)
    put(sc, "synth_glass", spark, DYN["mf"], beat0=4, transpose=12, legato=0.6)
    bass = [(40, 4), (40, 4), (35, 4), (35, 4), (36, 4), (36, 4), (31, 4), (31, 4),
            (33, 4), (33, 4), (35, 4), (35, 4)]
    put(sc, "synth_bass", bass, DYN["f"], transpose=-12, legato=0.8, spread=0.0)
    put(sc, "strings_low_spic", bass, DYN["mf"], legato=0.6)
    for b in range(12):
        sc.note(b, 0.2, "electronic_pulse", 28, DYN["f"], 0.0)
    sc.note(8, 4, "timpani_roll", 40, DYN["f"])
    sc.ramp("timpani_roll.gain", 8, 12, -18, 0)
    sc.note(8, 4, "percussion", 54, DYN["f"])
    # The E major chord.
    # The E major chord: each section one job, the brass on top.
    for inst, notes in [("brass_trumpets", (76, 80)), ("brass_horns", (64, 68, 71)), ("brass_trombones", (40, 52)),
                        ("strings_high", (80, 83, 88)), ("strings_mid", (64, 71)), ("strings_low", (40, 52))]:
        for q in notes:
            sc.note(12, 2.5, inst, q, DYN["ff"])
    sc.note(12, 1, "timpani", 40, DYN["ff"])
    sc.note(12, 4, "percussion", 49, DYN["f"])
    return sc


STINGERS = [level_clear, life_lost, game_over, victory]


# ------------------------------------------------------------ effects ---
# The gameplay effects (the chip effects' notes and gestures), built from
# the score's parts (audio/instruments/sfx.inc). Each keeps its root (the
# game pitches the brick and the paddle by playback rate from E5 and E3),
# its gesture and its length. Times in seconds (tempo 60). build.rs folds
# them to mono at 22.05 kHz for the mu-law effects path.
# name: (what it says, tail seconds, [(seconds, length, instrument, MIDI, velocity)])

S16 = 0.25 / 4  # a sixteenth at 240 BPM, in seconds

EFFECTS = {
    "paddle": ("A knock dropping onto E3: a pizzicato cello on a triangle knock, a click.", 0.08, [
        (0, 0.07, "sfx_knock", 52, 112), (0, 0.06, "sfx_pizz_lo", 52, 88), (0, 0.01, "sfx_click", 84, 70)]),
    "brick": ("A pop snapping down onto E5, a pizzicato violin in it, a glint of metal on top.", 0.1, [
        (0, 0.1, "sfx_pop", 76, 120), (0, 0.08, "sfx_pizz_hi", 76, 64), (0, 0.02, "sfx_glint", 100, 44)]),
    "armor": ("Two tinks, the second softer, on an anvil and a ring of metal: the armor holds.", 0.12, [
        (0, 0.04, "sfx_tink", 95, 124), (S16, 0.04, "sfx_tink", 95, 90), (0, 0.15, "sfx_ring", 90, 100),
        (0, 0.2, "sfx_kit", 56, 52), (0, 0.06, "sfx_glint", 83, 56)]),
    "capsule": ("A quick rising arpeggio, spiccato with the bell an octave up, swelling into its top.", 0.15, [
        (k * S16 / 2, 0.05, inst, p + o, v) for k, (p, v) in enumerate(
            [(64, 64), (67, 70), (71, 78), (76, 84), (79, 90), (83, 98)])
        for inst, o in (("sfx_spic", 0), ("sfx_bell", 12))]),
    "tiny": ("A sour falling arpeggio that fades as it shrinks.", 0.08, [
        (k * S16 / 2, 0.05, "sfx_sour", p, v) for k, (p, v) in enumerate(
            [(83, 108), (78, 98), (74, 88), (71, 78), (66, 66), (62, 54)])]),
    "oneup": ("A small fanfare up the triad on trumpets, landing on B and E in a little room.", 0.25, [
        (0, 0.04, "sfx_trumpet_stac", 76, 92), (S16 * 2 / 3, 0.04, "sfx_trumpet_stac", 79, 96),
        (2 * S16 * 2 / 3, 0.04, "sfx_trumpet_stac", 83, 100),
        (3 * S16 * 2 / 3, 0.17, "sfx_trumpet", 83, 104), (3 * S16 * 2 / 3, 0.17, "sfx_bell", 88, 90),
        (0, 0.08, "sfx_pizz_lo", 52, 90), (2 * S16 * 2 / 3, 0.08, "sfx_pizz_lo", 59, 88)]),
    "laser": ("One note falling two octaves in 60 ms: a zap.", 0.03, [(0, 0.0625, "sfx_zap", 64, 100)]),
    "superlaser": ("A heavier beam falling three octaves with a buzz, over a falling hiss.", 0.06, [
        (0, 0.06, "sfx_beam", 59, 96), (0, 0.06, "sfx_hiss", 84, 70)]),
    "asteroid": ("A low rumble swelling in over a wavering drone and a soft timpani roll.", 0.2, [
        (0, 0.25, "sfx_rumble", 48, 127), (0, 0.25, "sfx_drone", 40, 44), (0, 0.25, "sfx_timp_roll", 40, 30)]),
    "explosion": ("A crack falling into a rumble, over a bass drum and a timpani dropping a fifth.", 0.3, [
        (0, 0.2, "sfx_blast", 60, 127), (0, 0.12, "sfx_thump", 28, 64), (0, 0.3, "sfx_kit", 35, 56),
        (0, 0.3, "sfx_timp", 40, 48)]),
    "ball_lost": ("A falling figure, pizzicato over a soft triangle, fading as it falls.", 0.1, [
        (k * 0.05, 0.05, inst, p, v + dv) for k, (p, v) in enumerate([(71, 84), (67, 70), (64, 56), (59, 44)])
        for inst, dv in (("sfx_pizz_hi", 4), ("sfx_fall", 0))]),
    "launch": ("A whip up two octaves into E5 over a rising breath, on a low pizzicato and timpani tap.", 0.05, [
        (0, 0.0625, "sfx_whip", 76, 96), (0, 0.0625, "sfx_breath", 84, 64), (0, 0.06, "sfx_pizz_lo", 52, 92),
        (0, 0.08, "sfx_timp", 40, 64)]),
    "warp": ("A note rising three octaves over rushing air and a swelling tremolo, into an echoing room.", 0.3, [
        (0, 0.12, "sfx_rise", 100, 92), (0, 0.12, "sfx_air", 72, 92), (0.05, 0.4, "sfx_trem", 88, 70)]),
    "select": ("Two plucks, up: the second leans in.", 0.04, [
        (0, 0.03, "sfx_pizz_hi", 71, 80), (0, 0.03, "sfx_blip", 71, 70),
        (S16 / 2, 0.03, "sfx_pizz_hi", 76, 96), (S16 / 2, 0.03, "sfx_blip", 76, 84)]),
    "pause": ("Two plucks, down: the second steps back.", 0.04, [
        (0, 0.03, "sfx_pizz_hi", 76, 96), (0, 0.03, "sfx_fall", 76, 84),
        (S16 / 2, 0.03, "sfx_pizz_hi", 71, 80), (S16 / 2, 0.03, "sfx_fall", 71, 70)]),
    "steel": ("A short dull knock with a dry glint: the ball turned back, nothing broken.", 0.02, [
        (0, 0.025, "sfx_knock_hi", 59, 96), (0, 0.02, "sfx_clack", 76, 96), (0, 0.016, "sfx_glint", 76, 50)]),
    "gate": ("A grinding climb, then an open fifth on trumpet and violins that rings out: the wall gives way.", 0.3, [
        (k * 0.05, 0.05, "sfx_grind", p, 92) for k, p in enumerate([52, 59, 64, 71])] + [
        (0, 0.05, "sfx_trombone_stac", 52, 90), (0.1, 0.05, "sfx_trombone_stac", 64, 90),
        (0.2, 0.2, "sfx_trumpet", 76, 100), (0.2, 0.25, "sfx_trem", 83, 84),
        (0, 0.2, "sfx_timp_roll", 40, 56)]),
    "expiring": ("Two soft ticks falling a fifth, B to E: the power is about to go.", 0.05, [
        (0, 0.03, "sfx_tick", 83, 80), (0, 0.03, "sfx_bell", 83, 60),
        (2 * S16 * 2 / 3, 0.03, "sfx_tick", 76, 64), (2 * S16 * 2 / 3, 0.03, "sfx_bell", 76, 50)]),
}

# Level trims (dB): each effect brought to the loudness of the ABC effect
# it replaces (its loudest 30 ms of RMS), so the game's per-effect gains
# and its balance against the music still hold. The chip effects were
# near-square waves, loud for their peak; plucks and hits are spikier,
# so the trims lean on the effects' -1 dBFS limiter for a few dB.
# An effect a visual is timed to states its meter (bar seconds, beats):
# the warp's sweep runs one bar of it (src/cadence.rs, build.rs).
EFFECT_METERS = {"warp": (1.0, 4)}

EFFECT_TRIMS = {
    "armor": 3,
    "asteroid": -1.9,
    "ball_lost": -2.6,
    "brick": 3,
    "capsule": 3,
    "expiring": -3.9,
    "explosion": 1.6,
    "gate": 2.8,
    "laser": -0.2,
    "launch": 3,
    "oneup": 3,
    "paddle": -1.3,
    "pause": -6,
    "select": -0.5,
    "steel": -1.0,
    "superlaser": -2.9,
    "tiny": -0.7,
    "warp": 3,
}


def effect(name):
    say, tail, notes = EFFECTS[name]
    end = max(t + d for t, d, *_ in notes)
    sc = Score(name, f"Effect: {name}", None, 60, 1, loop=False, tail=tail,
               comment=f"""
Gameplay effect: {name}. {say}
Written by audio/compose_studio.py: do not edit.""", libs=("sfx",))
    sc.head = [line if not line.startswith("length") else f"length {end:g}" for line in sc.head]
    if name in EFFECT_METERS:
        bar, beats = EFFECT_METERS[name]
        sc.head.insert(sc.head.index("score 1") + 1, f"# meter {bar:g} {beats}")
    sc.trim(EFFECT_TRIMS.get(name, 0.0))
    for t, d, inst, p, v in notes:
        sc.note(t, d, inst, p, v, 0.0)
    return sc


REFERENCES = [act1_warmup, act2_frame, act3_armored, act4_vault, menu, act_clear]


def main():
    """`compose_studio.py` writes the score to audio/score/;
    `compose_studio.py --reference [DIR]` rewrites the Phase 3 cues."""
    if len(sys.argv) > 1 and sys.argv[1] == "--reference":
        out = pathlib.Path(sys.argv[2]) if len(sys.argv) > 2 else OUT
        for make in REFERENCES:
            sc = make()
            sc.write(out)
            print(f"{sc.name:14s} -> {os.path.relpath(out / (sc.name + '.score'))}")
        return
    out = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else HERE / "score"
    # The notes first: compose.py's lint (a lead that misses its chords, a
    # counter that crosses it), then each track's own (key and density).
    if not C.check_score([C.compose(r) for r in C.RECIPES] + [C.compose(C.MENU)]):
        raise SystemExit("the score lint failed: nothing written")
    for sc, mode in score():
        avg, peak = lint(sc, mode)
        sc.write(out)
        print(f"{sc.name:12s} {avg:4.1f} parts avg, {peak:2d} at most -> {os.path.relpath(out / (sc.name + '.score'))}")
    for name in EFFECTS:
        effect(name).write(out / "sfx")
    print(f"{len(EFFECTS)} effects -> {os.path.relpath(out / 'sfx')}")


if __name__ == "__main__":
    main()
