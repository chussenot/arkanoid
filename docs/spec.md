# SPEC — `arkanoid-rs`

A single-player Arkanoid/Breakout clone. Rust, native windowed UI, modern
GPU rendering. This file is the contract: implement what is written here,
in the order of the milestones, and ask before deviating.

## Technology decisions (made — do not relitigate)

- **Rendering: `wgpu`** (WebGPU API over Vulkan/Metal/DX12/GL). "Modern
  rendering" means no fixed-function OpenGL and no legacy immediate mode;
  wgpu is the idiomatic modern Rust choice and keeps a path to WASM later.
- **Windowing/input: `winit`**. **Timing: fixed-timestep simulation**
  (120 Hz logic tick) with rendering interpolation, decoupled from vsync.
- **Text: `glyphon`** (or wgpu-compatible equivalent) for score/lives HUD.
- **Audio: none in v1.** Design the event enum so a sound layer can
  subscribe later; do not pull an audio crate now.
- **No engine** (no bevy/ggez/macroquad): the point is a small, legible
  codebase. Dependency budget: wgpu, winit, glyphon, bytemuck, rand,
  anything else requires a code comment justifying it.
- Rust 2021, stable toolchain. `cargo clippy --all-targets -- -D warnings`
  clean, `cargo fmt` clean. No `unsafe` outside what wgpu setup strictly
  requires.

## Architecture

- `src/main.rs` — window/event loop wiring only.
- `src/game.rs` — pure simulation: `struct Game`, `fn tick(&mut self,
  input: &Input, dt_fixed)`. **No wgpu, winit, or I/O types in this
  module.** All gameplay logic must be unit-testable headless.
- `src/render.rs` — wgpu setup + one instanced-quad pipeline. Every
  entity (paddle, ball, bricks, walls, HUD panels) is a colored/textured
  quad instance; target: the whole frame in one or two draw calls.
- `src/levels.rs` — level definitions as const data (see Levels).
- `src/events.rs` — `enum GameEvent { BrickDestroyed, BallLost, ... }`
  emitted by the simulation each tick; the render layer consumes them for
  effects, a future audio layer subscribes here.
- Game states: `Menu → Playing ⇄ Paused → GameOver/Victory`, an explicit
  enum-driven state machine, no ad-hoc booleans.

## Gameplay spec

Playfield: fixed logical resolution 800×600, letterboxed/scaled to the
window (integer-independent; resize must not change physics).

**Paddle**: bottom-center, keyboard ←/→ (and A/D), speed 520 px/s,
clamped to walls. Mouse control optional in v1 only if trivial.

**Ball**: radius 6, launches from paddle on Space at 300 px/s, 60° up.
Speed increases 4% per paddle hit, capped at 700 px/s. Reflection off the
paddle depends on hit position: offset from paddle center maps linearly
to exit angle between 30° and 150°, so the player can aim. Perfectly
vertical trajectories must be impossible (clamp angle away from 90°±5°).

**Bricks**: grid up to 14×8, brick 52×22 px. Types: normal (1 hit,
scored by row: 10–70 pts), armored (2 hits, first hit changes color),
indestructible (silver, not counted for level completion). Collision:
AABB vs circle, resolving against the nearest face, with tunneling
prevented at max ball speed (substep the tick if displacement per tick
exceeds ball radius — assert this in a test, not a comment).

**Lives**: 3. Ball below the bottom edge → life lost, ball re-attaches to
paddle. 0 lives → GameOver with final score. All destructible bricks
cleared → next level; last level cleared → Victory screen.

**Power-ups (exactly these three, nothing more)**: dropped by 15% of
destroyed bricks, fall at 140 px/s, caught with the paddle:
- Widen (paddle ×1.5 for 15 s, timers stack by refresh, not addition)
- Slow (ball speed ×0.7 once, floor at launch speed)
- Multiball (spawn 2 extra balls from current ball position; life is
  lost only when the LAST ball drops)

**HUD**: score (top-left), lives as icons (top-right), level number
(top-center). Menu and pause screens: title + key hints, keyboard only.

## Levels

Three built-in levels as `&[&str]` grids in levels.rs, chars: `.` empty,
`1` normal, `2` armored, `X` indestructible. Level 1 simple rows;
level 2 introduces armored; level 3 uses indestructible bricks to shape
corridors. Adding a level must require touching only levels.rs.

## Rendering quality bar ("modern" means)

- Instanced rendering, sRGB surface, vsync on (FifoRelaxed if available).
- Interpolated render state between simulation ticks (no visible stutter
  when monitor Hz ≠ 120).
- Subtle juice, all cheap: ball leaves a 4-quad fading trail; bricks
  flash white 1 tick when hit; screen-space shake 3 px / 80 ms on brick
  destruction. No particle system in v1.
- Must run at 60+ fps on integrated graphics; if it cannot, simplify
  effects, never the simulation.

## Tests (required, headless — no GPU in CI)

Unit tests on `game.rs` only: paddle-offset → angle mapping including
clamps; speed cap; brick hit-point/scoring; armored two-hit behavior;
substepping prevents tunneling at max speed (property-style: random
angles at 700 px/s never cross a 22 px brick without a collision);
multiball last-ball-loses-life rule; power-up timer refresh semantics.
CI: fmt + clippy + test on ubuntu-latest (no windowed run needed).

## Milestones (commit after each, conventional commits)

1. Window + wgpu clear color + fixed-timestep loop skeleton.
2. Paddle + ball + wall bounces, launch, life loss (no bricks). Playable.
3. Bricks, collisions, scoring, level load, level progression.
4. Power-ups + HUD + state machine (menu/pause/gameover/victory).
5. Rendering polish (trail, flash, shake), README with controls +
   screenshot, CI.

Definition of done: `cargo run --release` from a fresh clone starts the
menu; a keyboard-only player can reach Victory; all tests green; clippy
clean; README explains controls and the fixed-timestep design in one
paragraph each.
