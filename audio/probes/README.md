# Engine probes

Small ABC files, one per feature of BRYX's engine fork. `build.rs` renders
them next to the cues, and `src/audio/probes.rs` checks the rendered
audio: that `%%hold` holds, that `!p!` is quieter than `!f!`, that a
chord sounds three pitches, and so on. They are referenced only from
tests, so none of them reaches the game binary.

Every probe runs at 120 BPM with sixteenth-note steps: one step is 125 ms,
2,756.25 samples at 22,050 Hz.
