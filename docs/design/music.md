# BRYX score

32 level tracks and a menu theme, written as an 80s chiptune score for the
Electric Pulse engine. This page is the listener's guide; the music itself
lives as decisions in [`audio/compose.py`](../../audio/compose.py), which
writes the `audio/cues/music_*.abc` files the build renders. The sound
design around it is in [`audio.md`](audio.md).

## Three themes

Every track is built from three themes, written once as scale degrees, so
each track's mode recolors them instead of replacing them. Shown here in E
Aeolian (`·` is a sixteenth rest; a note lasts until the next symbol).

**Ascent: the player's theme.** A leap up a fifth, on to the octave, a turn
back, then an answer that climbs past where it started. It is the first
thing the menu plays, the lead of most of Act II, and the finale's
headline.

    E B e d B A B G A B d e g

**The Wall: the obstacle.** A machine ostinato: a held note, its upper
neighbor and its lower one. It enters in Fortress (12), drives Act III,
and in the finale it is the ground everything stands on. Phrygian makes
its upper neighbor the dark F natural; harmonic minor makes its lower one
a bright D sharp.

    e e ^f e e e d e

**Spark: the hope.** A syncopated bounce up the pentatonic and back down.
It leads Act I, answers Ascent everywhere else, and in Shield (21) it is
held up against the Wall.

    B · d e · g · a g e · d ·

### How they change

| Variant | What it does | Heard in |
|---|---|---|
| inverted | every interval mirrored | Cascade, Double W, Diamond, Rain |
| retrograde | played backwards | Cascade, Hourglass |
| augmented | twice as slow | Heart, Bridges, Pillars, Bulwark |
| diminished | twice as fast | Arrow, Rocket, Funnel |
| displaced | pushed two steps off the beat | Checkers, Scatter, Lattice, Ring, Baffles, Minefield |
| sequenced | repeated a step up or down | Cascade, Arrow, Double W, Spiral, Chevron |
| staccato | each note clipped to a sixteenth | Capped columns, Baffles, Minefield |
| an octave or a fifth higher | the same line in a new register | Twin towers, Rocket, Crown, Split wall, Vault |

## One key, four colors

Every track is in E, because BRYX's brick sounds play the E minor
pentatonic over the music and a rally must always harmonize. The variety
comes from the mode:
- **Dorian** is the bright one (C sharp): Act I's optimism, Heart, Wave,
  the menu.
- **Aeolian** is plain minor, the score's home.
- **Phrygian** is the dark one (F natural): the Wall's tracks, Minefield.
- **Harmonic minor** is dramatic (D sharp): Bridges, Crown, Funnel, Split
  wall, and the finale.

## The four acts

The score follows the campaign's four acts:
- **I · Foundations**: Spark leads, 112–124 BPM, light textures.
- **II · Structures**: Ascent takes over, 108–128 BPM, and the Wall first
  appears.
- **III · Machines**: the Wall leads, with buzzing square waves and a
  resonant ladder filter, 124–134 BPM.
- **IV · Vault**: all three themes together, up to 144 BPM.

Every track has the same 16-bar form, so it loops cleanly:

| Section | Bars | What happens |
|---|---|---|
| intro | 2 | drums, bass, arpeggio |
| A | 4 | the first theme |
| B | 4 | the second theme, the arpeggio up an octave, a pad |
| A′ | 4 | the first theme again, with a counter-melody |
| turn | 2 | a breakdown that lands on E as the loop restarts |

Tracks run 27–36 seconds; the menu runs 42.

| # | Level | Mode, tempo | Themes (A → B, counter) | Palette, drums | Idea |
|---|---|---|---|---|---|
| **Act I · Foundations** | | | | | |
| 01 | Warm-up | E dorian, 112 BPM | spark → ascent_head | pulse, half | Spark in its plainest form over a Dorian i-IV sway; Ascent's head appears once, as a promise. |
| 02 | Cascade | E dorian, 114 BPM | spark_fall → spark_retro (+ ascent_inv) | glass, half | Everything descends: Spark stepped down a degree a bar, then backwards; Ascent turned upside down underneath. |
| 03 | Checkers | E aeolian, 116 BPM | spark → spark_inv (+ spark_shift) | pulse, break | Call and answer: Spark, then Spark inverted, with a counter-voice two steps behind like alternating squares. |
| 04 | Twin towers | E dorian, 118 BPM | ascent → spark (+ ascent_5th) | glass, four | Ascent arrives in full, then doubled a fifth above: two towers of the same theme. |
| 05 | Arrow | E aeolian, 124 BPM | ascent_dim → spark_seq | pulse, four | Ascent at double speed, straight as an arrow over a driving sixteenth bass. |
| 06 | Capped columns | E dorian, 116 BPM | ascent_stac → spark | soft, march | Ascent clipped short, note by note, like columns with caps; a marching snare holds the line. |
| 07 | Diamond | E dorian, 120 BPM | ascent → ascent_inv (+ spark) | glass, half | Symmetry: Ascent and its mirror image, facets of one theme. |
| 08 | Scatter | E aeolian, 122 BPM, swing 56 | spark_shift → ascent_head | pulse, break | Spark knocked off the beat and lightly swung; the first act closes on Ascent's head, unresolved. |
| **Act II · Structures** | | | | | |
| 09 | Picture frame | E aeolian, 120 BPM | ascent → spark_aug (+ ascent_aug) | soft, four | Ascent framed by itself: stated, then held in long notes beneath the B section. |
| 10 | Heart | E dorian, 108 BPM, swing 60 | ascent_aug → spark (+ spark_lo) | soft, half | The warm one: Ascent slowed to a sigh over a swung half-time beat. |
| 11 | Double W | E aeolian, 124 BPM | spark_seq → ascent_inv (+ spark_inv) | pulse, break | Zig-zag: Spark climbing a degree, then Ascent upside down; the arpeggio never stops changing direction. |
| 12 | Fortress | E phrygian, 118 BPM | ascent → wall | buzz, march | The Wall appears: its first full statement, in the bass, with Phrygian's F natural making it a fortress. |
| 13 | Lattice | E aeolian, 126 BPM | spark_shift → spark_inv (+ spark) | glass, disco | Interlocking Sparks, each displaced from the other, like a lattice. |
| 14 | Wave | E dorian, 114 BPM, swing 62 | ascent → spark_aug (+ spark) | soft, half | Swing and glide: Ascent surfs a Dorian swell. |
| 15 | Rocket | E aeolian, 128 BPM | ascent_hi → ascent_dim (+ ascent) | pulse, gallop | Launch: Ascent from a fifth up, then at double speed, over a galloping bass. |
| 16 | Bridges | E harmonic, 122 BPM | ascent_aug → spark (+ wall_hi) | glass, four | Ascent stretched across the Wall like a viaduct; harmonic minor's D sharp lifts the span. |
| **Act III · Machines** | | | | | |
| 17 | Spiral | E aeolian, 128 BPM | wall_seq → spark_seq (+ ascent_head) | buzz, disco | The machine act begins: the Wall turned into a rising sequence, an arpeggio that spirals up the octave. |
| 18 | Pillars | E phrygian, 126 BPM | wall_aug → ascent_aug | soft, half | Space between columns: a drone, the Wall in long notes, Ascent held like a pillar. |
| 19 | Crown | E harmonic, 130 BPM | ascent → ascent_hi (+ spark) | pulse, march | Regal: Ascent as a fanfare in harmonic minor, then crowned a fifth higher. |
| 20 | Hourglass | E aeolian, 128 BPM | ascent → ascent_retro (+ spark_retro) | glass, break | Time turned over: Ascent, then Ascent backwards, the sand running the other way. |
| 21 | Shield | E phrygian, 130 BPM | spark → wall (+ spark_lo) | buzz, four | Spark held up against the Wall: hope over the machine. |
| 22 | Armored frame | E phrygian, 132 BPM | wall_hi → wall_inv (+ wall) | buzz, drive | All armor: the Wall high, the Wall inverted, the Wall below. The heaviest track of the act. |
| 23 | Rain | E aeolian, 124 BPM, swing 58 | spark_fall → ascent_inv | glass, sparse | Falling droplets: Spark stepping down over a sparse, swung beat; a breath before the last act. |
| 24 | Funnel | E harmonic, 134 BPM | spark_seq → ascent_dim (+ wall_hi) | pulse, gallop | Narrowing: everything accelerates into a v-to-i cadence that pours into the next act. |
| **Act IV · Vault** | | | | | |
| 25 | Two rooms | E dorian, 132 BPM | ascent → wall (+ spark) | soft, four | Two rooms, two moods: a bright Dorian Ascent, then the Wall next door. |
| 26 | Ring | E aeolian, 134 BPM | spark → spark_shift (+ spark_shift) | glass, disco | A round: Spark chasing itself, two steps behind, around a circle of chords. |
| 27 | Baffles | E phrygian, 136 BPM | spark_shift → wall_seq (+ ascent_stac) | buzz, break | Deflections: displaced Spark, a Wall sequence, Ascent bouncing in staccato. |
| 28 | Chevron | E aeolian, 138 BPM | spark_fall → spark_seq (+ ascent_inv) | pulse, four | V-shape: Spark falls, then climbs back the same way. |
| 29 | Split wall | E harmonic, 138 BPM | ascent_hi → wall_hi (+ spark) | buzz, drive | The Wall split by Ascent breaking through it, a fifth above. |
| 30 | Bulwark | E phrygian, 132 BPM | wall_aug → ascent_aug (+ wall) | buzz, half | Massive and slow-feeling at speed: half-time drums, the Wall in long notes, a bulwark. |
| 31 | Minefield | E phrygian, 140 BPM | spark_shift → ascent_stac | glass, sparse | Tension before the end: sparse, staccato, every note placed carefully. |
| 32 | Vault | E harmonic, 144 BPM | ascent → spark (+ ascent_5th) | pulse, drive | The finale: Ascent over the Wall, then Spark, then Ascent in fifths with itself, all three themes at once. |

## Stingers

The four moments quote the themes:

| Stinger | What it quotes |
|---|---|
| Level clear | Spark bounding up, then the E minor triad climbing to a held high E |
| Life lost | Ascent's opening leaps turned upside down and falling; it stops on B, unresolved: keep going |
| Game Over | Ascent backwards and slowed over a Phrygian F in the bass, sinking to a low E |
| Victory | Ascent in harmonic minor with Spark answering above it, resolving to E **major**: the score's one major chord, saved for winning |

## Loaded levelsets

A loaded levelset's levels have no track of their own. They play the score
in a shuffled order drawn once per session (`director::shuffled_playlist`),
so every track sounds before any repeats, and each session plays a
different order.

## Production

- **Palettes** (`compose.py`):
  - *pulse*: classic duty-cycle leads;
  - *glass*: thin 8% pulses and a gliding triangle lead;
  - *buzz*: square waves through the drive and the ladder filter;
  - *soft*: wide pulses with slow attacks.
- **Drums**: a low triangle thump for the kick, a noise snare and a short
  noise hat.
- **FX buses**:
  - bus 0: drums and bass;
  - bus 1: a dotted-eighth echo on the melody;
  - bus 2: a sidechain pump on the arpeggio;
  - bus 3: the Moog-style ladder filter for the machine tracks.
- **Mastering.** `python3 audio/compose.py --render <tool>` scales each
  track until it peaks at or under 112 of 128 with no clipped sample, at
  no more than -19 dBFS RMS. The score's loudness spread is 1.5 dB, so a
  level change never jumps in volume. A test checks that no track clips.
- **Size.** The 33 tracks are about 17 minutes of 8-bit 22,050 Hz audio,
  about 22 MB of the binary. They are read in place by the mixer, never
  decoded into memory.

## Changing the music

1. Edit a recipe, a theme or a palette in `audio/compose.py`.
2. Build the renderer:
   `cc -std=c99 -O2 -D_DEFAULT_SOURCE -D_XOPEN_SOURCE=600 -Iaudio/engine
   audio/render.c audio/engine/*.c -lm -o /tmp/bryx-render`.
3. Run `python3 audio/compose.py --render /tmp/bryx-render`.
4. Rebuild the game.

The ABC files can also be edited by hand, but `compose.py` will overwrite
them on its next run.
