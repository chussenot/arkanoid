# BRYX game director review (v0.5.0)

This review sets the direction for the next development cycle. It reads
the game on `master` at `3197173` (v0.5.0, after PR #24). It reads the
code, not the intentions in the docs. Where a doc and the code disagree,
the code wins and the disagreement is listed.

It follows the first [director review](director-review.md). Its six
"ship now" decisions (D1–D6) are all implemented. Of its "next" items,
audio and the 2D shards are done; the per-act accent (A1), the 3D fill
light (A3) and key remapping (U4) are not. Section 1.4 has the evidence.

## Evidence base

- **Code read.** `src/game.rs` (rules and constants), `src/levels.rs` (the
  32 campaign grids), `src/rally.rs`, `src/cadence.rs`, `src/hud.rs`,
  `src/menu.rs`, `src/record.rs`, `src/main.rs`, `src/fx/`,
  `src/audio/director.rs`, `src/render3d/`, `src/autopilot/`.
- **Docs read.** `docs/spec.md`, `docs/design/*.md`, `README.md`,
  `audio/compose.py`.
- **Level census.** For each campaign level, computed from the grids:
  brick counts by type, hit points, brick score and shape.
- **Pacing probe.** The heuristic autopilot played the full campaign,
  every feature on, 8 seeds × 32 levels. It recorded per level:
  - seconds to clear;
  - the share of that time spent on the last 10% of bricks (the "hunt");
  - lives lost;
  - capsules spawned and caught;
  - the longest rally.

  The probe is a scratch test, not committed, like the first review's.
- **Limits.** The autopilot won 8 of 8 runs and lost one life in total.
  So it measures pacing and layout flow, not human difficulty. Nobody has
  played v0.5.0 for this review, and nobody has listened to the in-game
  mix on speakers (`docs/design/audio.md` says so). Every claim about
  feel is therefore a hypothesis, and section 8 keeps those separate.

---

## 1. The actual game

### 1.1 Implemented and solid

| Area | What is there | Where |
|---|---|---|
| Core simulation | 120 Hz fixed step, pure `Game`, substepped circle-vs-AABB collision, a pinned replay hash under `Features::V1` | `game.rs`, `docs/spec.md` |
| Ball and paddle | Launch at 300 px/s, ×1.04 per paddle hit; campaign cap ramps from 560 to 700 px/s; exit angle 30°–150° with a ±5° dead zone | `game.rs:41-54`, `:862`, `:1104-1111` |
| Campaign | 32 named levels in 4 acts of 8; Game Over continues from the act's first level | `levels.rs:102-463`, `game.rs:867`, `:1116` |
| Other level sets | v1 Classic, loadable levelsets, Shuffle, all in the level picker | `menu.rs` (PR #22) |
| Power-ups | Exactly 11, gated by `Features`: 15% drop chance, uniform odds | `game.rs:68`, `:1573-1598` |
| Asteroids | First at 4 s, then every 5–10 s, at most 3, 100 points each | `game.rs:144-161` |
| Stall rule | After 10 s with no progress, wall bounces bend the ball 7° | `game.rs:133-139` |
| Rally | Bricks broken between two paddle touches step up a 10-note E minor pentatonic; the effects' hue follows the same step | `rally.rs`, `audio/director.rs:74-76` |
| Kinetic effects | Lyon cracks and fracture pieces, HDR bloom and shockwave, skipped on idle frames; blast notes, blast cracks and level-end sweeps share one clock | `fx/`, `cadence.rs`, `docs/design/kinetic-fx.md` |
| Music | 32 per-level tracks from 3 themes (Ascent, Wall, Spark) in E: Dorian, Aeolian, Phrygian and harmonic minor, a different form per act | `docs/design/music.md`, `audio/compose.py` |
| Records | One best score each for Campaign and Classic; NEW BEST on the end screen | `record.rs:40-61`, `hud.rs:840-842` |
| Accessibility | Capsule letters, brick patterns, shape-marked selection, `BRYX_REDUCED_MOTION` | `hud.rs`, `palette.rs` |

### 1.2 Partial

- **3D renderer** (`--renderer 3d`): feature parity with 2D. Fixed camera
  (eye at 0, 900, 560; 50° FOV). One directional light plus a flat
  ambient (`render3d/mod.rs:633-634`). Nothing in it is specific to 3D.
- **Autopilot** (`--autopilot`): plays well. It cannot act as a demo: in
  the menu it presses Space, and it stalls on the end screens.
- **Settings**: environment variables only (`BRYX_FX`,
  `BRYX_REDUCED_MOTION`, `BRYX_SOUND`, `BRYX_DATA_DIR`). M cycles the
  sound. There is no settings screen.
- **Act identity**: it exists in the music only. The act names
  (Foundations, Structures, Machines, Vault) appear in
  `docs/design/music.md:106-114` and in comments in `compose.py`.
  No Rust code names an act, and nothing on screen shows one.

### 1.3 Docs-only, experimental or missing

- **Docs-only.** Act names; the layout rule "no breakable brick between
  two X bricks in one column" (`director-review.md`). No test enforces
  the rule, and two levels break it: Fortress (columns 2 and 11) and
  Picture frame (columns 0 and 13).
- **Experimental.** The 3D renderer; the TypeSafe model behind the
  autopilot.
- **Missing.**
  - Onboarding: no first-run detection, no tutorial level.
  - An end-of-run summary, and a level-clear interstitial.
  - Power-up timers on screen.
  - Player-facing seeds and replays: `Game::with_seed` is `#[cfg(test)]`
    (`game.rs:1031`), and `Game::new` seeds from OS entropy (`:996`).
  - Fullscreen, gamepad, key remapping, auto-pause when the window loses
    focus.
  - Sounds for: indestructible hits (metal ball included), a capsule
    spawning, wall bounces, glue release, a power-up expiring, the gate
    opening.
  - Records for levelsets and Shuffle.

### 1.4 Doc–code mismatches to fix in passing

- `CLAUDE.md` still says "Pre-implementation".
- `director-review.md:42,53,178` counts 9 capsule kinds; there are 11, so
  Tiny is 1 in 11, not 1 in 9.
- The README's victory row omits P and Esc.
- The levelset count reads 136 in some docs and 135 in others.
- The `render3d` module header says the HUD is out of scope; the HUD
  is drawn.
- In `compose.py`, the act comments contradict the recipes beneath them.

---

## 2. Creative identity

Take away the sound and BRYX is a clean, legible, well-built brick-breaker
with nothing in it a player would describe to a friend. The layouts,
power-ups and asteroids are genre staples, executed carefully.

**What is distinctive already exists, but sits in the presentation layer.**
Every brick broken between two paddle touches plays the next note up a
pentatonic scale. The effects' hue climbs with it, and blasts land on the
beat (`cadence.rs`). Twenty bricks off one shot sound like a phrase. Each
level has its own track, and the three themes are developed across four
acts. That is the hook: **in BRYX, a good shot is a melody.**

The rally is also the genre's best moment: the ball gets above the wall
and takes it apart from behind. BRYX is the only brick-breaker I know of
that makes that moment audible as music. Today the moment has **no stake**:
- the score ignores it (`game.rs:1507`: `self.score += brick.score;`);
- the HUD doesn't show it;
- no record keeps it.

The game's identity lives in a layer the rules never read.

The second identity asset is unrealized. Four acts with architectural
names (Foundations → Structures → Machines → Vault) suggest a building
being taken apart. On screen, all 32 levels share one field color
(`palette.rs:95`) and one background.

---

## 3. Gameplay

### 3.1 Core feel

The foundations are right:
- deterministic physics;
- an exit angle the player controls;
- a speed ramp per paddle hit;
- tunneling prevented;
- a stall rule.

Two verified defects hurt the first seconds of play.

- **The serve is skipped.** Starting a run and serving both read the
  *held* Space key, not a fresh press. In the menu, `input.space` sets
  `Playing` (`game.rs:1194`). On the next tick, 8 ms later, the attached
  ball launches if Space is still down (`:1267`), and a keypress lasts
  50–150 ms. So level 1 serves at once, and the serve prompt (D6,
  `hud.rs:344-346`) is never seen. Continue and Play again do the same:
  `restart_at` sets `Playing` (`:1141`), and the next tick launches. The
  player never gets to position the first serve.
- **The serve itself is fixed:** always 60°, up and to the right
  (`:47`, `:1414-1419`). The only choice is the paddle's x. That is fine
  if the serve is a deliberate moment; today it isn't one at all.

### 3.2 Mechanics: what the player decides

- **Real decisions:**
  - aim, through the paddle offset;
  - chase or ignore a capsule, an asteroid or the Break gate;
  - dodge Tiny.
- **Missing decisions:**
  - Nothing rewards a skilled line over a safe one. There is no combo, no
    multiplier, no speed, time, no-death or clear bonus.
  - Armored bricks need twice the hits for the same score (`:487-490`).
  - Asteroid points accrue only while the ball is in flight, so a slower
    level earns more (about 10 asteroids, up to 1,000 points, against an
    average of 1,980 brick points per level). **The scoring rewards
    stalling.**
- **Power-ups are uniform.** All 11 kinds have the same odds, about 19 of
  each per campaign. Every timed power lasts 15 s and survives a lost
  life. The player can't see any timer.

### 3.3 Campaign curve

| Act | Breakable bricks (avg/level) | Armored share | X share | Probe clear time (avg) |
|---|---|---|---|---|
| I | 35.6 | 24.6% | 2.4% | ~56 s |
| II | 43.1 | 16.8% | 11.5% | — |
| III | 54.9 | 24.1% | 3.9% | — |
| IV | 43.5 | 37.1% | 18.9% | ~70 s |

- **Nothing escalates steadily.** Brick count peaks in Act III; armored
  and X shares zig-zag; within an act the order is ungraded. In Act IV,
  L28 (40 HP) and L31 (39 HP) sit beside L30 (120 HP, the campaign's
  maximum). The speed cap is the only monotone ramp, and the 560 → 700
  px/s cap is reached after 16–22 paddle hits, so it matters only in long
  rallies.
- **The vertical field is never used.** The tallest grid is 8 rows, which
  ends 236 px down a 600 px field. The lower 60% of the field is empty in
  every level, so distance to the bricks never varies.
- **Ideas repeat.**
  - L11 Double W and L28 Chevron share their first two rows.
  - L7 Diamond and L15 Rocket share a nose.
  - Four levels are a frame around a core: L9, L22, L26, L30.
  - Four are walled chambers: L4, L12, L25, L29.
  - Five are dot or checker fields: L3, L8, L23, L31, plus L13's outer
    rows.
  - L24 Funnel is the top half of L20 Hourglass.
- **Hunts.** These levels end on a long last-brick hunt:

  | Level | Clear time | Hunt share |
  |---|---|---|
  | L12 Fortress | 135 s | — |
  | L18 Pillars | 126 s | 28% |
  | L13 Lattice | — | 33% |
  | L21 Shield | — | 25% |
  | L5 Arrow | — | 24% |

### 3.4 Act differentiation

Outside the music, Act I and Act IV differ only in numbers: the speed cap,
and more armored and X bricks. Act I already has every brick type
(armored from L2, X from L6), so no act introduces anything.

### 3.5 Finale

- **L32 Vault is not the campaign's hardest level by any measure.**
  - Breakable bricks: 48 (L20 and L30 have 84).
  - HP: 72 (L30 has 120).
  - Brick score: 2,160, the same as L4.
- **Only the music marks it as the end:** 150 BPM and all three themes.
- **It can be skipped.** A Break capsule on L32 warps straight to
  Victory, and a test pins that (`game.rs:3509`). The climax of 32
  levels can end by sliding the paddle out of a gate.

---

## 4. Audiovisual direction

### 4.1 What works

- One visual language: every entity is a quad, the palette is flat, and
  the shapes are legible. Effects are bounded and age-driven, and they
  answer events.
- Effects cost nothing when idle: the direct path matches pre-FX master.
  At full load on a Radeon 780M, 1920×1200 takes about 3.0 ms
  (`docs/design/kinetic-fx.md` §6).
- The music is composed, not looped stock: three themes, 22 variants,
  and a different form per act.

### 4.2 What doesn't

- **The rally plays out of key in five tracks (verified).**
  - The scale is `[0, 3, 5, 7, 10, 12, 15, 17, 19, 22]` semitones above E
    (`audio/director.rs:75-76`). Steps 5 and 10 are D natural.
  - The harmonic-minor tracks (16 Bridges, 19 Crown, 24 Funnel, 29 Split
    wall, 32 Vault) use D sharp; the Wall theme plays it as a written
    note (`music.md:25`).
  - Step 10 is also where the scale caps (`rally.rs:64`). So every
    rally longer than nine bricks holds D natural against the track's
    D sharp, including through the finale.
  - `music.md:70` promises "a rally must always harmonize".
- **The audio palette is thin.** There is one brick sample at 10 pitches.
  Hitting an indestructible brick, the gate opening and a power-up
  expiring make no sound. These are the moments a player most needs to
  hear.
- **Nobody has heard the mix.** The in-game level of −19.5 dBFS is
  computed, not heard.
- **The field never changes.** One `FIELD` color, one background for all
  four acts.

### 4.3 Challenge: four acts, four visual movements

The goal: each act gets its own look and sound, built from a small fixed
set of variables rather than new systems. Four primitives, one value per
act. They live as data in an `ActStyle` table in `palette.rs`, read by
both renderers:

| Primitive | What it is | Constraint |
|---|---|---|
| **Ground** | A static background motif drawn as quads under the bricks | Luminance at most 6% above `FIELD`, so the ball always reads first; never animated under reduced motion |
| **Accent** | One hue for walls, HUD rules and the effects' low rally steps | Passes contrast against every brick color; color-blind marks unchanged |
| **Pulse** | The one element that moves on the beat (from the existing cadence clock) | At most one per act; off under reduced motion |
| **Fracture** | How a brick breaks: a parameter set for the existing Lyon effects | No new effect types |

| | I · Foundations | II · Structures | III · Machines | IV · Vault |
|---|---|---|---|---|
| **Idea** | The surveyed ground | The frame goes up | The building runs | The thing it protects |
| **Ground** | Survey grid: dots at brick pitch, the lattice the bricks sit on | Girders: verticals and truss diagonals, behind the X bricks | Gear teeth along both side walls | Concentric tumbler rings around the top center |
| **Accent** | Warm chalk / ochre | Steel blue | Amber | Gold over deep violet |
| **Pulse** | Grid dots brighten on the downbeat | Beams light on each bar | Teeth advance one notch per beat: a clock | Rings turn one notch per bar |
| **Fracture** | Clean splits (today's shards) | Long splinters along the brick's axis | Sparks: more, smaller, faster | Cracks first, then a heavy break |
| **Music** (already composed, `music.md:106-114`) | Spark leads, light textures | Ascent takes over; the Wall enters (L12) | The Wall leads, buzzing squares | All three themes |
| **Layout rule** | Open shapes; armored only | X appears as architecture | Bricks reach lower (rows 9–12), so less travel and more speed | Mixed, with the deepest grids; an X ring around the finale |

The act transition is the movement's entrance:
- a title card ("II · Structures") on the downbeat of the new act's
  first track;
- the ground motif drawing itself in over one bar;
- a one-bar stinger rendered from the existing ABC pipeline.

Payoff: Vault's rings align when its last brick breaks.

**Why this, and not more shaders:** each act is 4 values in a table.
Nothing new runs in the frame loop except the ground quads (a few dozen
instances). 3D gets the accent and the ground for free, as quads on
the floor plane. A new act, or a levelset with its own style, means
adding a table row. Validation: screenshots of all four acts side by
side; a contrast check of the ball against every ground; and the
reduced-motion test.

---

## 5. Player journey and replayability

| Stage | Today | Problem |
|---|---|---|
| First launch | The menu: Campaign, Classic, Levelset, Shuffle, and a controls table | Nothing teaches; first-time players and returning players see the same screen |
| First serve | Skipped (section 3.1) | No aiming moment |
| During play | Score, lives, level | Power-up state invisible; the rally only heard |
| Level clear | Straight into the next level, with a beat-aligned sweep (PR #24) | No beat to breathe; act changes unmarked |
| Game Over | Continue from the act checkpoint, at 3 lives and score 0 | Continue erases the score, so a score chaser gains nothing from it |
| Victory | Restart at level 1 | No summary, no sense of what this run was |
| Records | One integer each for Campaign and Classic | Nothing for levelsets or Shuffle; no best rally, no best time |

**Replayability: the asset nobody uses.**
- The simulation is deterministic given a seed, and the v1 replay hash
  proves it.
- `Input` is a handful of booleans per tick. At 120 Hz, run-length
  encoded, a 40-minute campaign is a few kilobytes.

The game already has everything needed for:
- **seeded runs**: a seed you can type or share, and a daily seed from
  the date;
- **replay files**: the seed, the level set, the build version and the
  input stream;
- **ghosts**: a second `Game` stepped alongside from a replay, drawn as
  a translucent paddle. The simulation is pure, so this is cheap.
- **attract mode**: the menu, after 20 s idle, plays a bundled replay.
  That beats driving the autopilot, which presses Space in the menu
  and stalls on end screens.

**Risk:** replays are tied to a build. f32 math with `sin` and `cos` can
differ across platforms and compilers. Mitigation: store the build hash,
refuse a mismatched replay, and test that a recorded campaign reproduces
on CI's Linux runner.

---

## 6. 2D vs 3D

- **2D is the game.** It is the default; the HUD, legibility and
  accessibility work target it.
- **3D is a second camera on the same quads.** At parity, with a fixed
  camera, it offers no reason to choose it. Each new visual idea now
  costs twice.
- **Decision:**
  - Keep 3D opt-in and experimental.
  - Spend nothing on 3D parity or 3D-only features this cycle.
  - Design every visual change as data both renderers read (`ActStyle`).
    3D then keeps up without its own work.
- **The one reason 3D could earn its place is as a cinematic lens:** the
  camera moving on act transitions and at the Vault's opening, where
  play is paused. Revisit in Horizon 3, and only if the playtest shows
  someone choosing 3D. Until then, the stale module header and the
  missing fill light (A3) are not worth a cycle.

---

## 7. Beyond the current implementation

Every idea below is an improvement (I), a differentiation (D) or an
experiment (E). It reuses what exists. None adds a twelfth power-up or a
new dependency.

| # | Idea | Class | Player benefit | Rationale | Implementation | Risks | Validation |
|---|---|---|---|---|---|---|---|
| 1 | **Rally is the score.** A brick scores its value × the rally step (1–10). The HUD shows the step; the end screen keeps the best rally. | D | Skilled shots pay; the melody has stakes | Turns the game's identity into its rules; ends the reward for stalling | Move the rally count into `Game` as the source of truth (`rally.rs` reads it). Gate behind `Features` so the v1 hash holds | Explosive and Metal balls inflate rallies; balance asteroid points too | Headless tests; autopilot score variance across seeds; playtest: do players aim for rallies? |
| 2 | **Rally in key, cap on the tonic.** The director picks the scale per track (D sharp in harmonic minor), and step 10 becomes E, the octave above step 6 | I | The top of a rally resolves instead of clashing | Fixes a verified defect; the cap becomes a resolution | Per-track scale in `audio/director.rs`; test each track's scale against its mode | None | Unit test per track; listening pass |
| 3 | **Serve on a fresh press.** | I | An aimed first shot; the serve prompt finally shows | Fixes a verified defect | Use `space_pressed` for the menu and for launching from the paddle (`game.rs:1194`, `:1267`); held Space keeps firing guns | Replay hash: menu path only, check it | Test: holding Space from the menu doesn't serve |
| 4 | **Four movements** (section 4.3) | D | Acts feel like places; progress is visible | Realizes the existing music direction on screen | `ActStyle` table, ground quads, title card, stinger | Clutter behind the ball | Contrast check; reduced-motion test; playtest recall ("which act did you like?") |
| 5 | **Campaign re-curve inside acts.** Order each act by HP; make Vault the hardest level; no Break capsule on act finales (L8, 16, 24, 32); use rows 9–12 from Act III | I | Rising tension and a real finale | Verified flat curve; finale skippable | Data only, in `levels.rs` plus a drop-table rule; revisits the first review's "no reordering" decision, but within acts only | Breaks players' muscle memory (there are none yet) | Probe: clear time rising within each act; the reachability test |
| 6 | **Power-up legibility.** A timer ring on the paddle, a last-3-seconds blink and an expiry sound; per-act capsule weights (Act I generous, Act IV scarce on Player and Break) | I | Players can plan around powers | 11 opaque 15 s timers today | HUD quads; a weight table in data | Clutter on the paddle | Playtest: can players say what's active? |
| 7 | **Run card.** On Game Over and Victory: score, best rally (played back as its notes), levels cleared, time, seed | D | Each run leaves something to remember | Nothing summarizes a run today | HUD screen plus a short arpeggio from the existing cue | None | Playtest: "would you share this?" |
| 8 | **Records per level set.** Best score, best rally and best time for every set, Shuffle included | I | Every mode becomes worth replaying | Levelsets keep nothing today | Extend `record.rs`, keyed by set id | Migrating the file format | Test round-trip |
| 9 | **Seeds and replays** (section 5) | D | Fair challenges; a run worth sharing | Determinism is already there | Seed in `Game::new`; a replay writer and reader in `main.rs` | Cross-build determinism | CI test replaying a recorded campaign |
| 10 | **Daily seed and ghost** | E | A reason to come back each day | Builds on 9 | A second `Game`, drawn translucent | Distraction; the cost of a second sim is negligible | Retention in the playtest group |
| 11 | **Attract mode from a replay** | E | The menu demonstrates the game | The autopilot can't run unattended | Built on 9 | None | Idle the menu for 20 s |
| 12 | **The vault opens.** When Vault's last brick breaks, its X ring fractures with the existing effects, on the Victory E major chord | D | A finale people remember | Today the end is a state change | One fx call on the Victory event | None | Watch it at 1× |
| 13 | **First-run lesson in level 1.** Prompts appear in the field: aim, then the rally ("break three without touching the paddle") | I | New players learn the hook in 60 s | Nothing teaches the rally today | HUD prompts plus a first-run flag in `record.rs` | Patronizing returning players: show it once | Playtest: does a new player name the rally unprompted? |

**Explicitly not proposed:**
- new power-ups;
- moving or multi-hit brick mechanics;
- a level editor;
- new post-processing passes;
- 3D-only features;
- adaptive music stems.

Each is real work with a weaker case than the items above.

---

## 8. The ten most important weaknesses

### Verified problems

| # | Observation | Evidence | Impact | Recommendation | Priority |
|---|---|---|---|---|---|
| 1 | The rally, the game's identity, is invisible to the rules | `game.rs:1507` adds a flat brick score; `rally.rs` is presentation only; no combo or multiplier anywhere | The best moment pays nothing; BRYX plays like any brick-breaker | Idea 1 | P0 |
| 2 | The rally plays D natural against D sharp in five tracks, and holds it once capped | `audio/director.rs:75-76`; `music.md:25,70`; tracks 16, 19, 24, 29, 32 | The game's signature sound clashes, worst in the finale | Idea 2 | P0 |
| 3 | The serve is skipped on a held Space | `game.rs:1194`, `:1267`, `:1141` | No aimed first shot; D6's prompt is dead | Idea 3 | P0 |
| 4 | Acts are invisible | One `FIELD` (`palette.rs:95`); act names only in the music docs; no title card | 32 levels feel like one long level | Idea 4 | P0 |
| 5 | No escalation and no finale; the finale can be skipped | Section 3.3 census; probe times flat; `game.rs:3509` | The campaign has no shape and no climax | Idea 5 + idea 12 | P1 |
| 6 | Scoring rewards stalling and ignores difficulty | Asteroid points only accrue in flight (`:1280-1284`, `:1944`); armored = normal (`:487-490`) | Score doesn't measure skill | Idea 1, with asteroids worth more inside a rally and armored at ×1.5 | P1 |
| 7 | Power-ups are opaque and undifferentiated | No timer UI; uniform odds (`:1534`); about 19 Player and 19 Break per campaign | Players can't plan; Break and Player flatten difficulty | Idea 6 | P1 |
| 8 | Runs leave nothing behind | No summary; Continue resets score to 0 (`:1127`); no records for levelsets or Shuffle | No reason to replay or share | Ideas 7 and 8 | P1 |
| 9 | Layouts repeat and never use the field's depth; some hunts drag | Section 3.3; the layout rule is broken in Fortress and Picture frame, and no test enforces it | Sameness; dead time | Idea 5, plus a test for the X-column rule | P1 |
| 10 | Determinism isn't offered to players | `with_seed` is `#[cfg(test)]` (`game.rs:1031`); no replay I/O | The cheapest differentiator goes unused | Idea 9 | P2 |

Comfort gaps also belong in Horizon 1, but don't make the ten:
- no fullscreen toggle;
- no auto-pause on focus loss;
- no remapping;
- settings only through environment variables;
- missing sounds for indestructible hits, the gate and expiry.

### Hypotheses that need a human playtest

- **H1.** 560 px/s on level 1 is right for a first-time player. The
  autopilot can't tell.
- **H2.** Tiny (1 in 11 capsules) is more frustrating than interesting.
- **H3.** Continue at 3 lives is right. With score reset to 0, it may be
  too generous for completion and too harsh for score.
- **H4.** 27–36 s music loops over 60–70 s levels don't fatigue.
- **H5.** 15 s power-ups are the right length.
- **H6.** Asteroids add tension rather than noise.
- **H7.** The mix at −19.5 dBFS sits right against system audio.
- **H8.** Players notice the rally without being told. This is the one
  that decides idea 13.

**Playtest protocol** (S, run before Horizon 2 ships):
- **Players:** five, two of them new to the genre.
- **Session:** 30 minutes, Campaign from level 1, no instructions.
- **Record:** screen and audio.
- **Ask afterwards:**
  - "What made the sound change?"
  - "Which level do you remember?"
  - "When did you want to quit?"
- **Log:** levels reached, lives lost, Continue use.

Repeat with the same five players after Horizon 2.

---

## 9. Creative direction

### Statement

**BRYX is a brick-breaker you play like an instrument.** Every shot is a
phrase: the more bricks it takes before the ball comes home, the higher it
sings, the brighter it burns and the more it pays. The campaign is a
building taken apart in four movements, from the ground up to the vault.

### Principles

1. **One shot, one phrase.** The rally is the unit of play. Sound, light
   and score all read the same number.
2. **Every effect answers the player.** Nothing moves, flashes or plays
   unless the player or the beat caused it.
3. **Escalate one thing at a time.** Each act changes one rule or
   material, and the player can name it.
4. **Reward skill, never time.** No score accrues from waiting.
5. **Data before systems.** A new idea is a level, a palette row or a
   table entry before it is code.

### Pillars

1. **Musical play.** The rally, in key, scored.
2. **Legibility.** Flat quads, a calm field, color-blind marks; the ball
   always reads first.
3. **Four movements.** Acts as places, with an entrance and a finale.
4. **Fair and repeatable.** The same seed is the same game: replays,
   dailies, ghosts.

### Signature moments

1. **The capped rally.** The tenth brick lands on the tonic E. The field
   flashes in the act's accent, and the multiplier shows ×10.
2. **Getting upstairs.** The ball slips above the wall and runs the
   scale on its own. It already happens; scoring makes it the thing
   players chase.
3. **The act entrance.** "III · Machines" on the downbeat; the gear teeth
   draw in along the walls and start to tick.
4. **The vault opens.** The last breakable brick in Vault breaks, the X
   ring fractures, and the score's only major chord sounds.
5. **The run card.** Your best rally plays back as its notes, beside
   the seed that produced it.
6. **Last life.** The rally note drops an octave and the accent
   desaturates until the next clear.

### Visual direction

- The flat quad language stays; motion is the accent, not texture.
- Each act is one accent hue, one ground motif, one pulse and one way to
  break (section 4.3).
- Bloom and shockwave stay reserved for blasts and the capped rally.

### Audio direction

- In key, always: the scale is per track.
- Give every event the player must react to a sound: indestructible
  hit, gate, expiry.
- One stinger per act transition.
- No adaptive stems this cycle.

### Product direction

A small, finished, replayable game: one campaign, Classic, levelsets and
Shuffle, each with its own records, plus shareable seeds. Not a platform,
not an engine showcase.

---

## 10. Roadmap

### Horizon 1: Polish (fix what is verified)

| Initiative | Player value | Creative impact | Scope | Dependencies | Priority | Validation |
|---|---|---|---|---|---|---|
| Serve on a fresh press | Aimed first shot | Low | S | — | P0 | Held-Space test; replay hash unchanged |
| Rally in key, cap on the tonic | The signature sound resolves | High | S | — | P0 | Per-track scale test; listening pass |
| Human playtest and listening pass | Real difficulty and feel data | — | S | — | P0 | Protocol in section 8 |
| Unskippable act finales, X-column rule test, Fortress and Picture frame fixed | Fair finales, shorter hunts | Medium | S | — | P1 | Probe; new test |
| Comfort: fullscreen, focus pause, missing sounds | Fewer frustrations | Low | S | — | P1 | Manual checklist |

### Horizon 2: Differentiation (make BRYX itself)

| Initiative | Player value | Creative impact | Scope | Dependencies | Priority | Validation |
|---|---|---|---|---|---|---|
| **Rally is the score** (with asteroid and armored rebalance) | Skill pays | Defining | M | Rally in key | P0 | Headless tests; autopilot score spread; playtest H8 |
| Four movements (`ActStyle`, title cards, stingers) | Visible progress | Defining | M | — | P0 | Contrast and reduced-motion tests; playtest recall |
| Campaign re-curve inside acts, Vault as the finale, the vault opens | A shaped campaign with a climax | High | M | Four movements | P1 | Probe: rising clear time per act |
| Power-up legibility and per-act capsule weights | Plannable powers | Medium | S–M | — | P1 | Playtest |
| Run card and per-set records | A reason to replay | High | M | Rally score | P1 | Playtest "would you share?" |

### Horizon 3: Ambition (only after the H2 playtest)

| Initiative | Player value | Creative impact | Scope | Dependencies | Priority | Validation |
|---|---|---|---|---|---|---|
| Seeds and replay files | Fair, shareable runs | High | M | Run card | P1 | CI replays a recorded campaign |
| Daily seed and ghost | A daily return | High | L | Replays | P2 | Retention among testers |
| Attract mode from a bundled replay | The menu sells the game | Medium | S | Replays | P2 | Idle the menu |
| First-run lesson in level 1 | New players get the hook | Medium | S | Rally score | P2 | Playtest with new players |
| 3D as a cinematic lens (camera on act entrances and the vault) | A reason to pick 3D | Medium | M | Four movements; evidence anyone uses 3D | P2 | Playtest opt-in rate |

---

## 11. Verdict

1. **Strongest thing.** The rally. Each shot plays a pentatonic phrase,
   and light, blasts and level ends all keep the beat. It sits on a
   deterministic, testable simulation.
2. **Blocker.** That rally changes nothing in the rules, and the acts
   don't show on screen. So BRYX plays like a well-made generic
   brick-breaker with an exceptional soundtrack.
3. **Stop.**
   - Adding effects, shaders, renderer parity work and systems.
   - Treating autopilot benchmarks as difficulty data.
   - Shipping changes to feel that no human has played.
4. **Start.**
   - Playtesting with people.
   - Designing in data: the act table, capsule weights, the order
     inside each act.
   - Scoring the thing that makes BRYX different.
5. **Single next initiative.** **Rally is the score**, shipped with its two
   S-sized preconditions: rally in key, and serve on a fresh press.
   - It turns the existing audiovisual investment into gameplay.
   - It removes the reward for stalling.
   - It is a contained change in `game.rs` and the HUD, testable
     headless.
   - It is the first thing the playtest should measure.
6. **Identity in one sentence.** *BRYX is a brick-breaker where every
   shot is a melody, and the better you aim, the higher it sings.*
