# BRYX director review

A review of the whole game experience by discipline:
1. **Lead Game Designer**: systems, mechanics, progression, balance.
2. **Creative Director**: identity, tone, coherence.
3. **Art Director**: visual style and quality.
4. **UX/UI Lead**: interaction, interfaces, accessibility.

Each discipline makes recommendations; the **Game Director** decides, and
gameplay (A) and engine/graphics (B) engineers implement. The decisions
that change rules are recorded in [`docs/spec.md`](../spec.md); the visual
ones in [`DESIGN.md`](../../DESIGN.md).

## Evidence base

- **Code read**: `src/game.rs` (rules and constants), `src/levels.rs` (32
  campaign grids), `src/hud.rs` and both renderers.
- **Level census**: bricks, armored, indestructible and hit points per
  campaign level, plus brick score per level from `score_for_row`.
- **Pacing probe**: the heuristic autopilot played the full campaign on 6
  seeds. Per level it recorded:
  - seconds to clear;
  - seconds spent on the last 10% of breakable bricks (the "last-brick
    hunt");
  - seconds with the ball at the speed cap;
  - lives lost.

  The probe is a scratch test, not committed; its code is in this
  session's history.
- **Limits of the evidence**: the autopilot is a much steadier player than a
  person, so its times are a floor and its lives lost understate
  difficulty. It is good at finding where *time* goes; it cannot say what
  feels hard or fun. Every balance number below wants a human playtest
  before it is called final (see "Open questions").

## 1. Lead Game Designer

**Core loop.** Serve, rally, break, catch, clear. It is solid:
- the exit-angle mapping gives the paddle real aim;
- 4% speed growth per hit keeps rallies escalating;
- the stall nudge stops infinite orbits;
- nine capsules cover the classic verbs (size, speed, split, hold, shoot,
  life, skip) plus one deliberate negative (Tiny).

**Findings, by severity:**

| # | Finding | Evidence |
|---|---|---|
| G1 | **Two levels break the campaign's pacing.** L16 Bridges and L27 Baffles seal rows behind indestructible walls with narrow, offset gaps, so the last bricks need the ball to thread two gaps in a row. | Probe: Bridges 723 s, of which 381 s is the last 10%; Baffles 841 s. A typical level takes 40–90 s. These two are **57% of all campaign time**, nearly all at max ball speed. |
| G2 | **No difficulty curve.** Launch speed, speed cap, growth and drop rate are identical on levels 1 and 32, and layout difficulty is not ordered (hit points: L4 64, L5 24, L30 120, L31 39). | Level census; `game.rs` constants. |
| G3 | **A run is all-or-nothing.** The campaign takes about an hour (autopilot: 46 min with no losses), and a Game Over leaves no way back at all: the window must be closed and the game relaunched from level 1. | `tick`'s GameOver/Victory arms did nothing. |
| G4 | **Break is a degenerate score strategy.** Leaving through the gate paid +10,000, while a level's bricks are worth 720–3,780 (1,966 on average). One Break catch was worth about five cleared levels. | Brick-score census. |
| G5 | Capsule economy: 15% drop rate, nine equal odds, one of them negative (Tiny). It reads fine, but it is untested on people. | Constants. |

**Recommendations:**
- Reshape the two outlier levels (G1).
- Ramp the ball speed cap through the campaign (G2).
- Add checkpoints and a way to continue (G3).
- Pay Break out at the bricks it skips (G4).
- Leave the capsule odds alone until there is playtest data (G5).

## 2. Creative Director

**Identity.** "The Measured Field" (DESIGN.md) holds:
- technical, precise, playful without being childish;
- clearly independent of any arcade franchise;
- coherent across both renderers.

**Findings:**
- C1. The campaign has names but no **shape**. Thirty-two levels in a flat
  list give the player no milestones and no sense of a journey's stages.
- C2. Ending a run is the weakest moment in the experience. GameOver was a
  dead end with "Close the window to quit"; Victory, the one triumphant
  beat, offered nothing to do next.
- C3. There is no sound at all. For an arcade game this is the largest
  remaining gap in feel. It is a deliberate v1 omission (`events.rs` exists
  for a future audio layer), and the dependency budget is fixed.

**Recommendations:**
- Give the campaign four **acts** of eight levels, used as checkpoints (C1).
- Make every end screen a door, not a wall (C2).
- Plan audio as the next major investment, with a written dependency
  justification (C3).

## 3. Art Director

**State.** The visual overhaul (#9, #10) brought the game to a consistent,
legible standard:
- one palette;
- color-blind-safe marks;
- the HUD band;
- matching 2D and 3D lenses.

**Findings:**
- A1. Normal bricks are one vermilion on every level. This is correct for
  meaning, but across 32 levels the field looks the same from act to act.
- A2. 2D destruction is a one-frame white flash; 3D has tumbling ghosts.
  2D could use a short shard burst in the brick's hue, still event-bound
  and brief.
- A3. 3D lighting is a single key light; the scene reads flat beside the
  2D sprites' bevels. A rim or fill light would add depth without new
  assets.

**Recommendations:**
- A1 and A3 are polish for a later pass. A1 must not touch hue semantics:
  a per-act field-ground tone or rule accent would respect them.
- A2 is a good next task for an engine/graphics engineer.
- None blocks this round.

## 4. UX/UI Lead

**Findings:**
- U1. **The end screens were a dead end** (see G3/C2): no key did
  anything.
- U2. **Serving has no prompt.** A new player sees the ball sitting on the
  paddle with nothing saying Space launches it. The menu said so once,
  before play.
- U3. **No record.** Nothing persists between sessions, so a run has
  nothing to beat.
- U4. Accessibility:
  - color-blind safety, reduced motion (shake) and a keyboard-only reach
    are in place;
  - key remapping and adjustable game speed are not;
  - for players who find 700 px/s too fast, G2's ramp is the first real
    help.

**Recommendations:**
- End screens with explicit keys: Continue from the checkpoint, New game,
  Play again (U1).
- A serve prompt showing the level's number, its name and the launch key
  (U2).
- A local best score, shown on the menu and on the end screens (U3).
- Key remapping, as a later pass (U4).

## Game Director: decisions

The test for this round: what most improves a *real player's* hour with
BRYX, without touching what makes the code worth reading?
- v1's rules stay frozen, and the pinned replay hash still passes.
- No new dependencies.

### Ship now

| # | Decision | Answers | Owner |
|---|---|---|---|
| D1 | **Reshape Bridges and Baffles** (and any other level above ~120 s) so their last bricks are reachable without threading two offset gaps. Each keeps its name and idea, and stays demanding. | G1 | Gameplay engineer (A) |
| D2 | **Campaign speed ramp**: the ball's speed cap rises from 560 px/s on level 1 to 700 px/s on level 32. `--classic` and levelsets keep the flat 700. | G2, U4 | Director, with A |
| D3 | **Four acts, with checkpoints**: on a Game Over, Space continues from the first level of the current act (1, 9, 17, 25) with full lives and the score reset; Escape/P starts a new game. Victory: Space plays again. | G3, C1, C2, U1 | A |
| D4 | **Break pays out the bricks it skips**, not a flat +10,000. | G4 | A |
| D5 | **Best score**: kept per progression in a small local file (`src/record.rs`), shown on the menu and end screens, and marked NEW BEST in the reward gold. Autopilot runs never set it. | U3 | A (I/O stays outside `game.rs`) |
| D6 | **Serve prompt**: while the ball waits on the paddle, the field shows the level's number and name and a drawn Space key with "Launch". Hidden under the autopilot. | U2 | UX, via `hud.rs` |

**Why the score resets on Continue**: a best score must mean one unbroken
run. Continuing is for seeing the rest of the campaign, not for building a
score from many attempts.

### Next

- **Audio** (C3). This is the highest-value gap left. It needs a crate (a
  minimal `cpal` synthesizer, or `rodio`) and so a written justification
  against the fixed dependency budget. The `GameEvent` stream is already
  the hook.
- **2D destruction shards** (A2) and **3D fill light** (A3), for the
  engine/graphics engineer (B).
- **Per-act visual accent** (A1), done without touching hue semantics.
- **Key remapping** (U4).

### Not doing

- **Reordering the campaign by difficulty.** The speed ramp gives the
  progression its slope without discarding the levels' authored order; a
  reorder can follow a human playtest.
- **Changing capsule odds** (G5) before there is data from people.

## Open questions (need a human playtest)

1. Is 560 px/s right for level 1, or does a new player want lower still?
2. Does Continue at full lives make the last act too forgiving?
3. Is Tiny (1 in 9 drops) more frustrating than interesting?

## Results

Measured on the same probe (heuristic autopilot, 6 seeds, whole
campaign), before this round and after D1 and D2 together:

| Measure | Before | After |
|---|---|---|
| Campaign time, sum of per-level averages | 3,643 s (60.7 min) | **2,389 s (39.8 min)** |
| Slowest level ÷ median level | 13× (Baffles 841 s) | **2.6×** (Fortress 171 s) |
| Bridges (L16): clear / last 10% | 723 s / 381 s | **76 s / 11 s** |
| Baffles (L27): clear / last 10% | 841 s / 225 s | **145 s / 51 s** |
| Lattice (L13): clear / last 10% | 166 s / 44 s | **110 s / 24 s** |
| Seeds won | 6/6 | 6/6 |

The committed 10-seed benchmark (`benchmark_campaign`) agrees: average win
3,614 s → **2,362 s**, longest 4,733 s → **2,662 s**, all 10 won.

Notes on D1 and D2:
- **D1 levels.** The gameplay engineer reshaped Bridges into a viaduct (two
  decks over indestructible piers) and moved Baffles' plates below the
  brick rows, as staggered deflectors with straight lanes through them.
  They also opened Lattice's outer rows. The rule they found: *no
  breakable brick may sit between two indestructible bricks in the same
  column*; such bricks were always the last 10%. It is worth adding to
  `levels.rs`'s tests if new levels keep tripping it.
- **D2 changed where the time goes.** With the cap below 700 px/s until
  level 30, the ball no longer spends minutes at top speed, but slower
  balls make the remaining sealed pockets (Fortress, Baffles' tail) a
  little longer to finish. Both are now 2–3× a typical level, down from
  13×. They are on the watch list for the human playtest, not worth
  another change without one.
- **v1 untouched.** `--classic` benchmark numbers are identical to before,
  and the pinned replay hash passes.

Verification:
- 426 tests pass by default and 432 with `typesafe`; there are new tests
  for the speed ramp, checkpoints, Continue/New game/Play again, Break's
  payout, the record file, the serve prompt and the end-screen keys.
- Clippy is clean on Rust 1.97 and 1.99 (CI's).
- Screenshots of the new screens in both renderers are in
  `docs/design/director-round.png`; `docs/campaign.png` is regenerated from
  the current build.

### Still open

- Fortress (L12): 171 s, last 10% 33 s. Same boxed-pocket pattern; the
  first candidate if playtests agree.
- Audio, the next investment (see "Next").
- A human playtest against the three open questions above.
