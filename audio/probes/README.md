# Engine probes

Small ABC files, one per feature of BRYX's engine fork. `build.rs` renders
them next to the cues, and `src/audio/probes.rs` checks the rendered
audio: that `%%hold` holds, that `!p!` is quieter than `!f!`, that a
chord sounds three pitches, and so on. They are referenced only from
tests, so none of them reaches the game binary.

Every probe runs at 120 BPM with sixteenth-note steps: one step is 125 ms,
2,756.25 samples at 22,050 Hz.

## Studio probes

`studio_*.score` probe the studio (docs/design/orchestral-score.md), the
float stereo renderer the orchestral score is written for. `build.rs`
renders each to raw float stereo at 48 kHz, and `src/audio/studio_probes.rs`
measures it:

| Probe | Property |
|---|---|
| `studio_tuning` | a sampled violin A4 within 15 cents of 440 Hz, a synth sine within 1 |
| `studio_layers` | velocity picks the f layer: over 12 dB louder, and brighter |
| `studio_loop` | a note held 10 s, past its recording: every 100 ms within 3 dB |
| `studio_reverb` | the hall's RT60 at 500 Hz within 15% of its preset's 2.4 s |
| `studio_seating` | `orchestra.inc`'s violins left, celli right |
| `studio_limiter` | a tutti 18 dB too hot never passes -1 dBFS |
| `studio_seam` | a loop starts inside its end's tail, with no step at the wrap |
| `studio_sidechain` | a bus compressor keyed by a kick ducks the pad by over 6 dB |

They check what can be measured. Whether a render is music is for ears.
