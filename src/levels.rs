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
/// side walls, and a clear lane (row 3) the ball can travel through. The
/// floor has a four-brick gate in the middle: the only way in, so every
/// destructible brick is reached through it.
pub const LEVEL_3: &[&str] = &[
    "XXXXXXXXXXXXXX",
    "X111111111111X",
    "X122222222221X",
    "X1..........1X",
    "X111111111111X",
    "XXXXX....XXXXX",
];

/// All built-in levels, in play order.
pub const LEVELS: &[&[&str]] = &[LEVEL_1, LEVEL_2, LEVEL_3];

// -- the campaign -------------------------------------------------------------
//
// What a normal game plays: 32 levels (staircases, frames, columns, walls of
// armored and indestructible bricks), all original designs: this repo never
// reproduces another game's layouts (see ATTRIBUTION). `LEVELS` above stays
// v1's three: the tests play them, and so does `--classic`.

/// The campaign, in play order, each with its name.
pub const CAMPAIGN: &[(&str, &[&str])] = &[
    (
        "Warm-up",
        &[
            "..............",
            ".111111111111.",
            ".111111111111.",
            ".111111111111.",
        ],
    ),
    (
        "Cascade",
        &[
            "..........1111",
            ".........11112",
            "........11112.",
            ".......11112..",
            "......11112...",
            ".....11112....",
            "....11112.....",
            "...11112......",
        ],
    ),
    (
        "Checkers",
        &[
            "1.1.1.1.1.1.1.",
            ".1.1.1.1.1.1.1",
            "2.2.2.2.2.2.2.",
            ".2.2.2.2.2.2.2",
            "1.1.1.1.1.1.1.",
            ".1.1.1.1.1.1.1",
        ],
    ),
    (
        "Twin towers",
        &[
            ".1111....1111.",
            ".1221....1221.",
            ".1221....1221.",
            ".1221....1221.",
            ".1221....1221.",
            ".1111....1111.",
        ],
    ),
    (
        "Arrow",
        &[
            "......11......",
            "......11......",
            "......11......",
            "......11......",
            "..1...11...1..",
            "...1..11..1...",
            "....1.11.1....",
            ".....1111.....",
        ],
    ),
    (
        "Capped columns",
        &[
            "X.X.X.X.X.X.X.",
            "1.1.1.1.1.1.1.",
            "1.1.1.1.1.1.1.",
            "2.2.2.2.2.2.2.",
            "1.1.1.1.1.1.1.",
            "1.1.1.1.1.1.1.",
        ],
    ),
    (
        "Diamond",
        &[
            "......22......",
            ".....2112.....",
            "....211112....",
            "...21111112...",
            "...21111112...",
            "....211112....",
            ".....2112.....",
            "......22......",
        ],
    ),
    (
        "Scatter",
        &[
            "1...2...1...2.",
            "..1...2...1...",
            "2...1...2...1.",
            "..2...1...2...",
            "1...2...1...2.",
            "..1...2...1...",
        ],
    ),
    (
        "Picture frame",
        &[
            "X111111111111X",
            "1............1",
            "1.2222222222.1",
            "1.2111111112.1",
            "1.2222222222.1",
            "1............1",
            "21111111111112",
        ],
    ),
    (
        "Heart",
        &[
            "..111....111..",
            ".11111..11111.",
            "11111111111111",
            "11111111111111",
            ".111111111111.",
            "..1111111111..",
            "....111111....",
            "......11......",
        ],
    ),
    (
        "Double W",
        &[
            "2............2",
            "12..........21",
            ".12...22...21.",
            "..12.2112.21..",
            "...121..121...",
            "....1....1....",
        ],
    ),
    (
        "Fortress",
        &[
            "XXXXX....XXXXX",
            "X111X....X111X",
            "X121X....X121X",
            "X111X....X111X",
            "X222X....X222X",
        ],
    ),
    (
        "Lattice",
        &[
            "1.1.1.1.1.1.1.",
            "11111111111111",
            "X1X1X1X1X1X1X1",
            "11111111111111",
            "1.1.1.1.1.1.1.",
        ],
    ),
    (
        "Wave",
        &[
            "1....11....1..",
            "11..1111..11..",
            "111111111111..",
            ".111111111111.",
            "..11..1111..11",
            "..1....11....1",
        ],
    ),
    (
        "Rocket",
        &[
            "......22......",
            ".....2112.....",
            ".....1111.....",
            ".....1111.....",
            "....111111....",
            "...11.11.11...",
            "..1...11...1..",
        ],
    ),
    (
        "Bridges",
        &[
            "22222222222222",
            "11111111111111",
            ".1X..2112..X1.",
            "..X........X..",
            "..X........X..",
        ],
    ),
    (
        "Spiral",
        &[
            "11111111111111",
            "1............1",
            "1.1111111111.1",
            "1.1........1.1",
            "1.1.111111.1.1",
            "1.1.1......1.1",
            "1.1.11111111.1",
            "1.1..........1",
        ],
    ),
    (
        "Pillars",
        &[
            "22.22.22.22.22",
            "11.11.11.11.11",
            "11.11.11.11.11",
            "11.11.11.11.11",
            "XX.XX.XX.XX.XX",
        ],
    ),
    (
        "Crown",
        &[
            "1..1..11..1..1",
            "11.11.11.11.11",
            "11111111111111",
            "12121212121212",
            "11111111111111",
        ],
    ),
    (
        "Hourglass",
        &[
            "11111111111111",
            ".111111111111.",
            "..1111111111..",
            "....222222....",
            "....222222....",
            "..1111111111..",
            ".111111111111.",
            "11111111111111",
        ],
    ),
    (
        "Shield",
        &[
            "..2222222222..",
            ".211111111112.",
            ".211111111112.",
            "..2111111112..",
            "...21111112...",
            "XXXX.2112.XXXX",
        ],
    ),
    (
        "Armored frame",
        &[
            "22222222222222",
            "2............2",
            "2.1111111111.2",
            "2.1111111111.2",
            "2............2",
            "22222222222222",
        ],
    ),
    (
        "Rain",
        &[
            "1.1.1.1.1.1.1.",
            "1...1...1...1.",
            "..1...1...1...",
            ".1.1.1.1.1.1.1",
            "2...2...2...2.",
            "..2...2...2...",
        ],
    ),
    (
        "Funnel",
        &[
            "22222222222222",
            ".111111111111.",
            "..1111111111..",
            "...11111111...",
            "....111111....",
            ".....1111.....",
            "......11......",
        ],
    ),
    (
        "Two rooms",
        &[
            "XXXXXXXXXXXXXX",
            "X11111XX11111X",
            "X12221XX12221X",
            "X11111XX11111X",
            "X.....XX.....X",
        ],
    ),
    (
        "Ring",
        &[
            "....111111....",
            "..11......11..",
            ".1..222222..1.",
            ".1..2....2..1.",
            ".1..222222..1.",
            "..11......11..",
            "....111111....",
        ],
    ),
    (
        "Baffles",
        &[
            "11111111111111",
            "22222222222222",
            "11111111111111",
            "..............",
            "XXX...XX...XXX",
            "..............",
            "...XX.....XX..",
        ],
    ),
    (
        "Chevron",
        &[
            "2............2",
            "12..........21",
            ".12........21.",
            "..12......21..",
            "...12....21...",
            "....12..21....",
            ".....1221.....",
        ],
    ),
    (
        "Split wall",
        &[
            "11111XXXX11111",
            "11111....11111",
            "22222....22222",
            "11111....11111",
            "11111XXXX11111",
        ],
    ),
    (
        "Bulwark",
        &[
            "22222222222222",
            "21111111111112",
            "21111111111112",
            "21111111111112",
            "21111111111112",
            "22222222222222",
        ],
    ),
    (
        "Minefield",
        &[
            "1.2.1.X.1.2.1.",
            ".X.1.2.1.2.1.X",
            "2.1.X.1.2.1.2.",
            ".1.2.1.X.1.2.1",
            "X.2.1.2.1.X.1.",
        ],
    ),
    (
        "Vault",
        &[
            "XXXXXXXXXXXXXX",
            "X212121212121X",
            "X121212121212X",
            "X212121212121X",
            "X121212121212X",
            "X............X",
        ],
    ),
];

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
    fn level_3_floor_has_a_gate_into_the_box() {
        // Without it the box is closed on all four sides and its
        // destructible bricks can never be hit, so the level (and the game)
        // could not be won.
        let floor = parse_level(&[LEVEL_3[LEVEL_3.len() - 1]]);
        assert_eq!(floor.len(), 10);
        assert!(floor.iter().all(|b| b.kind == BrickKind::Indestructible));
        let gate: Vec<f32> = (5..9).map(|col| col as f32 * BRICK_WIDTH).collect();
        assert!(floor.iter().all(|b| !gate.contains(&b.x)));
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

    /// Every built-in grid: v1's three and the campaign.
    fn all_grids() -> impl Iterator<Item = &'static [&'static str]> {
        LEVELS
            .iter()
            .copied()
            .chain(CAMPAIGN.iter().map(|(_, grid)| *grid))
    }

    #[test]
    fn every_built_in_level_can_be_cleared() {
        // A ball can reach a brick if a path of cells leads to it from the
        // open field around the grid, through empty cells and through
        // destructible bricks (they break), never through an indestructible
        // one, moving between edge-sharing cells (two indestructible bricks
        // touching at a corner leave no gap a ball fits through). Level 3's
        // first, closed box failed exactly this.
        for (index, grid) in all_grids().enumerate() {
            let rows = grid.len();
            let cols = grid[0].chars().count();
            let cells: Vec<Vec<char>> = grid.iter().map(|r| r.chars().collect()).collect();
            // A ring of open field around the grid: indices shifted by one.
            let open = |r: usize, c: usize| {
                r == 0 || c == 0 || r > rows || c > cols || cells[r - 1][c - 1] != 'X'
            };
            let mut seen = vec![vec![false; cols + 2]; rows + 2];
            let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
            seen[0][0] = true;
            while let Some((r, c)) = stack.pop() {
                let neighbours = [
                    (r.wrapping_sub(1), c),
                    (r + 1, c),
                    (r, c.wrapping_sub(1)),
                    (r, c + 1),
                ];
                for (nr, nc) in neighbours {
                    if nr <= rows + 1 && nc <= cols + 1 && !seen[nr][nc] && open(nr, nc) {
                        seen[nr][nc] = true;
                        stack.push((nr, nc));
                    }
                }
            }
            for (r, row) in cells.iter().enumerate() {
                for (c, &cell) in row.iter().enumerate() {
                    assert!(
                        !matches!(cell, '1' | '2') || seen[r + 1][c + 1],
                        "grid {index}: the brick at row {r}, column {c} cannot be reached"
                    );
                }
            }
        }
    }

    /// A breakable brick with an indestructible one above it and another
    /// below it in the same column can only be hit from the side, through
    /// one brick-high lane: the pacing probe found such bricks were always
    /// a level's last 10%, hunted for minutes (docs/design/director-review.md).
    /// The campaign only: v1's three levels are frozen with its replay hash.
    #[test]
    fn no_campaign_brick_sits_between_two_indestructible_ones_in_its_column() {
        for (name, grid) in CAMPAIGN {
            let cells: Vec<Vec<char>> = grid.iter().map(|r| r.chars().collect()).collect();
            for c in 0..cells[0].len() {
                let column: Vec<char> = cells.iter().map(|row| row[c]).collect();
                let (Some(top), Some(bottom)) = (
                    column.iter().position(|&k| k == 'X'),
                    column.iter().rposition(|&k| k == 'X'),
                ) else {
                    continue;
                };
                for (r, &cell) in column.iter().enumerate().take(bottom).skip(top + 1) {
                    assert!(
                        !matches!(cell, '1' | '2'),
                        "{name}: the brick at row {r}, column {c} is sealed between \
                         indestructible bricks above and below"
                    );
                }
            }
        }
    }

    #[test]
    fn the_campaign_has_32_named_levels_each_with_bricks_to_break() {
        assert_eq!(CAMPAIGN.len(), 32);
        for (name, grid) in CAMPAIGN {
            assert!(!name.is_empty());
            assert!(
                parse_level(grid)
                    .iter()
                    .any(|b| b.kind != BrickKind::Indestructible),
                "{name} has nothing to clear"
            );
        }
    }

    #[test]
    fn all_built_in_levels_have_rectangular_rows() {
        for level in all_grids() {
            let width = level[0].chars().count();
            assert!(level.iter().all(|row| row.chars().count() == width));
            assert!(width <= 14);
            assert!(level.len() <= 8);
        }
    }
}
