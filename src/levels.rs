//! Level grids as const `&[&str]` data.
//!
//! Chars: `.` empty, `1` normal, `2` armored, `X` indestructible. Adding a
//! level must require touching only this file.
//!
//! ponytail: nothing in `game.rs` calls `parse_level`/reads `LEVELS` yet —
//! that lands with level load/progression (M3, arkanoid-5rs). Module-wide
//! `#[allow(dead_code)]` until then; delete it once M3 wires this module in.
#![allow(dead_code)]

/// Brick footprint in logical playfield pixels (spec: 52x22, grid up to
/// 14x8).
pub const BRICK_WIDTH: f32 = 52.0;
pub const BRICK_HEIGHT: f32 = 22.0;

/// What a brick is made of. Scoring (10-70 pts by row) and hit-count
/// behavior (armored takes 2 hits) belong to `game.rs`, not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrickKind {
    Normal,
    Armored,
    Indestructible,
}

/// One brick to spawn: top-left corner in grid-local pixels (col *
/// BRICK_WIDTH, row * BRICK_HEIGHT — grid origin at (0, 0)) plus its kind.
/// The consumer (`game.rs`) is responsible for translating this into the
/// 800x600 playfield (e.g. centering the grid), since that placement is a
/// gameplay/layout concern, not a level-data concern.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrickSpawn {
    pub x: f32,
    pub y: f32,
    pub kind: BrickKind,
}

/// Parse a level grid into brick spawns. `.` cells are skipped; any other
/// character maps to a `BrickKind` per the module doc. Row/column index
/// order matches the grid's reading order (row 0 = top).
pub fn parse_level(grid: &[&str]) -> Vec<BrickSpawn> {
    grid.iter()
        .enumerate()
        .flat_map(|(row, line)| {
            line.chars().enumerate().filter_map(move |(col, ch)| {
                let kind = match ch {
                    '1' => Some(BrickKind::Normal),
                    '2' => Some(BrickKind::Armored),
                    'X' => Some(BrickKind::Indestructible),
                    _ => None,
                };
                kind.map(|kind| BrickSpawn {
                    x: col as f32 * BRICK_WIDTH,
                    y: row as f32 * BRICK_HEIGHT,
                    kind,
                })
            })
        })
        .collect()
}

/// Level 1: simple rows, all normal bricks.
pub const LEVEL_1: &[&str] = &[
    "11111111111111",
    "11111111111111",
    "11111111111111",
    "11111111111111",
];

/// Level 2: introduces armored bricks (rows 0 and 3).
pub const LEVEL_2: &[&str] = &[
    "22222222222222",
    "11111111111111",
    "11111111111111",
    "22222222222222",
    "11111111111111",
];

/// Level 3: indestructible bricks shape a corridor — ceiling/floor walls,
/// side walls, and a clear lane (row 3) the ball can travel through.
pub const LEVEL_3: &[&str] = &[
    "XXXXXXXXXXXXXX",
    "X111111111111X",
    "X122222222221X",
    "X1..........1X",
    "X111111111111X",
    "XXXXXXXXXXXXXX",
];

/// All built-in levels, in play order.
pub const LEVELS: &[&[&str]] = &[LEVEL_1, LEVEL_2, LEVEL_3];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_1_row_is_all_normal_bricks_at_expected_positions() {
        let spawns = parse_level(&[LEVEL_1[0]]);
        assert_eq!(spawns.len(), 14);
        assert!(spawns.iter().all(|b| b.kind == BrickKind::Normal));
        assert_eq!(
            spawns[0],
            BrickSpawn {
                x: 0.0,
                y: 0.0,
                kind: BrickKind::Normal
            }
        );
        assert_eq!(
            spawns[13],
            BrickSpawn {
                x: 13.0 * BRICK_WIDTH,
                y: 0.0,
                kind: BrickKind::Normal
            }
        );
    }

    #[test]
    fn level_2_top_row_is_all_armored() {
        let spawns = parse_level(&[LEVEL_2[0]]);
        assert_eq!(spawns.len(), 14);
        assert!(spawns.iter().all(|b| b.kind == BrickKind::Armored));
    }

    #[test]
    fn level_3_top_row_is_all_indestructible() {
        let spawns = parse_level(&[LEVEL_3[0]]);
        assert_eq!(spawns.len(), 14);
        assert!(spawns.iter().all(|b| b.kind == BrickKind::Indestructible));
    }

    #[test]
    fn level_3_corridor_row_mixes_kinds_and_skips_empty_cells() {
        // "X1..........1X" -> walls + normal at the ends, dots skipped.
        let spawns = parse_level(&[LEVEL_3[3]]);
        assert_eq!(spawns.len(), 4);
        assert_eq!(
            spawns[0],
            BrickSpawn {
                x: 0.0,
                y: 0.0,
                kind: BrickKind::Indestructible
            }
        );
        assert_eq!(
            spawns[1],
            BrickSpawn {
                x: BRICK_WIDTH,
                y: 0.0,
                kind: BrickKind::Normal
            }
        );
        assert_eq!(
            spawns[2],
            BrickSpawn {
                x: 12.0 * BRICK_WIDTH,
                y: 0.0,
                kind: BrickKind::Normal
            }
        );
        assert_eq!(
            spawns[3],
            BrickSpawn {
                x: 13.0 * BRICK_WIDTH,
                y: 0.0,
                kind: BrickKind::Indestructible
            }
        );
    }

    #[test]
    fn all_built_in_levels_have_rectangular_rows() {
        for level in LEVELS {
            let width = level[0].chars().count();
            assert!(level.iter().all(|row| row.chars().count() == width));
            assert!(width <= 14);
            assert!(level.len() <= 8);
        }
    }
}
