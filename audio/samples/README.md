# Orchestral samples

A ~25 MB subset of [VSCO 2 Community Edition](https://github.com/sgossner/VSCO-2-CE)
(CC0 1.0), the sample-based half of BRYX's score
([`docs/design/orchestral-score.md`](../../docs/design/orchestral-score.md)).

- `strings/`, `brass/`, `percussion/`: 16-bit mono 44.1 kHz FLAC.
- `*.sfz`: one instrument each, in the SFZ subset the engine reads:
  `<region>` lines with `sample`, `lokey`/`hikey`, `pitch_keycenter`,
  `tune` (cents), `lovel`/`hivel`, and `loop_start`/`loop_end` for
  sustains, whose loop crossfade is baked into the file.

| Family | Instruments |
|---|---|
| Strings | violins, violas, celli (sections) and basses (solo): sustain (violins and celli in two dynamic layers), spiccato, tremolo, pizzicato (violins, celli) |
| Brass | horn (sustain in two layers, staccato), trumpet, trombone, tuba (sustain, staccato) |
| Percussion | timpani (5 drums, two layers), timpani rolls, `kit.sfz`: bass drum (35), suspended cymbal hit (49), cymbal swells short/medium/long (52-54), gong (55), anvil (56) |

Do not edit these files: rerun `curate.py VSCO_CHECKOUT` instead. It
checks every sustain's pitch against its name (and corrects the one VSCO
file named an octave off), so the map plays in tune.
