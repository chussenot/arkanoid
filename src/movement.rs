//! The campaign's four movements: Foundations, Structures, Machines and
//! the Vault, the acts the score already gives a form each
//! (`docs/design/music.md`), made visible (`docs/design/game-director-review.md`,
//! "Four acts, four visual movements").
//!
//! Each act is one row of data, read by both renderers and the HUD:
//! - a **ground**, a motif of thin lines drawn under the bricks;
//! - an **accent**, the one hue (`palette::ACT_ACCENTS`) the ground, the
//!   ceiling rule and the 3D walls take;
//! - its **pulse**: the ground brightens with the rally, so a long run
//!   lights the act's floor, and goes back down at the paddle;
//! - a **title**, shown with the serve prompt on the act's first level.
//!
//! Campaign only: Classic, levelsets and Shuffle have no acts and keep
//! the plain field. Pure: game in, geometry and colors out.

use crate::game::{
    Game, LevelSetKind, BRICK_GRID_MARGIN_TOP, LEVELS_PER_ACT, PLAYFIELD_HEIGHT, PLAYFIELD_WIDTH,
};
use crate::levels::{BRICK_HEIGHT, BRICK_WIDTH};
use crate::palette::{self, Rgb};

/// The motif under an act's bricks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ground {
    /// Foundations: the surveyed ground, dots on the grid the bricks sit on.
    Survey,
    /// Structures: the frame going up, beams and ties.
    Girders,
    /// Machines: gear teeth along both side walls.
    Gears,
    /// The Vault: a door's nested frames around the bricks.
    Vault,
}

/// One act's look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Movement {
    pub numeral: &'static str,
    pub name: &'static str,
    pub accent: Rgb,
    pub ground: Ground,
}

/// The four acts, in campaign order.
pub const MOVEMENTS: [Movement; 4] = [
    Movement {
        numeral: "I",
        name: "FOUNDATIONS",
        accent: palette::ACT_ACCENTS[0],
        ground: Ground::Survey,
    },
    Movement {
        numeral: "II",
        name: "STRUCTURES",
        accent: palette::ACT_ACCENTS[1],
        ground: Ground::Girders,
    },
    Movement {
        numeral: "III",
        name: "MACHINES",
        accent: palette::ACT_ACCENTS[2],
        ground: Ground::Gears,
    },
    Movement {
        numeral: "IV",
        name: "THE VAULT",
        accent: palette::ACT_ACCENTS[3],
        ground: Ground::Vault,
    },
];

/// The ground's mix toward its accent, in linear light: at rest, and with
/// a rally at the top of the scale. Both keep it a dark floor of hairlines
/// under bright objects (a test holds the ball at 7:1 over the brightest).
const GROUND_REST: f32 = 0.05;
const GROUND_LIT: f32 = 0.16;
/// The rally that lights the ground fully: the scale's ten steps.
const GROUND_FULL_RALLY: u32 = 10;

/// Line weight of every ground motif, in field pixels.
const LINE: f32 = 2.0;
/// The campaign's 14-column brick grid, centered as `game.rs` centers it:
/// the survey's dots sit on its corners, the girders on its columns.
const GRID_COLUMNS: f32 = 14.0;
const GRID_LEFT: f32 = (PLAYFIELD_WIDTH - GRID_COLUMNS * BRICK_WIDTH) / 2.0;
const GRID_TOP: f32 = BRICK_GRID_MARGIN_TOP;

/// The most rects any ground draws: what the renderers' instance buffers
/// make room for.
pub const MAX_GROUND_RECTS: usize = 200;

/// The movement the game is in: its campaign act, or `None` outside the
/// campaign.
pub fn of(game: &Game) -> Option<&'static Movement> {
    (game.level_set() == LevelSetKind::Campaign)
        .then(|| &MOVEMENTS[((game.level.max(1) - 1) / LEVELS_PER_ACT).min(MOVEMENTS.len() - 1)])
}

/// The movement whose first level this is, when it is one: the act the
/// serve prompt names.
pub fn opening(game: &Game) -> Option<&'static Movement> {
    of(game).filter(|_| (game.level.max(1) - 1).is_multiple_of(LEVELS_PER_ACT))
}

/// The ground's color for a rally of `rally` bricks: the field mixed toward
/// the accent, a little more with every brick of the run up to the top of
/// the scale. Linear RGBA.
pub fn ground_color(movement: &Movement, rally: u32) -> [f32; 4] {
    let lit = rally.min(GROUND_FULL_RALLY) as f32 / GROUND_FULL_RALLY as f32;
    let k = GROUND_REST + (GROUND_LIT - GROUND_REST) * lit;
    let field = palette::FIELD.linear();
    let accent = movement.accent.linear();
    let mix = |i: usize| field[i] + k * (accent[i] - field[i]);
    [mix(0), mix(1), mix(2), 1.0]
}

/// The ground's lines as rects (left, top, width, height) in field pixels,
/// all inside the field.
pub fn ground_rects(ground: Ground) -> Vec<[f32; 4]> {
    let mut rects = Vec::new();
    match ground {
        Ground::Survey => {
            // A dot on every corner of the brick grid, every other row,
            // down to the paddle's lane.
            let mut y = GRID_TOP;
            while y < PLAYFIELD_HEIGHT - 40.0 {
                for i in 0..=14 {
                    let x = GRID_LEFT + i as f32 * BRICK_WIDTH;
                    rects.push([x - LINE / 2.0, y - LINE / 2.0, LINE, LINE]);
                }
                y += 2.0 * BRICK_HEIGHT;
            }
        }
        Ground::Girders => {
            // Beams every two columns, ties every four rows.
            for i in (0..=14).step_by(2) {
                let x = GRID_LEFT + i as f32 * BRICK_WIDTH;
                rects.push([x - LINE / 2.0, GRID_TOP, LINE, PLAYFIELD_HEIGHT - GRID_TOP]);
            }
            let mut y = GRID_TOP;
            while y + LINE <= PLAYFIELD_HEIGHT {
                rects.push([GRID_LEFT, y, GRID_COLUMNS * BRICK_WIDTH, LINE]);
                y += 4.0 * BRICK_HEIGHT;
            }
        }
        Ground::Gears => {
            // Teeth along both side walls, long and short in turn.
            let mut y = GRID_TOP;
            let mut long = true;
            while y + LINE <= PLAYFIELD_HEIGHT {
                let w = if long { 14.0 } else { 6.0 };
                rects.push([0.0, y, w, LINE]);
                rects.push([PLAYFIELD_WIDTH - w, y, w, LINE]);
                y += BRICK_HEIGHT / 2.0;
                long = !long;
            }
        }
        Ground::Vault => {
            // Five nested frames around the bricks' middle: a door.
            let (cx, cy) = (PLAYFIELD_WIDTH / 2.0, GRID_TOP + 4.0 * BRICK_HEIGHT);
            for n in 1..=5 {
                let (hw, hh) = (70.0 * n as f32, 34.0 * n as f32);
                let (left, top) = (cx - hw, (cy - hh).max(0.0));
                let bottom = (cy + hh).min(PLAYFIELD_HEIGHT);
                rects.push([left, top, 2.0 * hw, LINE]);
                rects.push([left, bottom - LINE, 2.0 * hw, LINE]);
                rects.push([left, top, LINE, bottom - top]);
                rects.push([cx + hw - LINE, top, LINE, bottom - top]);
            }
        }
    }
    rects
}

#[cfg(test)]
mod tests {
    use super::*;

    fn campaign_at(level: usize) -> Game {
        let mut game = Game::new().with_campaign();
        game.level = level;
        game
    }

    #[test]
    fn each_act_is_its_movement_and_only_the_campaign_has_acts() {
        let names: Vec<&str> = [1, 8, 9, 16, 17, 24, 25, 32]
            .iter()
            .map(|&l| of(&campaign_at(l)).unwrap().name)
            .collect();
        assert_eq!(
            names,
            [
                "FOUNDATIONS",
                "FOUNDATIONS",
                "STRUCTURES",
                "STRUCTURES",
                "MACHINES",
                "MACHINES",
                "THE VAULT",
                "THE VAULT"
            ]
        );
        assert!(of(&Game::new()).is_none(), "Classic has no acts");
    }

    #[test]
    fn an_act_is_announced_on_its_first_level_only() {
        let opened: Vec<usize> = (1..=32)
            .filter(|&l| opening(&campaign_at(l)).is_some())
            .collect();
        assert_eq!(opened, [1, 9, 17, 25]);
    }

    #[test]
    fn every_ground_fits_the_field_and_the_renderers_room() {
        for movement in MOVEMENTS {
            let rects = ground_rects(movement.ground);
            assert!(!rects.is_empty());
            assert!(
                rects.len() <= MAX_GROUND_RECTS,
                "{:?}: {}",
                movement.ground,
                rects.len()
            );
            for [x, y, w, h] in rects {
                assert!(w > 0.0 && h > 0.0);
                assert!(
                    x >= 0.0 && x + w <= PLAYFIELD_WIDTH + 0.01,
                    "{:?}",
                    movement.ground
                );
                assert!(
                    y >= 0.0 && y + h <= PLAYFIELD_HEIGHT + 0.01,
                    "{:?}",
                    movement.ground
                );
            }
        }
    }

    #[test]
    fn the_four_grounds_and_accents_are_distinct() {
        for (i, a) in MOVEMENTS.iter().enumerate() {
            for b in &MOVEMENTS[i + 1..] {
                assert_ne!(a.ground, b.ground);
                assert_ne!(a.accent, b.accent);
            }
        }
    }

    /// WCAG relative luminance of a linear color.
    fn luminance(c: [f32; 4]) -> f32 {
        0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
    }

    fn contrast(a: [f32; 4], b: [f32; 4]) -> f32 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    /// The ground stays a floor: lit by the longest rally, the ball keeps
    /// 7:1 over it and its lines never get as bright as the dimmest brick
    /// (indestructible steel). At rest they sit near the ceiling's
    /// hairlines, about 1.3:1 on the field; lit, about 2:1.
    #[test]
    fn the_brightest_ground_keeps_the_ball_and_bricks_on_top() {
        for movement in MOVEMENTS {
            let ground = ground_color(&movement, GROUND_FULL_RALLY);
            assert!(
                contrast(palette::BALL.linear(), ground) >= 7.0,
                "{}: the ball",
                movement.name
            );
            for brick in [
                palette::BRICK_NORMAL,
                palette::BRICK_ARMORED,
                palette::BRICK_ARMORED_HIT,
                palette::BRICK_INDESTRUCTIBLE,
            ] {
                assert!(
                    contrast(brick.linear(), ground) >= 1.05,
                    "{}: {brick:?}",
                    movement.name
                );
            }
        }
    }

    #[test]
    fn a_rally_lights_the_ground_up_to_the_top_of_the_scale() {
        let m = &MOVEMENTS[0];
        let lum = |c: [f32; 4]| c[0] + c[1] + c[2];
        let steps: Vec<f32> = (0..=12).map(|r| lum(ground_color(m, r))).collect();
        assert!(
            steps.windows(2).all(|w| w[1] >= w[0]),
            "never dims as it climbs"
        );
        assert_eq!(steps[10], steps[12], "full at the tenth brick");
        assert!(steps[10] > steps[0]);
    }
}
