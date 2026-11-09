# Product

<!-- impeccable:product-schema 1 -->

## Platform

desktop

Native desktop game: Rust, `wgpu` rendering, `winit` windowing, keyboard only.
Not a website or web app; no browser, HTML or CSS is involved. (None of the
schema's `web`, `ios`, `android` or `adaptive` fits.)

## Users

Two audiences, equally:

- **Players** who build or download BRYX and play a few levels at a time.
  They need to read the field instantly: where the ball is, what each brick
  and capsule is, how many lives remain.
- **Rust and graphics developers** exploring a small, legible `wgpu` codebase
  who also play it. For them the game is the proof of the code: it has to
  look like the care its source shows.

People watching the autopilot clear levels are a third, smaller audience.

## Product Purpose

A single-player brick-breaker that plays well and reads well, built without
an engine (`wgpu` + `winit` + `glyphon`) in a codebase small enough to read
end to end. Success: a player can pick it up with no explanation, and a
developer can see in both the game and the code that it was made deliberately.

## Positioning

An independent, open-source (MIT) arcade game whose craft is visible: no
engine, a pure simulation separate from rendering, two renderers (2D quads
and a 3D perspective view) of the same game, and an autopilot that can play
it, optionally steered by a TypeSafe System One model through the `judgment`
crate. It is not a remake of any commercial game and must not borrow one's
identity.

## Operating Context

- Launched from a terminal (`cargo run --release`), in a resizable window over
  a fixed 800×600 logical playfield that is scaled and letterboxed, never
  stretched.
- Keyboard only: arrows or A/D to move, Space to launch, release a glued
  ball or fire a laser, Escape or P to pause.
- Flags change the session: `--renderer 3d`, `--classic` (v1 rules and
  levels), `--levelset <path>` (community LBreakoutHD levelsets, fetched
  separately), `--assets <dir>` (an optional CC0 sprite pack), `--autopilot`.
- Fully offline. The only network use is the optional autopilot model, and
  only when built with `--features typesafe` and given an API key.

## Capabilities and Constraints

- States: Menu → Playing ⇄ Paused → GameOver / Victory, an explicit state
  machine.
- Content: a 32-level named campaign (default), v1's three levels
  (`--classic`), or loaded levelsets. Bricks are normal, armored (two hits)
  or indestructible.
- Eleven capsules: Widen, Slow, Multiball, Glue, Laser, Super laser, Tiny,
  Player, Break, Explosive ball, Metal ball. Plus drifting asteroids that explode, laser bolts, and the
  Break gate in the right wall.
- Presentation is fixed by the spec's technology choices: one instanced-quad
  pipeline in 2D, a cube/sphere pipeline in 3D, `glyphon` text. A procedural
  sprite atlas is the default; a sprite pack is optional.
- Visual work never changes physics, collisions, the 120 Hz fixed timestep,
  scoring, levels, controls, CLI behavior or the autopilot. The dependency
  budget is fixed; any addition needs a written justification.

## Brand Commitments

- The name is **BRYX**, written in capitals.
- No imitation of Arkanoid or Taito: no logo, typography, artwork, colors,
  sprites or presentation borrowed from it, and no transcribed levels.
- Independent open-source arcade craft: technical, precise, playful without
  being childish. Not a SaaS dashboard, not a mobile monetization interface,
  not a nostalgic replica.

## Evidence on Hand

- Screenshots in the repository: `image.png` (2D), `render3d.png` (3D),
  `docs/autopilot.png`, `docs/campaign.png` (all 32 levels),
  `docs/levelsets-mosaic.png`.
- The design contract in `docs/spec.md`; the autopilot's design and
  measurements in `docs/autopilot.md`.
- No logo, no marketing assets, no player testimonials or download figures
  exist. Do not invent them.

## Product Principles

1. **The field is always legible.** Ball, paddle, every brick kind, every
   capsule and every hazard are told apart at a glance, mid-rally.
2. **Show the craft.** Every visual choice is one a reader of the source
   would recognize as deliberate and could find in one place.
3. **One game, two lenses.** The 2D and 3D renderers share the same meaning
   for every color, shape and piece of text.
4. **Presentation never touches the simulation.** Effects follow the game's
   events; they never alter what happens.
5. **Honest about the machine.** When the autopilot is playing, the interface
   says so.

## Accessibility & Inclusion

- **Color-blind safe identification is required.** Capsules and brick kinds
  must be distinguishable without color: shape, letter or pattern carries the
  meaning as well as hue.
- Keyboard only, with the controls shown on screen in the menu and pause
  states.
- Text stays legible at the 800×600 logical resolution.
