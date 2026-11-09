# BRYX visual overhaul: report

Branch `claude/impeccable-visual`. Workflow: the Impeccable design skill,
vendored in `.claude/skills/impeccable/`. The skill was run in this order:
1. `init`, which wrote [`PRODUCT.md`](../../PRODUCT.md);
2. `document`, which recorded the incumbent system (that `DESIGN.md` is kept
   in commit `6c0d82b`);
3. `critique` and `audit` on the incumbent;
4. the brief-pinned direction, "Open-source arcade craft";
5. implementation;
6. visual verification of every state in both renderers;
7. a final critique;
8. the new [`DESIGN.md`](../../DESIGN.md).

## A. Initial assessment

The evidence was 10 screenshots: five states (Menu, Playing, Paused,
GameOver, Victory) in two renderers, all under Xvfb. The before contact
sheet is `docs/design/before.png`.

**Critique (design review only; the web detector does not apply to a native
game): 19/40.**

| Severity | Finding |
|---|---|
| P0 | The 3D renderer drew no HUD and no state screens: Menu, Paused, GameOver and Victory were pixel-identical, and score and lives were invisible. |
| P1 | Gamma error. Colors were authored as sRGB but written to an sRGB surface as if linear, so `#D94040` showed as `#ED8989`: a pastel wash, with weak contrast between field and entities. |
| P1 | Identification by color only. The nine capsules were flat squares told apart by hue (Laser, Break and Player are warm neighbors; Player was the ball's yellow). The four brick kinds were colors. This failed PRODUCT.md's color-blind requirement. |
| P1 | The HUD sat inside the ball's flight space (y = 18 px) over the ceiling run. |
| P2 | The asteroid was the ball sprite tinted orange. |
| P2 | Flat type hierarchy: one system sans-serif at 52 and about 20 px; no caption/value structure; proportional digits. |
| P2 | Resizing the window stretched the field, against the spec's own letterbox contract. |
| P3 | Life icons repeated the ball's yellow square. |

**Audit (native, adapted to desktop): 8/20, Poor.**

| # | Dimension | Score | Key finding |
|---|---|---|---|
| 1 | Accessibility | 1 | Color-only identification. Text in physical pixels did not scale with the window. No reduced-motion option. |
| 2 | Performance | 3 | Fine. Text was reshaped every frame (a few strings), and fonts came from a system scan. |
| 3 | Appearance and theming | 1 | Colors duplicated across 3 files, with the gamma error. |
| 4 | Platform conformance | 2 | Resize stretched the field non-uniformly; the HUD did not scale with the window. |
| 5 | Adaptivity | 1 | One window size only. Stretched on resize. |

## B. Design direction

**"The Measured Field"**: the brief's "open-source arcade craft" carried
into the game's own material. The field is a dark, calibrated 800×600
instrument. Everything on it is told apart by more than hue. The HUD reads
like a readout in its own band. The signature move is a graduated ceiling
rule: a tick every 25 px and a major tick every 100 px.

Taken from the brief's priorities, in order:
- **Readability first**: a HUD band outside the field, a ball that is the
  brightest object, letters and patterns.
- **Hierarchy**: dim tracked captions over bold values; one display size for
  the wordmark and one title size.
- **Consistency**: one palette module and one HUD layer for both renderers.

Rejected:
- Nostalgic arcade pastiche, which would also risk imitating Arkanoid/Taito.
- Neon and glow effects.
- A dashboard look.

## C. Changes implemented

Code (no physics, collision, timestep, scoring, level, control, CLI or
autopilot change):
- `src/palette.rs` (new). Every color authored once as sRGB hex, converted
  with `linear()` for shaders and `srgb()` for recipes. This fixes the gamma
  wash. It also holds the capsule letters (`W S M G L X T P B`).
- `src/hud.rs` (new). The shared interface layer:
  - an 800×648 canvas (48 px band plus the 800×600 field), letterboxed;
  - the band (score with dimmed leading zeros, the autopilot tag, the level
    as `05/32 Arrow`, lives as small paddles);
  - the graduated ceiling rule;
  - state screens with drawn keycaps;
  - capsule letters.

  `layout()` is pure and unit-tested; `Hud` owns one flat-quad pipeline and
  one glyphon renderer.
- `src/assets.rs`:
  - brick marks: armored plate and rivets, a crack on the hit armored brick,
    hatching on indestructible bricks. They are applied to the procedural
    bricks *and* composited onto `--assets` pack bricks;
  - a pill capsule sprite and a cratered rock asteroid sprite. Both are
    procedural-only, so existing packs keep loading;
  - recipes now work in sRGB.
- `src/render.rs`: the world is drawn in the letterboxed field viewport. The
  inline HUD, scrim and text code is replaced by `hud.rs`. The atlas is
  uploaded as `Rgba8UnormSrgb`, and capsules and asteroids use their own
  sprites.
- `src/render3d/`:
  - it now has the same HUD and state screens;
  - it draws from the session's shared atlas: the duplicated
    `render3d/atlas.rs` is deleted, as its own doc asked;
  - bricks are textured on top and front faces;
  - capsules are pill-proportioned, rolling, and carry projected letters;
  - all colors come from `palette`.
- `src/game.rs`: two read-only accessors for the HUD (`level_count`,
  `level_set`).
- `src/main.rs`: the window opens at the canvas size; the renderers learn
  whether the autopilot plays.
- `BRYX_REDUCED_MOTION=1` turns off screen and camera shake.

Assets:
- **JetBrains Mono** Regular and Bold, v2.304, unmodified, SIL OFL 1.1, in
  `assets/fonts/` with `OFL.txt`. Provenance and SHA-256 are in
  `ATTRIBUTION`.
- No other third-party content; all sprites are procedural and original.

## D. Design system

See [`DESIGN.md`](../../DESIGN.md). In short:
- **Surfaces**: letterbox `#040507`, band `#080A0F`, field `#10141C`.
- **Text**: `#E9ECF3`, `#8E97AB` and `#485166` (score zeros only).
- **Entities**: ball `#FFF2CF` (brightest by test); the four brick colors
  with their patterns; nine capsule hues with letters; cool stone asteroid
  `#8B7F9A` with a warm explosion.
- **Type scale** (JetBrains Mono): 64 / 44 / 20 / 16 / 15 / 13 / 11 / 10 px.
- **Motion**: event-bound and brief; no interface animation.
- **Rules**: Never-Alone (no meaning on hue alone), One-Palette,
  Brightest-Ball.

## E. Impeccable workflow

| Step | Status |
|---|---|
| `init` | Done. PRODUCT.md, after an interview (audience: both equally; accessibility: color-blind safe; autopilot: subtle indicator). |
| `document` | Done. Scan mode: incumbent DESIGN.md, commit `6c0d82b`. The qualitative interview was compressed: the incumbent had no direction to name, and the brief pinned the new one. The `.impeccable/design.json` sidecar was skipped: it holds HTML/CSS snippets for a web live panel, which does not apply to a native renderer. |
| `critique` | Done. 19/40, persisted with `critique-storage` (local runtime file, gitignored by the skill). |
| `audit` | Done. Native variant, 8/20. |
| Direction | Pinned by the brief ("Open-source arcade craft"). Per the skill, a pinned direction beats the `concept-seed` roll, so it was not run; the Operate mode rules applied, since the HUD is a working interface. |
| `craft-floor` | Read before the first edit. Applied: no eyebrow above titles; no hero-metric template on GameOver (one summary line); monospace justified by data and measurement; keys drawn rather than written as text. |
| Final critique | 33/40, up from 19/40 (persisted with `critique-storage`). Visibility of status 4, consistency 4, recognition 4, help 4, aesthetic 3, user control 3, error recovery still 2: losing a life is acknowledged only by the icon count. |
| `polish` | Done as part of the verification rounds: menu layout moved off the brick rows, larger 3D capsules, scrim raised to 95% (blending is linear, so 84% read lighter than intended). |

## F. Visual verification

Every state was captured in both renderers from a release build under Xvfb
(software Vulkan, llvmpipe). A frozen "showcase" frame was added: all nine
capsules, every brick kind, a real asteroid, laser bolts, the autopilot tag.
It is a staged state, not a gameplay moment. The tooling is a throwaway
patch of `main.rs` in a scratch build, never committed.

- `docs/design/before.png`: before, 2D (top) and 3D (bottom), all five
  states.
- `docs/design/after-2d.png`: after, 2D: menu, play (autopilot), showcase,
  pause, game over, victory.
- `docs/design/after-3d.png`: after, 3D, the same six frames.
- `image.png` and `render3d.png` (README): real autopilot play frames from
  the new build.

What was checked in the captures:
- No HUD text overlaps the field.
- Every capsule letter is distinct and readable at 1×.
- The four brick patterns are distinct.
- The asteroid reads as a rock, not a ball.
- The 3D renderer shows the same band and screens as 2D.
- Only automated, headless tests verified the letterbox; no window was
  resized by hand.

## G. Tests

Results are recorded in the pull request description, which is the record
for this branch. New tests:
- `palette`: the sRGB round-trip; all capsule letters and hues distinct;
  every letter at least 4.5:1 on its capsule; text contrast on the band and
  field; the ball brightest.
- `hud`: letterbox fit; score padding; every state named; controls on menu
  and pause; the autopilot tag only with the autopilot; capsule letters only
  in play; final score on GameOver and Victory; nothing off-canvas and band
  text inside the band, in every state.
- `assets`: brick kinds differ by mark; packs keep the procedural capsule and
  gain marks.

The pinned v1 replay hash test is unchanged and passing: the simulation is
untouched.

## H. Remaining issues

- **Real-hardware check**: verification ran on llvmpipe under Xvfb. A run on
  a GPU, on a HiDPI display, and with a hand-resized window is still owed.

Resolved in the follow-up pass (see the "Follow-ups" section below): the
3D titles over the scene, the missing life-lost moment, the spherical 3D
asteroid, and the unpatterned tumble ghosts.
- **Pack art**: marks are composited onto pack bricks; a pack whose bricks
  are already busy may need its own marks.
- **No in-game restart** from GameOver or Victory; the screen honestly says
  to close the window. Adding a key would be a controls change, out of scope.

## Follow-ups

The four interface issues the first pass left open, each fixed in the
shared layer where it applies to both renderers.

| Issue | Fix | Verified by |
|---|---|---|
| 3D titles sat over the dimmed bricks and paddle | A **slate**: an opaque, ruled, full-width band of field ink behind each state screen's text (`hud::slate`). It is drawn in both renderers; in 2D it covers nothing that mattered. | Screenshots of menu and pause in both renderers |
| A lost life had no on-screen moment | A **life cue** in `hud.rs`: for 0.9 s, the LIVES caption turns red, the lost icon fades in its slot, and the field's floor lights once. A Player capsule gets the same beat in gold. It is a render-side diff of `Game::lives` (`LivesWatch`) and honors `BRYX_REDUCED_MOTION`. | `a_lost_life_lights_the_floor_once_then_clears`; a frame captured 0.4 s after a forced loss, in both renderers |
| The 3D asteroid was a sphere | A **rock mesh**: a coarse sphere with each vertex pushed in or out by a hash of its quantized position (so seam and pole duplicates move together), flat-shaded per face, turning slowly. It takes one more draw call. | `rock_mesh_is_jagged_closed_and_flat_shaded` (checks jag range, flat normals, and that every edge is shared by exactly two faces); zoomed screenshot |
| 3D tumble ghosts lost their pattern | Ghosts carry their brick's sprite, and the shader picks textured faces by the *object-space* normal, so the pattern stays on its faces while the cube spins. | `a_tumbling_brick_keeps_its_pattern`. Not screenshotted: a ghost lives 0.5 s and the harness cannot freeze one mid-tumble. |

`docs/design/followups.png`: a life lost 0.4 s ago in 2D and 3D, and the 3D rock enlarged. `after-2d.png` and `after-3d.png` are refreshed with the slate.
