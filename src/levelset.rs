//! Loader for external LBreakoutHD-style `.lbl` levelset files (format
//! documented in `docs/levelset-format.md`; source: midzer/lbreakouthd
//! commit `3cd2a6160941557f48c49b184f0ad47ddd882c23`, GPLv3).
//!
//! Two layers: [`parse_levelset`] is pure (no I/O, no panics) and does the
//! actual grid-to-`BrickKind`/`PowerUpKind` mapping, so it can be unit
//! tested with inline strings; [`load_file`]/[`load_dir`] are thin
//! `std::fs` wrappers a runtime caller uses to read real files out of a
//! `levels/` directory (populated by `scripts/fetch-levelsets.sh`,
//! arkanoid-v2-a3 -- that directory is gitignored, nothing under it ships
//! in this repo).
//!
//! Mapping policy (per this bead's acceptance criteria): our own
//! `BrickKind`/`PowerUpKind` have far fewer distinctions than LBreakoutHD's
//! 26 brick chars / 29 bonus chars, so this is a many-to-few, lossy
//! mapping -- normal-tier bricks collapse to `Normal`, every multi-hit or
//! regenerating brick collapses to `Armored` (always 2 hits in our
//! engine), walls and energy-ball-only bricks collapse to `Indestructible`
//! ("silver", per docs/spec.md). Only the 3 bonus chars whose semantics
//! actually match one of our exactly-3 power-ups map to one; every other
//! recognized bonus char, and any character outside either documented
//! table, degrades to "no drop" -- but unlike upstream (which silently
//! falls through on unrecognized bonus chars), every such cell pushes a
//! `source:line: message` warning onto the returned `LevelSet`, satisfying
//! "warn with file:line, never panic or silently misrender".
//!
//! Wired into `main.rs` via `mod levelset;` and `--levelset <path>`
//! (arkanoid-v2-a4), which only reads `LevelSet.{levels,warnings}` to log
//! what loaded -- it doesn't yet feed a loaded set into `Game`'s level
//! progression (arkanoid-v2-a5's job, since that touches `game.rs`).
//! Until then `ExternalLevel`'s/`ExternalBrickSpawn`'s individual fields
//! are read only by this module's own tests, hence the module-wide
//! `#[allow(dead_code)]` still standing.
#![allow(dead_code)]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::events::PowerUpKind;
use crate::levels::{BrickKind, BRICK_HEIGHT, BRICK_WIDTH};

/// Levelset grid shape (docs/levelset-format.md: `EDIT_WIDTH`/`EDIT_HEIGHT`,
/// 14 columns x 18 rows, addressed `[x][y]`).
pub const GRID_WIDTH: usize = 14;
pub const GRID_HEIGHT: usize = 18;

/// One brick to spawn, in the same grid-local-pixel convention as
/// `levels::BrickSpawn`, plus the power-up (if any) the bonus grid pairs
/// with this cell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExternalBrickSpawn {
    pub x: f32,
    pub y: f32,
    pub kind: BrickKind,
    pub powerup: Option<PowerUpKind>,
}

/// One parsed `Level:` block.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalLevel {
    pub author: String,
    pub name: String,
    pub bricks: Vec<ExternalBrickSpawn>,
}

/// Everything parsed from one levelset file, plus every load-time warning
/// (unsupported chars, recognized-but-unmapped bonus chars) generated
/// along the way. Never an `Err` for content problems -- a malformed grid
/// just stops the file at the point that goes wrong (mirroring upstream's
/// own "loop until a block fails to match, treat that like EOF" loader),
/// so callers always get whatever levels parsed cleanly plus a full
/// warning trail, never a panic.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LevelSet {
    pub levels: Vec<ExternalLevel>,
    pub warnings: Vec<String>,
}

/// Outcome of looking up one brick-grid character (`brick_conv_table`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrickCell {
    /// `.` or space -- designated empty, never warns.
    Empty,
    Kind(BrickKind),
    /// Not in the documented 26-character table -- warns, maps to empty.
    Unsupported,
}

/// Maps one brick-grid character per docs/levelset-format.md's table.
/// `F`..`K` are the runtime-only uppercase aliases of `f`..`k` (grown
/// bricks) -- treated as synonyms, not errors, per that doc.
fn brick_cell(ch: char) -> BrickCell {
    match ch {
        '.' | ' ' => BrickCell::Empty,
        // Walls and energy-ball-only bricks: our ball has no energy-ball
        // mode, so both collapse to permanently indestructible ("silver").
        'E' | '#' | '@' => BrickCell::Kind(BrickKind::Indestructible),
        // Multi-hit (b/c/v: 2/3/4 hits) and regenerating (y/z: 2/3 hits)
        // bricks all collapse to our one `Armored` tier (always 2 hits).
        'b' | 'c' | 'v' | 'y' | 'z' => BrickCell::Kind(BrickKind::Armored),
        // Everything else destructible in one hit: the 1-hit multiplier
        // brick `a`, plain color variants `d`..`k`, the regenerating
        // 1-hit `x`, explosive `*`, grow `!`, and `F`..`K` as aliases of
        // `f`..`k`.
        'a' | 'd' | 'e' | 'f' | 'g' | 'h' | 'i' | 'j' | 'k' | 'x' | '*' | '!' | 'F' | 'G' | 'H'
        | 'I' | 'J' | 'K' => BrickCell::Kind(BrickKind::Normal),
        _ => BrickCell::Unsupported,
    }
}

/// Outcome of looking up one bonus-grid character (`extra_conv_table`).
/// Deliberately a *different* function/table from `brick_cell` -- several
/// characters (`+ - < > * ! b f` ...) mean unrelated things in each grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BonusCell {
    /// `.` -- designated empty, never warns.
    Empty,
    /// Semantics match one of our exactly-3 power-ups.
    Mapped(PowerUpKind),
    /// A real upstream bonus/malus char (score bonus, extra life, malus
    /// effects, ...) that has no equivalent in our 3-power-up set --
    /// degrades to "no drop", but (unlike upstream's silent fallthrough)
    /// still warns.
    Recognized,
    /// Not in the documented 29-character table -- warns, maps to no drop.
    Unsupported,
}

fn bonus_cell(ch: char) -> BonusCell {
    match ch {
        '.' => BonusCell::Empty,
        '+' => BonusCell::Mapped(PowerUpKind::Widen), // lengthen paddle
        '<' => BonusCell::Mapped(PowerUpKind::Slow),  // slow ball
        'b' => BonusCell::Mapped(PowerUpKind::Multiball), // extra ball
        '0' | '1' | '2' | '3' | '4' | '5' | 'g' | '-' | 'l' | 's' | 'm' | 'w' | 'f' | 'p' | '?'
        | '>' | 'j' | 'd' | 'c' | '~' | '!' | '&' | '*' | '}' | '{' | 'W' => BonusCell::Recognized,
        _ => BonusCell::Unsupported,
    }
}

/// Parses the full text of one `.lbl`-style levelset. `source` is only
/// used to label warnings (typically the file path); pass anything
/// descriptive in tests.
pub fn parse_levelset(source: &str, content: &str) -> LevelSet {
    let lines: Vec<&str> = content.lines().collect();
    // "Version:" is only recognized as the very first line; anything else
    // there just means the file has no header and starts straight into
    // `Level:` blocks (a missing header is valid, not an error).
    let mut i = if lines.first().is_some_and(|l| l.starts_with("Version:")) {
        1
    } else {
        0
    };

    let mut levels = Vec::new();
    let mut warnings = Vec::new();
    while let Some(level) = parse_one_level(source, &lines, &mut i, &mut warnings) {
        levels.push(level);
    }
    LevelSet { levels, warnings }
}

/// Parses one `Level:` block starting at `*i`, advancing `*i` past it.
/// Returns `None` on anything that doesn't match the expected shape --
/// EOF, or a label that isn't `Level:`/`Bricks:`/`Bonus:`, or a grid
/// that runs out of lines -- mirroring upstream's own "loop until a block
/// fails to parse, treat that like EOF" loader. Never panics.
fn parse_one_level(
    source: &str,
    lines: &[&str],
    i: &mut usize,
    warnings: &mut Vec<String>,
) -> Option<ExternalLevel> {
    if !lines.get(*i)?.starts_with("Level:") {
        return None;
    }
    *i += 1;
    let author = (*lines.get(*i)?).to_string();
    *i += 1;
    let name = (*lines.get(*i)?).to_string();
    *i += 1;

    if !lines.get(*i)?.starts_with("Bricks:") {
        return None;
    }
    *i += 1;
    let brick_grid = read_grid(source, lines, i, warnings, "brick", brick_cell_to_kind)?;

    if !lines.get(*i)?.starts_with("Bonus:") {
        return None;
    }
    *i += 1;
    let bonus_grid = read_grid(source, lines, i, warnings, "bonus", bonus_cell_to_powerup)?;

    let mut bricks = Vec::new();
    for row in 0..GRID_HEIGHT {
        for col in 0..GRID_WIDTH {
            if let Some(kind) = brick_grid[row][col] {
                bricks.push(ExternalBrickSpawn {
                    x: col as f32 * BRICK_WIDTH,
                    y: row as f32 * BRICK_HEIGHT,
                    kind,
                    // A bonus char over an empty brick cell never triggers
                    // (docs/levelset-format.md) -- moot here since we only
                    // reach this branch when the brick cell is occupied.
                    powerup: bonus_grid[row][col],
                });
            }
        }
    }
    Some(ExternalLevel {
        author,
        name,
        bricks,
    })
}

/// Reads exactly `GRID_HEIGHT` lines starting at `*i` into a `GRID_HEIGHT`
/// x `GRID_WIDTH` grid of `Option<T>`, warning (via `to_cell`) on each
/// column. A line shorter than `GRID_WIDTH` chars pads the missing
/// columns as empty rather than panicking (the original parser reads a
/// fixed-size buffer here; we don't have one to overrun). Returns `None`
/// if the file runs out of lines before the grid is complete.
fn read_grid<T: Copy>(
    source: &str,
    lines: &[&str],
    i: &mut usize,
    warnings: &mut Vec<String>,
    grid_name: &str,
    to_cell: fn(char) -> (Option<T>, Option<&'static str>),
) -> Option<[[Option<T>; GRID_WIDTH]; GRID_HEIGHT]> {
    let mut grid = [[None; GRID_WIDTH]; GRID_HEIGHT];
    for row in grid.iter_mut() {
        let line = *lines.get(*i)?;
        let line_no = *i + 1; // 1-based, for human-readable warnings.
        *i += 1;
        for (col, cell) in row.iter_mut().enumerate() {
            let ch = line.chars().nth(col).unwrap_or('.');
            let (value, warn) = to_cell(ch);
            *cell = value;
            if let Some(reason) = warn {
                warnings.push(format!(
                    "{source}:{line_no}: {grid_name} char {ch:?} at col {col} {reason}"
                ));
            }
        }
    }
    Some(grid)
}

fn brick_cell_to_kind(ch: char) -> (Option<BrickKind>, Option<&'static str>) {
    match brick_cell(ch) {
        BrickCell::Empty => (None, None),
        BrickCell::Kind(k) => (Some(k), None),
        BrickCell::Unsupported => (None, Some("is unsupported -> mapped to empty")),
    }
}

fn bonus_cell_to_powerup(ch: char) -> (Option<PowerUpKind>, Option<&'static str>) {
    match bonus_cell(ch) {
        BonusCell::Empty => (None, None),
        BonusCell::Mapped(k) => (Some(k), None),
        BonusCell::Recognized => (None, Some("has no matching power-up -> no drop")),
        BonusCell::Unsupported => (None, Some("is unsupported -> no drop")),
    }
}

/// Reads and parses one `.lbl`-style file from disk.
pub fn load_file(path: &Path) -> io::Result<LevelSet> {
    let content = fs::read_to_string(path)?;
    Ok(parse_levelset(&path.display().to_string(), &content))
}

/// Scans `dir` (non-recursive) for `.lbl` files and loads each one,
/// sorted by filename for a deterministic load order. `dir` is typically
/// `levels/`, populated at runtime by `scripts/fetch-levelsets.sh`
/// (arkanoid-v2-a3) -- gitignored, so this returns `Ok(vec![])` on a
/// checkout where that script hasn't run yet, not an error.
pub fn load_dir(dir: &Path) -> io::Result<Vec<(PathBuf, LevelSet)>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("lbl"))
        })
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            let level_set = load_file(&p)?;
            Ok((p, level_set))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal but complete one-level file: a single `a` (normal) brick
    /// at [0][0] with a `+` (Widen) bonus over it, an `E` (indestructible)
    /// brick at [1][0] with no bonus, everything else empty.
    fn one_level_fixture() -> String {
        let mut bricks = vec!["..............".to_string(); GRID_HEIGHT];
        bricks[0].replace_range(0..2, "aE");
        let mut bonus = vec!["..............".to_string(); GRID_HEIGHT];
        bonus[0].replace_range(0..1, "+");
        format!(
            "Level:\nSomeone\nLevel One\nBricks:\n{}\nBonus:\n{}\n",
            bricks.join("\n"),
            bonus.join("\n")
        )
    }

    #[test]
    fn brick_table_matches_documented_categories() {
        assert_eq!(brick_cell('.'), BrickCell::Empty);
        assert_eq!(brick_cell(' '), BrickCell::Empty);
        for ch in ['E', '#', '@'] {
            assert_eq!(brick_cell(ch), BrickCell::Kind(BrickKind::Indestructible));
        }
        for ch in ['b', 'c', 'v', 'y', 'z'] {
            assert_eq!(brick_cell(ch), BrickCell::Kind(BrickKind::Armored));
        }
        for ch in [
            'a', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'x', '*', '!', 'F', 'G', 'H', 'I', 'J',
            'K',
        ] {
            assert_eq!(brick_cell(ch), BrickCell::Kind(BrickKind::Normal));
        }
        assert_eq!(brick_cell('?'), BrickCell::Unsupported);
    }

    #[test]
    fn uppercase_grown_brick_aliases_match_their_lowercase_base() {
        for (upper, lower) in [
            ('F', 'f'),
            ('G', 'g'),
            ('H', 'h'),
            ('I', 'i'),
            ('J', 'j'),
            ('K', 'k'),
        ] {
            assert_eq!(brick_cell(upper), brick_cell(lower));
        }
    }

    #[test]
    fn bonus_table_matches_documented_categories() {
        assert_eq!(bonus_cell('.'), BonusCell::Empty);
        assert_eq!(bonus_cell('+'), BonusCell::Mapped(PowerUpKind::Widen));
        assert_eq!(bonus_cell('<'), BonusCell::Mapped(PowerUpKind::Slow));
        assert_eq!(bonus_cell('b'), BonusCell::Mapped(PowerUpKind::Multiball));
        for ch in [
            '0', '1', '2', '3', '4', '5', 'g', '-', 'l', 's', 'm', 'w', 'f', 'p', '?', '>', 'j',
            'd', 'c', '~', '!', '&', '*', '}', '{', 'W',
        ] {
            assert_eq!(bonus_cell(ch), BonusCell::Recognized, "char {ch:?}");
        }
        assert_eq!(bonus_cell('_'), BonusCell::Unsupported);
    }

    #[test]
    fn brick_and_bonus_grids_disagree_on_overlapping_chars_by_design() {
        // '+' grows the paddle in Bonus but is entirely unsupported in
        // Bricks; 'b' is a 2-hit armored brick but an extra-ball bonus.
        assert_eq!(brick_cell('+'), BrickCell::Unsupported);
        assert_eq!(bonus_cell('+'), BonusCell::Mapped(PowerUpKind::Widen));
        assert_eq!(brick_cell('b'), BrickCell::Kind(BrickKind::Armored));
        assert_eq!(bonus_cell('b'), BonusCell::Mapped(PowerUpKind::Multiball));
    }

    #[test]
    fn parses_one_level_with_matching_brick_and_bonus_cells() {
        let set = parse_levelset("test.lbl", &one_level_fixture());
        assert!(set.warnings.is_empty());
        assert_eq!(set.levels.len(), 1);
        let level = &set.levels[0];
        assert_eq!(level.author, "Someone");
        assert_eq!(level.name, "Level One");
        assert_eq!(level.bricks.len(), 2);
        assert_eq!(
            level.bricks[0],
            ExternalBrickSpawn {
                x: 0.0,
                y: 0.0,
                kind: BrickKind::Normal,
                powerup: Some(PowerUpKind::Widen),
            }
        );
        assert_eq!(
            level.bricks[1],
            ExternalBrickSpawn {
                x: BRICK_WIDTH,
                y: 0.0,
                kind: BrickKind::Indestructible,
                powerup: None,
            }
        );
    }

    #[test]
    fn version_header_is_consumed_and_does_not_break_parsing() {
        let content = format!("Version: 1.45\n{}", one_level_fixture());
        let set = parse_levelset("test.lbl", &content);
        assert_eq!(set.levels.len(), 1);
    }

    #[test]
    fn multiple_level_blocks_all_parse() {
        let content = one_level_fixture() + &one_level_fixture();
        let set = parse_levelset("test.lbl", &content);
        assert_eq!(set.levels.len(), 2);
    }

    #[test]
    fn unsupported_brick_char_warns_with_file_and_line_and_maps_to_empty() {
        let mut content = one_level_fixture();
        // Row 0 is line 5 (Level:/author/name/Bricks: are lines 1-4).
        content = content.replacen("aE", "a?", 1);
        let set = parse_levelset("bad.lbl", &content);
        assert_eq!(set.levels[0].bricks.len(), 1); // '?' -> empty, dropped.
        assert_eq!(set.warnings.len(), 1);
        assert!(set.warnings[0].contains("bad.lbl:5"));
        assert!(set.warnings[0].contains('?'));
    }

    #[test]
    fn recognized_but_unmapped_bonus_char_warns_and_drops_to_no_powerup() {
        let mut content = one_level_fixture();
        content = content.replacen('+', "0", 1); // '0' = +200 score, unmapped.
        let set = parse_levelset("bad.lbl", &content);
        assert_eq!(set.levels[0].bricks[0].powerup, None);
        assert_eq!(set.warnings.len(), 1);
        assert!(set.warnings[0].contains("no matching power-up"));
    }

    #[test]
    fn unsupported_bonus_char_also_warns() {
        let mut content = one_level_fixture();
        content = content.replacen('+', "_", 1);
        let set = parse_levelset("bad.lbl", &content);
        assert_eq!(set.levels[0].bricks[0].powerup, None);
        assert_eq!(set.warnings.len(), 1);
        assert!(set.warnings[0].contains("unsupported"));
    }

    #[test]
    fn truncated_file_stops_cleanly_without_panicking() {
        let set = parse_levelset("short.lbl", "Level:\nAuthor\nName\nBricks:\n");
        assert!(set.levels.is_empty());
        assert!(set.warnings.is_empty());
    }

    #[test]
    fn empty_file_yields_no_levels_and_no_warnings() {
        let set = parse_levelset("empty.lbl", "");
        assert!(set.levels.is_empty());
        assert!(set.warnings.is_empty());
    }

    #[test]
    fn load_dir_on_a_missing_directory_yields_no_levelsets_not_an_error() {
        let result = load_dir(Path::new("/nonexistent/path/for/arkanoid-tests"));
        assert_eq!(result.unwrap(), Vec::new());
    }

    // -- arkanoid-v2-a6: our own hand-written fixture levelset -------------
    //
    // `tests/fixtures/homemade.lbl` is an original, MIT-licensed 3-level
    // file (not derived from LBreakoutHD's GPLv3 level data -- see
    // `tests/fixtures/README.md` for provenance) authored to exercise every
    // character in both `docs/levelset-format.md` tables at least once,
    // plus one unsupported char per grid (`Q` in Bricks, `_` in Bonus).
    //
    // Layout:
    // - Level 1 "Clearable Cluster": 6 plain `a` (Normal) bricks, nothing
    //   else -- small and armor-free so a scripted headless play can
    //   realistically clear it.
    // - Level 2 "Full Table Sweep": row 0 + row 1 of Bricks hold all 26
    //   documented non-empty brick chars plus `Q`; row 0-2 of Bonus hold
    //   all 29 documented non-empty bonus chars plus `_`. Bonus row 0's
    //   `+`/`</`b` sit over Bricks row 0's `E`/`#`/`@` specifically so the
    //   "mapped" power-ups actually attach to a real spawn.
    // - Level 3 "Corner Sentinels": a handful of `E`/`b` bricks, just to
    //   round the fixture out to the bead's "3 levels" requirement.

    fn homemade_fixture() -> &'static str {
        include_str!("../tests/fixtures/homemade.lbl")
    }

    #[test]
    fn homemade_fixture_has_exactly_three_levels() {
        let set = parse_levelset("tests/fixtures/homemade.lbl", homemade_fixture());
        assert_eq!(set.levels.len(), 3);
        assert_eq!(set.levels[0].name, "Clearable Cluster");
        assert_eq!(set.levels[1].name, "Full Table Sweep");
        assert_eq!(set.levels[2].name, "Corner Sentinels");
    }

    #[test]
    fn homemade_fixture_exercises_every_supported_brick_and_bonus_char() {
        let set = parse_levelset("tests/fixtures/homemade.lbl", homemade_fixture());
        let sweep = &set.levels[1];

        let at = |col: usize, row: usize| {
            sweep
                .bricks
                .iter()
                .find(|b| b.x == col as f32 * BRICK_WIDTH && b.y == row as f32 * BRICK_HEIGHT)
                .unwrap_or_else(|| panic!("no spawn at col {col} row {row}"))
        };

        // The three "Mapped" bonus chars sit over Bricks row 0's three
        // Indestructible chars -- proves both that the mapping is correct
        // and that Bricks/Bonus are read from two independent tables (`b`
        // means Armored in one and Multiball in the other).
        assert_eq!(at(0, 0).kind, BrickKind::Indestructible); // 'E'
        assert_eq!(at(0, 0).powerup, Some(PowerUpKind::Widen)); // '+'
        assert_eq!(at(1, 0).kind, BrickKind::Indestructible); // '#'
        assert_eq!(at(1, 0).powerup, Some(PowerUpKind::Slow)); // '<'
        assert_eq!(at(2, 0).kind, BrickKind::Indestructible); // '@'
        assert_eq!(at(2, 0).powerup, Some(PowerUpKind::Multiball)); // 'b' (Bonus)
        assert_eq!(at(3, 0).kind, BrickKind::Armored); // 'b' (Bricks)
        assert_eq!(at(3, 0).powerup, None, "'0' is Recognized, not Mapped");
        assert_eq!(at(6, 1).kind, BrickKind::Normal, "'F' is an alias of 'f'");

        // Exactly the 26 documented non-empty brick chars produced a spawn
        // -- the 27th non-'.' cell (`Q`, row 1 col 12) is Unsupported and
        // must map to no spawn at all.
        assert_eq!(sweep.bricks.len(), 26);
        assert!(sweep
            .bricks
            .iter()
            .all(|b| !(b.x == 12.0 * BRICK_WIDTH && b.y == BRICK_HEIGHT)));
    }

    #[test]
    fn homemade_fixture_warns_on_every_unsupported_or_unmapped_char_and_nothing_else() {
        let set = parse_levelset("tests/fixtures/homemade.lbl", homemade_fixture());

        // Levels 1 and 3 use only supported chars and never populate
        // Bonus, so every warning in the whole fixture comes from level 2.
        let count = |needle: &str| set.warnings.iter().filter(|w| w.contains(needle)).count();

        assert_eq!(count("brick char 'Q'"), 1, "the one unsupported brick char");
        assert_eq!(count("bonus char '_'"), 1, "the one unsupported bonus char");
        // The 26 documented-but-unmapped Bonus chars ('0'..'W') each warn
        // once with this reason.
        assert_eq!(count("no matching power-up"), 26);
        assert_eq!(set.warnings.len(), 28, "nothing else should ever warn");
    }

    #[test]
    fn scripted_headless_play_clears_the_fixtures_first_level() {
        use crate::events::GameEvent;
        use crate::game::{Brick, Game, GameState, Input};

        let set = load_file(Path::new("tests/fixtures/homemade.lbl"))
            .expect("fixture file must load from disk");
        let level_one = &set.levels[0];
        assert_eq!(level_one.name, "Clearable Cluster");
        assert!(!level_one.bricks.is_empty());

        // Mirrors `game::build_bricks`'s centering math (private to that
        // module, for a 14-column grid in the 800-wide playfield) plus its
        // top margin -- exact pixel placement doesn't matter for this test,
        // only that the bricks land somewhere above the paddle.
        const OFFSET_X: f32 = 36.0;
        const MARGIN_TOP: f32 = 60.0;

        let mut game = Game::with_seed(0xA6);
        game.state = GameState::Playing;
        game.bricks = level_one
            .bricks
            .iter()
            .map(|spawn| Brick {
                x: spawn.x + OFFSET_X + BRICK_WIDTH / 2.0,
                y: spawn.y + MARGIN_TOP + BRICK_HEIGHT / 2.0,
                width: BRICK_WIDTH,
                height: BRICK_HEIGHT,
                kind: spawn.kind,
                hits_remaining: match spawn.kind {
                    BrickKind::Normal => 1,
                    BrickKind::Armored => 2,
                    BrickKind::Indestructible => 0,
                },
                score: 10,
            })
            .collect();

        const DT: f32 = 1.0 / 120.0;
        const MAX_TICKS: u32 = 20_000; // ~166s of sim time -- generous.

        // Bang-bang autoplay: always steer the paddle toward the ball and
        // hold Space (a no-op once the ball is launched) -- a scripted
        // headless "play" of the level, not a hand-rolled physics shortcut.
        for _ in 0..MAX_TICKS {
            let input = Input {
                left: game.ball.x < game.paddle.x,
                right: game.ball.x > game.paddle.x,
                space: true,
                pause: false,
            };
            game.tick(&input, DT);

            assert_ne!(
                game.state,
                GameState::GameOver,
                "scripted play lost all lives before clearing the fixture's level 1"
            );
            if game.events.contains(&GameEvent::LevelCleared) {
                return;
            }
        }
        panic!("scripted play did not clear the fixture's level 1 within {MAX_TICKS} ticks");
    }
}
