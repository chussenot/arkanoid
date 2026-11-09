---
name: BRYX
description: Open-source arcade craft. A measured field, legible at a glance through either lens.
colors:
  letterbox: "#040507"
  band: "#080A0F"
  field: "#10141C"
  rule: "#2C3445"
  rule-strong: "#56617A"
  wall-3d: "#2A3140"
  text: "#E9ECF3"
  text-dim: "#8E97AB"
  text-faint: "#485166"
  ball: "#FFF2CF"
  paddle: "#C6CEDC"
  brick-normal: "#E0533F"
  brick-armored: "#4D7FD6"
  brick-armored-hit: "#DE9A2C"
  brick-indestructible: "#4A5160"
  capsule-widen: "#3DC46B"
  capsule-slow: "#3F8EF5"
  capsule-multiball: "#9C66F2"
  capsule-glue: "#EE62A0"
  capsule-laser: "#EE463B"
  capsule-super-laser: "#2BCFD8"
  capsule-tiny: "#9BA5B9"
  capsule-player: "#F0C232"
  capsule-break: "#FF8A2B"
  asteroid: "#8B7F9A"
  explosion: "#FFB55E"
typography:
  display:
    fontFamily: "JetBrains Mono"
    fontSize: "64px"
    fontWeight: 700
    letterSpacing: "0.28em"
  title:
    fontFamily: "JetBrains Mono"
    fontSize: "44px"
    fontWeight: 700
    letterSpacing: "0.2em"
  value:
    fontFamily: "JetBrains Mono"
    fontSize: "20px"
    fontWeight: 700
  summary:
    fontFamily: "JetBrains Mono"
    fontSize: "16px"
    fontWeight: 700
  body:
    fontFamily: "JetBrains Mono"
    fontSize: "15px"
    fontWeight: 400
  note:
    fontFamily: "JetBrains Mono"
    fontSize: "13px"
    fontWeight: 400
  key:
    fontFamily: "JetBrains Mono"
    fontSize: "11px"
    fontWeight: 700
  caption:
    fontFamily: "JetBrains Mono"
    fontSize: "10px"
    fontWeight: 700
    letterSpacing: "0.16em"
spacing:
  band-height: "48px"
  margin-x: "20px"
  caption-top: "8px"
  value-top: "20px"
  control-row: "34px"
  keycap-height: "22px"
components:
  keycap:
    backgroundColor: "{colors.band}"
    textColor: "{colors.text}"
    typography: "{typography.key}"
    height: "22px"
    padding: "0 7px"
  capsule:
    textColor: "{colors.field}"
    typography: "{typography.caption}"
    width: "26px"
    height: "13px"
  autopilot-tag:
    textColor: "{colors.text}"
    typography: "{typography.caption}"
    height: "17px"
    padding: "0 7px"
---

# Design System: BRYX

<!-- Source of truth in code: src/palette.rs (every color), src/hud.rs
(canvas, type scale, band, state screens), src/assets.rs (sprite recipes and
brick marks). This file explains them; when they disagree, the code wins and
this file is stale. -->

## Overview

**Creative North Star: "The Measured Field"**

BRYX is a precise instrument for a fast game. The field is a dark,
calibrated 800×600 surface: a graduated ceiling rule says so, and every
object on it is told apart by more than color. The HUD reads like an
instrument readout, with captions over values and fixed-width numerals that
never jitter. It sits in its own band above the field, so it never competes
with the ball. Both renderers, the 2D quads and the 3D perspective view,
show the same words, numbers, colors and marks: one game, two lenses.

The look comes from the code's own discipline rather than from nostalgia.
One palette module, one HUD layer, one type family, procedural sprites
whose recipes fit on a screen. It is playful through color and motion in
the field, and restrained everywhere else.

**Key Characteristics:**
- A dark blue-black field. Warm and cool entity hues both read against it,
  and the ball is the brightest thing on screen.
- Hue never carries meaning alone. Capsules carry letters, brick kinds
  carry patterns, and the asteroid has its own silhouette.
- One family, JetBrains Mono (SIL OFL 1.1), bundled. Uppercase tracked
  captions sit over bold values.
- Flat chrome: no shadows or gradients in the interface. Depth lives in the
  sprites (bevels, gloss) and in the 3D lighting.
- Drawn keys. Controls appear as outlined keycaps, never as a sentence.

## Colors

Every color is authored once in `src/palette.rs` as sRGB hex and converted
at the point of use:
- **Linear** for shader colors and clears. The surface is sRGB; this fixed
  a gamma error that washed the whole game out.
- **sRGB bytes** for the atlas recipes, uploaded as `Rgba8UnormSrgb`.

### Surfaces
- **Letterbox** (#040507): outside the canvas when the window's aspect
  differs.
- **Band** (#080A0F): the HUD band, darker than the field so the two read as
  separate zones.
- **Field** (#10141C): the ground of both renderers. It is also the scrim
  (95%) and the ink for capsule letters.
- **Rule** (#2C3445) and **Rule Strong** (#56617A): the ceiling rule and its
  ticks, and keycap and tag outlines.
- **Wall** (#2A3140): the 3D renderer's side and top walls.

### Text
- **Text** (#E9ECF3): values, titles, keys. 16.7:1 on the band.
- **Text Dim** (#8E97AB): captions and secondary lines. 6.8:1 on the band,
  6.3:1 on the field.
- **Text Faint** (#485166): the score's leading zeros only, never content.

### Field objects
- **Ball** (#FFF2CF): warm white, the highest luminance of any moving object
  (pinned by a test). Its trail is the same sprite, fading.
- **Paddle** (#C6CEDC): brushed metal, tinted toward the active capsule's
  hue: Glue, Laser or Super laser.
- **Bricks**:
  - normal: Vermilion (#E0533F), a plain panel;
  - armored: Steel (#4D7FD6), a recessed plate and four rivets;
  - armored after one hit: Amber (#DE9A2C), the same plate with a crack;
  - indestructible: Graphite (#4A5160), diagonal hatching.
- **Capsules** (hue and letter):
  - Widen W (#3DC46B), Slow S (#3F8EF5), Multiball M (#9C66F2);
  - Glue G (#EE62A0), Laser L (#EE463B), Super laser X (#2BCFD8);
  - Tiny T (#9BA5B9), Player P (#F0C232), Break B (#FF8A2B).
- **Effects**: the Break gate and the laser bolts reuse their capsule's hue,
  so cause and effect read as one thing.
- **Hazards**: Asteroid (#8B7F9A) is cool cratered stone; its Explosion
  (#FFB55E) is the only warm thing about it.

### Named Rules
**The Never-Alone Rule.** No meaning rides on hue alone. A new capsule needs
a unique letter; a new brick kind needs a mark that the
`brick_kinds_differ_by_mark_not_only_by_color` test can tell apart.

**The One-Palette Rule.** A color literal outside `src/palette.rs` is a bug.
Both renderers import the same constants.

**The Brightest-Ball Rule.** Nothing that moves is brighter than the ball.

## Typography

**Family:** JetBrains Mono, Regular and Bold, bundled in `assets/fonts/` and
loaded from the binary. The system font database is never scanned.

**Character:** a typeface made for reading code, used here for numbers that
change sixty times a second and for labels set like an instrument's. It is
monospace for a measured reason: fixed advances keep the score from
jittering, and they let `hud::text_width` size keycaps and tags without
shaping text.

### Hierarchy (canvas pixels)
- **Display** (700, 64 px, tracking 0.28em): the BRYX wordmark on the menu
  only.
- **Title** (700, 44 px, tracking 0.2em): PAUSED, GAME OVER, VICTORY.
- **Value** (700, 20 px): the score.
- **Summary** (700, 16 px): the level line and the final-score line.
- **Body** (400, 15 px): control actions.
- **Note** (400, 13 px): the quit hint and the menu subtitle (bold,
  tracked).
- **Key** (700, 11 px): keycap labels.
- **Caption** (700, 10 px, tracking 0.16em, uppercase): SCORE, LEVEL, LIVES,
  the AUTOPILOT tag, capsule letters (11 px).

### Named Rules
**The Caption-Value Rule.** Band data is always a dim tracked caption over a
bright value. No kicker sits above a title on the state screens.

## Layout

- **Canvas**: 800×648 = a 48 px band plus the fixed 800×600 field. The window
  opens at canvas size and scales uniformly, letterboxed, never stretched
  (`hud::Letterbox`).
- **Field**: the simulation's own coordinates, untouched. Both renderers draw
  the world in the field's viewport, so a resize changes nothing the
  simulation sees.
- **Band**:
  - score left (20 px margin), with the autopilot tag beside it while
    `--autopilot` plays;
  - level centered: number, count, name;
  - lives right, one small paddle each, "+N" past six.
- **Ceiling rule**: 1 px at the field's top edge, a tick every 25 px and a
  major tick every 100 px. Ticks rise into the band, never into the field.
  This is the signature move: the field drawn as the measured instrument it
  is.
- **State screens** sit centered in the field over the scrim, their text on
  a slate (below). The controls table puts keycaps right-aligned and actions
  left-aligned on the field's center line, 34 px per row.

## Elevation & Depth

The interface is flat: no shadows, no blur, no gradients. Separation comes
from tone (band darker than field) and 1 px rules. Depth belongs to the
world only:
- **2D**: beveled bricks with a top-down gloss ramp, a brushed-metal paddle,
  a specular ball, a cratered asteroid, all lit from the top left.
- **3D**: Blinn-Phong lighting in linear space. Brick patterns are textured
  on the top and front faces, and stay on them while a destroyed brick
  tumbles away. The asteroid is a jagged, flat-shaded rock mesh.

## Shapes

- **Rectilinear by default**: bricks, paddle, band, keycaps and tags are
  square-cornered.
- **Round only where it carries meaning**: the ball is round; the capsule is
  a pill so it can never be mistaken for a brick; the asteroid has an
  irregular outline (2D sprite, 3D faceted mesh) so it can never be mistaken
  for the ball.
- **Keycaps**: a 1 px outline with a 2 px heavier bottom edge, the one hint
  of affordance in the chrome.

## Components

### Capsule
A 26×13 near-white pill sprite tinted with its hue, with its letter set as
text in field ink at its center. In 3D, a box with pill proportions at 1.5×
scale rolls about its long axis; its letter is projected flat onto it.
Letters appear only while playing; on state screens the scrim covers the
capsules.

### Keycap
Band fill, Rule Strong outline and bottom edge, Key-style label, 22 px
tall, at least square. Keys in a group are alternatives (4 px apart);
groups are 14 px apart.

### Slate
A full-width band of opaque field ink behind a state screen's text, ruled
top and bottom in Rule (1 px), like the ceiling. The scrim dims the scene;
the slate clears it from behind the words, which matters in 3D, where the
perspective puts the bricks and paddle where the title and prompt sit.
Full-bleed, so it reads as a band across the instrument, not a card.

### Life cue
One beat of 0.9 s, eased out, when the lives count changes:
- **Lost**: the LIVES caption turns Laser red; the lost life's icon stays in
  its slot in red, fading (and dropping 5 px unless reduced motion is set);
  a 3 px line at the field's floor lights red and fades.
- **Gained** (Player capsule): the caption turns Player gold, and the new
  icon starts gold and settles to metal.

It is drawn by the shared HUD layer, so both renderers show it, and it is
diffed render-side from `Game::lives`, never hooked into the simulation.

### Autopilot tag
An outlined box with a solid 4 px square and AUTOPILOT in caption style,
beside the score. Shown whenever `--autopilot` drives the input (PRODUCT.md,
"Honest about the machine").

### Motion
- Effects follow game events and are brief: brick flash (one frame), shake
  (2D 3 px for 80 ms, 3D camera 26 units for 150 ms), a four-step trail,
  explosions (350 ms), 3D tumble ghosts (500 ms).
- Nothing flashes above 3 Hz. The only interface animation is the single
  0.9 s life cue.
- `BRYX_REDUCED_MOTION=1` turns shake off in both renderers.

## Do's and Don'ts

### Do:
- **Do** take every color from `src/palette.rs` and convert it with
  `linear()` for shaders, or `srgb()` for recipes.
- **Do** give any new capsule a unique letter and any new brick kind a
  distinct mark.
- **Do** put new interface text through `hud::layout`, so both renderers
  show it and the off-canvas test covers it.
- **Do** keep band text inside the 48 px band and field furniture out of the
  field.

### Don't:
- **Don't** borrow any element of Arkanoid's or Taito's presentation: logo,
  lettering, palette, sprites or screens (PRODUCT.md, "Brand Commitments").
- **Don't** write a color as a linear float triple by eye. That is how the
  pastel wash happened.
- **Don't** draw interface text inside the field during play. Capsule
  letters are the one exception, and they belong to their capsules.
- **Don't** add shadows, glows or gradients to the chrome.
- **Don't** load system fonts. The bundled face is what makes every machine
  read the same.
