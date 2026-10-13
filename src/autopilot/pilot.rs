//! The deterministic half of the autopilot: where a ball meets the paddle
//! line, which paddle position sends it at a target, and the held-key
//! [`Input`] that moves the paddle there.
//!
//! Everything here is geometry the simulation already defines, so none of
//! it is asked of a model: a model answers what code cannot compute
//! (which part of the board is worth attacking, `advisor.rs`), and this
//! module turns that answer into a paddle position every tick. Recomputed
//! from the live `Game` each tick, so a brick bounce that changes a ball's
//! course is absorbed on the next tick rather than predicted.

use crate::game::{Ball, Paddle, PADDLE_SPEED, PLAYFIELD_WIDTH};

/// Largest paddle-hit offset the pilot plans for (±1.0 is the paddle's
/// edges). Past it, a few pixels of prediction error or of the 4.3 px a
/// paddle moves per tick put the ball off the edge and lose it.
pub const MAX_AIM_OFFSET: f32 = 0.75;

/// Launch direction of a served ball (`game.rs`'s `BALL_LAUNCH_ANGLE_DEG`):
/// up and to the right, whatever the paddle does.
const LAUNCH_ANGLE_DEG: f32 = 60.0;

/// Where and when a ball's center reaches the paddle's contact line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Intercept {
    pub x: f32,
    /// Seconds from now.
    pub time: f32,
}

/// Folds an unbounded x back into `[lo, hi]` the way the side walls reflect
/// a ball: a triangle wave with period `2 * (hi - lo)`.
pub fn fold(x: f32, lo: f32, hi: f32) -> f32 {
    let width = hi - lo;
    if width <= 0.0 {
        return lo;
    }
    let mut t = (x - lo).rem_euclid(2.0 * width);
    if t > width {
        t = 2.0 * width - t;
    }
    lo + t
}

/// The y a ball's center is at when it touches the paddle's top face.
pub fn contact_line(paddle: &Paddle, ball: &Ball) -> f32 {
    paddle.top() - ball.radius
}

/// Where `ball` will cross `line_y` going down, assuming no brick on the
/// way: straight down if it is falling, via the ceiling if it is rising.
/// `None` for a ball resting on the paddle, or already past the line.
pub fn predict_intercept(ball: &Ball, line_y: f32) -> Option<Intercept> {
    if ball.attached || ball.vy == 0.0 {
        return None;
    }
    let distance_y = if ball.vy > 0.0 {
        if ball.y > line_y {
            return None;
        }
        line_y - ball.y
    } else {
        // Up to the ceiling (center at y = radius), then all the way down.
        (ball.y - ball.radius) + (line_y - ball.radius)
    };
    let time = distance_y / ball.vy.abs();
    let x = fold(
        ball.x + ball.vx * time,
        ball.radius,
        PLAYFIELD_WIDTH - ball.radius,
    );
    Some(Intercept { x, time })
}

/// The ball the paddle must deal with next: the one reaching the contact
/// line soonest. Every other ball in a multiball is on its own; the game
/// only spends a life when the last one drops.
pub fn next_intercept<'a>(
    balls: impl IntoIterator<Item = &'a Ball>,
    paddle: &Paddle,
) -> Option<Intercept> {
    balls
        .into_iter()
        .filter_map(|ball| predict_intercept(ball, contact_line(paddle, ball)))
        .min_by(|a, b| a.time.total_cmp(&b.time))
}

/// Inverse of `offset_to_exit_angle_deg` on its linear part: the offset
/// that sends the ball out at `angle_deg`.
fn offset_for_angle(angle_deg: f32) -> f32 {
    (90.0 - angle_deg) / 60.0
}

/// The paddle-hit offset that sends a ball hit at `hit_x` on the contact
/// line `line_y` straight at `target`, clamped to [`MAX_AIM_OFFSET`]. No
/// wall bank is considered: a wall's mirror image of a target is always
/// farther away horizontally than the target, so banking only ever needs a
/// shallower shot than the direct one.
pub fn aim_offset(hit_x: f32, line_y: f32, target: (f32, f32)) -> f32 {
    let (target_x, target_y) = target;
    let rise = (line_y - target_y).max(1.0);
    let angle_deg = rise.atan2(target_x - hit_x).to_degrees();
    offset_for_angle(angle_deg).clamp(-MAX_AIM_OFFSET, MAX_AIM_OFFSET)
}

/// Paddle center that meets a ball at `hit_x` with `offset`, kept inside
/// the walls.
pub fn paddle_x_for(hit_x: f32, offset: f32, paddle: &Paddle) -> f32 {
    let half = paddle.width / 2.0;
    (hit_x - offset * half).clamp(half, PLAYFIELD_WIDTH - half)
}

/// Paddle center from which a serve (always launched at 60°, up and to
/// the right) reaches `target`, kept inside the walls: a target too far
/// left for any serve gets the leftmost one.
pub fn serve_x_for(target: (f32, f32), ball: &Ball, paddle: &Paddle) -> f32 {
    let (target_x, target_y) = target;
    let run = (ball.y - target_y).max(0.0) / LAUNCH_ANGLE_DEG.to_radians().tan();
    let half = paddle.width / 2.0;
    (target_x - run).clamp(half, PLAYFIELD_WIDTH - half)
}

/// Can the paddle travel from `from` to `to` in `time` seconds? A 10%
/// margin covers the half-step deadband of [`steer`].
pub fn reachable(from: f32, to: f32, time: f32) -> bool {
    (to - from).abs() <= PADDLE_SPEED * time * 0.9
}

/// Held keys that move the paddle from `paddle_x` towards `target_x` this
/// tick. Holds still within half a tick's travel, so the paddle settles
/// instead of oscillating around the target.
pub fn steer(paddle_x: f32, target_x: f32, dt: f32) -> (bool, bool) {
    let diff = target_x - paddle_x;
    let deadband = PADDLE_SPEED * dt / 2.0;
    if diff > deadband {
        (false, true)
    } else if diff < -deadband {
        (true, false)
    } else {
        (false, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::offset_to_exit_angle_deg;

    fn ball(x: f32, y: f32, vx: f32, vy: f32) -> Ball {
        Ball {
            x,
            y,
            vx,
            vy,
            radius: 6.0,
            attached: false,
        }
    }

    #[test]
    fn fold_reflects_off_both_walls() {
        assert_eq!(fold(50.0, 0.0, 100.0), 50.0);
        assert_eq!(fold(120.0, 0.0, 100.0), 80.0);
        assert_eq!(fold(-30.0, 0.0, 100.0), 30.0);
        assert_eq!(fold(230.0, 0.0, 100.0), 30.0);
    }

    #[test]
    fn a_falling_ball_is_predicted_straight_down() {
        let b = ball(400.0, 300.0, 0.0, 300.0);
        let hit = predict_intercept(&b, 540.0).unwrap();
        assert!((hit.x - 400.0).abs() < 1e-3);
        assert!((hit.time - 0.8).abs() < 1e-4);
    }

    #[test]
    fn a_rising_ball_is_predicted_via_the_ceiling_and_a_wall() {
        // Up 294 px to the ceiling, down 534 px: 828 px at 300 px/s vertical,
        // 2.76 s, during which it drifts 828 px right from x = 700.
        let b = ball(700.0, 300.0, 300.0, -300.0);
        let hit = predict_intercept(&b, 540.0).unwrap();
        assert!((hit.time - 2.76).abs() < 1e-3);
        assert!((hit.x - fold(700.0 + 828.0, 6.0, 794.0)).abs() < 1e-2);
    }

    #[test]
    fn a_ball_past_the_line_or_on_the_paddle_has_no_intercept() {
        assert!(predict_intercept(&ball(400.0, 560.0, 0.0, 300.0), 540.0).is_none());
        let mut resting = ball(400.0, 540.0, 0.0, 0.0);
        resting.attached = true;
        assert!(predict_intercept(&resting, 540.0).is_none());
    }

    #[test]
    fn aim_offset_sends_the_ball_at_the_target_through_the_real_mapping() {
        // The angle the simulation itself produces for the chosen offset
        // must point at the target, on both sides of the hit point.
        for target in [(250.0, 200.0), (560.0, 150.0)] {
            let (hit_x, line_y) = (400.0, 540.0);
            let offset = aim_offset(hit_x, line_y, target);
            let angle = offset_to_exit_angle_deg(offset).to_radians();
            let x_at_target_height = hit_x + (line_y - target.1) / angle.tan();
            assert!(
                (x_at_target_height - target.0).abs() < 2.0,
                "offset {offset} lands at {x_at_target_height}, not {}",
                target.0
            );
        }
    }

    #[test]
    fn aim_offset_never_exceeds_the_safe_limit() {
        // Almost level with the hit point, far to the left.
        let offset = aim_offset(700.0, 540.0, (40.0, 500.0));
        assert_eq!(offset, -MAX_AIM_OFFSET);
    }

    #[test]
    fn paddle_x_for_puts_the_hit_point_at_the_offset_and_respects_walls() {
        let paddle = Paddle {
            x: 400.0,
            y: 562.0,
            width: 100.0,
            height: 16.0,
        };
        assert_eq!(paddle_x_for(400.0, 0.5, &paddle), 375.0);
        assert_eq!(paddle_x_for(10.0, 0.5, &paddle), 50.0);
    }

    #[test]
    fn serve_x_for_lines_up_a_60_degree_launch() {
        let paddle = Paddle {
            x: 400.0,
            y: 562.0,
            width: 100.0,
            height: 16.0,
        };
        let b = ball(400.0, 548.0, 0.0, 0.0);
        let target = (500.0, 200.0);
        let x = serve_x_for(target, &b, &paddle);
        let landed = x + (b.y - target.1) / 60f32.to_radians().tan();
        assert!((landed - target.0).abs() < 1e-2);
        // Too far left for any serve: the leftmost one.
        assert_eq!(serve_x_for((60.0, 200.0), &b, &paddle), 50.0);
    }

    #[test]
    fn steer_settles_inside_half_a_tick_of_travel() {
        let dt = 1.0 / 120.0;
        assert_eq!(steer(100.0, 200.0, dt), (false, true));
        assert_eq!(steer(200.0, 100.0, dt), (true, false));
        assert_eq!(steer(100.0, 101.0, dt), (false, false));
    }
}
