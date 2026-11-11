# BRYX: the Phase 3 reference cues

Phase 3 of the orchestral score (`orchestral-score.md`): six cues written
for the studio, to prove the direction before the 32-track rewrite. This
page says what each cue is, how they were written, what was measured,
and what the listening gate decides. Phase 2's engine report is
`studio-report.md`.

## The six cues

Each cue is a real track of the score, re-orchestrated, not rewritten. A
cue that passes the gate becomes that track in Phase 4.

| Cue | Track | Mode, tempo | Form (bars) | Character |
|---|---|---|---|---|
| `act1_warmup` | 01 Warm-up | E dorian, 112, swing 54 | intro 2 · A 4 (Spark) · B 4 (Ascent's head) · A′ 4 · turn 2 | Restrained electronic tension: drone, soft pulse, bell arpeggio, tremolo and pizzicato strings, one soft horn |
| `act2_frame` | 09 Picture frame | E aeolian, 120 | A 4 (Ascent) · B 4 (Spark, stretched) · break 2 · A′ 4 · turn 2 | Evolving orchestral movement: horns and violins on Ascent, the first low brass, a moving bass, the sequence |
| `act3_armored` | 22 Armored frame | E phrygian, 132 | intro 2 · A 4 (the Wall high) · B 4 (the Wall inverted) · break 2 · A′ 4 | Mechanical precision, written by register: the saw bass alone owns the Wall down low, celli chop off-beats against it, the high Wall on bright spiccato, the sequence carries the motion, low brass only closes phrases |
| `act4_vault` | 32 Vault | E harmonic, 144 → 150 | A 4 (Ascent) · B 4 (Spark) · A′ 4 (all three) · break 2 · turn 2 | Scale: the full brass statement held back since Act I, the score's only tutti, after a real pianissimo; one melody at a time, so it is heard |
| `menu` | the menu | E dorian, 92 | intro 2 · A 4 (Ascent, augmented) · B 4 (Spark) · A′ 4 · turn 2 | An invitation, not a fanfare |
| `act_clear` | the act stinger | E aeolian, 160 | 3 bars | Spark bounding up, Ascent's head over a bass walking to E, a held E minor; strings and horns, a preview of Act II |

The cues are `audio/reference/cues/*.score`, written by
`audio/compose_studio.py`. It imports the themes, the modes, the
progressions and `harmonize()` from `compose.py`, so the two scores cannot
drift. It is deterministic: two runs write the same bytes.

## How they were written

- **Contrast before size.** Each act owns a palette and a dynamic ceiling.
  Act IV's tutti is the only one in the score, and a pianissimo comes
  before it.
- **Melody goes to articulated voices,** because VSCO has no legato
  transitions.
  - Themes are carried by tongued brass, pizzicato, spiccato or bells.
  - Sustained strings carry harmony, never an exposed line.
  - A doubling is there for its colour: horns with violins an octave up,
    or the menu's horns with a soft synth lead that carries the connection
    the samples cannot.
- **A swell is orchestration.** Sections enter, sustain turns to tremolo,
  a filter opens, a timpani rolls; a gain ramp sits under that, never in
  place of it.
- **The orchestra breathes; the machine does not.** Orchestral notes get
  a few milliseconds of deterministic timing and velocity spread. Synths
  and electronic percussion stay on the grid.
- **One owner per register, about four jobs at once.** Melody, bass,
  harmony and motion; colour on top. Size comes from doubling and register,
  not from the number of parts (see "The first listening").
- **The rally stays in key.** Every pitched note of the six cues is in its
  track's mode (checked: 0 outside), and strong-beat melody notes fit
  their chord (`compose.harmonize`).

## What was measured

From the float renders, in dBFS RMS.

| Cue | Overall | By section | Peak |
|---|---|---|---|
| Act I | −23.8 | intro −26.3 · A −25.9 · B −21.1 · A′ −23.6 · turn −28.3 | −6.2 |
| Menu | −21.7 | intro −21.9 · A −23.8 · B −21.6 · A′ −20.3 · turn −21.6 | −6.4 |
| Act II | −20.2 | A −19.5 · B −20.8 · break −22.9 · A′ −18.9 · turn −21.3 | −4.4 |
| Act III | −19.5 | intro −19.3 · A −19.3 · B −19.3 · break −20.5 · A′ −19.4 | −4.5 |
| Act IV | −17.4 | A −16.6 · B −16.9 · A′ −16.0 · break −27.9 · turn −19.9 | −1.1 |
| Act clear | −21.1 | fanfare −18.7 | −4.5 |

**The arc.** Each act sits above the one before it, and Act IV's recap is
3.4 dB above anything in Act III. Act II and Act III are close in level
on purpose: one is melodic, the other mechanical, and that is their
contrast, not volume. Act IV's break falls 12 dB under its recap, more
in its second bar, once the hall has rung down.

**The melody** was measured as the theme against everything else, in
the A section, in the 300 Hz–3 kHz band where it is heard:

| Cue | First draft | Now |
|---|---|---|
| Act I | −4.2 dB | −2.1 dB |
| Act II | **−8.4 dB** | +2.1 dB |
| Act III | +3.7 dB | +1.1 dB (the new lead: spiccato violas and violins) |
| Act IV | −2.9 dB | +6.8 dB |
| Menu | +0.6 dB | +0.6 dB |

The first draft buried Act II's Ascent, the melodic act's own theme, under
its accompaniment. The theme now sits on the horns' f layer, and the bass,
the pad and the sequence step back.

**As the game plays them.** QOA holds 35–43 dB SNR overall and 36–48 dB
in every section measured. Act III is the lowest overall (35 dB), with
its spiccato and metal in front. Its noise follows the signal, so a quiet section is no
worse than a loud one. At the game's output, after its music gain, the
noise sits at −59 to −66 dBFS. The weakest section is Act I's recap
(36 dB): pizzicato and bell attacks are the hardest material for QOA's
20-sample slices.

**Seams.** None of the five loops has a step at its wrap: the curvature
across each seam is under half of the largest inside the loop.

**Size.** The five loops run 26.5–41.7 s, at about 38 kB a second as QOA:
1.0–1.6 MB each. The 32 tracks, at these lengths, come to about 39 MB.

## The first listening, and the revision

The first listening (Phase 3, round one) found:
- **Act I:** restrained, and its recap survives QOA.
- **Act II:** Ascent reads as a line without legato.
- **Act III:** acceptable, but not liked ("maybe because it's mud").
- **Act IV and the act stinger:** these felt like more rather than
  bigger: "a lot of instruments", which could be "easier to hear".

Counting what sounds at once, per sixteenth, from the scores, confirmed
both:

| Cue | Parts sounding, avg / max | Low parts (below C4), avg | Notes below C3, avg |
|---|---|---|---|
| Act II (liked, for reference) | 6.0 / 11 | 4.0 | 3.5 |
| Act III, first draft | 6.4 / 12 | 4.3 | 5.2 |
| Act III, revised | 5.1 / 8 | 2.1 | 3.3 |
| Act IV, first draft | 9.2 / 17 | 6.1 | 8.3 |
| Act IV, revised | 5.6 / 10 | 3.6 | 3.9 |
| Act clear, first draft | 6.6 / 12 | 3.9 | 2.3 |
| Act clear, revised | 4.4 / 9 | 2.7 | 1.7 |

**Act III's first draft** put the Wall in four low instruments at once
(celli, basses, the saw bass, and tuba and trombone stabs), and its
melody was the Wall too, so nothing stood against it. The revision gives
each register one owner:
- the saw bass alone plays the Wall;
- the celli chop off-beats against it;
- the high Wall moves to bright spiccato violas and violins;
- the machine's sequence carries the motion, as the chiptune track's
  "trance" arpeggio did;
- low brass only closes each phrase.

The mud band (150–300 Hz) fell 4 dB, from −9.1 to −13.2 dB of the total,
and 1–3 kHz and 3–8 kHz gained definition.

**Act IV's first draft** stacked every idea at once. The revision plays
one melody at a time:
- A: the brass statement over the Wall, with trombones holding the
  harmony;
- B: Spark on spiccato and bell, with the sequence;
- A′: Ascent on the brass, the Wall below with the basses joining, and
  Spark reduced to the bell answering in the second half of each bar,
  where Ascent holds.

The pad, the tuba's staccato and the violin doubling went. **The act
stinger** keeps its three bars and halves its parts; its last chord is
strings, horns and bass.

## The gate

The listening files are the QOA-decoded audio, what the game would play,
with each loop heard twice so the seam is heard too. Listen for:

1. **Contrast.** Do the four acts feel like an arc (restraint, movement,
   precision, scale), or like one cue getting louder?
2. **Act I.** Is the tension restrained but alive? In its recap, do the
   pizzicato and bell attacks survive QOA at a normal listening level?
3. **Act II.** Does Ascent on the horns read as a line, with no legato
   and no smear?
4. **Act III.** Machine, or mud? Is the Wall precise, and is the low end
   clear?
5. **Act IV.** Does the tutti feel earned? Is the pianissimo real? Does
   the restart land?
6. **Menu.** Does it invite?
7. **Act clear.** Does it read as an ending that points forward?

If the direction holds, Phase 4 writes the 32 tracks, the menu and the
stingers in this studio, and the game moves to them. If it does not,
say what is wrong; the fix comes before the rewrite.
