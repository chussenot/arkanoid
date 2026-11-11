# BRYX: the cinematic electronic-orchestral score

Phase 1 of the audio transformation: research, artistic direction, and the
engine architecture that Phase 2 builds. This is a design document, not
a description of current behavior. `audio.md` and `music.md` remain true
until the phases below land.

**Phases:**
1. research and architecture (this document);
2. the sound engine;
3. six reference cues (**gate: approved by ear**; see [`reference-cues.md`](reference-cues.md));
4. the score;
5. integration.

No phase starts before the one before it is validated.

## 1. What the reference teaches

The reference is TRON: Legacy (2010).
- Daft Punk composed and produced it.
- Joseph Trapanese arranged and orchestrated it.
- Gavin Greenaway conducted an orchestra of about 85–90 players at AIR Lyndhurst.

We take its production philosophy, never its material: no melodies,
motifs, arrangements, samples or presets. The sources and their
reliability are listed at the end; anything not verified is marked as
such.

**Verified, and what each means for BRYX:**

| What is documented | Source | What BRYX takes from it |
|---|---|---|
| The orchestra is written first. Bangalter: "no way we were going to do this film score with two synthesizers and a drum machine" | FactMag 2010 | Strings, brass and timpani carry the themes. The synths are a second voice, not the bed. |
| The synths were confined to "only using this pulsing bass", because "anything that was in the medium range… would conflict with dialogue" | Bangalter, A24 podcast, via Far Out and Vice | **Frequency roles.** Synths own the low pulse (below about 250 Hz) and the high sparkle (above about 3 kHz). The orchestra owns the middle. |
| Every cue was finished before the sound effects, and was "a major factor in defining sound effects" | Addison Teague, via Mix Online | Gameplay sounds are BRYX's "dialogue". The score leaves them the 300 Hz–3 kHz band, and the effects are designed against the finished score. |
| "Classic Hollywood scores… clash[ed] against electronics and 1970s science fiction"; "a lot of negative space… less is more" | Bangalter, via Collider | Restraint is the default and density is earned. Many tracks start sparse. |
| The music came before the script; the film was "cut to the music" | Collider; Dazed & Confused 2010 | The music sets the pacing: act forms, transitions and finales are written as music first. |
| Ostinatos everywhere: rising and falling staccato string figures, low-string ostinati, filtered synth figures, drones under the themes | Cue By Cue analysis | The Wall is an ostinato. Every track has a repeating cell, and orchestration changes over it. |
| The main theme rises, then winds down and resolves. It is introduced on low horns and later restated electronically | mfiles; Cue By Cue | Ascent is stated by horns and answered by synth, and each lens develops it. |
| Arpeggios and strings "finish each other's phrases" | AllMusic | Call and response between string figures and the synth sequence, rather than doubling. |
| Climaxes are built from a timpani-led crescendo, noble horn lines and gradual layering | mfiles | The four acts are one long crescendo, and Act IV is the summit. |
| Influences the composers named: Wendy Carlos, Vangelis, Herrmann, Steiner, Carpenter, Glass, Jarre | LA Times and FactMag (paraphrase) | Minimalist repetition (Glass), the Herrmann ostinato as tension, Steiner's thematic development, Vangelis's sustained synth brass. |

**Not documented (do not repeat as fact):**
- The hardware used. Forum claims mention a Modcan modular, an Oto Biscuit and u-he Zebra, all unverified.
- Harmonic analysis, including chromatic mediants or specific progressions.
- Tempos.

We describe sounds by their synthesis, not by gear:
- **Bass:** low sawtooth layers with little detune and a mostly closed resonant low-pass, opened by an envelope.
- **Sequence:** an octave-leaping 16th-note line over a drone.
- **Pads:** gated or slow-attack pads.

## 2. BRYX's artistic direction

**Statement.** A mechanical, futuristic arcade score with orchestral scale:
human orchestral expression set against machine-precise electronic
repetition. It is dark and monumental, rhythmic without being EDM, and
melodic first.

**What stays from the current score** (`music.md`):
- E minor as the shared tonal center, with the four modes: Dorian, Aeolian, Phrygian and harmonic minor.
- The three themes: Ascent, the Wall and Spark.
- The four acts.
- Deterministic composition (`compose.py`) and rendering at build time.
- The rally keeps its notes: E minor pentatonic with the playing track's own seventh, topping out on the tonic.
- Harmony stays within each track's mode. Mediant colors (VI and III) are welcome, but chromatic chords that clash with the rally's E–G–A–B are not.

**Themes, re-orchestrated, not rewritten:**

| Theme | Role | Treatment |
|---|---|---|
| Ascent | The player | Stated by horns, or by high strings in octaves. Synth sequence answers between phrases. Harmony widens act by act, and the full brass statement is saved for Act IV. |
| The Wall | The machine | Ostinato in cellos and basses, doubled an octave below by a filtered sawtooth bass. Short metallic accents. Each act adds a layer: bass, then low brass, then timpani, then the full low orchestra. |
| Spark | Hope | High strings over a bell-like synth arpeggio and a warm pad. In Act IV it is lifted into the full orchestra. |

**The acts as one arc:**

| Act | Mood | Leading theme | Orchestra | Electronics | Dynamic ceiling |
|---|---|---|---|---|---|
| I · Foundations | Discovery | Spark | Delicate strings (tremolo, harmonics), restrained horns, soft timpani | Sparse pulse, drone, bell arpeggio | mp to mf |
| II · Structures | Scale, rising tension | Ascent | Full string section; horns enter often; first low brass | Driving sequence, moving bass | f |
| III · Machines | Mechanical, relentless | The Wall | Cellos, basses and low brass in front; rhythmic string patterns; timpani and metal | Aggressive bass, filter sweeps, sidechained pulse | f to ff, dark |
| IV · Vault | Monumental climax | All three, combined | Tutti, brass chorales, cymbal swells, timpani rolls | Dense but controlled sequence and bass | ff, with real pianissimo before it |

**Mix hierarchy:**
1. **Gameplay sounds** stay intelligible. Their band (300 Hz–3 kHz) stays clear of synths, and they are mixed above the score.
2. **Stingers** duck the score briefly, as they do today.
3. **The score** sits under both. Its dynamics are preserved, with no master pumping.

**What stays in the score's palette, and what goes:**
- **Kept:** brass, strings, timpani, sawtooth bass and arpeggio.
- **Ruled out:** generic risers, "whoosh" sweeteners and EDM drops.
- **Effects:** short and readable, built from the same parts. For example, a launch is a filtered rise over a low pizzicato or timpani tap, and an act clear previews the next act's orchestra.

## 3. Where the engine stands (audit of `master` at 764fe90)

The engine is Electric Pulse forked by BRYX, about 3,000 lines of C. Its ceiling for this score:

| Limit | Where | Consequence |
|---|---|---|
| No sample playback at all | `audio_dsp.c:82` (`dsp_osc_next` is the only source) | Strings and brass are impossible |
| Mono from end to end | `audio_mix.c:278,405-410`; `render.c:37`; `build.rs:165` | No orchestral seating, no stereo hall |
| 22,050 Hz output | `build.rs:26`; `cues.rs:10`; `probes.rs:11`; `compose.py:1292` | Nothing above about 11 kHz: no air on strings or cymbals |
| Stored as 8-bit mu-law | `mulaw.rs`; `build.rs:181` | About 37 dB of signal-to-noise in quiet passages: pianissimo and reverb tails hiss |
| Filter coefficients fixed, independent of sample rate | `audio_fx.c:117,143,380,392` | Raising the rate silently moves every cutoff |
| Mono reverb: 4 combs, 2 all-passes, no pre-delay | `audio_fx.c:238-297` | No hall and no depth |
| 4 effect buses, no per-voice sends, no group buses, hard clamp on the master | `electric_pulse.h:22`; `audio_mix.c:380-408` | No orchestra, electronic and percussion stems; no limiter |
| 12 tracks of 3–6 notes; chords of 3 notes at most | `electric_pulse.h:17`; `audio_mix.c:9-13`; `abc.c:837` | Too few for a section-based arrangement |
| No sawtooth or sine (`wave=sine` silently becomes square); no sub-oscillator; at most 2 stacked oscillators; no per-voice filter or filter envelope; linear envelopes | `audio_dsp.c:82-128`; `abc.c:216-233` | No analog bass and no pads |
| Note level is an integer 0–127 after a 60 % scale | `audio_mix.c:115-126` | Quiet parts lose their dynamics. The "soft" pad at `!p!` computes to amplitude 0, so it is silent today |
| Automation updates once per step, at most 32 ramps; the delay ignores tempo ramps | `audio_mix.c:309-325`; `audio_fx.c:312` | Swells zipper |
| Runtime mixer: linear SRC, an interpolation glitch at the loop seam, a linear crossfade, one music stem | `mixer.rs:101-109,258-261` | Click at every loop point |

Today audio is 23 MB of the 40 MB release binary. Rendering all 56 cues
takes 16 s, serially.

## 4. Architecture

Extend the engine; do not replace it. The C renderer stays a build-time
tool, and the game ships rendered audio. That keeps real-time cost and
risk where they are now: in a small Rust mixer, not a sampler running
in the audio callback. Every BRYX change stays marked `BRYX` in the fork,
and changes to existing behavior sit behind opt-in directives (the fork's
rule in `PROVENANCE.md`).

### 4.1 Signal path

```
compose.py ──ABC + %%bus/%%instrument──▶ abc.c
                                          │
             ┌───────── track (×32) ──────┴────────────┐
             │ source: synth voice │ sampler voice      │
             │ per-voice SVF + filter env + LFOs       │
             │ amp (float, velocity curve) · pan       │
             └──────┬──────────── sends ───────────────┘
                    ▼
     group buses: orchestra · electronic · percussion · fx
       (EQ, drive, ladder, compressor, sidechain key)
                    ▼
     aux returns: room · stage · hall · long-tail (stereo FDN)
                  delay A/B (tempo-synced ping-pong)
                    ▼
     master: float sum → gentle bus compressor → lookahead limiter
                    ▼
     48 kHz stereo f32 ──▶ QOA encode ──▶ include_bytes!
```

### 4.2 The engine (Phase 2 work list)

- **Float throughout, stereo throughout, at 48 kHz.**
  - Filter coefficients are derived from the sample rate.
  - The existing integer path stays for the old effect cues until they are re-authored.
- **Sampler voice:**
  - Zones with key and velocity ranges, a root key, round-robin groups and articulation sets.
  - Loop points, set by a build-time crossfade loop finder because VSCO's WAVs carry none.
  - Release tails and start offsets.
  - Stereo samples.
  - Cubic (Hermite) interpolation for pitch and rate conversion.
  - Instruments are defined in a **subset of SFZ**, the open text format the VSCO 2 CE mappings already use: `<region> sample= lokey= hikey= pitch_keycenter= lovel= hivel= seq_length= seq_position= loop_start= loop_end= offset= ampeg_*`.
  - A small C WAV reader at build time handles 16- and 24-bit PCM.
- **Synth voice:**
  - New waveforms: band-limited sawtooth (PolyBLEP), sine, and a sub-oscillator (−1 octave, square or sine).
  - Up to 7 stacked, detuned oscillators with seeded random phase, so output stays deterministic.
  - Mono or poly with legato-only portamento.
  - Exponential ADSR.
- **Per-voice filter:** a state-variable filter (low-pass, high-pass, band-pass, with resonance) or the existing ladder, each with its own envelope and amount, keyboard tracking, and LFO.
- **Two free LFOs per voice:** sine, triangle, square or sample-and-hold, synced or free, routed to pitch, filter, amplitude, pan or pulse width.
- **Effects:**
  - A stereo FDN reverb (8 lines, Householder matrix) with pre-delay, size, high and low damping, modulation and width. Four presets: room, stage, hall, long.
  - A ping-pong delay with filtered feedback that follows tempo ramps.
  - EQ: shelves and a peaking band.
  - A feed-forward compressor, keyed sidechain, and a lookahead master limiter at −1 dBFS.
- **Mixing model:**
  - Up to 32 tracks, each with up to 8 notes.
  - Per-track pan, width and send levels.
  - Named buses: `%%bus orchestra`, `%%bus electronic`.
  - Chords of 6 notes.
  - Per-sample parameter smoothing, so ramps no longer zipper.
- **Named instruments.** A preset library, `audio/instruments/*.inst`, defines `strings_high`, `strings_low`, `brass_horns`, `brass_low`, `timpani`, `cymbals`, `synth_bass`, `synth_pad`, `synth_sequence`, `drone` and `percussion`.
  - Each preset fixes register, articulation set, gain, pan, bus and sends.
  - `compose.py` writes musical intent: role, dynamics and articulation. It no longer writes oscillator parameters.

### 4.3 Samples

We ship a curated subset of **VSCO 2 Community Edition** with percussion
from **VCSL**. Both are CC0 1.0, the only licence that lets raw samples
sit in an MIT repository and its binaries without conditions.

| Group | Contents | Estimated size, 16-bit mono 44.1 kHz |
|---|---|---|
| String sustains | 51 pitches across violins, violas, cellos and basses (sections; contrabass solo, non-vibrato) | 18 MB |
| String spiccato and tremolo | 1 layer, 1 round robin | 9 MB |
| Brass sustains | 41 pitches across horn, trumpet, trombone and tuba; swells built from envelopes across the dynamic layers | 14.5 MB |
| Brass staccato | About 45 pitches | 3.5 MB |
| Percussion | Timpani hits and rolls, suspended-cymbal crescendos, bass drum | 16 MB |

The total is about 60 MB of 16-bit audio, or about 30 MB stored as FLAC.

**Known gaps:**
- There is no contrabass section, so solo bass is stacked with cello an octave up.
- The brass are solo players, so sections are built by stacking and detuning.
- There is no legato; overlapping notes with crossfades stand in.

`ATTRIBUTION` will list every source file and its CC0 statement.

Rejected libraries (their licences are incompatible):
- Sonatina: CC Sampling Plus 1.0, a retired licence that forbids commercial redistribution of copies.
- The Philharmonia samples: their terms forbid making them available "as is".
- Virtual Playing Orchestra: it bundles libraries under mixed licences.
- No Budget Orchestra.

### 4.4 Storage and playback

| Option | 33 tracks of about 45 s, 48 kHz stereo | Cost |
|---|---|---|
| s16 PCM | ≈ 285 MB | Unacceptable binary size |
| mu-law stereo | ≈ 140 MB, and still hisses | No |
| **QOA (Quite OK Audio)**, 3.2 bits/sample | ≈ 57 MB | No new crate: the encoder is a vendored MIT single-file C header in the build tool; the decoder is about 150 lines of Rust |
| Opus at 128 kbps | ≈ 24 MB | Needs libopus at build time and a decoder crate: two dependencies beyond the budget |

**Choice: QOA.** Its quality is transparent for most material at this
bitrate, and decoding is cheap and fixed per frame. The mixer decodes
the playing track **in the callback, one frame (20 samples) at a time**:
- no allocation and no lock;
- constant work per output sample;
- nothing to predecode, so startup and memory stay as they are.

The binary grows from about 40 MB to about 80 MB, almost all of it music.

**Runtime fixes in the same pass:**
- Cubic interpolation where the device rate is not 48 kHz.
- Fix the loop-seam glitch.
- Equal-power crossfade.
- Remove the per-`Play` string search (pre-resolve cue indices).
- Preallocate `scratch`.

### 4.5 Build time

Orchestral rendering is heavier, an estimated 5–10× per track. `build.rs`
renders cues **in parallel** with `std::thread` (no new dependency) and
only reruns when a cue, an instrument or a sample changes. The target is
under 60 s for a clean build on CI.

## 5. Validation (Phase 2 gate)

Probes, extending `audio/probes` and `src/audio/probes.rs`:
- Sampler: pitch tracking against the root key, ±3 cents.
- Zone and velocity-layer selection.
- Loop seams free of clicks.
- Release tails.
- Stereo separation and pan law.
- Reverb RT60 per preset, within ±15 %.
- Delay timing and ping-pong channel.
- Filter −3 dB point, independent of sample rate.
- LFO stability.
- Limiter ceiling and the absence of clipping.
- Determinism: the same render twice, byte for byte.
- Render cost per voice.
- Sample memory at build time.
- QOA round trip: SNR, and decode with no allocation.

Each instrument family gets a short reference render. A technical report
records capabilities and measured costs.
Both are done: see [`studio-report.md`](studio-report.md) and
`audio/reference/`.

**The ear gate.** Phase 3's six cues (Act I sparse, Act II melodic,
Act III mechanical, Act IV monumental, menu, and an act stinger) are
rendered and checked with analysis: loudness, spectrum by band, stereo
correlation and peaks. They are then **listened to by a person**. The
author of this document cannot hear audio. The 32-track rewrite starts
only after that listening approves the direction.

## 6. Phase 4: the score in the studio

Everything the game plays is now rendered by the studio.

**What moved:**
- the 32 level tracks and the menu (QOA stereo, streamed);
- the five stingers (QOA stereo, one-shots);
- the 18 effects (studio parts, folded to the mu-law effects path, still
  pitched by playback rate).

`build.rs` no longer embeds the ABC render of a cue that has a studio
score. `audio/compose.py` stays the source of the themes and the track
recipes.

**How the score was written** (`audio/compose_studio.py`):
- The approved Phase 3 cues are tracks 01, 09, 22, 32 and the menu.
- The other 28 orchestrate `compose.py`'s own parts by act, with the rules
  the references proved (`reference-cues.md`).
- The composer refuses a pitched note outside the track's mode, and a
  texture over 7 parts on average or 12 at once.

**Measured:**

| | Value |
|---|---|
| Act levels | I −24.5, II −20.7, III −20.3, IV −17.2 dBFS on average |
| Music | 38 MB |
| Release binary | 58.0 MB (40 MB before) |
| Cold build render | about 70 s on 4 cores; cached after |
| Brick | E5 |
| Paddle | E3 (−3 cents) |
| Timed stingers | within 0.3 s of their slots |
| Effects | 16 of 18 within 1.5 dB of the loudness of the ones they replace |
| Brick over the music, 300 Hz–3 kHz | +17.5 dB (Act IV) to +25 dB |

**Found and fixed on the way:**
- **QOA's cold start** coded each loop's first slice with up to 7 times its
  usual error, a click at every wrap. The encoder is now warmed on the
  loop's end (`audio/render.c`), and a test checks every loop's first
  frame.
- **Short samples were not tuned.** The violin pizzicato wandered ±15
  cents note to note; maps now carry each short note's measured `tune`.
- **The brick lost loudness.** Spikier than the chip's square, it plays at
  1.1 instead of 0.6 (`director.rs`, `BRICK_GAIN`).

**Left for Phase 5:**
- the balance of music against effects in play;
- the rewrite of `audio.md`, `music.md` and the engine's `PROVENANCE.md`;
- the retirement of the ABC music, which `compose.py` still writes.

## Sources

| Source | Reliability |
|---|---|
| FactMag, "Daft Punk tell all about TRON: Legacy" (2010) | Verified |
| Collider interview | Verified |
| Consequence, Trapanese profile (2014) | Verified |
| Far Out and Vice on the A24 podcast (2026) | Verified |
| mfiles review; Cue By Cue analysis; Grammy.com retrospective | Verified as descriptions of the music |
| Mix Online, Oscar sound watch | Verified from excerpt only |
| Wikipedia, *Tron: Legacy (soundtrack)*: credits and citations | Verified |
| Reverb Machine's "End of Line" recreation | The author's reconstruction, not the film's documented settings |
| VSCO 2 CE: github.com/sgossner/VSCO-2-CE (CC0 1.0); VCSL: github.com/sgossner/VCSL (CC0 1.0) | Licences verified |
