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

`docs/design/audio.md` says why each change was made, and what it measured.

## Changing it

Edit here, and mark the change `BRYX`. Keep any change to how an existing
file renders behind an opt-in directive, so the fork can still go
upstream. Then re-render the score (`docs/design/music.md`, "Changing the
music") and listen.
