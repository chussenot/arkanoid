# Kinetic effects

Every important gameplay event now has a visual response of its own, drawn
the same way by both renderers and timed with the note the sound director
plays for the same event. This page records the study, the architecture,
the decisions and the measurements behind it. The creative principle is
**Kinetic Arcade**:
- precise rather than noisy;
- physical rather than decorative;
- expressive rather than exaggerated.

Each effect says something about the event that caused it, and none
obscures the ball, the bricks or the trajectory.

## 1. Assessment: what existed, what was missing

**Events.**
- `game.rs` pushes `GameEvent`s each 120 Hz tick. `main.rs` hands them to
  the renderer and the audio `Director`, then drains them.
- Gameplay is pure, and the pinned replay hash covers state, not events.
- **Missing:** where a ball struck a brick, from which side, and with what.
  - An armored brick's first hit had no event at all; the director
    inferred it by diffing armored bricks tick to tick.
  - An indestructible rebound left no trace anywhere.

**Juice, duplicated across the two lenses:**

| | 2D | 3D |
|---|---|---|
| Ball trail | 4 fading quads (spec) | 4 fading spheres |
| Brick hit | one-frame white flash | one-frame emissive brick |
| Brick destroyed | one-frame white ghost | tumbling cube |
| Asteroid explosion | expanding circle | rock debris |
| Shake | screen | camera |

Each renderer diffed or ingested on its own, and nothing was shared.

**Geometry:** 2D was one instanced-quad pipeline; 3D was cube, sphere and
rock meshes. Neither could draw an arbitrary polygon or a stroke.

**Shaders and passes.**
- The WGSL was inline Rust strings; the HUD's was built with `format!`.
- Both renderers wrote linear color straight to the sRGB surface: no HDR
  headroom, no offscreen target, no post-processing.

**Music:**
- The director plays each broken brick one step up an E-minor pentatonic
  scale. The run resets on a paddle hit, a serve, a lost life or a cleared
  level.
- That counter was private to the audio code.

## 2. Architecture

```
Game tick ── GameEvent (+ BrickStruck: contact point, face normal, striker)
   ├─ audio::Director ─ note     each keeps its own rally::Rally, fed the
   └─ fx::FxSystem::observe      same events: the same step, the same tick
          │  spawns effects with all their parameters fixed
          ▼  (variety from a seed hashed from the event, never a global RNG)
   every frame: FxSystem::update(dt) ─ FxFrame { triangles, pulses, glows }
          ▼
   renderer (2D or 3D)
     ├─ scene ─ HDR target (Rgba16Float) or, when the frame has no light
     │          and no shockwave, the surface itself
     ├─ FxGpu::draw_geometry: one indexed draw for every effect
     ├─ FxGpu::finish: bloom (5 passes) + composite (WESL shaders)
     └─ HUD overlay on the surface, never post-processed
```

**Responsibilities:**
- **Gameplay emits semantic events.**
  - `GameEvent::BrickStruck` is new. It is pushed at the exact contact, by
    balls and by bolts, and for indestructible bricks too.
  - No rule reads it, and the replay hash is unchanged.
- **`rally.rs` holds the shared rhythm.** The director and the effects
  each own one, so they agree by construction, with no shared state and no
  dependency between them.
- **`fx/mod.rs` interprets the events.**
  - It turns events into effects, manages their lifetimes, and builds each
    frame.
  - Every effect is a pure function of its age.
- **`fx/geometry.rs` builds the shapes with Lyon.**
  - **Templates** are tessellated once (ring, arc, disc, a library of shard
    and rock outlines). A stroked template keeps each vertex's point on the
    path and its offset at half-width 1, so it can be drawn at any radius
    and width without tessellating again.
  - **Cracks and fracture pieces** are tessellated once per impact, into a
    fixed pool of meshes whose buffers are reused.
  - **Quads** (glints, sparks, streaks, hairlines) bypass Lyon.
- **`fx/gpu.rs` is the GPU side, shared by both renderers:**
  - the HDR target;
  - the geometry pipeline (flat for 2D, onto the board through the camera
    and depth-tested for 3D);
  - the bloom chain and the composite.
- **`src/shaders/*.wesl` organizes the shader code.**
  - Shared modules (`fx/params`, `fx/color`, `fx/distort`, `fx/noise`,
    `fx/fullscreen`) are imported by three entry shaders.
  - build.rs compiles them to WGSL.
- **The existing audio engine is unchanged.**
  - The director's rally logic moved into `rally.rs`, unchanged in
    behavior; its pitch tests pass as they were.

**3D adaptation.**
- An effect lies on the surface it happened on: field x/y become the
  board's x/z, and a height lifts it (a brick's top, the floor, a ball's
  center).
- It is projected through the frame's camera, shake included, and
  depth-tested against the scene, so a wall hides what is behind it.
- Rings become ellipses in perspective. Fragments hop off the board in a
  parabola.
- Shockwave centers and radii are projected to the screen per renderer
  (`fx::gpu::ScreenMap`).

## 3. The grammar

| Event | Signature | Communicates |
|---|---|---|
| Normal brick broken | Soft glow from the brick's own outline, in the rally's color; 4 splinters thrown out of the struck face; light running both ways along that face from the impact | Where it was hit, from which side |
| Armored, first hit | A crack from the real impact point (3–4 jagged arms to the edges), its amber light running out at 900 px/s; then a dark fissure that stays until the brick breaks | Damage, located |
| Armored, broken | The brick comes apart **along that same crack**: Lyon cuts the pieces from the crack paths, so they fit together, then they fly from the impact point, turning; a stronger glow, a ring, a small shockwave | The final fracture of the damage you saw |
| Indestructible | A cold rebound arc on the struck face, facing the way the ball left; light along the face; no debris | Resistance, not damage |
| Explosive ball | White-hot core, a pressure ring, a shockwave; **then** each neighbor breaks as the front reaches it (delay = distance / 420 px/s) | The blast first, its aftermath second |
| Asteroid | Rock chips (lumpy, notched, never glowing), a ring of dust, a little fire inside, a shockwave | A different material |
| Metal ball | Sparks off every brick it strikes (more off an indestructible one); a steel streak with a white-hot core | High energy, hard contact |
| Explosive ball in flight | A chartreuse streak | Charged |
| Fast unpowered ball (> 520 px/s) | A faint trace of its speed | Momentum, never a permanent trail |
| Level cleared | A line of light sweeping down the field with the fanfare | Structure: the level's end |
| Warp | The same, sweeping right toward the gate in Break's orange | Leaving through the gate |

**Rhythm.**
- The light's hue is the rally's step: the note the director plays for the
  same brick, warm at the root and cooling as the run climbs.
- The upper octave (steps 5–9) adds a ring, and the top step two.
- Momentum shows in shape and hue, never in more brightness.
- Successive breaks in a rally read as a sequence: a rising scale, seen.

## 4. Post-processing

| | Choice | Why |
|---|---|---|
| Scene target | `Rgba16Float`, surface-sized, rebuilt on resize | Light above 1.0 without banding |
| Selectivity | The scene pipelines clamp to 1.0; only effect light exceeds it; bright-pass at 1.0 with a 0.25 soft knee | Bloom is selective by construction, no mask; the palette and HUD never bloom |
| Bloom | Dual-filter chain: bright-pass + Karis average to 1/4, down to 1/8 and 1/16, two additive tent upsamples | Wide, soft, cheap; Karis stops fireflies |
| Shockwave | Up to 8 pulses; displacement follows x·e^(−x²) across a band, with a slight angular irregularity; a thin light crest | Refraction, not smear; a pressure front, not a drawn circle |
| Highlight roll-off | Above 1.0 the excess spills to the other channels, then clamps; identity below | Stacked light turns white-hot instead of clipping its hue; palette exact |
| Motion streaks | Geometry, not a post pass | A post-blur would smear the trajectory the player reads; a tapered streak behind the ball cannot get ahead of it |
| Vignette | Not used | No measured gain in focus; the field already reads as the instrument it is |
| HUD | Drawn after the composite, on the surface | Never distorted, never bloomed |
| Field mask | Distortion and bloom only inside the field's rect | The band never moves |

**The direct path.**
- A frame with no effect light and no shockwave goes straight to the
  surface through surface-format twins of the scene pipelines. That is
  most frames of ordinary play.
- It is exactly what the composite would produce, because the spill is the
  identity at or below 1.0. Switching paths is invisible.
- `BRYX_FX=geometry` and `BRYX_FX=off` always take it.

**WESL's concrete benefit.**
- The composite and bloom shaders share the uniform layout (`fx/params`),
  the fullscreen triangle, the bright-pass and the spill.
- The geometry shader shares the field size with them.
- Before, the HUD built its shader with `format!` and each renderer
  repeated its constants.
- Unused functions are stripped from each output.

## 5. Dependency decisions

| Crate | Version | Kind | Evidence | Rationale |
|---|---|---|---|---|
| `lyon_tessellation` | 1.0.23 | runtime | Pure Rust; depends only on `lyon_path`/`lyon_geom`, `euclid`, `arrayvec`, `num-traits`, `float_next_after`; no GPU types | Strokes with joins and caps, arcs, concave fills: cracks and fracture pieces cut along them are the cases a quad cannot be. `lyon_algorithms` is not needed. |
| `wesl` | 0.6.0 | build-dependency | Outputs WGSL text: no `naga`, no `wgpu` in its tree, so it is independent of wgpu 30; prototyped (imports resolve, unused items stripped); every output validated headless by wgpu's own naga in `fx::gpu`'s tests | Shader modules shared by three entry shaders; zero run-time cost |

`wesl` 0.6.0 needs Rust 1.97.1 or later. CI and `mise` use the latest
stable; a local 1.97.0 toolchain must update.

## 6. Performance

Measured with `src/bin/bench_render3d.rs`:
- **Renderers:** `BENCH_RENDERER=2d|3d`.
- **Effects level:** `BRYX_FX=off|geometry|full`.
- **Load:** `BENCH_FX_LOAD=0` runs no effects. Otherwise a stress tick runs
  every 6 frames on a fixed 1/60 s clock: six breaks with contacts (one
  armored fracture, one metal), a blast with two neighbors, an asteroid,
  and a metal strike on an indestructible brick.
- **Scene:** a full 14×8 board and multiball, offscreen, 600 measured
  frames, on **llvmpipe**: a CPU rasterizer and the only GPU here.
  Fullscreen passes are its worst case. A real GPU runs this bloom chain
  in well under a millisecond, but GPU timestamps were not available to
  measure that.

| Frame (mean / p95, ms) | 3D | 2D |
|---|---|---|
| master, no effects | 4.08 / 5.83 | (no offscreen path on master) |
| idle, `full` (direct path) | 3.84 / 5.29 | 2.34 / 3.35 |
| under load, `off` | 3.97 / 4.86 | 2.22 / 3.27 |
| under load, `geometry` | 4.71 / 6.49 | 3.56 / 4.86 |
| under load, `full` | 11.82 / 15.31 | 10.59 / 14.29 |

**Before tuning:**
- The first version rendered every frame through the HDR target, with a
  half-resolution bloom and a full trigonometric pulse loop per pixel. It
  cost 6.5 / 5.0 ms idle and 23.1 / 21.0 ms under load.
- **Quarter-resolution bloom** and a **per-pulse band rejection** before
  any transcendental fixed the cost under load.
- **The direct path** brought idle frames back to master's cost.

**CPU and memory:**
- `FxSystem::update` takes 0.05–0.07 ms mean (p95 under 0.09 ms) under
  that load.
- The peak frame was 6,840 vertices, 18,315 indices and 8 pulses: one
  indexed draw call.
- The limits are 24,576 vertices, 49,152 indices, 192 effects, 48 pooled
  meshes, 128 remembered cracks and 8 pulses.
- After warm-up the hot path allocates nothing. A test floods the system
  and asserts that no buffer's capacity changes.

| Passes per frame | 2D | 3D |
|---|---|---|
| Direct (idle, `geometry`, `off`) | 1 (as master) | 3 (as master) |
| Light or shockwave | 8: scene, 5 bloom, composite, HUD | 9: backdrop, scene, 5 bloom, composite, HUD |

## 7. Remaining work

- **GPU timing on real hardware.** Add timestamp queries to the bench
  where the adapter supports them, and record the bloom chain's cost on an
  integrated and a discrete GPU.
- **Explosion sound vs. light.** The blast's neighbors break visibly as the
  front reaches them, up to about 0.19 s after the blast. Their notes sound
  at once, as a chord (the director caps three per tick). Delaying those
  notes would match exactly, but that is a change to the audio, which this
  work left alone.
- **Music structure beyond the rally.** Effects follow the rally's steps
  and the level-end stingers. They do not follow the score's beat: the
  director exposes no beat clock, and an effect pulsing to the music would
  be the audio-reactive animation the brief rules out. A downbeat-aligned
  level-clear sweep would need that clock.
- **Blast-damaged armored bricks.** A blast that cracks an armored brick
  without breaking it emits no contact, so it gets no crack geometry; the
  sprite's crack still shows.
- **Shake** stays per-renderer, as before; it is driven by score changes,
  not events.
- **Visual tuning by people.** Every value here was judged on screenshots
  under Xvfb, at 1× and slowed 7×. A hands-on playtest on real hardware is
  still to be done.
