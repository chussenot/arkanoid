# BRYX studio: Phase 2 technical report

Phase 2 of the orchestral score (`orchestral-score.md`) built the sound
engine the score will be written for. This report says what the engine
can do, what was measured, what is not yet known, and what the listening
gate has to decide. It describes `claude/orchestral-score` at the commit
that adds it.

**Summary.**
- **Built:** the studio is complete, tested, and wired into the build and
  the game. It renders a `.score` (sampled orchestra plus analog-style
  synths, buses, reverbs, a limiter) to 48 kHz stereo QOA, and the game
  streams that QOA in place of a cue's ABC render.
- **Measured:** every property the design asked for, except the few
  listed below as not measured.
- **Not yet in the game:** no music has moved yet. The game still plays
  the ABC score, so nothing a player hears has changed.
- **Not answerable by measurement:** whether the studio sounds like
  BRYX's score should. The five reference renders below exist for that
  question.

## 1. What was built

| Part | Where | Size | Tests |
|---|---|---|---|
| Score format, scheduler, mixer, render | `audio/engine/studio.c` | 1,086 lines | studio probes (§3) |
| Shared helpers: RNG, ADSR, patch defaults | `studio_common.c`, `studio.h` | 304 | through the modules |
| Synth voice | `studio_synth.{h,c}` | 728 | `tests/test_synth.c`, 59 checks |
| Sampler: SFZ subset, FLAC | `studio_sampler.{h,c}` | 1,066 | `tests/test_sampler.c`, all pass |
| Effects: reverb, delay, EQ, dynamics | `studio_fx.{h,c}` | 975 | `tests/test_fx.c`, 62 checks |
| Instrument library and mixing desk | `audio/instruments/*.inc` | 3 files | probes, reference renders |
| Samples: VSCO 2 CE subset, CC0 | `audio/samples/` | 26.7 MB, 295 regions, 24 maps | sampler tests, probes |
| Renderer CLI: `.score` to QOA, f32 or s16 | `audio/render.c` | | build |
| Build step: parallel and cached | `build.rs` | | `cues.rs` tests |
| QOA streaming decoder | `src/audio/qoa.rs` | 289 lines | bit-exact against the reference |
| Studio music in the mixer | `src/audio/mixer.rs` | | 6 new tests |
| Studio probes | `audio/probes/studio_*.score` | 8 probes | `src/audio/studio_probes.rs` |

No dependency was added to `Cargo.toml`. `dr_flac.h` (public domain) and
`qoa.h` (MIT) are vendored C headers, compiled only into the build-time
renderer (`audio/vendor/README.md`).

### Signal path

```
note ─▶ voice (sampler | synth) ─▶ instrument (gain lane, pan, sends)
     ─▶ bus (EQ, compressor, optional key, gain lane, sends) ─▶ master
                     sends ─▶ aux (reverb | delay) ─▶ master
master: gain lane ─▶ glue compressor ─▶ 5 ms lookahead limiter (−1 dBFS)
     ─▶ TPDF dither (fixed seed) ─▶ 16-bit ─▶ QOA ─▶ embedded in the binary
game: QOA ─▶ streamed a 20-frame slice at a time ─▶ cubic resample to the
     device rate ─▶ music bus (duck, crossfade) ─▶ soft limiter ─▶ cpal
```

### The instrument palette (`audio/instruments/`)

| Family | Instruments |
|---|---|
| Strings | `strings_high`, `_mid`, `_low` and `_bass` (sustain); `_spic`, `_trem` and `_pizz` articulations |
| Brass | `brass_horns`, `brass_trumpets`, `brass_trombones`, `brass_low`, each with a `_stac` version |
| Percussion | `timpani`, `timpani_roll`, `percussion` (bass drum, cymbal, swells, gong, anvil) |
| Electronic | `synth_bass`, `synth_sequence`, `synth_pad`, `synth_lead`, `synth_glass`, `electronic_drone`, `electronic_air`, `electronic_pulse`, `electronic_metal` |
| Desk | aux `room`, `stage`, `hall`, `long`, `echo`; buses `orchestra`, `electronic` (a 1.2 kHz cut), `percussion`; master |

Sections are built from solo players by stacking detuned, delayed copies
(horns ×3, trombones ×2, basses ×2). Seating follows a concert stage:
violins left, violas centre, celli and basses right.

## 2. Measured, module by module

Measured by each module's own C tests at 48 kHz unless stated.

**Synth** (`test_synth.c`):
- **Pitch:** within 0.0001 cents at 55 and 440 Hz, at both 48 and
  44.1 kHz.
- **Aliasing:** a 3,520 Hz saw keeps its aliasing 33.5 dB under its
  harmonics (a naive saw: 9 dB). Triangle 41.7 dB, square 33.4 dB.
- **Filters:**
  - SVF low-pass: −3.01 dB at its cutoff.
  - Resonance 0.8: +25.3 dB at the cutoff (theory: 25.6).
  - Rate independence: 48 and 44.1 kHz agree within 0.33 dB.
  - Ladder: a 4-pole slope.
- **Envelopes:** attack and release within their times; a released voice
  adds nothing after it ends.
- **Modulation:** a 5 Hz, 30-cent vibrato measures ±30.0 cents. Glide
  reaches −1.3 cents of its target in 1.5 times its time.
- **Cost:** 430–730 simple voices, or 130–195 seven-voice stereo ladder
  pads, in real time on one core.

**Sampler** (`test_sampler.c`):
- **Pitch:** tracking within 0.000 cents, including the
  sample-rate-converted case (a 44.1 kHz recording played at 48 kHz).
- **Mapping:** zone and velocity-layer selection are exact.
- **Loops:** the seam's second difference matches the signal's own
  (ratio 1.000), and the level holds.
- **Release:** ends within 1 ms of its time, fading to zero, with no step.
- **Loading:** all 24 maps (28.6 M frames, 108.9 MB as float) in 0.47 s.

**Effects** (`test_fx.c`):

| Reverb preset | Measured RT60, mid-band | Pre-delay | L/R correlation |
|---|---|---|---|
| room | 0.578 s (preset 0.6) | 5.00 ms | +0.04 |
| stage | 1.167 s (1.2) | 12.00 ms | −0.11 |
| hall | 2.358 s (2.4) | 25.00 ms | −0.01 |
| long | 5.085 s (5.5) | 40.00 ms | +0.08 |

At 44.1 kHz the presets measure within 3% of these. Other effects:
- **Ping-pong delay:** the repeats alternate exactly (L 1.0, R 0.5,
  L 0.25).
- **Biquads:** within 0.001 dB of their design.
- **Limiter:** holds −1 dBFS on +13 dBFS peaks, and is bit-transparent
  below the ceiling (output = input, delayed 240 frames).

**QOA** (`qoa.rs`, the mixer):
- **Decoding:** bit for bit against the reference decoder on a
  committed fixture.
- **Real-time behaviour:** no allocation, and a fixed amount of work per
  frame. This holds by construction (no heap type in the decoder or the
  stream). No test hooks the allocator.
- **Error:** on a 24 s hybrid render, QOA measures 44.4 dB SNR against
  the float master.

## 3. Measured on the studio's own renders

`audio/probes/studio_*.score`, rendered by `build.rs` and checked by
`src/audio/studio_probes.rs` in CI:

| Probe | Measured | Required |
|---|---|---|
| Tuning | VSCO violins A4 +1.5 cents (a 100 ms window wanders −3.5 to +5.5, the players' intonation); synth sine 0.00 | ±15, ±1 |
| Velocity layers | f layer 28.2 dB over p, brightness 0.174 vs 0.151 | > 12 dB, brighter |
| Held past the recording | celli E2 held 10 s: every 100 ms within ±2.1 dB | ±3 dB |
| Hall decay | RT60 at 500 Hz 2.29 s; shorter at 4 kHz | 2.4 s ±15% |
| Seating | violins L−R +6.4 dB, celli −4.3 dB | > ±2 dB |
| Limiter | tutti driven 18 dB hot: peak −1.000 dBFS | ≤ −0.99 |
| Loop seam | curvature across the wrap 0.0004, inside the loop up to 0.0037 | under half |
| Keyed compressor | pad ducked about 20 dB 20–80 ms after each kick, back within 3 dB | > 6 dB |

### Defects the probes found, all fixed

- **The loop seam stepped by 0.23 at every wrap,** against 0.004 anywhere
  inside the loop; every music loop would have clicked. Two causes:
  - each pass seeded its voices and advanced its round robins
    differently;
  - a reverb's free-running modulation never repeats in step with a loop.

  The fix: every pass now plays identical voices, a loop warms up for
  10 s, and the 100 ms that truly follow the kept pass are crossfaded
  into its start. The probe failed with that crossfade disabled
  (0.226 against 0.004), so it guards the fix.
- **A stolen voice was cut mid-sound** (a full-scale step). It now fades
  over 5 ms in its own slot.
- **`F#3` in a score was read as `F` and a comment.** A `#` now starts a
  comment only at the start of a line or after a space.
- **Uneven sample levels between keys.** The curation normalised per
  instrument, keeping VSCO's session levels: timpani drum 1 sat 20–25 dB
  under its neighbours, and the tuba jumped 21 dB between two zones.
  Each key's loud layer now reaches the same loudness. Each soft layer
  keeps its own note's ratio, clamped to −18..−6 dB.

## 4. Reference renders

`audio/reference/*.score` are five family checks: plain E minor material,
deliberately none of the score's themes. Render one with the build's
renderer:

```bash
R=$(ls -t target/*/build/bryx-*/out/bryx-cue-render | head -1)
$R audio/reference/hybrid.score hybrid.f32       # or .qoa, .s16
```

| Render | Length | Render time | Peak | RMS | L/R correlation |
|---|---|---|---|---|---|
| strings | 30.7 s | 1.4 s | −4.5 dBFS | −22.2 dBFS | 0.70 |
| brass | 28.0 s | 1.4 s | −3.4 | −22.6 | 0.69 |
| percussion | 30.0 s | 1.4 s | −4.3 | −27.1 | 0.88 |
| synths | 18.9 s | 1.6 s | −2.8 | −19.3 | 0.63 |
| hybrid tutti | 30.0 s | 2.3 s | −1.0 (limiter) | −17.6 | 0.71 |

Energy by band, in dB relative to the whole render:

| Render | 20–60 Hz | 60–150 | 150–300 | 300 Hz–1 kHz | 1–3 kHz | 3–8 kHz | 8–16 kHz |
|---|---|---|---|---|---|---|---|
| strings | −37 | −9 | −6 | −3 | −10 | −19 | −41 |
| brass | −40 | −17 | −10 | −1 | −10 | −22 | −37 |
| synths | −8 | −7 | −5 | −7 | −11 | −19 | −33 |
| hybrid | −7 | −7 | −9 | −5 | −12 | −19 | −42 |

The research's mix rule shows in the numbers: synths own the low pulse,
and the orchestra owns 300 Hz–3 kHz.

## 5. What is not known, or not done

These are ranked by how much they could change the score.

1. **Whether it sounds right.** Every number above can hold, and the
   music can still sound synthetic. This needs ears (§6).
2. **No true legato.** VSCO 2 CE has no legato transitions, so a sampled
   melodic line is note after note with overlapping attacks. This
   matters most for Act II's melodic strings and horns. The mitigations
   are slower attacks, overlap, and doubling the line with
   `synth_lead`. If they are not enough, a legato sample set is the next
   step, and that is a new licensing question.
3. **The top octave was dark; diagnosed and lifted.** Measured stage by
   stage on the strings reference, in dB at 8–12 kHz relative to
   300 Hz–1 kHz:

   | Stage | Contribution |
   |---|---|
   | VSCO's recordings | Most of it: one channel of a violin section sits at −38 dB |
   | Mono fold-down at curation | About 2 dB more |
   | Sampler | Transparent: a note at its keycenter renders within 0.3 dB of its file up to 12 kHz |
   | Reverb return | 1–4 dB darker than the dry signal, but the mix sits on the dry. Cutting the hall's damping from 5 to 6.5 kHz moved the return by 0.6 dB and the mix by 0.1 dB, so the presets stay |

   The fix is on the orchestra bus only (`desk.inc`): a +2 dB presence peak
   at 3.2 kHz and a +4 dB shelf above 7 kHz. The strings gain 3.3 dB at
   8–12 kHz and 1.7 dB at 2–4 kHz, peaks move by under 1 dB, and the
   synths and percussion are untouched. The strings still sit about 7 dB
   under the synths up there: defined, not bright.
4. **Two dynamic layers, switched at velocity 80.** A crescendo across
   separate notes changes timbre in one step. Swells inside a held note
   use gain ramps (`ramp <inst>.gain`) instead, as the references do.
5. **QOA on quiet passages.** 44 dB SNR is ordinary for QOA, and its
   noise follows the signal, so it is least masked on sparse material.
   Listen to Act I's quietest moments in particular.
6. **Size.** Today's 33 music tracks average 31 s. As 48 kHz stereo QOA
   they would take about 39 MB, against 23 MB of mu-law now. Longer
   tracks scale linearly. The release binary is 40 MB today.
7. **Determinism across machines.** A render is byte-identical on one
   machine. Across C libraries, `sinf`, `expf` and `tanf` may differ in
   their last bits. Renders are build outputs, not committed, so this
   moves no hash; the game's replay hashes never involve audio.
8. **Smaller gaps:**
   - the synth's sub oscillator is sine only;
   - LFOs are free-running, not tempo-synced;
   - stingers and effects still come from the ABC path (the mixer streams
     studio audio for music only).

From §5 of the design, these are not measured by any test:
- a per-voice cost for the sampler (the whole renders run 13–22 times
  faster than real time);
- an allocator hook on the audio callback;
- byte-for-byte determinism across machines.

## 6. The gate

The design's gate stands: the score is not rewritten until the engine
convincingly renders an orchestral and electronic test, and this author
cannot hear. Listen to the five renders, and in particular:

- **Strings:** does the swell sound like a section or a keyboard? Does
  the spiccato sit tight?
- **Brass:** do the horns sound like a section? Do the stabs have bite?
- **Percussion:** do the timpani have weight? Does the gong bloom?
- **Synths:** is the bass punchy without mud, and the sequence crisp? Do
  the glass and air sparkle?
- **Hybrid:** do the orchestra and the machine sit in one space? Is the
  arrival big? Is it dark, or right?

If the answer is broadly yes, Phase 3 writes the six reference cues in
this studio. If not, say what is wrong, and the fix (curation, desk,
instruments or engine) comes first.
