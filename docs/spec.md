# SPEC — BRYX

A single-player brick-breaking game. Rust, native windowed UI, modern
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
  subscribe later; do not pull an audio crate now. *(Added after v1: see
  "Sound" below. The event enum was the hook, as designed.)*
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
- Game states: `Menu → Playing ⇄ Paused → GameOver/Victory → Playing`
  (see "Ending a run"), an explicit enum-driven state machine, no ad-hoc
  booleans.

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

**Power-ups (exactly these nine, nothing more)**: dropped by 15% of
destroyed bricks, uniformly among the nine, fall at 140 px/s, caught with
the paddle:
- Widen (paddle ×1.5 for 15 s, timers stack by refresh, not addition)
- Slow (ball speed ×0.7 once, floor at launch speed)
- Multiball (spawn 2 extra balls from current ball position; life is
  lost only when the LAST ball drops)
- Glue, Laser, Super laser, Tiny, Player, Break (added after v1, see below)

**HUD**: score (top-left), lives as icons (top-right), level number
(top-center). Menu and pause screens: title + key hints, keyboard only.

## Levels

Three built-in levels as `&[&str]` grids in levels.rs, chars: `.` empty,
`1` normal, `2` armored, `X` indestructible. Level 1 simple rows;
level 2 introduces armored; level 3 uses indestructible bricks to shape
corridors. Adding a level must require touching only levels.rs.

**The campaign (added after v1)**: 32 more built-in levels in levels.rs
(`CAMPAIGN`, each with a name the HUD shows): staircases, frames, columns,
walls of armored and indestructible bricks, all original designs (this
repo never reproduces another game's layouts; see ATTRIBUTION). A normal game plays
the campaign; `--classic` and the tests play v1's three above. Every
built-in grid, v1's and the campaign's, is at most 14×8, has a destructible
brick, and has every destructible brick reachable from the open field
through empty cells and destructible bricks only (checked by a test: a box
of indestructible bricks with no way in, like level 3's first version,
fails it).

**Campaign rules (added after v1; `--classic` and loaded levelsets keep
v1's)**, Game Director decisions recorded in `docs/design/director-review.md`:
- The ball's speed cap ramps with the campaign level: 560 px/s on level 1,
  rising linearly to 700 px/s on level 32, so difficulty climbs with
  progress whatever each layout asks. The 700 px/s tunneling guarantee
  above is unchanged: it is still the highest speed any ball reaches.
- The 32 levels form four acts of eight. A Game Over continues from the
  first level of the act it happened in (level 1, 9, 17 or 25).

**Ending a run (added after v1; all progressions)**: GameOver and Victory
are not dead ends. On a fresh key press (a key still held from the last
rally does not count):
- GameOver: Space continues from the act's checkpoint, with full lives
  and the score reset to 0; Escape or P starts a new game from level 1.
  Where the checkpoint is level 1 (v1, a one-act levelset), both mean
  "play again".
- Victory: Space (or Escape or P) plays again from level 1.

So the state machine is `Menu → Playing ⇄ Paused → GameOver/Victory →
Playing`.

**Best score (added after v1)**: the best campaign and `--classic` scores
are kept in a small per-user file (`src/record.rs`; never sent anywhere).
The menu shows the best; the end screen marks a new one. Runs played by
`--autopilot` show the record but never set it. Loaded levelsets keep no
record.

**Sound (added after v1)**, designed in `docs/design/audio.md`:
- Cues are ABC text files in `audio/cues/`. `build.rs` renders them at build
  time with the vendored Electric Pulse engine (`audio/engine/`,
  unmodified, C99), and the binary embeds the PCM: the game runs no C.
- Playback goes through `cpal` (the default `audio` feature; justified in
  `Cargo.toml`) into BRYX's own mixer (`src/audio/`): panned effects,
  pitched by playback rate; stingers that duck the music; one music loop
  per act, crossfaded.
- Sound is presentation only. `src/audio/director.rs` reads each tick's
  events and the game's state, like the renderers, and never writes to
  them.
- M cycles the sound: on, effects only, off (`BRYX_SOUND` sets the
  start). No device means a silent game, never a crash.
  `--no-default-features` builds a silent game with no C compiler and no
  ALSA.

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

## Additions after v1

Each is behind `game::Features`, so `--classic` plays v1's rules exactly:
the pinned replay hash in `game.rs` runs under `Features::V1` and must not
move.

- **Glue** (power-up, 15 s, timer refreshes on re-catch): a ball hitting
  the paddle sticks to it where it landed and rides along, keeping the
  exit velocity the hit gave it (the same offset → angle mapping). A
  Space *press* releases every glued ball (holding Space does not: that is
  the serve); a ball also lets go by itself after 3 s, and when Glue runs
  out. The rest of the fleet keeps moving meanwhile.
- **Laser** (power-up, 15 s): while Space is held, the paddle fires a pair
  of bolts from 8 px in from each end every 0.3 s, at 800 px/s. A bolt
  gives the first brick it meets one hit, as a ball would (an armored brick
  loses one), and is spent; an indestructible brick stops it unharmed.
- **Super laser** (power-up, 15 s): while Space is held, one wide bolt from
  the paddle's center every 0.5 s, at 1000 px/s. It destroys every
  destructible brick in its way, armored ones outright, and flies on until
  an indestructible brick stops it.
  Both guns: no bolt during a serve (Space launches); a bolt blows up an
  asteroid it touches (a Laser bolt is spent doing so); at most 16 bolts in
  flight; a bolt that takes a level's last brick clears it; catching either
  gun replaces the one carried, timer full again. A bolt moves at most
  17 px a tick, under a brick's 22 px height, so none can skip a brick.
- **Tiny** (power-up, 15 s, the one that hurts): paddle ×0.6. Tiny and Widen
  cancel each other: catching one ends the other, so the paddle is only
  ever one size off.
- **Player** (power-up): one extra life, up to 9
  in all (the HUD's room for life icons).
- **Break** (power-up): opens a gate in the right
  wall, from 40 px above the paddle's center to the bottom of the field.
  While it is open the paddle can slide out through the wall; once it is
  all the way out (its left edge past the wall) the level is left as if
  cleared, scoring the bricks still standing (no bonus: the gate saves time,
  never score; it paid a flat +10,000 until the Game Director review), with
  the paddle back in the middle for the next serve
  (Victory if it was the last level). The gate stays open until it is
  used, a life is lost or the level ends. Balls still bounce off the wall
  there.
- **Asteroids**: from 4 s into a serve's flight, then every 5–10 s, one
  drifts in from the top edge (at most 3 at once), over the bricks, down at
  40 px/s with a sideways drift and wobble that flip at the walls. A ball
  touching one blows it up (+100) and bounces off it as off a round
  obstacle (velocity mirrored about the line between centers, speed
  unchanged); the paddle touching one blows it up too (+100, harmless). The
  explosion hits every destructible brick within 40 px of the asteroid's
  center (closest point of the brick) once, as a ball would: normal bricks
  break (scored, and rolling their power-up drop), armored ones lose a hit,
  indestructible ones are untouched; a blast that takes a level's last
  bricks clears it. One that passes the paddle just leaves. A lost life or
  a new level clears them.

One rule applies whatever the features: **stall breaking**. After 10 s of
flight with no ball touching the paddle, hitting a destructible brick or
blowing up an asteroid, each wall or indestructible-brick bounce turns the
ball 7° (alternating direction, never flatter than 20° from horizontal,
speed unchanged) until one of those happens. Without it a ball can settle
into a closed orbit among indestructible bricks (level 3's box allows one)
that no input can reach.
