# The Electric Pulse audio engine: BRYX's fork

These files were copied from
[chussenot/electric-pulse](https://github.com/chussenot/electric-pulse),
`src/`, at commit `06c79fe5ef9e206e64d09e86fa94f2146ae898ee`. It is the same
author's engine, brought in so BRYX sounds like a sibling of Electric Pulse.

Since then BRYX has **forked** it. The changes below fix how the engine
renders, and add what the score needed. Every change is marked `BRYX` in
the source. A file that uses none of the new directives renders as it did
before, only more precisely. The changes are written to be offered back
upstream.

| File | Role |
|---|---|
| `audio_engine.[ch]` | Render entry points (`audio_engine_render_abc_file`, and BRYX's `_s16`) |
| `abc.c`, `electric_pulse.h` | ABC DSL parser, `AbcMusic` to `SeqSong` |
| `audio_seq.[ch]` | Step timeline, note events, held notes |
| `audio_mix.[ch]` | Voices, oscillators, instruments, ADSR, the mix |
| `audio_dsp.[ch]` | Oscillators (square, pulse, triangle, noise), envelopes |
| `audio_fx.[ch]` | FX buses: drive, low-pass, ladder, delay, sidechain |
| `audio_song_builtin.[ch]` | Built-in menu song; only linked because `audio_engine.c` references it |

Not copied: the jam generator, MIDI export, the GUI, and the tools.

Electric Pulse has no license file at that commit. Its owner, who also owns
BRYX, asked for the engine to be reused, and then for it to be changed
directly in BRYX.

## What the fork changes

**Precision.** The original synthesized every voice in 8-bit output steps:
- each voice's amplitude was its full swing (a pad at ±3 steps);
- every envelope and FX multiply truncated to an integer.

So BRYX's quieter voices sounded at 3–5 bits and lost up to 5 dB of
level to rounding.

The fork changes this in three places:
- **Inside the engine**, samples carry 8 more bits (`DSP_FRAC_BITS`), so
  the mix runs at 16-bit scale. Every knee and clip in `audio_fx.c` moved
  by the same 8 bits and behaves as before. Products that can outgrow 32
  bits are taken in 64.
- **`audio_engine_render_abc_file_s16`** returns that mix as signed 16-bit.
- **The original 8-bit functions** now quantize the 16-bit mix once, with
  triangular dither.

**New ABC directives, all opt-in:**

| Directive | Effect |
|---|---|
| `%%hold on` | A written note sounds once, for its whole length (`E16` is one note, not 16). Without it, every step strikes again, as before. A held note's envelope, vibrato and gate span every step it covers. |
| `%%master <percent>` | Output gain after the mix, in tenths of a percent. Mastering moves one number instead of every voice's amplitude. |
| `%%effect N ... mode=insert` | The bus *replaces* its voices' dry signal (an insert) instead of adding a processed copy to it (a send), so a filter can darken and a drive can color. |
| `%%effect N ... sidechain_key=<voice>` | The sidechain fires on that voice's note-ons (the kick), and ducks this bus only. Unkeyed, it keeps the original behavior: it fires on its own voices' accents and ducks the whole mix. |

**Instrument parameters ABC could not reach.** The mixer always supported
them; now `%%instrument` and `V:` set them:

| Key | Meaning |
|---|---|
| `stack=2` | Two oscillators per voice |
| `detune=<cents>` | Their spread |
| `pwm=<percent>` | Pulse-width sweep depth |
| `pwm_rate=<mHz>` | Its rate, in millihertz |
| `vibrato_rate=<mHz>` | Vibrato rate, in millihertz |
| `vibrato_delay=<ms>` | Vibrato fades in over this long after the note starts |
| `accent=<percent>` | Gain added on accented steps (default 20) |

### Phase 2: expression

**Dynamics.**

| Syntax | Effect |
|---|---|
| `!pp!`, `!p!`, `!mp!`, `!mf!`, `!f!`, `!ff!` (and `!ppp!`, `!fff!`) | Set the voice's velocity until the next mark (40, 56, 72, 88, 104 and 120; 24 and 127). Unmarked notes keep the original velocity, 88. |
| `!>!` | Accents the next note. |
| `accents=marked` (instrument) | Only `!>!` notes are accented. The original accents every beat (`accents=beats`, the default). |

**Chords.** `[EGB]16` sounds three notes in one voice. A song that writes
a chord gets six voices per track instead of three, so a held triad's
release still rings under the next one. Songs without chords keep three.
Before, `[` was skipped, so `[EGB]` played three notes in a row.

**Drum synthesis.**

| Key | Effect |
|---|---|
| `pitch=<semitones>`, `pitch_time=<ms>` | A note starts that far off its pitch and sweeps home on a square-law curve. Negative values rise. A kick is `pitch=24 pitch_time=55`. |
| `noise=long` | 15-bit LFSR noise, stepped at sixteen times the note's frequency, as the NES does: the note sets the color. |
| `noise=metal` | The LFSR's short loop, which rings with a pitch of its own. |
| `noise=white` | The original, one random bit per sample. Also the default. |

**Automation.** `%%ramp <target> <param> <from> <to> <start step> <end step>`:
- Between the two steps (absolute, from the first), the parameter moves
  linearly. After the end step it stays until another ramp moves it.
- Bus parameters (target `bus0`..`bus3`): `ladder_cutoff`,
  `ladder_resonance`, `lowpass`, `delay_mix`, `drive`, `mix`.
- Voice parameter (target a voice or instrument name): `duty`, taken at
  each note-on.

One element reader (`parse_element`) serves the main loop and `|: :|`
repeats, so a decoration, a chord or a note reads the same in both. The original would
have read the `f` of `!f!` as the note F.

Every file that uses none of these renders byte-for-byte as it did after
phase 1: checked on all 52 cues.

### Phase 3: room, tempo, and the seam

| Directive | Effect |
|---|---|
| `%%loop on` | The song renders twice, and only the second pass is kept. Its start carries the end's echoes, reverb and release tails, so a looped track has no seam. Without it, a loop restarted from silence. |
| `%%bandlimit on`, or `bandlimit=on` per instrument | Square and pulse edges are smoothed with PolyBLEP, so high notes stop folding their harmonics back as inharmonic noise. Off, the edges stay hard as before. |
| `%%effect N ... reverb=<mix> reverb_size=<0..100> reverb_damp=<0..100>` | A small mono reverb on the bus, after its delay: Freeverb's four damped combs and two allpasses, in fixed point. |
| `%%ramp song tempo <from> <to> <start> <end>` | The tempo moves step by step: an accelerando or a ritardando. |
| `%%ramp busN reverb ...` | The reverb's mix, as a ramp. |

**Limits.** 12 voices and instruments (from 8), 4,096 steps a voice (from
1,024: 256 bars), and 64 patterns and arrangement slots (from 16 and 32).
The parsed music and the song are now megabytes, so they live on the heap,
not on a thread's stack (a Windows main thread has 1 MB).

Every file that uses none of these renders byte-for-byte as it did after
phase 2: checked on the 52 cues and the phase 1 and 2 probes. The check
caught one regression while this was written: the tempo code clamped
every song to 400 BPM, which changed the seven effects written at 480.

**Not done: tuplets.** The engine quantizes to sixteenth-note steps, so a
true triplet needs a finer grid for the whole song (swing, delays and
ramps all count steps). Faking one by rounding would be worse than none.

## Probes

`audio/probes/` holds one small ABC per feature. `build.rs` renders them,
and `src/audio/probes.rs` measures each in CI:
- that a held note finishes its attack;
- that `!f!` is 1.86 times `!p!`;
- that a chord sounds its three notes and nothing else;
- that the kick sweeps down to its note;
- that the noise color follows the note and metal repeats;
- that a ramp closes a filter;
- that the keyed sidechain ducks;
- that a quiet voice keeps its shape;
- that band-limiting cuts the aliases by more than 20 dB and keeps the note;
- that the reverb rings and decays;
- that a tempo ramp shortens its steps by the right amount;
- that a loop's first samples carry its last note's tail;
- that twelve voices hold one note each through 70 bars.

`docs/design/audio.md` says why each change was made, and what it measured.

## Changing it

Edit here, and mark the change `BRYX`. Keep any change to how an existing
file renders behind an opt-in directive, so the fork can still go
upstream. Then re-render the score (`docs/design/music.md`, "Changing the
music") and listen.
