# Fixture levelsets

`homemade.lbl` is an original, hand-written 3-level file in the
`.lbl` format documented in `docs/levelset-format.md`. It is our own
content, written from scratch for this test suite, MIT-licensed like the
rest of this repo -- it is not derived from, or copied out of, any
LBreakoutHD/LBreakout2 levelset (those are GPLv3 and, per `ATTRIBUTION`,
never committed to this repo).

It exists to exercise `src/levelset.rs`'s parser end to end:

- every character documented in both the `Bricks:` and `Bonus:` tables
  at least once (level 2, "Full Table Sweep"),
- one unsupported character in each grid (`Q` in Bricks, `_` in Bonus) to
  exercise the warn-and-empty / warn-and-no-drop fallback paths,
- a small, armor-free level (level 1, "Clearable Cluster") a scripted
  headless play can realistically clear, for the integration test.

See the `homemade_fixture_*` and `scripted_headless_play_*` tests in
`src/levelset.rs` for exactly what's asserted against it.
