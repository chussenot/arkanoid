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
    def __init__(self, name, title, mode, bpm, bars, loop=True, tail=0.0, comment=""):
        self.name, self.mode, self.bars = name, mode, bars
        self.head = [f"# {line}" for line in comment.strip().splitlines()]
        self.head += [
            "score 1", f"title {title}", f"# mode E {mode}", f"tempo 0 {bpm}",
            f"length {bars * 4}",
        ]
        self.head.append("loop on" if loop else f"tail {tail}")
        self.head += ["seed 1"] + [f"include {{INCLUDES}}/{lib}.inc" for lib in ("desk", "orchestra", "electronic")]
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
    sc.trim(2.0)
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
    sc = Score("act_clear", "Act clear", "aeolian", 160, 3, loop=False, tail=0.9, comment="""
Phase 3 reference cue: the act stinger (an act's last level cleared),
the fanfare of audio/cues/act_clear.abc re-orchestrated: Spark bounding
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
    sc.ramp("master.gain", 11, 14.4, 0, -40)
    return sc


# ------------------------------------------------------------ the score ---
# The other 28 tracks: compose.py's own parts (lead, counter, bass,
# arpeggio, pad, drums, as lines of degrees with a dynamic per step),
# orchestrated by act with the rules the reference cues proved. The notes
# and their rhythm are the score's; the studio decides who plays them.

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
    sc.trim({1: -0.5, 2: -1.5, 3: -1.5, 4: 2.0}[act])
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
    scale = {(4 + s) % 12 for s in C.MODES[mode]}
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


def score():
    """The whole score: the approved reference cues where they exist, the
    arranger everywhere else."""
    fixed = {1: act1_warmup, 9: act2_frame, 22: act3_armored, 32: act4_vault}
    out = []
    for level in range(1, C.LEVEL_TRACKS + 1 if hasattr(C, "LEVEL_TRACKS") else 33):
        sc = fixed[level]() if level in fixed else arrange(level)
        sc.name = f"music_{level:02d}"
        out.append((sc, track(level)["mode"]))
    menu_sc = menu()
    menu_sc.name = "music_menu"
    out.append((menu_sc, "dorian"))
    clear = act_clear()
    out.append((clear, "aeolian"))
    return out


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
    for sc, mode in score():
        avg, peak = lint(sc, mode)
        sc.write(out)
        print(f"{sc.name:12s} {avg:4.1f} parts avg, {peak:2d} at most -> {os.path.relpath(out / (sc.name + '.score'))}")


if __name__ == "__main__":
    main()
