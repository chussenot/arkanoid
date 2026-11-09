---
name: BRYX
description: Incumbent visual system as found before the visual overhaul (scan-mode record).
colors:
  clear: "#3F4555"
  brick-normal: "#ED8989"
  brick-armored: "#95B3D7"
  brick-armored-hit: "#F3CB6C"
  brick-indestructible: "#898990"
  ball: "#FFE57C"
  paddle: "#E7EDF9"
  wall-3d: "#6F7686"
  capsule-widen: "#A0EDAA"
  capsule-slow: "#A0C4F9"
  capsule-multiball: "#E7A0F3"
  capsule-glue: "#F9B3D3"
  capsule-laser: "#F9897C"
  capsule-super-laser: "#95F9F9"
  capsule-tiny: "#C4C4C8"
  capsule-player: "#FFE57C"
  capsule-break: "#FFC459"
  asteroid: "#EDB389"
  hud-text: "#E6E8F0"
  title-text: "#FFD25A"
typography:
  title:
    fontFamily: "system sans-serif (glyphon Family::SansSerif)"
    fontSize: "52px"
    lineHeight: "58px"
  hud:
    fontFamily: "system sans-serif (glyphon Family::SansSerif)"
    fontSize: "22px"
    lineHeight: "26px"
  detail:
    fontFamily: "system sans-serif (glyphon Family::SansSerif)"
    fontSize: "20px"
    lineHeight: "26px"
spacing:
  hud-margin: "18px"
  life-icon: "16px"
  life-gap: "8px"
---

# Design System: BRYX (incumbent record)

<!-- Scan-mode record of the system as it stood before the visual overhaul
(branch claude/impeccable-visual). It is kept in git history as the
"before" reference; the overhaul replaces this file with the new system. -->

## Overview

There is no named direction yet. The game reads as a working prototype:
a flat blue-gray field, pastel bricks and capsules, a white metallic paddle,
a yellow ball, and default sans-serif text for the HUD and the four overlay
screens. Color constants are duplicated in `src/render.rs`,
`src/render3d/mod.rs` and `src/assets.rs`.

**Key characteristics:**
- Every color is authored as an sRGB-looking triple but written to an sRGB
  surface as if it were linear, so the screen shows a lighter, paler color
  than the one authored. For example, `brick-normal` is authored `#D94040`
  and shows as `#ED8989`, and the clear color is authored `#0D0F17` and
  shows as `#3F4555`. The frontmatter records the colors as **shown**.
- Brick kinds and capsules are identified by hue alone.
- The 2D renderer draws a HUD (score, level, lives) and overlay screens. The
  3D renderer draws neither.

## Colors

Pastel and low-contrast, because the colors are lifted by the gamma issue
above.
- **Field** (`clear`): the background of both renderers.
- **Brick kinds**: normal (coral), armored (steel blue), armored after one
  hit (amber), indestructible (gray).
- **Ball and Player capsule** share one yellow, and the life icons use it
  too.
- **Nine capsule hues**, most of them pastel. Laser, Break and Asteroid are
  all warm oranges and reds.

## Typography

A single system sans-serif at three sizes:
- **Title**: 52 px, used for BRYX, PAUSED, GAME OVER and VICTORY!
- **HUD**: 22 px, used for "Score: N" and "Level N: name".
- **Detail**: 20 px, used for the line under the title.

The text is laid out in physical pixels, so it does not scale with the
window.

## Layout

- The 800×600 playfield fills the window and is stretched non-uniformly on
  resize; letterboxing is not implemented yet.
- The HUD sits inside the playfield, 18 px from the top, which is inside the
  ball's flight space.
- Overlay screens put the title 40 px above center and the detail 30 px
  below it, over a 62% black scrim.

## Elevation & Depth

- **2D**: the procedural sprites carry bevels, gloss and specular highlights.
- **3D**: Blinn-Phong lighting (ambient 0.32), with tumble ghosts for
  destroyed bricks and camera shake.

## Components

- **Bricks**: beveled panels.
- **Paddle**: brushed metal; tinted pink while Glue is active, red with
  Laser, and cyan with Super laser.
- **Ball**: specular sphere sprite with a four-step trail.
- **Capsules**: 18 px flat squares in their hue.
- **Asteroids**: the ball sprite tinted orange.
- **Explosions**: a growing, fading disc lasting 0.35 s.
- **Lives**: 16 px yellow squares at the top right.
