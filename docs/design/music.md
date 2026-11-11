# BRYX score

32 level tracks and a menu theme: a cinematic electronic-orchestral score,
a sampled orchestra against analog-style synths. This page is the
listener's guide. The music lives as decisions in two files:
- [`audio/compose.py`](../../audio/compose.py): the notes. Themes, harmony,
  forms, and one recipe per level.
- [`audio/compose_studio.py`](../../audio/compose_studio.py): who plays
  them. It orchestrates those notes act by act and writes the
  `audio/score/*.score` files the build renders.

The sound design around it is in [`audio.md`](audio.md). The score began as
a chiptune score on the same notes; why and how it became this one is in
[`orchestral-score.md`](orchestral-score.md).

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
| inverted | every interval mirrored | Cascade, Double W, Diamond, Rain, Armored frame |
| retrograde | played backwards | Cascade, Hourglass |
| augmented | twice as slow | Heart, Bridges, Pillars, Bulwark, Lattice |
| diminished | twice as fast | Arrow, Rocket, Funnel |
| displaced | pushed two steps off the beat | Checkers, Scatter, Lattice, Ring, Baffles, Minefield |
| sequenced | repeated a step higher every bar, a ratchet | Arrow, Double W, Spiral, Armored frame, Funnel, Chevron |
| staccato | each note clipped to a sixteenth | Capped columns, Baffles, Minefield |
| an octave or a fifth higher | the same line in a new register | Twin towers (a fifth), Rocket, Crown, Split wall (an octave) |

### From motif to phrase

A theme is not looped; it is developed into a four-bar phrase
(`phrase()` in `compose.py`), the way a melody is built rather than
repeated:
- **A two-bar theme** (Ascent) becomes a *period*: stated, then answered,
  the answer closing on a half cadence.
- **A one-bar theme** (Spark) becomes a *sentence*: the idea, the idea a
  step higher, its first half twice at the peak, then a close.
- **The Wall** becomes a *climb*: restated a step higher every bar, a
  ratchet, then a close.
- **Every A and B section ends on F sharp** (F in Phrygian), a tone of
  every chord that ends a progression here, so each section leaves the
  door open for the next.
- **The recap climbs.** In A′ the first bar that can rise a third without
  passing C sharp 6 does, so the last statement of the theme reaches
  higher than any before it.

## One key, four colors

Every track is in E, because BRYX's brick sounds play the E minor
pentatonic over the music and a rally must always harmonize. The rally
takes its seventh from the track it plays over: `build.rs` reads each
track's `% mode` line, and the director plays D sharp, not D, over the
harmonic-minor tracks (a test holds every rally note to its track's
mode). The variety comes from the mode:
- **Dorian** is the bright one (C sharp): Act I's optimism, Heart, Wave,
  the menu.
- **Aeolian** is plain minor, the score's home.
- **Phrygian** is the dark one (F natural): the Wall's tracks, Minefield.
- **Harmonic minor** is dramatic (D sharp): Bridges, Crown, Funnel, Split
  wall, and the finale.

## Harmony

Each mode has its own progressions (`PROGS`), chosen to keep its color
chord and to end on the chord that leads home. A section's four bars end
on that chord, so the seam where the loop restarts is a real cadence:
- **Dorian**: IV, its bright chord, and home by bVII (Warm-up's i–IV–i–IV
  bVII); Cascade's bass walks down E, D, C sharp, B under i–bVII–IV6–v.
- **Aeolian**: home by the "mixed cadence" bVI–bVII–i, or by v.
- **Phrygian**: home by bII, the F major chord falling to E minor: the
  Phrygian cadence.
- **Harmonic minor**: home by V, B major, its D sharp the leading tone.

Contrast comes from harmony too:
- **The B section has its own progression**, starting away from the
  tonic. Some move two bars a chord, some two chords a bar.
- **The melody fits the chords.** A note on beat 1 or 3, or held from a
  beat, must be one of four things: a chord tone, a color tone (a seventh
  or ninth not a half step from the chord), an appoggiatura that steps on
  to a chord tone, or a neighbor note reached by step from one. Otherwise
  `harmonize()` moves it one degree, toward where the line goes next. It
  touches 1–21% of a lead's notes, most often under 10%, so the themes
  stay themselves.
- **The pad** moves to the nearest tone of each new chord instead of
  leaping.
- **The bass pushes** the next chord a sixteenth early in the busier
  styles.

## The four acts

The score follows the campaign's four acts:
- **I · Foundations**: Spark leads, 112–124 BPM. Restrained electronic
  tension: a drone, a soft pulse, a bell arpeggio, pizzicato and tremolo
  strings, one soft horn. The quietest act.
- **II · Structures**: Ascent takes over, 108–128 BPM, and the Wall first
  appears. Evolving orchestral movement: horns and violins on the themes,
  a moving synth bass with spiccato celli, the sequence.
- **III · Machines**: the Wall leads, 124–134 BPM. Mechanical precision:
  the saw bass alone on the Wall, celli chopping off-beats against it,
  bright spiccato on top, metal ticks, a pad pumped by the pulse, low
  brass only to close a phrase.
- **IV · Vault**: all three themes together, up to 144 BPM. Scale: trumpets
  and horns on Ascent, timpani, a gong on the recap. The only act with a
  tutti, and the loudest.

The acts average −24.5, −20.7, −20.3 and −18.2 dBFS: the campaign rises.

Every track is 16 bars, so each costs the same in the binary and loops
cleanly, but each act shapes those bars its own way:

| Act | Form |
|---|---|
| I, and the menu | intro 2 · A 4 · B 4 · A′ 4 · turn 2 |
| II | A 4 · B 4 · break 2 · A′ 4 · turn 2: straight in, the floor drops out before the recap |
| III | intro 2 · A 4 · B 4 · break 2 · A′ 4: the recap leads straight back round |
| IV | A 4 · B 4 · A′ 4 · break 2 · turn 2: the last act winds up before every restart |

| Section | What happens |
|---|---|
| intro | a hat and the arpeggio; the bass enters on 1 and 3, then in full with the kick and a snare pickup. The lead sounds the E that resolves the loop's cadence |
| A | the first theme as a phrase |
| B | the second theme, over B's own progression and bass, with the pad. A light fill mid-section, a roll into A′ |
| A′ | the recap: the theme climbing higher, and the counter-line |
| break | the floor drops out: no drums, one bass thump a bar, the pad and the theme's head in long notes; then the bass rebuilds under a hat |
| turn | a breath (no kick, bass or arpeggio) with the theme's head, then everything back, and the lead walks on to E as the loop restarts |

Measured on the renders, a track's quietest two seconds sit 6–18 dB under
its loudest: least in Act I, which stays small, and most in Act II's
breaks. The forms, the dynamics per section and the orchestration all
contribute.

### Counter-lines

The counter plays under the recap. It is one of three kinds:
- **A theme**, set an octave or so under the lead (`counter_register`),
  fitted to the chords, then voice-led against the lead: a note that
  meets the lead in a tritone on a strong beat, or forms a fifth or
  octave straight after the same interval, moves a step. Vault's recap
  carries all three themes at once: the Wall in the bass, Ascent in the
  lead, Spark between.
- **`auto`**: a line written by rule. It moves in half notes, with a
  passing note where the lead holds. It uses chord tones a third, sixth
  or tenth under the lead, moves by step, goes against the lead when the
  lead leaps, and never moves in parallel fifths or octaves. Heard in
  Checkers, Double W, Rocket, Hourglass and Shield.
- **`canon`**: the lead itself a beat later and an octave down. That is
  Ring's round.

Twin towers is the one deliberate exception: its counter is Ascent a
diatonic fifth above itself, a power-chord doubling, in Dorian where every
one of those fifths is perfect.

Tracks run 27–36 seconds; the menu runs 42.

| # | Level | Mode, tempo | Themes (A → B, counter) | Harmony | Palette, drums | Idea |
|---|---|---|---|---|---|---|
| **Act I · Foundations** | | | | | | |
| 01 | Warm-up | E dorian, 112 BPM, swing 54 | spark → ascent_head | lift | pulse, half | Spark in its plainest form over a Dorian i-IV sway; Ascent's head appears in the middle, as a promise. |
| 02 | Cascade | E dorian, 114 BPM | spark_fall → spark_retro (+ ascent_inv) | fall_dorian | glass, half | Everything descends: Spark stepped down a degree at a time, then backwards; Ascent turned upside down underneath; a bass that walks down E, D, C sharp, B. |
| 03 | Checkers | E aeolian, 116 BPM | spark → spark_inv (+ auto) | epic | pulse, break | Call and answer: Spark, then Spark inverted; under the recap a second voice moves in thirds and sixths through the squares the lead leaves empty. |
| 04 | Twin towers | E dorian, 118 BPM | ascent → spark (+ ascent_5th) | climb | glass, four | Ascent arrives in full, then doubled a fifth above: two towers of the same theme. |
| 05 | Arrow | E aeolian, 124 BPM | ascent_dim → spark_seq | drive | pulse, four | Ascent at double speed, straight as an arrow over a driving sixteenth bass; Spark climbs a step a bar. |
| 06 | Capped columns | E dorian, 116 BPM | ascent_stac → spark | sway | soft, march | Ascent clipped short, note by note, like columns with caps; a marching snare holds the line. |
| 07 | Diamond | E dorian, 120 BPM, swing 54 | ascent → ascent_inv (+ spark) | circle | glass, half | Symmetry: Ascent and its mirror image, facets of one theme. |
| 08 | Scatter | E aeolian, 122 BPM, swing 56 | spark_shift → ascent_head | epic | pulse, break | Spark knocked off the beat and lightly swung; Ascent's head returns in the middle, still a promise, as the first act ends. |
| **Act II · Structures** | | | | | | |
| 09 | Picture frame | E aeolian, 120 BPM | ascent → spark_aug (+ ascent_aug) | heroic | soft, four | Ascent framed by itself: stated, then, in the recap, held in long notes beneath its own return. |
| 10 | Heart | E dorian, 108 BPM, swing 60 | ascent_aug → spark (+ spark) | lift | soft, half | The warm one: Ascent slowed to a sigh over a swung half-time beat; each loop's last bar slows, a breath out. |
| 11 | Double W | E aeolian, 124 BPM | spark_seq → ascent_inv (+ auto) | drive | pulse, break | Zig-zag: Spark climbing a degree a bar, then Ascent upside down; the arpeggio never stops changing direction. |
| 12 | Fortress | E phrygian, 118 BPM | ascent → wall | dark | buzz, march | The Wall appears: its first full statement, in the bass, with Phrygian's F natural making it a fortress; the filter squeals as B climbs. |
| 13 | Lattice | E aeolian, 126 BPM, swing 54 | spark_shift → spark_inv (+ spark_aug) | circle | glass, disco | Interlocking Sparks: one knocked off the beat, and in the recap Spark in long notes threaded underneath. |
| 14 | Wave | E dorian, 114 BPM, swing 62 | ascent → spark_aug (+ spark) | lift | soft, half | Swing and glide: Ascent surfs a Dorian swell. |
| 15 | Rocket | E aeolian, 128 BPM (to 138) | ascent_hi → ascent_dim (+ auto) | climb | pulse, gallop | Launch: Ascent an octave up, then at double speed while the tempo climbs, over a galloping bass. |
| 16 | Bridges | E harmonic, 122 BPM | ascent_aug → spark (+ wall_hi) | heroic | glass, four | Ascent stretched across the Wall like a viaduct; harmonic minor's D sharp lifts the span. |
| **Act III · Machines** | | | | | | |
| 17 | Spiral | E aeolian, 128 BPM | wall_seq → spark_seq (+ ascent_head) | circle | buzz, disco | The machine act begins: the Wall turned into a rising sequence, an arpeggio that spirals up the octave. |
| 18 | Pillars | E phrygian, 126 BPM | wall_aug → ascent_aug | pedal | soft, half | Space between columns: a bass that keeps one note a bar, the Wall in long notes, Ascent held like a pillar, in the one machine track with a room. |
| 19 | Crown | E harmonic, 130 BPM | ascent → ascent_hi (+ spark) | heroic | pulse, march | Regal: Ascent as a fanfare in harmonic minor, then crowned an octave higher, broadening into its last cadence. |
| 20 | Hourglass | E aeolian, 128 BPM | ascent → ascent_retro (+ auto) | fall | glass, break | Time turned over: Ascent, then Ascent backwards, the sand running the other way; the tempo flows forward through A and back through B. |
| 21 | Shield | E phrygian, 130 BPM | spark → wall (+ auto) | dark | buzz, four | Spark held up against the Wall: hope over the machine. |
| 22 | Armored frame | E phrygian, 132 BPM | wall_hi → wall_inv (+ wall_inv_aug) | dark | buzz, drive | All armor: the Wall high and ratcheting up, the Wall inverted and sinking, and in the recap the inverted Wall at half speed beneath the high one. The heaviest track of the act. |
| 23 | Rain | E aeolian, 124 BPM, swing 58 | spark_fall → ascent_inv | fall | glass, sparse | Falling droplets: Spark stepping down over a sparse, swung beat; a breath before the last act. |
| 24 | Funnel | E harmonic, 134 BPM (to 146) | spark_seq → ascent_dim (+ wall_hi) | cadence | pulse, gallop | Narrowing: Spark ratchets up a step a bar as the tempo climbs to 146, Ascent doubles its speed, and a V-to-i pours into the next act. |
| **Act IV · Vault** | | | | | | |
| 25 | Two rooms | E dorian, 132 BPM | ascent → wall (+ spark) | lift | soft, four | Two rooms, two moods: a bright Dorian Ascent, then the Wall next door, in a bigger room. |
| 26 | Ring | E aeolian, 134 BPM, swing 55 | spark → spark_shift (+ canon) | circle | glass, disco | A round: Spark chasing itself a beat behind, around a circle of chords. |
| 27 | Baffles | E phrygian, 136 BPM | spark_shift → wall_seq (+ ascent_stac) | dark | buzz, break | Deflections: displaced Spark, a Wall sequence, Ascent bouncing in staccato, and an echo thrown off the end of every B bar. |
| 28 | Chevron | E aeolian, 138 BPM | spark_fall → spark_seq (+ ascent_inv) | fall | pulse, four | V-shape: Spark falls, then climbs back the same way, and the tempo makes the same V. |
| 29 | Split wall | E harmonic, 138 BPM | ascent_hi → wall_hi (+ spark) | cadence | buzz, drive | The Wall split by Ascent breaking through it, an octave above. |
| 30 | Bulwark | E phrygian, 132 BPM | wall_aug → ascent_aug (+ wall) | pedal | buzz, half | Massive and slow-feeling at speed: half-time drums, the Wall in long notes, a bulwark. |
| 31 | Minefield | E phrygian, 140 BPM | spark_shift → ascent_stac | dark | glass, sparse | Tension before the end: sparse, staccato, every note placed carefully. |
| 32 | Vault | E harmonic, 144 BPM (to 150) | ascent → spark (+ spark) | heroic | pulse, drive | The finale: Ascent over the Wall, then Spark, the tempo pushing to 150; in the recap, at its loudest, all three at once, the Wall below, Ascent above, Spark between. |

## Stingers

The five moments quote the themes:

| Stinger | What it quotes | Who plays it |
|---|---|---|
| Level clear | Spark bounding up, then the E minor triad climbing to a held high E over a G and B | spiccato violins and the bell, then strings and horns; light, as it is heard 32 times a run |
| Act clear | Spark bounding up, Ascent's head over a bass walking up to E, a held E minor | spiccato and the bell, then horns over celli, then strings, horns and bass |
| Life lost | Ascent's opening leaps turned upside down and falling, fading as they fall; it stops on B, unresolved: keep going | horns over celli |
| Game Over | Ascent backwards and slowed over a Phrygian F in the bass, thirds above, sinking to a low E as the tempo slows | a horn, the strings' thirds, low strings and tuba, in the long hall |
| Victory | Ascent in harmonic minor with Spark answering above it over the pulse, broadening into a full E **major** chord: the score's one major chord, saved for winning | trumpets and horns, spiccato and the bell, the saw bass, then the full orchestra |

## Hearing a track

The menu's last row, Music, is a jukebox: ←/→ walks the 33 tracks by name
and each plays while the row is highlighted (`src/menu.rs`).

## Loaded levelsets

A loaded levelset's levels have no track of their own. They play the score
in a shuffled order drawn once per session (`director::shuffled_playlist`),
so every track sounds before any repeats, and each session plays a
different order.

## Orchestration

How `compose_studio.py` turns the notes into a track. The rules came out
of the reference cues and their listening ([`reference-cues.md`](reference-cues.md)).

- **Tracks 01, 09, 22, 32 and the menu** are the reference cues, written
  by hand. The other 28 take `compose.py`'s parts (lead, counter, bass,
  arpeggio, pad, drums, with a dynamic for every step) and orchestrate
  them by act.
- **Melody goes to articulated voices.** The samples have no legato
  transitions, so a theme is carried by tongued brass, pizzicato, spiccato
  or the bell, a level above its accompaniment. Sustained strings carry
  harmony. A theme in long notes (the augmented variants) gets a soft
  synth lead under it for the connection.
- **One owner per register, about four jobs at once**: melody, bass,
  harmony, motion. The composer refuses a track that averages more than
  seven parts or sounds more than twelve at once.
- **Who plays what** depends on the act and on the theme's family:

  | | Spark | Ascent | The Wall |
  |---|---|---|---|
  | Act I | pizzicato violins, the bell above | horns, softly | spiccato violas |
  | Act II | violins, detached, the bell above | horns, violins an octave up | spiccato violas and violins |
  | Act III | spiccato violins, the bell | horns in their middle | spiccato violas and violins |
  | Act IV | spiccato violins, the bell | trumpets, horns an octave below | spiccato violas and violins |

- **The bass**: pizzicato celli in Act I; the saw bass from Act II, with
  spiccato celli on its beats in Acts II and IV, and celli chopping
  off-beats against it in Act III.
- **The arpeggio** is the machine: the synth sequence, or the bell on the
  glass tracks; thinned to eighths in Act I.
- **The pad**: tremolo strings (I), sustained strings (II), a synth pad
  pumped by the pulse (III), tremolo strings and, in the recap, trombones
  (IV).
- **Drums**: the kick is the electronic pulse. Snare and hat become metal
  ticks, from Act III only. The rolls into the recap are timpani.
  Timpani mark the phrases from Act III; a cymbal swells into the recap
  from Act II; Act IV's recap lands on a gong.
- **Dynamics** (`SECTION_LEVEL` in `compose.py`): intro mp, A and B mf,
  break p, the recap f, and a swell into the recap beat by beat; each act
  shifts and caps them (`ACT_SHIFT`, `ACT_CEILING`), and sets its overall
  level (`ACT_TRIM`). A swell is orchestration (sections entering, tremolo,
  a filter opening, a roll), with a gain ramp under it, never instead.
- **The orchestra breathes; the machine does not.** Orchestral notes get
  a few milliseconds of deterministic timing and velocity spread; synths
  stay on the grid.
- **Per-track touches** (recipe keys), where a track's idea calls for them:
  - Heart sighs, slowing in each loop's last bar. Hourglass's tempo flows
    forward through A and back through B, and Chevron's makes a V. Crown
    broadens into its last cadence. Funnel, Rocket and Vault accelerate
    through B and settle back.
  - Two rooms moves B into the long hall. Bridges, Rain, Ring, Wave,
    Heart and Pillars sit deeper in the hall; Arrow and Minefield
    stay nearly dry.
  - The machine tracks' bass filter opens across B.
  - Scatter and Baffles throw an echo off the end of every B bar.
- **In the recipe table below**, "palette" and "drums" are `compose.py`'s
  names for a track's texture, written for the chip score. They now steer
  the arranger: the palette picks the arpeggio's voice, the drum style
  its pattern.
- **Seamless loops.** Each track renders with its tails wrapped (`loop
  on`) and is stored so it decodes cleanly across the wrap.
- **Size.** The 33 tracks are about 17 minutes of 48 kHz stereo, 38 MB
  as QOA. They are decoded as they play, never into memory.

## The score lint

`python3 audio/compose.py` prints, for every track:
- how many of the lead's strong beats miss the chord, and how many notes
  the harmonic fit moved;
- for the counter: its attacks, how many it shares with the lead, how
  often it crosses above the lead, meets it in a tritone on an attack, or
  moves with it in parallel fifths or octaves.

A track fails in five cases:
- more than 10% of its strong beats miss the chord;
- the counter meets the lead in any tritone on an attack;
- under a fifth of the attacks in the counter's span belong to one voice
  alone (it only moves in the lead's rhythm);
- the counter sits above the lead on more than a quarter of its attacks;
- there are more than two parallel fifths or octaves.

`compose_studio.py` runs this lint first and writes nothing if a track
fails. It then checks each orchestrated track: every pitched note in the
track's mode (the rally plays over it), and a texture that can be heard.

## How the score was reviewed

The second version of the score came from a review against the
[music-composition-skills](https://github.com/jtydhr88/music-composition-skills)
library (MIT), which covers melody, harmony, progressions, counterpoint,
form, arrangement and a checklist of the habits of generated music. Four
reviews of the first version measured what a listener would hear:
- **Harmony.** About half the lead's strong beats missed the chord
  (chance level), and no loop seam ever cadenced.
- **Development.** The themes were looped, not developed, and the recap
  was a copy.
- **Counter-lines.** Three counters were octave doublings.
- **Energy.** Every track sat at one level from end to end.

Everything above under "From motif to phrase", "Harmony", the forms, the
counter-lines and the lint is the answer. The library was used as a
reference; none of it is vendored.

## Changing the music

1. Edit a recipe or a theme in `audio/compose.py`, or the orchestration in
   `audio/compose_studio.py`.
2. Run `python3 audio/compose_studio.py`. It lints, then rewrites
   `audio/score/`.
3. Rebuild the game: `build.rs` renders what changed.

The scores can be edited by hand, but `compose_studio.py` will overwrite
them on its next run. `python3 audio/compose_studio.py --reference`
rewrites the Phase 3 reference cues in `audio/reference/cues/`.
