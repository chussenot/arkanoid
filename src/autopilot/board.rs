//! What the autopilot knows about the bricks: the board split into five
//! lanes with the counts that matter for aiming, an ASCII map of it, and
//! the deterministic lane pick used whenever the model is not consulted,
//! not confident, or not back yet.
//!
//! Counting bricks is a fact, not a judgment, so it is computed here and
//! handed to the model as data (`advisor.rs`) rather than asked of it.

use crate::game::{Brick, Game, PLAYFIELD_WIDTH};
use crate::levels::BrickKind;

/// Lanes the board is split into for aiming: few enough to be one atomic
/// Choice for a model, wide enough (160 px, three bricks) that an aim
/// error of a few pixels still lands in the lane it meant.
pub const LANES: usize = 5;
pub const LANE_WIDTH: f32 = PLAYFIELD_WIDTH / LANES as f32;

/// Map cell size. Narrower than a 52 px brick and exactly a brick's 22 px
/// height, so two brick centers never share a cell.
const CELL_WIDTH: f32 = 50.0;
const CELL_HEIGHT: f32 = 22.0;
const MAP_COLUMNS: usize = (PLAYFIELD_WIDTH / CELL_WIDTH) as usize;

/// Aim height for a lane with no brick the ball can reach: high enough to
/// carry a shot through a gap, low enough to stay a steep, safe angle.
const OPEN_LANE_TARGET_Y: f32 = 120.0;

/// One lane's counts.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Lane {
    /// Bricks that still count towards clearing the level.
    pub destructible: u32,
    /// Destructible bricks with nothing below them: a straight shot from
    /// the paddle hits these first.
    pub exposed: u32,
    /// Indestructible bricks, which only deflect.
    pub indestructible: u32,
    /// Where a shot into this lane is aimed: just under its lowest exposed
    /// destructible brick, or the lane's middle if it has none.
    pub target: (f32, f32),
}

/// The board as the autopilot sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Board {
    pub lanes: [Lane; LANES],
    /// Rows top to bottom, one char per 50 px column: `.` empty, `1`
    /// normal, `2` armored with both hits left, `X` indestructible (the
    /// alphabet of `levels.rs`; an armored brick hit once reads `1`).
    pub map: Vec<String>,
}

impl Board {
    pub fn from_game(game: &Game) -> Self {
        Self::from_bricks(&game.bricks)
    }

    pub fn from_bricks(bricks: &[Brick]) -> Self {
        let mut lanes = [Lane::default(); LANES];
        for (index, lane) in lanes.iter_mut().enumerate() {
            lane.target = (lane_center(index), OPEN_LANE_TARGET_Y);
        }

        // The lowest brick in each map column is what a ball coming up from
        // the paddle meets first.
        let mut lowest: [Option<&Brick>; MAP_COLUMNS] = [None; MAP_COLUMNS];
        for brick in bricks {
            let lane = &mut lanes[lane_of(brick.x)];
            if brick.kind == BrickKind::Indestructible {
                lane.indestructible += 1;
            } else {
                lane.destructible += 1;
            }
            let column = column_of(brick.x);
            if lowest[column].is_none_or(|b| brick.y > b.y) {
                lowest[column] = Some(brick);
            }
        }

        let mut best_exposed_y = [f32::MIN; LANES];
        for brick in lowest.into_iter().flatten() {
            if brick.kind == BrickKind::Indestructible {
                continue;
            }
            let index = lane_of(brick.x);
            lanes[index].exposed += 1;
            if brick.y > best_exposed_y[index] {
                best_exposed_y[index] = brick.y;
                lanes[index].target = (brick.x, brick.y + brick.height / 2.0);
            }
        }

        Self {
            lanes,
            map: render_map(bricks),
        }
    }

    /// The lane a shot goes to when no model answer is used: the one with
    /// the most to gain (exposed bricks weigh three times a buried one),
    /// nearest the middle on a tie. `skip` steps down that ranking, which is
    /// how the autopilot breaks out of a loop where the same shot keeps
    /// bouncing between indestructible bricks and the paddle.
    pub fn heuristic_lane(&self, skip: usize) -> usize {
        let mut ranked: Vec<usize> = (0..LANES)
            .filter(|&i| self.lanes[i].destructible > 0)
            .collect();
        if ranked.is_empty() {
            return LANES / 2;
        }
        let worth = |i: usize| self.lanes[i].exposed * 3 + self.lanes[i].destructible;
        let off_center = |i: usize| i.abs_diff(LANES / 2);
        ranked.sort_by(|&a, &b| {
            worth(b)
                .cmp(&worth(a))
                .then(off_center(a).cmp(&off_center(b)))
        });
        ranked[skip % ranked.len()]
    }
}

pub fn lane_center(index: usize) -> f32 {
    (index as f32 + 0.5) * LANE_WIDTH
}

fn lane_of(x: f32) -> usize {
    ((x / LANE_WIDTH) as usize).min(LANES - 1)
}

fn column_of(x: f32) -> usize {
    ((x.max(0.0) / CELL_WIDTH) as usize).min(MAP_COLUMNS - 1)
}

fn render_map(bricks: &[Brick]) -> Vec<String> {
    let row_of = |y: f32| (y.max(0.0) / CELL_HEIGHT) as usize;
    let (Some(first), Some(last)) = (
        bricks.iter().map(|b| row_of(b.y)).min(),
        bricks.iter().map(|b| row_of(b.y)).max(),
    ) else {
        return Vec::new();
    };
    let mut rows = vec![vec!['.'; MAP_COLUMNS]; last - first + 1];
    for brick in bricks {
        rows[row_of(brick.y) - first][column_of(brick.x)] = match brick.kind {
            BrickKind::Indestructible => 'X',
            BrickKind::Armored if brick.hits_remaining > 1 => '2',
            _ => '1',
        };
    }
    rows.into_iter().map(String::from_iter).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brick(x: f32, y: f32, kind: BrickKind) -> Brick {
        Brick {
            x,
            y,
            width: 52.0,
            height: 22.0,
            kind,
            hits_remaining: if kind == BrickKind::Armored { 2 } else { 1 },
            score: 10,
        }
    }

    #[test]
    fn lanes_count_destructible_exposed_and_indestructible_bricks() {
        let board = Board::from_bricks(&[
            brick(80.0, 71.0, BrickKind::Normal),
            brick(80.0, 93.0, BrickKind::Armored),
            brick(400.0, 71.0, BrickKind::Normal),
            brick(400.0, 93.0, BrickKind::Indestructible),
        ]);
        let left = board.lanes[0];
        assert_eq!((left.destructible, left.exposed), (2, 1));
        assert_eq!(left.target, (80.0, 104.0), "under the lowest brick");
        let center = board.lanes[2];
        assert_eq!(
            (center.destructible, center.exposed, center.indestructible),
            (1, 0, 1),
            "a brick behind an indestructible one is not exposed"
        );
    }

    #[test]
    fn the_map_uses_the_level_alphabet_one_char_per_column() {
        let board = Board::from_bricks(&[
            brick(80.0, 71.0, BrickKind::Normal),
            brick(400.0, 93.0, BrickKind::Indestructible),
            brick(780.0, 93.0, BrickKind::Armored),
        ]);
        assert_eq!(board.map, vec![".1..............", "........X......2"]);
    }

    #[test]
    fn heuristic_prefers_exposed_bricks_then_the_middle_and_skip_rotates() {
        let board = Board::from_bricks(&[
            brick(80.0, 71.0, BrickKind::Normal),
            brick(400.0, 71.0, BrickKind::Normal),
            brick(720.0, 71.0, BrickKind::Normal),
            brick(720.0, 93.0, BrickKind::Normal),
        ]);
        assert_eq!(board.heuristic_lane(0), 4, "two bricks beat one");
        assert_eq!(board.heuristic_lane(1), 2, "then the middle");
        assert_eq!(board.heuristic_lane(2), 0);
        assert_eq!(board.heuristic_lane(3), 4, "wraps around");
    }

    #[test]
    fn an_empty_board_aims_at_the_middle() {
        let board = Board::from_bricks(&[]);
        assert_eq!(board.heuristic_lane(0), LANES / 2);
        assert!(board.map.is_empty());
    }
}
