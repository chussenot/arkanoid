# LBreakoutHD levelset format (supported subset)

Researched directly from the real parser, not from memory or secondary
write-ups. Source: [github.com/midzer/lbreakouthd](https://github.com/midzer/lbreakouthd)
(the maintained fork/HD remake of LGames' LBreakout2 — same on-disk levelset
format; LBreakout2's own upstream at lgames.sourceforge.net ships the
identical parser lineage), commit `3cd2a6160941557f48c49b184f0ad47ddd882c23`
(`configure.ac` reports version `1.1.8`), license GPLv3 (`COPYING`). Key
files read:

- `libgame/levels.c` — file/level parsing (`levels_load`, `level_load`,
  `levelset_get_version`).
- `libgame/gamedefs.h` — `Level`/`LevelSet` structs, `EDIT_WIDTH`/`EDIT_HEIGHT`.
- `libgame/bricks.c` — `brick_conv_table`, `extra_conv_table`, and the
  `bricks_init()` loop that turns a `Level`'s char grids into game bricks
  (this is the "brick creation from char" step the `Level` struct's own
  comment refers to).
- `libgame/tools.c` — `parse_version()`.
- Two shipped levelsets read in full: `src/levels/Arkanoid` (8 levels,
  version `1.00`, author "Lelldorin") and `src/levels/Classique` (5 levels,
  version `1.45`, author "Bertrand GRONDIN"). Both parse cleanly under the
  rules below; the brick chars they actually use are
  `. a b c d e E f g h i j k v x z` and the bonus chars are
  `. 0 1 2 3 4 5 b c d f g j l m p s w W + - < > ~`.

## File layout

```
Version: <major>.<minor>      (optional, only recognized as the first line)
Level:
<author line>
<name line>
Bricks:
<18 lines of 14 characters>
Bonus:
<18 lines of 14 characters>
Level:
...repeat per level, until EOF...
```

- **Header**: `levelset_get_version()` peeks the first line; if it starts
  with `Version:` it's parsed as `<major>.<minor>` (`parse_version`, e.g.
  `1.45` → version 1, update 45; `1.4` → update 40, i.e. a single digit
  after the dot is treated as tenths) and consumed. If the first line is
  anything else, the file pointer is rewound to byte 0 and the whole file
  is read as `Level:` blocks starting from line 1 — **a missing header is
  valid**, not an error, and defaults to version 1.00.
- **Multi-level files**: there is no level count field anywhere in the
  file or the `Level`/`LevelSet` structs. `levels_load()` just calls
  `level_load()` in a loop and appends to a list until it returns `NULL`
  (EOF, or a line that fails to match `Level:` / `Bricks:` / `Bonus:`).
  A file is simply as many back-to-back blocks as it contains; `Arkanoid`
  has 8, `Classique` has 5. (`MAX_LEVELS = 40` exists in `gamedefs.h` but
  is never referenced by the loader — it is not a real cap, do not treat
  it as one.)
- **Per-level header**: `Level:` (matched by `strncmp` on the first 6
  bytes — trailing garbage on that line is ignored), then an author line
  and a name line, each read verbatim (`\n`/trailing `\r` stripped by
  `next_line()`, everything else kept as-is including empty strings) and
  truncated to 31 bytes.
- **Grid sections**: `Bricks:` then exactly `EDIT_HEIGHT` (18) lines, each
  read with `fgets` and required to be **at least** `EDIT_WIDTH` (14)
  bytes after CR/LF stripping — only the first 14 bytes of each line are
  used, so a longer line doesn't error but its extra bytes are silently
  ignored. Same shape again after a `Bonus:` line for the second
  ("extras") grid. Grid columns are addressed `[x][y]`, `x` in `0..14`,
  `y` in `0..18` — i.e. **14 columns × 18 rows**, not 18×14.
- Both `Bricks:` and `Bonus:` labels are matched with `strncmp` against a
  fixed prefix the same way `Level:` is.
- At runtime these 14×18 grids are copied into the actual play map
  (`MAP_WIDTH=16 × MAP_HEIGHT=24`) offset by a permanent 1-tile
  indestructible border wall on every side — that border is not part of
  the levelset file at all, engine-added at load time. Not something a
  levelset reader needs to reproduce structurally, just don't be surprised
  the two grid sizes differ from the play-field size.

## Brick grid (`Bricks:`) — 26 characters, from `brick_conv_table`

Any character not in this table, and not `.` or space, triggers the
engine's own `printf("unknown: %i,%i: %c\n", ...)` and the tile is left
however it was already initialized: **empty** (`bricks_init` clears the
whole map to `MAP_EMPTY` before this loop runs). `.` and space are the
two designated empty chars and never warn.

| char | brick type | durability | notes |
|---|---|---|---|
| `.` | empty | — | designated empty tile, never warns |
| ` ` (space) | empty | — | also treated as empty, never warns |
| `E` | wall | indestructible | `MAP_WALL`, score 0 |
| `#` | brick | **-1 = energy-ball only** | `MAP_BRICK`, score 1000; normal hits do nothing |
| `@` | brick | **-1 = energy-ball only** | `MAP_BRICK_CHAOS` — chaotic reflection on hit, score 1000 |
| `a` | brick | 1 hit | score ×1 |
| `b` | brick | 2 hits | score ×2 |
| `c` | brick | 3 hits | score ×3 |
| `v` | brick | 4 hits | score ×4 |
| `x` | regenerating brick | 1 hit, heals | `MAP_BRICK_HEAL`, score ×2 |
| `y` | regenerating brick | 2 hits, heals | `MAP_BRICK_HEAL`, score ×4 |
| `z` | regenerating brick | 3 hits, heals | `MAP_BRICK_HEAL`, score ×6 |
| `d`,`e`,`f`,`g`,`h`,`i`,`j`,`k` | brick | 1 hit each | plain color variants, same score; `f`..`k` are also the base ids that "grown" bricks (below) reuse |
| `*` | explosive brick | 1 hit | `MAP_BRICK_EXP` — destroys neighbors, score ×2 |
| `!` | grow brick | 1 hit | `MAP_BRICK_GROW` — on destruction, spawns a random brick in a neighboring empty tile, score ×2 |
| `F`,`G`,`H`,`I`,`J`,`K` | "grown" brick | same as `f`..`k` respectively | **not meant to appear in hand-authored level files** — uppercase marks a brick that was created at runtime by a `!` grow-brick, tracked separately only so the warp-limit percentage doesn't count it; visually and mechanically identical to its lowercase counterpart. A loader can treat `F..K` as aliases of `f..k` rather than rejecting them, since they round-trip through level *snapshots* (mid-game saves), not through hand-authored levelset files. |

`is_destructible()` (used only for the file's `normal_brick_count` stat,
not for rendering) additionally special-cases: `a`-`k`, `x`-`z`, `v`, `*`,
`!` count as destructible; `E`, `#`, `@` do not (consistent with the table
above — `#`/`@` need an energy ball, which the engine doesn't count as
"normal" destruction).

## Bonus/extras grid (`Bonus:`) — 29 characters, from `extra_conv_table`

This grid is parallel to the brick grid: same 14×18 shape, one line of
14 chars per row, and cell `[x][y]` here is the power-up dropped when the
brick at the *same* `[x][y]` in the brick grid is destroyed (a bonus char
over an empty brick cell is simply never triggered).

**Important divergence from the brick grid**: the real engine does *not*
warn on an unrecognized extras char — the lookup loop (`bricks.c` ~line
752) just falls through with no match and no `printf`, leaving the cell's
extra silently at `EX_NONE`. Any char here that isn't in the table below,
including `.`, ends up with no power-up, with zero diagnostic output from
upstream. **Our own loader must not copy that silence**: per this bead's
acceptance criteria, treat any bonus char outside the table the same way
as an unsupported brick char — warn, then map to "no power-up" — rather
than reproducing the original engine's silent fallthrough.

| char | extra | category |
|---|---|---|
| `.` | none | designated empty, never warns |
| `0` | +200 score | bonus |
| `1` | +500 score | bonus |
| `2` | +1000 score | bonus |
| `3` | +2000 score | bonus |
| `4` | +5000 score | bonus |
| `5` | +10000 score | bonus |
| `g` | goldshower | bonus |
| `-` | shorten paddle | malus |
| `+` | lengthen (widen) paddle | bonus |
| `l` | extra life | bonus |
| `s` | slime paddle (catches ball) | bonus |
| `m` | metal ball | bonus |
| `b` | extra ball (multiball) | bonus |
| `w` | protective wall | bonus |
| `f` | frozen paddle | malus |
| `p` | weapon (paddle can shoot) | bonus |
| `?` | random extra | either |
| `>` | fast ball | malus |
| `<` | slow ball | bonus |
| `j` | joker | bonus |
| `d` | darkness | malus |
| `c` | chaos (erratic paddle/ball behavior) | malus |
| `~` | ghost paddle | malus |
| `!` | disable (paddle input disabled) | malus |
| `&` | add time (bonus-level timer) | bonus |
| `*` | explosive ball | bonus |
| `}` | bonus magnet | bonus |
| `{` | malus magnet | malus |
| `W` | weak ball | malus |

Note the char sets for the two grids overlap (`+`, `-`, `<`, `>`, `*`,
`!`, `b`, `f` etc. mean completely different things depending on which
grid they're read from) — a loader must never share one lookup table
between the brick pass and the bonus pass.

## What's explicitly out of scope here

- **Virtual/bonus levelset names** (`fname[0] == '!'`, e.g. `!JUMPING_JACK!`)
  are not files at all — `levelset_load()` special-cases them to build a
  single procedurally-typed bonus level (Jumping Jack / Outbreak / Barrier
  / Sitting Ducks / Hunter / Invaders) with no on-disk grid to parse. Not
  a levelset **format** concern; a loader only needs to recognize these
  names are not real paths, not parse their (nonexistent) content.
- Windows `\r\n` line endings are already handled by `next_line()`
  stripping a trailing `\r` after the `\n` — not a format variant to
  special-case, just confirms `\r`-terminated lines round-trip.
- Brick/extra numeric "id" (sprite index) and exact per-hit score value
  in the tables above are the original engine's own internals — a Rust
  loader mapping this onto this game's own `BrickKind`/`PowerUpKind`
  (bead `arkanoid-v2-a2`) only needs the *categories* in these tables
  (durability, regenerating/explosive/grow behavior, bonus-vs-malus), not
  the original sprite ids or exact score multipliers.
