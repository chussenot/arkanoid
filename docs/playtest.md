# Playtesting BRYX

A protocol for the human playtest and listening pass the
[Game Director review](design/game-director-review.md) asks for (section 8),
and the tools that make it measurable. No autopilot benchmark answers these
questions: it plays too well, and it hears nothing.

## What the sessions decide

| Hypothesis | Evidence |
|---|---|
| **H8** Players notice the rally (the climbing notes) without being told | Interview: "What made the sound change?" |
| Rally scoring (`--rally-score`) should become the default | Round B against round A: interview answer, longest rally, score spread |
| **H1** 560 px/s is right for level 1 | Lives lost on levels 1–3; observer notes |
| **H2** Tiny frustrates more than it helps | Observer notes on catches and dodges; `T` in `capsule_letters` |
| **H3** Continue with 3 lives is right | Runs per player; where game overs fall |
| **H4** The 27–36 s music loops do not tire | Interview; listening pass |
| **H5** 15 s power-ups are the right length | Observer notes on the expiry warning |
| **H6** Asteroids add tension, not noise | Observer notes |
| **H7** The mix sits right | Listening pass |
| The four acts read as places | Interview: "Which level do you remember?" |
| Levels that drag (the campaign re-curve) | `levels that drag` in the summary |

## Setup

Build once, release mode:

```bash
cargo build --release
```

Give each player their own records and log, so no one starts with another's
bests to beat:

```bash
export BRYX_DATA_DIR=playtest/p1          # this player's records
target/release/bryx --playtest-log playtest/p1-a.csv                 # round A
target/release/bryx --playtest-log playtest/p1-b.csv --rally-score   # round B
```

- `--playtest-log FILE` appends one row per level played; a new file
  starts with its header. Several sessions can share a file: each has its
  own `session` number.
- Nothing is sent anywhere. The log and the records stay on the machine.
- Sound on, headphones or speakers the player chose; fullscreen with F11.

## Protocol

- **Players:** five, two of them new to the genre.
- **Round A (baseline):** 30 minutes, Campaign from level 1, no
  instructions beyond "the arrow keys move, Space serves". The facilitator
  watches, takes notes and does not help.
- **Round B (`--rally-score`):** the same players, ideally another day.
  Players are better the second time whatever the rules, so compare their
  rallies and answers between rounds rather than their raw scores.
- **Record** the screen and the sound, with the player's consent.

### What the facilitator notes

- The first moment the player reacts to the rally's notes, if they do.
- Any frustration: where, and what caused it (a hunt, Tiny, an asteroid).
- Whether they notice an act's title, its floor or its fanfare.
- Whether they see the expiry warning before a power runs out.
- When they would have stopped, if they say so.

### Interview, after each round

1. What made the sound change while you played?
2. Which level do you remember, and why?
3. When did you want to quit?
4. Round B only: did the ×N beside the score change how you aimed?
5. Round B only: did the score feel fairer or less fair than in round A?

## Listening pass

On the speakers or headphones the game will usually be played on:

- [ ] The mix: effects sit over the music, nothing clips, the level is
      comfortable for 30 minutes.
- [ ] A long rally on level 16, 19, 24, 29 or 32 resolves, never clashes
      (#26).
- [ ] `steel` (indestructible bricks) is audible but not tiring on Fortress
      (level 12).
- [ ] `gate` (Break) and `expiring` (a power's last 2 s) read at once.
- [ ] `act_clear` (levels 8, 16, 24) feels like the end of a chapter.
- [ ] The level loops do not tire over a full level.

## Reading the log

```bash
scripts/playtest-summary.py playtest/*.csv
```

It groups rows by condition (human or autopilot, rally scoring off or on)
and prints, for each: runs and how they ended, then per level the plays,
median seconds, median hunt and its share of the level, lives lost per play
and median longest rally. Last come the **levels that drag**: a hunt over a
quarter of the level, or a median over twice the condition's median level.

| Column | Meaning |
|---|---|
| `session`, `run` | A session (seconds since the epoch at start), and the run within it; a Continue or Play again is a new run |
| `player` | `human`, or `autopilot` under `--autopilot` |
| `mode` | `campaign`, `classic`, `shuffle`, or `levelset:` and its name |
| `rally_score` | Whether `--rally-score` was on |
| `level`, `name` | The level, as the HUD names it |
| `outcome` | `cleared`, `warped` (Break gate), `victory`, `game_over`, or `abandoned` (Q or closing the window) |
| `secs` | Seconds in play, pauses excluded |
| `hunt_secs` | Seconds from the last tenth of its breakable bricks to its end |
| `lives_lost` | Lives lost on the level |
| `longest_rally` | Most bricks broken between two paddle touches |
| `bricks`, `capsules` | Bricks broken; capsules caught |
| `capsule_letters` | The capsules caught, in order, by their letters (W, S, M, G, L, X, T, P, B, E, I) |
| `score` | Points scored on the level |

## Decisions to take afterwards

- **Rally scoring:** make it the default if most players say the ×N changed
  how they aimed and their longest rallies rise in round B. Otherwise keep
  the flag and revisit the multiplier (the laser question in #29).
- **Campaign re-curve:** reorder within acts, and resize or reshape the
  levels the summary flags, starting with those flagged in both rounds.
- **First-run lesson:** build it if few players name the rally unprompted
  (H8).
