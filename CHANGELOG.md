# Changelog
All notable changes to this project will be documented in this file. See [conventional commits](https://www.conventionalcommits.org/) for commit guidelines.

- - -
## 0.5.0 - 2026-11-10
#### Features
- (**bench**) opt-in GPU stage timing, adapter line and BENCH_SIZE - (3d24472) - Clement Hussenot
- (**fx,audio**) blast notes, blast cracks and level-end sweeps on one shared clock - (e4e9349) - Claude, *Claude Opus 5.5*
#### Documentation
- (**fx**) measure the kinetic effects on a Radeon 780M - (c663f8c) - Clement Hussenot
- the shared cadence of blasts, cracks and level ends - (c7c1c3a) - Claude, *Claude Opus 5.5*
#### Miscellaneous Chores
- (**mise**) add deps task installing ALSA headers and vulkan-tools - (26341b4) - Clement Hussenot
- Update the pact events - (58127a4) - Clement Hussenot

- - -

## 0.4.0 - 2026-11-09
#### Features
- (**audio**) the sound pass: every effect, stinger and track on the new engine - (efbd862) - Claude, *Claude Opus 5.5*
- (**audio**) engine phase 3: seamless loops, reverb, tempo, band-limiting - (791423c) - Claude, *Claude Opus 5.5*
- (**audio**) engine phase 2: dynamics, chords, drum synthesis, automation - (762de11) - Claude, *Claude Opus 5.5*
- (**audio**) fork the engine for a 16-bit mix, held notes and real buses - (1858e98) - Claude, *Claude Opus 5.5*
- (**audio**) one track per level, a shuffled score for levelsets - (61d0382) - Claude, *Claude Opus 5.5*
- (**audio**) sound and music, rendered by the Electric Pulse engine - (b767b8a) - Claude, *Claude Opus 5.5*
- (**autopilot**) play the game itself, shot strategy from TypeSafe System One - (1390dd2) - Claude, *Claude Opus 5.5*
- (**fx**) kinetic effects with Lyon geometry and WESL post-processing - (6f63171) - Claude, *Claude Opus 5.5*
- (**game**) report every brick contact as a BrickStruck event - (55779d7) - Claude, *Claude Opus 5.5*
- (**game**) campaign speed ramp, act checkpoints, continue and fair Break - (651d468) - Claude, *Claude Opus 5.5*
- (**game**) add the arcade original's Player and Break capsules - (70d2537) - Claude, *Claude Opus 5.5*
- (**game**) add the Laser, Super laser and Tiny capsules - (e7bdc0d) - Claude, *Claude Opus 5.5*
- (**game**) an asteroid's explosion breaks the bricks around it - (56a5bce) - Claude, *Claude Opus 5.5*
- (**game**) add the Glue power-up and drifting asteroids; autopilot forecasts exactly - (b403c98) - Claude, *Claude Opus 5.5*
- (**hud**) serve prompt, end-screen keys and a local best score - (aaeb8e5) - Claude, *Claude Opus 5.5*
- (**levels**) open up Bridges and Baffles so their last bricks are reachable - (1fd8e04) - Claude, *Claude Opus 5.5*
- (**levels**) add a 32-level campaign in the spirit of the arcade original - (fa2f078) - Claude, *Claude Opus 5.5*
- (**levelset**) drop the bonuses authors placed, mapped to all nine power-ups - (4208b2b) - Claude, *Claude Opus 5.5*
- (**menu**) pick Campaign, Classic, a levelset or a Shuffle in the menu - (1a5f6c7) - Claude, *Claude Opus 5.5*
- (**music**) develop the themes, cadence every loop, shape each act - (74f95ea) - Claude, *Claude Opus 5.5*
- (**music**) a 32-track score on three themes, and themed stingers - (722c3dd) - Claude, *Claude Opus 5.5*
- (**powerups**) add the Explosive ball and Metal ball capsules - (63b7074) - Claude, *Claude Opus 5.5*
- (**render**) life cue, state-screen slate, 3D rock and patterned ghosts - (488d8b4) - Claude, *Claude Opus 5.5*
- (**render**) one palette and one HUD layer for both renderers - (75538e6) - Claude, *Claude Opus 5.5*
#### Bug Fixes
- (**audio**) music waits for fanfares, the serve and the warp - (1f6b6ea) - Claude, *Claude Opus 5.5*
- (**levels**) open a gate in level 3's floor so the game can be won - (92a98c4) - Claude, *Claude Opus 5.5*
- (**render3d**) use as_chunks for fixed-size triangle chunks - (48eb8de) - Claude, *Claude Opus 5.5*
#### Documentation
- (**autopilot**) design, latency budget, measurements and the level 3 limit - (82bc7cf) - Claude, *Claude Opus 5.5*
- (**design**) director review results, campaign mosaic and new screens - (2dd159f) - Claude, *Claude Opus 5.5*
- (**design**) record the follow-up fixes and refresh the screenshots - (bf6ef3f) - Claude, *Claude Opus 5.5*
- (**design**) record the new visual system and the overhaul report - (03b5fd9) - Claude, *Claude Opus 5.5*
- (**design**) record product context and the incumbent visual system - (6c0d82b) - Claude, *Claude Opus 5.5*
- (**music**) the score's guide, and audio, spec and README updates - (70f90bf) - Claude, *Claude Opus 5.5*
- (**readme**) add a mosaic of twelve fetched levels and the script that renders it - (f1f16f4) - Claude, *Claude Opus 5.5*
- the kinetic effects' design, measurements and contract - (7add7c8) - Claude, *Claude Opus 5.5*
- audio design, spec Sound section, README sound and M key - (4cd3cdf) - Claude, *Claude Opus 5.5*
- director review, and spec rules for runs, acts, speed and records - (cf86de0) - Claude, *Claude Opus 5.5*
- present the project as BRYX and drop the arcade-game inspiration lines - (a00364d) - Claude, *Claude Opus 5.5*
- describe the campaign, with a mosaic of its 32 levels - (5b4eb1e) - Claude, *Claude Opus 5.5*
- specify the Player and Break capsules; map the capsules to the arcade original - (3045dbf) - Claude, *Claude Opus 5.5*
- specify the Laser, Super laser and Tiny capsules; README capsule table - (6dd3797) - Claude, *Claude Opus 5.5*
- specify Glue, asteroids and stall breaking; update the autopilot page - (e87e063) - Claude, *Claude Opus 5.5*
#### Build system
- (**deps**) bump judgment to 0.17 - (7df3c30) - Claude, *Claude Opus 5.5*
#### Refactoring
- (**audio**) count the rally in one place both layers can share - (6f24065) - Claude, *Claude Opus 5.5*
- rename the project from arkanoid to BRYX - (68041a0) - Claude, *Claude Opus 5.5*
#### Miscellaneous Chores
- (**audio**) vendor the Electric Pulse audio engine, unmodified - (012ae31) - Claude, *Claude Opus 5.5*
- (**skills**) vendor the Impeccable design skill into .claude/skills - (1a7e2ba) - Claude, *Claude Opus 5.5*
- drop an agent worktree committed by mistake and ignore the directory - (4223362) - Claude, *Claude Opus 5.5*
- add the MIT LICENSE file and Cargo license metadata - (6b2fd8b) - Claude, *Claude Opus 5.5*

- - -

## 0.3.0 - 2026-08-17
#### Bug Fixes
- **(cargo)** set default-run so cargo run/mise run run pick the game - (55a88d5) - Clement HUSSENOT-DESENONGES
#### Documentation
- **(readme)** add 3D renderer capture, confirm epic/presentation-3d ci gate - (d4ab585) - Clement HUSSENOT-DESENONGES
- **(readme)** add procedural vs pack screenshot section - (4750920) - Clement HUSSENOT-DESENONGES
#### Features
- **(assets)** load Kenney CC0 puzzle pack via --assets, fall back to procedural - (c952cbb) - Clement HUSSENOT-DESENONGES
- **(assets)** generate procedural texture atlas recipe - (e2d2c66) - Clement HUSSENOT-DESENONGES
- **(events)** add BrickDestroyedAt for 3D destroy-tumble geometry - (f594820) - Clement HUSSENOT-DESENONGES
- **(render)** sample the sprite atlas for paddle/ball/brick quads - (5988856) - Clement HUSSENOT-DESENONGES
- **(render3d)** add juice -- hit-flash, destroy-tumble, trail, shake, spin - (59efaa6) - Clement HUSSENOT-DESENONGES
- **(render3d)** texture brick front faces from a local atlas mirror - (034c324) - Clement HUSSENOT-DESENONGES
- **(render3d)** add perspective 3D pipeline skeleton + --renderer flag - (2ac4fd1) - Clement HUSSENOT-DESENONGES
- **(textures)** add TextureSource enum + atlas abstraction - (ba7c710) - Clement HUSSENOT-DESENONGES
#### Miscellaneous Chores
- Update the ATTRIBUTION - (d675c7d) - Clement HUSSENOT-DESENONGES
#### Performance Improvements
- **(render3d)** measure full-board+multiball+tumble fps, add math property tests - (402bdb6) - Clement HUSSENOT-DESENONGES
#### Tests
- **(assets)** pack-loader test against a committed fixture atlas - (2148a22) - Clement HUSSENOT-DESENONGES

- - -

## 0.2.0 - 2026-08-17
#### Bug Fixes
- Remove the placeholder in the README - (0f2169d) - Clement HUSSENOT-DESENONGES
#### Documentation
- **(levels)** document the LBreakoutHD levelset format subset - (c59f1d0) - Clement HUSSENOT-DESENONGES
- **(readme)** explain thousands-of-levels fetch and GPL/MIT rationale - (237a6b9) - Clement HUSSENOT-DESENONGES
#### Features
- **(cli)** add --levelset flag wiring and mod levelset; - (d2a7cf5) - Clement HUSSENOT-DESENONGES
- **(levels)** wire --levelset into Game's level progression - (fd223c0) - Clement HUSSENOT-DESENONGES
- **(levels)** level-set progression + RANDOM10 mode - (0d9fac6) - Clement HUSSENOT-DESENONGES
- **(levels)** src/levelset.rs loader for LBreakoutHD .lbl levelsets - (8e7acd8) - Clement HUSSENOT-DESENONGES
- **(levels)** add LBreakoutHD levelset fetch script - (4e9a587) - Clement HUSSENOT-DESENONGES
- **(v2)** deterministic-replay foundation + CLI/fleet conventions - (546ba69) - Clement HUSSENOT-DESENONGES
#### Miscellaneous Chores
- **(pact)** sync the coordination protocol block - (6695156) - Clement HUSSENOT-DESENONGES
- Add the pact and beads files - (2c8b3d1) - Clement HUSSENOT-DESENONGES
- Pact sync - (8013a6d) - Clement HUSSENOT-DESENONGES
- Add a screenshot - (1d757cf) - Clement HUSSENOT-DESENONGES
#### Tests
- **(levels)** add homemade fixture levelset + loader/scripted-clear tests - (6ac32f8) - Clement HUSSENOT-DESENONGES

- - -

## 0.1.0 - 2026-08-08
#### Documentation
- add game design spec and pact merge strategy - (99770c6) - Clement HUSSENOT-DESENONGES
#### Features
- **(game)** implement milestones 1-5 of arkanoid-rs - (778e9a8) - Clement HUSSENOT-DESENONGES
- **(render)** draw multiball extra balls and falling powerups - (846c016) - Clement HUSSENOT-DESENONGES
#### Miscellaneous Chores
- **(mise)** add build/test/lint tasks - (6de8ee3) - Clement HUSSENOT-DESENONGES
- **(pact)** sync the coordination protocol block - (4b06d42) - Clement HUSSENOT-DESENONGES
- Add audit about how pact work on this repo - (f8d8afa) - Clement HUSSENOT-DESENONGES
- Bootstrap the repo - (1e0fe44) - Clement HUSSENOT-DESENONGES
- Root commit - (3e7e3ab) - Clement HUSSENOT-DESENONGES
- Root commit - (b018367) - Clement HUSSENOT-DESENONGES

- - -

Changelog generated by [cocogitto](https://github.com/cocogitto/cocogitto).