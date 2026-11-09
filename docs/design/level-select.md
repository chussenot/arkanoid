# Choosing what to play

Until now the levels a run played were picked on the command line:
- the campaign by default;
- v1's three levels with `--classic`;
- a fetched LBreakoutHD set with `--levelset <path>`.

A player who launched the game could not reach Classic or a levelset
without quitting and relaunching with a flag they had to read about first.
This page records how the menu now offers every mode. It was worked in three
passes: game UX, UI design, engineering.

## 1. Game UX

### What the player is choosing

| Mode | What it is | Who wants it |
|---|---|---|
| Campaign | 32 named levels in four acts, every addition on | Everyone; the default |
| Classic | v1's three levels under v1's rules | The game as first specified; a quick, pure run |
| Levelset | One of the 135 fetched LBreakoutHD sets | Players who want variety or a known set |
| Shuffle | Ten levels drawn from every fetched set | A short session that is different every time |

Shuffle is new. The README already promised a RANDOM10 mode, and
`levelset::random10` existed, but nothing ever offered it. Shuffle draws
across all sets rather than inside one, so it works without first picking
a set. That one keystroke makes it the quickest way into the corpus.

### Decisions

- **The menu is the picker. There is no separate screen.**
  - A second screen costs a keypress on every launch and hides the choice
    behind a label.
  - The menu already exists, and it is where every run starts.
  - The default highlight is Campaign, so Space starts exactly what it
    always did.
- **A vertical list, one mode per row, rather than tabs or a carousel.**
  - A list is the menu pattern every player already reads: ↑/↓ moves, the
    highlighted row is what Space plays.
  - It shows every option at once, with a one-line reason to pick each.
  - Tabs would hide the descriptions. A carousel hides the alternatives.
- **The levelset is a value on its row, changed with ←/→.**
  - 135 sets do not fit as rows, and a nested list would be a second screen
    again.
  - "Row with a value you cycle" is the standard console options pattern.
  - A position counter (`004/135`) says how far the list goes.
  - The OS's key repeat scrolls when an arrow is held.
- **A mode with nothing to play is still listed.**
  - Without fetched sets, the Levelset row stays, dimmed, and says how to
    get sets: `none found: run scripts/fetch-levelsets.sh`.
  - ↑/↓ step over it.
  - Hiding it would hide the feature from exactly the player who has not
    found it yet.
  - Shuffle is hidden in that case: it would only repeat the same hint.
- **Each mode shows its own best score**, on its row.
  - Before, the menu showed only the best of the mode the flags chose.
  - Loaded sets keep no record (`record.rs`), so they show their position
    instead.
- **Q returns to the menu** from the pause, Game Over and Victory screens.
  - Without a way back, picking in the menu would still mean relaunching to
    switch modes.
  - Q ("quit to menu") is free: Escape and P already pause and resume, and
    M is the sound.
  - It is never live during play, where a stray press would end a run.
  - The picker remembers its row and set across the trip.
- **The flags still work, and now preselect.** `--classic` highlights
  Classic, and `--levelset <file>` highlights that set. Launching with a
  flag and pressing Space plays exactly what it did before.

## 2. UI design

The menu follows DESIGN.md:
- flat chrome;
- the slate;
- drawn keycaps;
- JetBrains Mono;
- no meaning on hue alone.

```
                         B R Y X

   ┃ CAMPAIGN     32 named levels in four acts          BEST 012340
     CLASSIC      3 levels, the original rules           BEST 003400
     LEVELSET     ‹ Africa ›  58 levels                      004/135
     SHUFFLE      10 levels drawn from all 135 sets

           [↑][↓] Mode    [←][→] Set    [SPACE] Play
   ────────────────────────────────────────────────────────────
                  [←][→]  [A][D]   Move
                         [SPACE]   Launch, release, fire
                       [ESC]  [P]   Pause
```

- **Rows.**
  - 560 px wide and 40 px tall, 6 px apart, centered.
  - Three columns:
    - the mode name in Summary bold, tracked 0.08em;
    - the detail in Note, dim;
    - the aside, right-aligned, in Note bold, tracked like a caption.
  - The detail column starts 168 px in, past the longest name.
- **Highlight.**
  - The highlighted row is filled with the band's ink and outlined in Rule
    Strong, with a 3 px bar in Text on its left edge.
  - So fill, outline and bar all carry the selection, never color alone
    (PRODUCT.md, color-blind safety).
  - Other rows have no box: the list stays quiet and the highlight is the
    only shape in it.
- **The value.**
  - The set name is drawn in bold between chevrons, `‹ Africa ›`.
  - The chevrons brighten on the highlighted row, where ←/→ act.
  - Names longer than 14 characters are cut with an ellipsis.
  - The position counter reuses the level readout's style: bright digits,
    dim total.
- **Navigation line.**
  - The picker's keys sit on one centered line under the list, read as a
    sentence of verbs: Mode, Set, Play.
  - "Set" appears only when a set can be picked.
  - A hairline in Rule separates the picker from the game's controls table,
    which stays because PRODUCT.md requires the controls on screen.
- **No "PRESS SPACE TO START".** The `[SPACE] Play` keycap in the
  navigation line says it, in the same drawn-key language as everything
  else.
- **Q** appears as a drawn key, "Main menu", as the last row of the pause
  screen's controls table and of the end screens' key rows.

Both renderers draw it, because it is laid out by `hud::layout`. The
off-canvas test covers it with the longest values every column can hold.

## 3. Engineering

- **`src/menu.rs` is the picker. It is pure.**
  - It holds the modes, the highlighted row and the levelset list, and turns
    key presses (`Nav`) into choices.
  - `Menu::build` returns a fresh `Game` of the highlighted mode, waiting on
    its menu screen.
  - `Menu::view` returns plain data (`hud::MenuView`) for the HUD to draw.
  - Its tests cover navigation, wrapping, the disabled row, set cycling,
    each mode's game, and Shuffle drawing from every set.
- **The game behind the menu is rebuilt on every change.**
  - `main` swaps in `menu.build()` whenever the highlight or the set moves.
  - So the Space that starts a run goes through `Game::tick`'s existing
    `Menu -> Playing` transition, untouched.
  - The band, the record and the music always describe the highlighted mode.
  - `game.rs` does not change: the simulation never learns there is a
    picker.
  - Both renderers skip their brick diff on the menu, so a swap is never
    drawn as bricks breaking.
- **Interface keys are not simulation input.** ↑/↓, ←/→ on the menu, and Q
  are read in `main.rs` from key presses, repeats included, like M for
  sound. `game::Input` is unchanged, and the pinned replay hash with it.
- **Levelsets are found by content, not name.**
  - `levelset::load_dir` used to keep only `*.lbl` files. But the fetch
    script leaves LBreakoutHD's sets bare (`levels/NewBricks`), so
    `--levelset levels/` found nothing.
  - It now reads every visible file and keeps those that hold a level. That
    also catches the dotted names (`P.I.T.A.`, `T.C.S.`) a name rule
    misses, and leaves out upstream's `Makefile.am`.
  - Reading all 135 sets (2.2 MB) at start takes a few tens of
    milliseconds: the menu is up 42 ms after launch, process start
    included.
- **Sound.** Leaving a run for the menu cuts a held stinger, plays the
  select blip, and lifts the pause's dimming for the menu music.

## Follow-ups not taken

- **Remember the last mode across launches.** It would be a small file
  beside the records, but it is a preference the flags already cover for
  people who always play one mode.
- **Per-set best scores.** `record.rs` keeps no record for loaded sets,
  because a path is a poor key. Set names are a better one now, but that is
  a records change of its own.
- **Type-to-jump in the set list.** At 135 sets, held arrows take about four
  seconds end to end. Revisit if the corpus grows.
