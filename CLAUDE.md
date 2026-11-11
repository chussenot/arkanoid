# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.



## Project status

Pre-implementation. **`docs/spec.md` is the full contract for this project — read it before writing any code.** There is no `Cargo.toml`/`src/` yet; the next work is Milestone 1 (window + wgpu clear color + fixed-timestep loop skeleton — see spec's Milestones section).

## Build & Test

Rust 2021, stable toolchain (managed by `mise.toml`). `mise.toml` also defines tasks wrapping these:

```bash
mise run run                                 # == cargo run --release (launch the game)
mise run build                               # == cargo build
mise run test                                # == cargo test (headless, src/game.rs — no GPU needed)
mise run clippy                              # == cargo clippy --all-targets -- -D warnings — must be clean
mise run fmt                                 # == cargo fmt
mise run fmt-check                           # == cargo fmt --check
mise run ci                                  # fmt-check + clippy + test — the full pre-commit/CI gate
```

`prek` (config in `.pre-commit-config.yaml`) runs trailing-whitespace/EOF/YAML/large-file checks; `prek install` to activate the git hook. CI is fmt + clippy + test on ubuntu-latest only — no windowed/GPU run is needed or expected.

## Architecture (per docs/spec.md — technology choices are decided, not up for relitigation)

- **Rendering**: `wgpu`, one instanced-quad pipeline, every entity (paddle, ball, bricks, walls, HUD) is a quad instance, target one-or-two draw calls per frame. No engine (no bevy/ggez/macroquad) — the point is a small, legible codebase.
- **Windowing/input**: `winit`. **Timing**: fixed-timestep simulation at 120 Hz, decoupled from vsync, with render-side interpolation between ticks.
- **Text**: `glyphon` for the score/lives/level HUD.
- **Kinetic effects** (`docs/design/kinetic-fx.md`): `src/fx/` turns `GameEvent`s (with `BrickStruck`'s contact data) into bounded, age-driven effects. Lyon (`lyon_tessellation`) builds cracks, fracture pieces and stroked templates; quads stay quads. `fx::gpu` (shared by both renderers) owns the HDR scene target, bloom and shockwave composite, skipped on frames that need none. Shaders are WESL modules in `src/shaders/`, compiled to WGSL by `build.rs` (`wesl` needs Rust ≥ 1.97.1). `src/rally.rs` is the rally step the director's notes and the effects' hues share. `BRYX_FX=off|geometry|full`.
- **Audio** (added after v1, `docs/design/audio.md`): every sound is a `.score` text file in `audio/score/`, written by `audio/compose_studio.py` (orchestration) from `audio/compose.py` (the notes; `docs/design/music.md`) and never edited by hand. `build.rs` renders them at build time with BRYX's studio (`audio/engine/studio*.c`, a C host tool; samples in `audio/samples/`, CC0) and caches the renders by content: music and stingers to 48 kHz stereo QOA (`src/audio/qoa.rs`), effects to 8-bit mu-law (`src/audio/mulaw.rs`). `src/audio/` plays them through `cpal` (director → mixer → output); the mixer must not touch the heap on the callback's path (a test checks it). The Electric Pulse ABC engine in `audio/engine/` is kept only for its probes (see PROVENANCE.md: mark changes `BRYX`). Audio only reads `GameEvent`s and game state, like the renderers. The `audio` feature is on by default; `--no-default-features` builds silent.
- Dependency budget is fixed: wgpu, winit, glyphon, bytemuck, rand. Anything beyond that needs a code comment justifying it (as `cpal` and the `cc` build-dependency have in `Cargo.toml`).

Module boundaries (the strict separation is the architectural point — don't blur it):
- `src/main.rs` — window/event loop wiring only.
- `src/game.rs` — pure simulation (`struct Game`, `fn tick(&mut self, input: &Input, dt_fixed)`). No wgpu/winit/I/O types here; this is what keeps gameplay logic unit-testable headless.
- `src/render.rs` — wgpu setup + the instanced-quad pipeline.
- `src/levels.rs` — level grids as const `&[&str]` data (`.` empty, `1` normal, `2` armored, `X` indestructible): `LEVELS` (v1's three, played by the tests and `--classic`) and `CAMPAIGN` (32 named levels, a normal game). Adding a level must require touching only this file, and a test rejects any grid with a destructible brick no ball can reach. Campaign levels are original designs: never transcribe another game's levels (see `ATTRIBUTION`).
- `src/events.rs` — `enum GameEvent { BrickDestroyed, BallLost, ... }` emitted by the simulation each tick; render (and later audio) consume it.
- Game states are an explicit enum-driven machine: `Menu → Playing ⇄ Paused → GameOver/Victory → Playing` (Space continues from the act checkpoint, Escape/P starts over; see the spec's "Ending a run"). No ad-hoc booleans for state.

## Gameplay contract highlights (full detail in docs/spec.md)

- Fixed logical playfield 800×600; window resize scales/letterboxes, never changes physics.
- Ball exit angle is a linear map from paddle-hit-offset to 30°–150°, clamped away from 90°±5° (no perfectly vertical trajectories); speed +4% per paddle hit, capped at 700 px/s (the campaign's cap ramps from 560 px/s on level 1 to 700 on level 32; `--classic` keeps the flat 700).
- Collision is AABB-vs-circle resolved against the nearest face; tunneling at max ball speed is prevented by substepping the tick — this needs a property-style test (random angles at 700 px/s never cross a 22 px brick uncaught), not just an implementation.
- Exactly eleven power-ups: Widen (paddle ×1.5, 15s, timers refresh not stack-add), Slow (ball ×0.7 once, floored at launch speed), Multiball (2 extra balls; life lost only when the *last* ball drops), Glue (ball sticks to the paddle, a Space press releases it), Laser (held Space fires twin bolts), Super laser (held Space fires a piercing bolt), Tiny (paddle ×0.6; cancels Widen and vice versa), Player (extra life), Break (a gate in the right wall; sliding the paddle out skips the level), Explosive ball (a struck brick blows up with its eight neighbors), Metal ball (breaks any destructible brick and flies through). Do not add a twelfth without updating the spec first.
- Post-v1 additions (Glue, Laser, Super laser, Tiny, Player, Break, Explosive ball, Metal ball, asteroids) sit behind `game::Features`; `--classic` turns them off and the pinned replay hash runs under `Features::V1`. See the spec's "Additions after v1".

## Conventions

- Conventional Commits (`cog.toml` / Cocogitto tracks these for changelog generation).
- Commit after each milestone in docs/spec.md, in order — don't jump ahead or batch milestones into one commit.
- The spec explicitly says: ask before deviating from it.

<!-- pact:begin hash:8546f7af -->
## pact coordination protocol

Claude Code loads this file, not `AGENTS.md`, so the protocol is imported
here instead of copied — one source of truth, in the file the other agents
already read. Run `pact init` to refresh it.

@AGENTS.md
<!-- pact:end -->
