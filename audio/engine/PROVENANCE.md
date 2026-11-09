# Vendored: the Electric Pulse audio engine

These files are copied **unmodified** from
[chussenot/electric-pulse](https://github.com/chussenot/electric-pulse),
`src/`, at commit `06c79fe5ef9e206e64d09e86fa94f2146ae898ee`. It is the same
author's engine, brought in so BRYX sounds like a sibling of Electric Pulse.

| File | Role |
|---|---|
| `audio_engine.[ch]` | Render entry points (`audio_engine_render_abc_file`) |
| `abc.c`, `electric_pulse.h` | ABC DSL parser, `AbcMusic` to `SeqSong` |
| `audio_seq.[ch]` | Step timeline, note events |
| `audio_mix.[ch]` | Voices, oscillators, instruments, ADSR |
| `audio_dsp.[ch]` | Oscillators (square, pulse, triangle, noise), envelopes |
| `audio_fx.[ch]` | FX buses: drive, low-pass, delay, sidechain |
| `audio_song_builtin.[ch]` | Built-in menu song; only linked because `audio_engine.c` references it |

Not copied: the jam generator, MIDI export, the GUI, and the tools.

Electric Pulse has no license file at that commit. Its owner, who also owns
BRYX, asked for the engine to be reused here.

**To update**: copy the same files from a newer Electric Pulse commit, record
the commit hash above, and keep them byte-for-byte. A behavior change for
BRYX belongs in `audio/render.c` or `src/audio/`, never in this folder.
The cues in `audio/cues/` are BRYX's own.
