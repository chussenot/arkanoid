//! Pure simulation: `struct Game`, `fn tick(&mut self, input: &Input, dt_fixed)`.
//!
//! No wgpu, winit, or I/O types belong in this module — it must stay
//! unit-testable headless. Populated starting at Milestone 1.

use crate::events::{GameEvent, PowerUpKind};
use crate::levels::{self, BrickKind};

/// Player input for one fixed tick. Minimal for now; later milestones
/// (pause menu navigation, etc.) may extend this.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Input {
    pub left: bool,
    pub right: bool,
    pub space: bool,
    pub pause: bool,
}

/// Fixed logical playfield (spec: 800x600 — resize scales/letterboxes the
/// render, never the physics).
pub const PLAYFIELD_WIDTH: f32 = 800.0;
pub const PLAYFIELD_HEIGHT: f32 = 600.0;

const PADDLE_WIDTH: f32 = 100.0;
const PADDLE_HEIGHT: f32 = 16.0;
const PADDLE_SPEED: f32 = 520.0;
/// Gap between the paddle's bottom edge and the playfield's bottom edge.
const PADDLE_MARGIN_BOTTOM: f32 = 30.0;

const BALL_RADIUS: f32 = 6.0;
const BALL_LAUNCH_SPEED: f32 = 300.0;
const BALL_SPEED_CAP: f32 = 700.0;
const BALL_SPEED_GROWTH: f32 = 1.04;
/// Launch angle in degrees, math convention (90° = straight up, 0°/180° =
/// horizontal). Fixed direction (up-and-right); the spec only pins the
/// magnitude, not the side.
const BALL_LAUNCH_ANGLE_DEG: f32 = 60.0;

/// Exit-angle range for a paddle-hit reflection (same math convention as
/// the launch angle above), and the dead zone around 90° that keeps a
/// reflection from ever coming out perfectly vertical.
const EXIT_ANGLE_MIN_DEG: f32 = 30.0;
const EXIT_ANGLE_MAX_DEG: f32 = 150.0;
const EXIT_ANGLE_DEAD_ZONE_HALF_DEG: f32 = 5.0;

const STARTING_LIVES: u32 = 3;

/// Gap between the playfield's top edge and the brick grid, so the HUD's
/// level number (top-center, per spec) has room above the bricks.
const BRICK_GRID_MARGIN_TOP: f32 = 60.0;

// -- power-ups ------------------------------------------------------------

/// Chance that destroying a brick drops a power-up (spec: 15%).
const POWERUP_DROP_CHANCE: f64 = 0.15;
/// Fall speed for a dropped power-up (spec: 140 px/s).
const POWERUP_FALL_SPEED: f32 = 140.0;
/// Paddle width multiplier while Widen is active (spec: x1.5).
const WIDEN_MULTIPLIER: f32 = 1.5;
/// How long a Widen effect lasts before the paddle returns to base width
/// (spec: 15s). Re-catching Widen while one is already active resets this
/// back to the full duration rather than adding to it (spec: "timers
/// refresh, not stack-add").
const WIDEN_DURATION_SECS: f32 = 15.0;
/// Ball speed multiplier applied once when Slow is caught (spec: x0.7),
/// floored at `BALL_LAUNCH_SPEED` so it can never slow a ball below its
/// own launch speed.
const SLOW_MULTIPLIER: f32 = 0.7;
/// Angle (each way) the two balls spawned by Multiball fan out from the
/// source ball's own direction, so the fleet doesn't stay perfectly
/// overlapped -- three ball-shaped stacks moving as one -- for the rest of
/// the level.
const MULTIBALL_SPREAD_DEG: f32 = 20.0;

/// Top-level application state. Spec: "explicit enum-driven machine ...
/// no ad-hoc booleans for state." `Menu` and `Paused` join `Playing`/
/// `GameOver`/`Victory` this milestone, completing the full machine
/// (`Menu -> Playing <-> Paused -> GameOver/Victory`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GameState {
    #[default]
    Menu,
    Playing,
    Paused,
    GameOver,
    Victory,
}

/// Bottom-center paddle. Every field is `pub` so the render track (M2,
/// arkanoid-2g9) can read it directly to place a quad.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Paddle {
    /// Center x.
    pub x: f32,
    /// Center y (fixed — the paddle never moves vertically).
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Paddle {
    fn new() -> Self {
        Self {
            x: PLAYFIELD_WIDTH / 2.0,
            y: PLAYFIELD_HEIGHT - PADDLE_MARGIN_BOTTOM - PADDLE_HEIGHT / 2.0,
            width: PADDLE_WIDTH,
            height: PADDLE_HEIGHT,
        }
    }

    fn left(&self) -> f32 {
        self.x - self.width / 2.0
    }

    fn right(&self) -> f32 {
        self.x + self.width / 2.0
    }

    fn top(&self) -> f32 {
        self.y - self.height / 2.0
    }

    fn bottom(&self) -> f32 {
        self.y + self.height / 2.0
    }
}

/// The ball. `pub` fields for the same reason as [`Paddle`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ball {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub radius: f32,
    /// Resting on the paddle, waiting for Space to launch. While true the
    /// ball rides the paddle and `vx`/`vy` are ignored.
    pub attached: bool,
}

impl Ball {
    /// A ball resting centered on top of `paddle`.
    fn attached_to(paddle: &Paddle) -> Self {
        Self {
            x: paddle.x,
            y: paddle.top() - BALL_RADIUS,
            vx: 0.0,
            vy: 0.0,
            radius: BALL_RADIUS,
            attached: true,
        }
    }

    fn speed(&self) -> f32 {
        (self.vx * self.vx + self.vy * self.vy).sqrt()
    }
}

/// One brick on the field. `pub` fields for the same reason as
/// [`Paddle`]/[`Ball`] — the render track (arkanoid-chy) draws these
/// directly as quads, picking a color from `kind`/`hits_remaining`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Brick {
    /// Center x/y.
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub kind: BrickKind,
    /// Hits left before destruction: 1 for Normal, 2 for Armored — the
    /// first hit on an armored brick (2 -> 1) is the "changes color/state"
    /// moment the spec asks for; render can key its color off this number
    /// directly instead of a separate flag. Indestructible bricks carry a
    /// value here too but it's never read: they're never destroyed.
    pub hits_remaining: u8,
    /// Points awarded when this brick is destroyed (spec: scored by row,
    /// 10-70). Always 0 for Indestructible, which is never destroyed.
    pub score: u32,
}

impl Brick {
    fn left(&self) -> f32 {
        self.x - self.width / 2.0
    }

    fn right(&self) -> f32 {
        self.x + self.width / 2.0
    }

    fn top(&self) -> f32 {
        self.y - self.height / 2.0
    }

    fn bottom(&self) -> f32 {
        self.y + self.height / 2.0
    }
}

/// A power-up capsule falling from a destroyed brick, not yet caught or
/// missed. `pub` fields for the same reason as `Brick`/`Ball` — the render
/// track draws these as quads keyed on `kind`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PowerUp {
    /// Center x/y.
    pub x: f32,
    pub y: f32,
    pub kind: PowerUpKind,
}

/// Row-based points for a destroyed brick (spec: 10-70 pts by row). Row 0
/// is the top of the grid and scores highest — the classic Breakout
/// convention that bricks farther from the paddle are worth more — capped
/// at a floor of 10 so a level deeper than 7 rows (grid can go up to 8)
/// never scores below the spec's range.
fn score_for_row(row: usize) -> u32 {
    70u32.saturating_sub((row as u32) * 10).max(10)
}

/// Builds the brick layout for `levels::LEVELS[level_index]`, centering
/// the grid horizontally in the 800-wide playfield (levels can vary in
/// column count) and offsetting it down by `BRICK_GRID_MARGIN_TOP`.
fn build_bricks(level_index: usize) -> Vec<Brick> {
    let grid = levels::LEVELS[level_index];
    let cols = grid.first().map_or(0, |row| row.chars().count());
    let offset_x = (PLAYFIELD_WIDTH - cols as f32 * levels::BRICK_WIDTH) / 2.0;

    levels::parse_level(grid)
        .into_iter()
        .map(|spawn| {
            let row = (spawn.y / levels::BRICK_HEIGHT).round() as usize;
            let hits_remaining = match spawn.kind {
                BrickKind::Normal => 1,
                BrickKind::Armored => 2,
                BrickKind::Indestructible => 0,
            };
            let score = match spawn.kind {
                BrickKind::Indestructible => 0,
                _ => score_for_row(row),
            };
            Brick {
                x: spawn.x + offset_x + levels::BRICK_WIDTH / 2.0,
                y: spawn.y + BRICK_GRID_MARGIN_TOP + levels::BRICK_HEIGHT / 2.0,
                width: levels::BRICK_WIDTH,
                height: levels::BRICK_HEIGHT,
                kind: spawn.kind,
                hits_remaining,
                score,
            }
        })
        .collect()
}

/// True if `ball` (a circle) overlaps `brick` (an AABB) — closest-point-
/// on-rect test, the same technique `resolve_paddle_collision` uses.
fn ball_hits_brick(ball: &Ball, brick: &Brick) -> bool {
    let closest_x = ball.x.clamp(brick.left(), brick.right());
    let closest_y = ball.y.clamp(brick.top(), brick.bottom());
    let dx = ball.x - closest_x;
    let dy = ball.y - closest_y;
    dx * dx + dy * dy <= ball.radius * ball.radius
}

/// Reflects `ball`'s velocity off whichever face of `brick` it penetrated
/// least deeply (the nearest face, per spec), and nudges the ball back out
/// along that axis by the penetration depth so it doesn't immediately
/// re-collide on the next substep.
///
/// `overlap_x`/`overlap_y` come from treating the ball as its own AABB
/// (side `2*radius`) against `brick`'s AABB: whichever axis has the
/// smaller overlap is the one the ball most recently crossed into, hence
/// "nearest face". Both are guaranteed non-negative whenever
/// `ball_hits_brick` returned true for this pair (the circle-vs-rect
/// distance check is strictly tighter than this AABB approximation).
fn reflect_off_brick_face(ball: &mut Ball, brick: &Brick) {
    let overlap_x = (ball.radius + brick.width / 2.0) - (ball.x - brick.x).abs();
    let overlap_y = (ball.radius + brick.height / 2.0) - (ball.y - brick.y).abs();

    if overlap_x < overlap_y {
        ball.vx = -ball.vx;
        ball.x += if ball.x < brick.x {
            -overlap_x
        } else {
            overlap_x
        };
    } else {
        ball.vy = -ball.vy;
        ball.y += if ball.y < brick.y {
            -overlap_y
        } else {
            overlap_y
        };
    }
}

/// How many equal sub-steps to split one fixed tick's ball movement into,
/// so no single step moves the ball farther than its own radius (spec:
/// "substep the tick if displacement per tick exceeds ball radius" — this
/// is what actually prevents a fast ball from tunneling clean through a
/// brick without a collision check ever landing inside it).
fn substep_count(speed: f32, radius: f32, dt_fixed: f32) -> u32 {
    if radius <= 0.0 {
        return 1;
    }
    let travel = speed * dt_fixed;
    if travel <= radius {
        1
    } else {
        (travel / radius).ceil() as u32
    }
}

/// Linear map from paddle-hit offset to exit angle (spec: offset from
/// paddle center -> angle in [30°, 150°], clamped away from 90°±5° so a
/// reflection can never come out perfectly vertical).
///
/// `offset_norm` is the hit position relative to paddle center, normalized
/// by half the paddle width (so ±1.0 is the paddle's edges); values beyond
/// ±1.0 are clamped there first (the ball can be slightly outside the
/// paddle rect at the moment of collision).
///
/// Convention: angle is measured from the horizontal (90° = straight up,
/// 0°/180° = horizontal). A hit right of paddle center sends the ball out
/// below 90° (up-and-right); a hit left of center sends it out above 90°
/// (up-and-left).
fn offset_to_exit_angle_deg(offset_norm: f32) -> f32 {
    let offset_norm = offset_norm.clamp(-1.0, 1.0);
    let half_range = (EXIT_ANGLE_MAX_DEG - EXIT_ANGLE_MIN_DEG) / 2.0; // 60°, symmetric about 90°
    let angle = 90.0 - offset_norm * half_range;

    if (angle - 90.0).abs() < EXIT_ANGLE_DEAD_ZONE_HALF_DEG {
        if angle >= 90.0 {
            90.0 + EXIT_ANGLE_DEAD_ZONE_HALF_DEG
        } else {
            90.0 - EXIT_ANGLE_DEAD_ZONE_HALF_DEG
        }
    } else {
        angle
    }
}

/// Grow a ball's speed by one paddle hit's worth (+4%), capped (spec: cap
/// at 700 px/s). Once capped, further hits are no-ops — applying this
/// repeatedly never exceeds the cap.
fn grow_ball_speed(current_speed: f32) -> f32 {
    (current_speed * BALL_SPEED_GROWTH).min(BALL_SPEED_CAP)
}

/// Rotates a 2D velocity vector by `angle_deg` degrees. Only used to fan
/// Multiball's two spawned balls out from the source ball's own direction
/// so they don't travel as a perfectly overlapped stack; the rotation
/// direction (clockwise vs counter-clockwise on screen) doesn't matter for
/// that purpose, only that `+angle_deg` and `-angle_deg` diverge.
fn rotate_velocity(vx: f32, vy: f32, angle_deg: f32) -> (f32, f32) {
    let (sin, cos) = angle_deg.to_radians().sin_cos();
    (vx * cos - vy * sin, vx * sin + vy * cos)
}

/// Simulation state, advanced only by [`Game::tick`].
#[derive(Debug)]
pub struct Game {
    /// Events emitted during the most recent `tick()`. The caller
    /// (`main.rs`'s loop) drains this after every tick — see `events.rs`
    /// for the full emission/drain contract.
    pub events: Vec<GameEvent>,
    pub paddle: Paddle,
    /// The "main" ball. Always present while `state == Playing` (attached
    /// to the paddle pre-launch, or in flight). Kept as a single field
    /// rather than folded into a `Vec` so a fresh single-ball game keeps
    /// exactly the shape it always had; `extra_balls` is where Multiball's
    /// additional balls live.
    pub ball: Ball,
    /// Extra balls in flight from a caught Multiball power-up. The fleet
    /// currently in play is `ball` plus these. When `ball` is lost, one
    /// entry here (if any) is promoted into its place; a life is spent
    /// only once both `ball` is lost *and* this is empty (spec: "life is
    /// lost only when the LAST ball drops").
    pub extra_balls: Vec<Ball>,
    pub lives: u32,
    pub state: GameState,
    /// Every brick still standing on the current level, including
    /// indestructible ones (they stay forever, so they stay in this list).
    pub bricks: Vec<Brick>,
    pub score: u32,
    /// 1-based current level number (spec: HUD shows level number
    /// top-center; `levels::LEVELS[level - 1]` is the grid in play).
    pub level: usize,
    /// Power-ups currently falling, not yet caught or missed.
    pub powerups: Vec<PowerUp>,
    /// Seconds left on an active Widen effect; 0 means inactive (paddle at
    /// base width). Catching another Widen while this is already positive
    /// resets it back to `WIDEN_DURATION_SECS` rather than adding to it —
    /// this field *is* the timer, so "refresh not stack" is just "assign,
    /// don't add" at the call site (see `apply_powerup`).
    pub widen_timer: f32,
    /// Edge-detects `Input::pause`, which reports a held key state (like
    /// `left`/`right`), not a press event. Without this, holding Pause
    /// down would toggle Playing/Paused every tick for as long as the key
    /// stayed held. Not part of the `GameState` machine itself — purely
    /// input debouncing for the one control that toggles it.
    pause_was_held: bool,
}

impl Default for Game {
    fn default() -> Self {
        Self::new()
    }
}

impl Game {
    pub fn new() -> Self {
        let paddle = Paddle::new();
        let ball = Ball::attached_to(&paddle);
        Self {
            events: Vec::new(),
            paddle,
            ball,
            extra_balls: Vec::new(),
            lives: STARTING_LIVES,
            state: GameState::Menu,
            bricks: build_bricks(0),
            score: 0,
            level: 1,
            powerups: Vec::new(),
            widen_timer: 0.0,
            pause_was_held: false,
        }
    }

    /// Advance the simulation by one fixed timestep. Dispatches on
    /// `state` first -- the explicit state machine the spec asks for
    /// (`Menu -> Playing <-> Paused -> GameOver/Victory`), rather than a
    /// scatter of booleans layered on top of a single "is playing" path.
    pub fn tick(&mut self, input: &Input, dt_fixed: f32) {
        let pause_pressed = input.pause && !self.pause_was_held;
        self.pause_was_held = input.pause;

        match self.state {
            GameState::Menu => {
                if input.space {
                    self.state = GameState::Playing;
                }
            }
            GameState::Paused => {
                if pause_pressed {
                    self.state = GameState::Playing;
                }
            }
            GameState::GameOver | GameState::Victory => {
                // Terminal states: the simulation stays frozen. Whether a
                // key returns to the menu from here is a render/HUD
                // concern (arkanoid-p4f), not a simulation one.
            }
            GameState::Playing => {
                if pause_pressed {
                    self.state = GameState::Paused;
                    return;
                }
                self.tick_playing(input, dt_fixed);
            }
        }
    }

    /// The actual gameplay tick, run only while `state == Playing`.
    fn tick_playing(&mut self, input: &Input, dt_fixed: f32) {
        if self.widen_timer > 0.0 {
            self.widen_timer = (self.widen_timer - dt_fixed).max(0.0);
        }
        self.paddle.width = if self.widen_timer > 0.0 {
            PADDLE_WIDTH * WIDEN_MULTIPLIER
        } else {
            PADDLE_WIDTH
        };

        self.move_paddle(input, dt_fixed);

        if self.ball.attached {
            // Ride the paddle until launched.
            self.ball.x = self.paddle.x;
            self.ball.y = self.paddle.top() - self.ball.radius;
            if input.space {
                self.launch_ball();
            }
            return;
        }

        self.advance_balls(dt_fixed);
        self.update_powerups(dt_fixed);
    }

    /// Moves every ball in play (`ball` plus `extra_balls`) through one
    /// fixed tick, substepping if needed to keep each step's displacement
    /// within the ball radius (see `substep_count`) so a fast ball can't
    /// cross a brick's 22px span between two collision checks without
    /// either check landing inside it. Substep count is driven by the
    /// fastest ball in the fleet, so a solo fast ball during multiball is
    /// still covered.
    fn advance_balls(&mut self, dt_fixed: f32) {
        let max_speed = std::iter::once(self.ball.speed())
            .chain(self.extra_balls.iter().map(Ball::speed))
            .fold(0.0_f32, f32::max);
        let substeps = substep_count(max_speed, BALL_RADIUS, dt_fixed);
        let sub_dt = dt_fixed / substeps as f32;

        for _ in 0..substeps {
            let mut main_ball = self.ball;
            let main_lost = self.step_ball(&mut main_ball, sub_dt);
            self.ball = main_ball;

            let mut extra_balls = std::mem::take(&mut self.extra_balls);
            extra_balls.retain_mut(|ball| {
                let lost = self.step_ball(ball, sub_dt);
                if lost {
                    self.events.push(GameEvent::BallLost);
                }
                !lost
            });
            self.extra_balls = extra_balls;

            if self.destructible_bricks_remaining() == 0 {
                // Takes priority over any loss resolved below: clearing
                // the level already resets `ball`/`extra_balls` for the
                // next one (or ends the game in Victory).
                self.advance_level();
            } else if main_lost {
                self.events.push(GameEvent::BallLost);
                if let Some(promoted) = self.extra_balls.pop() {
                    // Multiball rule: another ball is still in play, so no
                    // life is spent -- it just takes over the main slot.
                    self.ball = promoted;
                } else {
                    self.lose_life();
                }
            }

            // A level-clear or life-loss reset above always leaves exactly
            // one freshly attached ball with no extras -- stop this tick's
            // remaining substeps in that case, same as a state change away
            // from Playing (GameOver/Victory).
            if self.state != GameState::Playing || self.ball.attached {
                break;
            }
        }
    }

    /// Moves `ball` by one substep and resolves wall/brick/paddle
    /// collisions against the current board. A destroyed brick updates
    /// `bricks`/`score`/`events` (and may roll a power-up drop) but never
    /// triggers a level-advance directly -- the caller checks
    /// `destructible_bricks_remaining()` once after every ball in the
    /// substep has moved, so two balls clearing the last two bricks in the
    /// same substep can't race each other into `advance_level` twice.
    ///
    /// Returns `true` if `ball` is now past the bottom edge (lost).
    fn step_ball(&mut self, ball: &mut Ball, sub_dt: f32) -> bool {
        ball.x += ball.vx * sub_dt;
        ball.y += ball.vy * sub_dt;

        resolve_wall_collisions(ball);
        self.resolve_brick_collisions(ball);
        self.resolve_paddle_collision(ball);

        ball.y - ball.radius > PLAYFIELD_HEIGHT
    }

    fn move_paddle(&mut self, input: &Input, dt_fixed: f32) {
        let dir = match (input.left, input.right) {
            (true, false) => -1.0,
            (false, true) => 1.0,
            _ => 0.0,
        };
        self.paddle.x += dir * PADDLE_SPEED * dt_fixed;
        let half = self.paddle.width / 2.0;
        self.paddle.x = self.paddle.x.clamp(half, PLAYFIELD_WIDTH - half);
    }

    fn launch_ball(&mut self) {
        self.ball.attached = false;
        let angle_rad = BALL_LAUNCH_ANGLE_DEG.to_radians();
        self.ball.vx = BALL_LAUNCH_SPEED * angle_rad.cos();
        self.ball.vy = -BALL_LAUNCH_SPEED * angle_rad.sin();
    }

    /// AABB-vs-circle collision against the paddle, resolved against the
    /// nearest face (closest-point-on-rect test). Only considered while
    /// the ball is moving downward — a ball moving up can't be hitting
    /// the paddle it just bounced off in the same tick.
    fn resolve_paddle_collision(&self, ball: &mut Ball) {
        if ball.vy <= 0.0 {
            return;
        }

        let closest_x = ball.x.clamp(self.paddle.left(), self.paddle.right());
        let closest_y = ball.y.clamp(self.paddle.top(), self.paddle.bottom());
        let dx = ball.x - closest_x;
        let dy = ball.y - closest_y;
        if dx * dx + dy * dy > ball.radius * ball.radius {
            return;
        }

        let half_width = self.paddle.width / 2.0;
        let offset_norm = ((ball.x - self.paddle.x) / half_width).clamp(-1.0, 1.0);
        let angle_rad = offset_to_exit_angle_deg(offset_norm).to_radians();
        let speed = grow_ball_speed(ball.speed());

        ball.vx = speed * angle_rad.cos();
        ball.vy = -speed * angle_rad.sin();
        // Push the ball back above the paddle so it doesn't immediately
        // re-collide next tick.
        ball.y = self.paddle.top() - ball.radius;
    }

    /// AABB-vs-circle collision against every brick, resolved against the
    /// nearest face. At most one brick is resolved per call (per substep,
    /// per ball) -- two bricks occupying the same few square pixels of
    /// ball travel within one substep isn't a real scenario at these
    /// speeds/sizes.
    fn resolve_brick_collisions(&mut self, ball: &mut Ball) {
        let Some(index) = self
            .bricks
            .iter()
            .position(|brick| ball_hits_brick(ball, brick))
        else {
            return;
        };

        reflect_off_brick_face(ball, &self.bricks[index]);

        if self.bricks[index].kind == BrickKind::Indestructible {
            return;
        }

        self.bricks[index].hits_remaining = self.bricks[index].hits_remaining.saturating_sub(1);
        if self.bricks[index].hits_remaining > 0 {
            // Armored brick's first hit: state changed (2 -> 1 hit left),
            // not destroyed yet -- no score, no event.
            return;
        }

        let brick = self.bricks.remove(index);
        self.score += brick.score;
        self.events.push(GameEvent::BrickDestroyed);
        self.maybe_drop_powerup(brick.x, brick.y);
    }

    /// Spec: 15% of destroyed bricks drop one of the three power-ups,
    /// uniformly at random, falling from where the brick was.
    fn maybe_drop_powerup(&mut self, x: f32, y: f32) {
        if !rand::random_bool(POWERUP_DROP_CHANCE) {
            return;
        }
        let kind = match rand::random_range(0..3) {
            0 => PowerUpKind::Widen,
            1 => PowerUpKind::Slow,
            _ => PowerUpKind::Multiball,
        };
        self.powerups.push(PowerUp { x, y, kind });
        self.events.push(GameEvent::PowerUpSpawned(kind));
    }

    fn destructible_bricks_remaining(&self) -> usize {
        self.bricks
            .iter()
            .filter(|b| b.kind != BrickKind::Indestructible)
            .count()
    }

    /// All destructible bricks on the current level are gone: load the
    /// next level, or declare Victory if that was the last one.
    fn advance_level(&mut self) {
        self.events.push(GameEvent::LevelCleared);

        if self.level >= levels::LEVELS.len() {
            self.state = GameState::Victory;
            self.events.push(GameEvent::Victory);
            return;
        }

        self.level += 1;
        self.bricks = build_bricks(self.level - 1);
        self.ball = Ball::attached_to(&self.paddle);
        self.extra_balls.clear();
        self.powerups.clear();
    }

    /// Spends one life. Callers only reach this once the whole fleet is
    /// gone -- `advance_balls` promotes an `extra_balls` entry into `ball`
    /// instead of calling this as long as any ball survives (spec: "life
    /// is lost only when the LAST ball drops"). Ends in GameOver if that
    /// was the last life, otherwise respawns a single fresh ball on the
    /// paddle.
    fn lose_life(&mut self) {
        self.events.push(GameEvent::LifeLost);
        self.lives = self.lives.saturating_sub(1);
        self.ball = Ball::attached_to(&self.paddle);
        self.extra_balls.clear();
        self.powerups.clear();

        if self.lives == 0 {
            self.state = GameState::GameOver;
            self.events.push(GameEvent::GameOver);
        }
    }

    /// Advances every falling power-up, applying/removing on catch and
    /// dropping the ones that reach the bottom uncaught.
    fn update_powerups(&mut self, dt_fixed: f32) {
        let mut powerups = std::mem::take(&mut self.powerups);
        powerups.retain_mut(|powerup| {
            powerup.y += POWERUP_FALL_SPEED * dt_fixed;

            if self.powerup_caught_by_paddle(powerup) {
                self.apply_powerup(powerup.kind);
                self.events.push(GameEvent::PowerUpCaught(powerup.kind));
                return false;
            }
            powerup.y <= PLAYFIELD_HEIGHT
        });
        self.powerups = powerups;
    }

    /// Point-in-rect: is `powerup`'s position within the paddle's bounds?
    /// A falling power-up has no footprint of its own worth modeling for
    /// catch purposes -- the paddle is the target and is comfortably
    /// larger than any reasonable icon.
    fn powerup_caught_by_paddle(&self, powerup: &PowerUp) -> bool {
        powerup.x >= self.paddle.left()
            && powerup.x <= self.paddle.right()
            && powerup.y >= self.paddle.top()
            && powerup.y <= self.paddle.bottom()
    }

    /// Applies one of the exactly-three power-up effects (spec: do not add
    /// a fourth -- this match has no wildcard arm specifically so adding a
    /// variant to `PowerUpKind` without handling it here fails to build).
    fn apply_powerup(&mut self, kind: PowerUpKind) {
        match kind {
            PowerUpKind::Widen => {
                self.widen_timer = WIDEN_DURATION_SECS;
            }
            PowerUpKind::Slow => {
                for ball in std::iter::once(&mut self.ball).chain(self.extra_balls.iter_mut()) {
                    let speed = ball.speed();
                    if speed <= 0.0 {
                        continue; // attached (or degenerate); no direction to scale.
                    }
                    let new_speed = (speed * SLOW_MULTIPLIER).max(BALL_LAUNCH_SPEED);
                    let scale = new_speed / speed;
                    ball.vx *= scale;
                    ball.vy *= scale;
                }
            }
            PowerUpKind::Multiball => {
                let source = self.ball;
                for sign in [-1.0_f32, 1.0_f32] {
                    let (vx, vy) =
                        rotate_velocity(source.vx, source.vy, sign * MULTIBALL_SPREAD_DEG);
                    self.extra_balls.push(Ball { vx, vy, ..source });
                }
            }
        }
    }
}

/// Bounce off the left/right/top playfield edges (bottom is handled
/// separately — it's a life loss, not a bounce). Pure function of `ball`
/// -- no board state involved.
fn resolve_wall_collisions(ball: &mut Ball) {
    if ball.x - ball.radius < 0.0 {
        ball.x = ball.radius;
        ball.vx = ball.vx.abs();
    } else if ball.x + ball.radius > PLAYFIELD_WIDTH {
        ball.x = PLAYFIELD_WIDTH - ball.radius;
        ball.vx = -ball.vx.abs();
    }
    if ball.y - ball.radius < 0.0 {
        ball.y = ball.radius;
        ball.vy = ball.vy.abs();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 120.0;

    fn held(input_field: impl Fn(&mut Input)) -> Input {
        let mut input = Input::default();
        input_field(&mut input);
        input
    }

    /// A fresh `Game` forced straight into `Playing`, skipping the Menu ->
    /// Playing transition -- for tests exercising in-flight gameplay
    /// mechanics, where the transition itself isn't what's under test
    /// (that has its own dedicated tests below).
    fn playing_game() -> Game {
        let mut game = Game::new();
        game.state = GameState::Playing;
        game
    }

    // -- offset -> exit angle mapping ---------------------------------

    #[test]
    fn exit_angle_maps_offset_linearly_between_the_30_and_150_degree_ends() {
        assert!((offset_to_exit_angle_deg(1.0) - EXIT_ANGLE_MIN_DEG).abs() < 1e-4);
        assert!((offset_to_exit_angle_deg(-1.0) - EXIT_ANGLE_MAX_DEG).abs() < 1e-4);
    }

    #[test]
    fn exit_angle_clamps_offsets_beyond_the_paddle_edges_to_the_range_ends() {
        // A hit registered outside the paddle rect (shouldn't normally
        // happen, but the mapping must not extrapolate past 30/150).
        assert!((offset_to_exit_angle_deg(5.0) - EXIT_ANGLE_MIN_DEG).abs() < 1e-4);
        assert!((offset_to_exit_angle_deg(-5.0) - EXIT_ANGLE_MAX_DEG).abs() < 1e-4);
    }

    #[test]
    fn exit_angle_clamps_away_from_the_90_degree_dead_zone() {
        // Dead center: would land exactly on 90° (perfectly vertical) —
        // must be pushed to a dead-zone boundary instead.
        let angle = offset_to_exit_angle_deg(0.0);
        assert!((angle - 90.0).abs() >= EXIT_ANGLE_DEAD_ZONE_HALF_DEG - 1e-4);

        // Anything that maps inside (85°, 95°) gets clamped to whichever
        // boundary is on the same side.
        let angle = offset_to_exit_angle_deg(0.02); // maps to 90 - 1.2 = 88.8°
        assert!((angle - (90.0 - EXIT_ANGLE_DEAD_ZONE_HALF_DEG)).abs() < 1e-4);
        let angle = offset_to_exit_angle_deg(-0.02); // maps to 91.2°
        assert!((angle - (90.0 + EXIT_ANGLE_DEAD_ZONE_HALF_DEG)).abs() < 1e-4);
    }

    #[test]
    fn exit_angle_right_at_the_dead_zone_boundary_is_not_clamped() {
        // offset_norm chosen so the raw angle lands exactly on 85° — the
        // boundary itself is allowed, only the open interval inside it
        // isn't.
        let offset_norm = (90.0 - 85.0) / 60.0;
        let angle = offset_to_exit_angle_deg(offset_norm);
        assert!((angle - 85.0).abs() < 1e-4);
    }

    // -- ball speed cap -------------------------------------------------

    #[test]
    fn ball_speed_grows_by_4_percent_per_hit_below_the_cap() {
        let grown = grow_ball_speed(BALL_LAUNCH_SPEED);
        assert!((grown - BALL_LAUNCH_SPEED * 1.04).abs() < 1e-3);
    }

    #[test]
    fn ball_speed_never_exceeds_the_700_cap_even_after_many_hits() {
        let mut speed = BALL_LAUNCH_SPEED;
        for _ in 0..200 {
            speed = grow_ball_speed(speed);
            assert!(speed <= BALL_SPEED_CAP);
        }
        assert!((speed - BALL_SPEED_CAP).abs() < 1e-4);
    }

    #[test]
    fn ball_speed_growth_clamps_a_single_hit_that_would_overshoot_the_cap() {
        assert_eq!(grow_ball_speed(690.0), BALL_SPEED_CAP);
    }

    // -- state machine: Menu / Paused ------------------------------------

    #[test]
    fn game_starts_in_the_menu_state() {
        let game = Game::new();
        assert_eq!(game.state, GameState::Menu);
    }

    #[test]
    fn menu_ignores_movement_input_and_only_space_starts_play() {
        let mut game = Game::new();
        game.tick(&held(|i| i.left = true), DT);
        assert_eq!(game.state, GameState::Menu);

        game.tick(&held(|i| i.space = true), DT);
        assert_eq!(game.state, GameState::Playing);
    }

    #[test]
    fn pause_toggles_on_press_not_on_hold() {
        let mut game = playing_game();
        let press_pause = held(|i| i.pause = true);

        game.tick(&press_pause, DT);
        assert_eq!(game.state, GameState::Paused, "first press pauses");

        // Holding the key down must not re-toggle on the very next tick.
        game.tick(&press_pause, DT);
        assert_eq!(game.state, GameState::Paused, "held key doesn't re-toggle");

        game.tick(&Input::default(), DT); // release
        game.tick(&press_pause, DT); // press again
        assert_eq!(game.state, GameState::Playing, "second press resumes");
    }

    #[test]
    fn paused_freezes_the_simulation() {
        let mut game = playing_game();
        game.state = GameState::Paused;
        let paddle_x_before = game.paddle.x;

        game.tick(&held(|i| i.right = true), DT);

        assert_eq!(game.paddle.x, paddle_x_before);
    }

    // -- life loss / game over -----------------------------------------

    #[test]
    fn ball_past_the_bottom_edge_costs_a_life_and_reattaches_the_ball() {
        let mut game = playing_game();
        game.ball.attached = false;
        game.ball.x = 400.0;
        game.ball.y = PLAYFIELD_HEIGHT + game.ball.radius + 1.0;
        game.ball.vy = 100.0; // moving down when it happened

        game.tick(&Input::default(), DT);

        assert_eq!(game.lives, 2);
        assert_eq!(game.state, GameState::Playing);
        assert!(game.ball.attached);
        assert!(game.events.contains(&GameEvent::BallLost));
        assert!(game.events.contains(&GameEvent::LifeLost));
    }

    #[test]
    fn losing_the_last_life_transitions_to_game_over() {
        let mut game = playing_game();
        for _ in 0..3 {
            game.events.clear();
            game.ball.attached = false;
            game.ball.x = 400.0;
            game.ball.y = PLAYFIELD_HEIGHT + game.ball.radius + 1.0;
            game.ball.vy = 100.0;
            game.tick(&Input::default(), DT);
        }

        assert_eq!(game.lives, 0);
        assert_eq!(game.state, GameState::GameOver);
        assert!(game.events.contains(&GameEvent::GameOver));
    }

    #[test]
    fn game_over_freezes_the_simulation() {
        let mut game = Game::new();
        game.lives = 0;
        game.state = GameState::GameOver;
        let paddle_x_before = game.paddle.x;

        game.tick(&held(|i| i.right = true), DT);

        assert_eq!(game.paddle.x, paddle_x_before);
    }

    // -- playability: a scripted Input sequence over many ticks ---------

    #[test]
    fn scripted_input_sequence_moves_paddle_and_launches_the_ball() {
        let mut game = playing_game();
        let start_x = game.paddle.x;

        let move_left = held(|i| i.left = true);
        for _ in 0..30 {
            game.tick(&move_left, DT);
        }
        assert!(game.paddle.x < start_x, "paddle should have moved left");
        assert!(game.ball.attached, "ball still rides the paddle pre-launch");

        game.tick(&held(|i| i.space = true), DT);
        assert!(!game.ball.attached, "space launches the ball");
        assert!(game.ball.vy < 0.0, "launch goes upward");
        assert!(
            (game.ball.speed() - BALL_LAUNCH_SPEED).abs() < 1e-3,
            "launches at the spec'd 300 px/s"
        );
    }

    #[test]
    fn paddle_movement_is_clamped_to_the_playfield_walls() {
        let mut game = playing_game();
        let hold_left = held(|i| i.left = true);
        for _ in 0..1000 {
            game.tick(&hold_left, DT);
        }
        assert_eq!(game.paddle.x, game.paddle.width / 2.0);

        let hold_right = held(|i| i.right = true);
        for _ in 0..2000 {
            game.tick(&hold_right, DT);
        }
        assert_eq!(game.paddle.x, PLAYFIELD_WIDTH - game.paddle.width / 2.0);
    }

    // -- wall bounces ----------------------------------------------------

    #[test]
    fn ball_bounces_off_left_right_and_top_walls() {
        let mut game = playing_game();
        game.ball.attached = false;

        game.ball.x = game.ball.radius - 1.0;
        game.ball.y = 300.0;
        game.ball.vx = -100.0;
        game.ball.vy = 0.0;
        game.tick(&Input::default(), DT);
        assert!(game.ball.vx > 0.0, "left wall reflects vx positive");

        game.ball.x = PLAYFIELD_WIDTH - game.ball.radius + 1.0;
        game.ball.vx = 100.0;
        game.tick(&Input::default(), DT);
        assert!(game.ball.vx < 0.0, "right wall reflects vx negative");

        game.ball.x = 400.0;
        game.ball.y = game.ball.radius - 1.0;
        game.ball.vx = 0.0;
        game.ball.vy = -100.0;
        game.tick(&Input::default(), DT);
        assert!(game.ball.vy > 0.0, "top wall reflects vy positive");
    }

    // -- paddle reflection integration ------------------------------------

    #[test]
    fn ball_hitting_the_paddle_bounces_up_and_speeds_up() {
        let mut game = playing_game();
        game.ball.attached = false;
        game.ball.x = game.paddle.x; // dead center hit
        game.ball.y = game.paddle.top() - game.ball.radius;
        game.ball.vx = 0.0;
        game.ball.vy = BALL_LAUNCH_SPEED; // moving down into the paddle

        game.tick(&Input::default(), DT);

        assert!(game.ball.vy < 0.0, "reflects back upward");
        assert!(
            (game.ball.speed() - grow_ball_speed(BALL_LAUNCH_SPEED)).abs() < 1e-2,
            "speed grew by the 4% hit bonus"
        );
    }

    #[test]
    fn tick_can_be_called_repeatedly_without_panicking() {
        let mut game = playing_game();
        let input = Input::default();
        for _ in 0..1000 {
            game.tick(&input, DT);
        }
    }

    // -- bricks: hit points, scoring, armored two-hit behavior ----------

    /// A ball parked just below `brick`'s bottom face, moving straight up
    /// into it -- the next `tick()` is guaranteed to register a hit on
    /// that face (mirrors how `ball_hitting_the_paddle_...` above sets up
    /// its paddle-collision test).
    fn approach_from_below(brick: &Brick) -> Ball {
        Ball {
            x: brick.x,
            y: brick.bottom() + BALL_RADIUS,
            vx: 0.0,
            vy: -300.0,
            radius: BALL_RADIUS,
            attached: false,
        }
    }

    fn test_brick_at(x: f32, y: f32, kind: BrickKind, hits_remaining: u8, score: u32) -> Brick {
        Brick {
            x,
            y,
            width: levels::BRICK_WIDTH,
            height: levels::BRICK_HEIGHT,
            kind,
            hits_remaining,
            score,
        }
    }

    fn test_brick(kind: BrickKind, hits_remaining: u8, score: u32) -> Brick {
        test_brick_at(400.0, 300.0, kind, hits_remaining, score)
    }

    /// A second brick placed well away from the ball's path in these
    /// tests, purely so destroying the *target* brick doesn't also empty
    /// `game.bricks` and trigger a real level-completion transition --
    /// that's covered by its own dedicated tests below.
    fn decoy_brick() -> Brick {
        test_brick_at(50.0, 50.0, BrickKind::Normal, 1, 10)
    }

    #[test]
    fn score_for_row_runs_70_at_the_top_down_to_a_floor_of_10() {
        assert_eq!(score_for_row(0), 70);
        assert_eq!(score_for_row(1), 60);
        assert_eq!(score_for_row(5), 20);
        assert_eq!(score_for_row(6), 10);
        // A grid deeper than 7 rows (spec allows up to 8) must not score
        // below the spec's 10-70 range.
        assert_eq!(score_for_row(7), 10);
        assert_eq!(score_for_row(100), 10);
    }

    #[test]
    fn normal_brick_is_destroyed_in_one_hit_and_scores_by_row() {
        let mut game = playing_game();
        let brick = test_brick(BrickKind::Normal, 1, 40);
        game.bricks = vec![decoy_brick(), brick];
        game.ball = approach_from_below(&brick);

        game.tick(&Input::default(), DT);

        assert_eq!(game.bricks.len(), 1, "one hit destroys a normal brick");
        assert_eq!(game.bricks[0], decoy_brick(), "the decoy is untouched");
        assert_eq!(game.score, 40, "scored the brick's row value");
        assert!(game.events.contains(&GameEvent::BrickDestroyed));
        assert!(game.ball.vy > 0.0, "reflects back down off the brick");
    }

    #[test]
    fn armored_brick_survives_one_hit_and_is_destroyed_by_a_second() {
        let mut game = playing_game();
        let brick = test_brick(BrickKind::Armored, 2, 50);
        game.bricks = vec![decoy_brick(), brick];
        game.ball = approach_from_below(&brick);

        game.tick(&Input::default(), DT);

        assert_eq!(game.bricks.len(), 2, "armored brick survives the first hit");
        let armored = game.bricks.iter().find(|b| b.kind == BrickKind::Armored);
        assert_eq!(
            armored.map(|b| b.hits_remaining),
            Some(1),
            "first hit changes its state (2 -> 1 hits left)"
        );
        assert!(!game.events.contains(&GameEvent::BrickDestroyed));
        assert_eq!(game.score, 0, "not scored until actually destroyed");

        game.events.clear();
        let armored = *armored.unwrap();
        game.ball = approach_from_below(&armored);
        game.tick(&Input::default(), DT);

        assert_eq!(game.bricks.len(), 1, "second hit destroys it");
        assert_eq!(game.bricks[0], decoy_brick(), "only the decoy is left");
        assert_eq!(game.score, 50);
        assert!(game.events.contains(&GameEvent::BrickDestroyed));
    }

    #[test]
    fn indestructible_brick_is_never_destroyed_and_does_not_block_level_completion() {
        let mut game = playing_game();
        let wall = test_brick_at(100.0, 100.0, BrickKind::Indestructible, 0, 0);
        let normal = test_brick_at(400.0, 300.0, BrickKind::Normal, 1, 30);
        game.bricks = vec![wall, normal];
        // Pretend this is the last level so clearing it is observable as
        // an immediate, unambiguous state transition.
        game.level = levels::LEVELS.len();
        game.ball = approach_from_below(&normal);

        game.tick(&Input::default(), DT);

        assert_eq!(
            game.bricks.len(),
            1,
            "only the indestructible brick is left"
        );
        assert_eq!(game.bricks[0].kind, BrickKind::Indestructible);
        assert_eq!(
            game.state,
            GameState::Victory,
            "the indestructible brick must not block level completion"
        );
    }

    // -- level load / progression ----------------------------------------

    #[test]
    fn new_game_starts_on_level_1_with_its_bricks_loaded() {
        let game = Game::new();
        assert_eq!(game.level, 1);
        assert!(!game.bricks.is_empty());
    }

    #[test]
    fn clearing_all_destructible_bricks_advances_to_the_next_level() {
        let mut game = playing_game();
        let brick = test_brick(BrickKind::Normal, 1, 10);
        game.bricks = vec![brick];
        game.ball = approach_from_below(&brick);

        game.tick(&Input::default(), DT);

        assert_eq!(game.level, 2, "level 1 cleared -> now on level 2");
        assert_eq!(game.state, GameState::Playing);
        assert!(!game.bricks.is_empty(), "level 2's bricks are loaded");
        assert!(game.ball.attached, "ball re-attaches for the new level");
        assert!(game.events.contains(&GameEvent::LevelCleared));
    }

    #[test]
    fn clearing_the_final_level_transitions_to_victory_instead_of_loading_another() {
        let mut game = playing_game();
        game.level = levels::LEVELS.len();
        let brick = test_brick(BrickKind::Normal, 1, 10);
        game.bricks = vec![brick];
        game.ball = approach_from_below(&brick);

        game.tick(&Input::default(), DT);

        assert_eq!(game.state, GameState::Victory);
        assert!(game.events.contains(&GameEvent::Victory));
    }

    // -- tunneling prevention: substep mechanics --------------------------

    #[test]
    fn substep_count_is_1_when_a_ticks_travel_fits_within_the_ball_radius() {
        // The real game's numbers: 700 px/s capped speed, 120 Hz tick.
        assert_eq!(substep_count(BALL_SPEED_CAP, BALL_RADIUS, DT), 1);
    }

    #[test]
    fn substep_count_scales_up_so_each_step_never_exceeds_the_radius() {
        let speed = 2000.0; // hypothetical, well beyond the real 700 cap
        let count = substep_count(speed, BALL_RADIUS, DT);
        assert!(count > 1, "a large enough travel must be split up");
        let per_step_travel = speed * DT / count as f32;
        assert!(per_step_travel <= BALL_RADIUS + 1e-4);
    }

    #[test]
    fn substep_count_does_not_divide_by_a_zero_radius() {
        assert_eq!(substep_count(BALL_SPEED_CAP, 0.0, DT), 1);
    }

    // -- tunneling prevention: property test (randomized angles) ---------

    #[test]
    fn ball_at_max_speed_never_tunnels_through_a_brick_at_any_approach_angle() {
        // Property required by the spec: a ball at the fastest speed the
        // game allows (700 px/s) approaching a standard 52x22 brick from
        // any direction is always caught -- it never ends up on the far
        // side of the brick without a collision ever having been
        // detected. Runs the same `substep_count`/`ball_hits_brick`
        // production functions `advance_balls` uses, just without a full
        // `Game` around them.
        let brick = test_brick(BrickKind::Normal, 1, 10);

        const SAMPLES: usize = 300;
        const START_DISTANCE: f32 = 150.0;

        for _ in 0..SAMPLES {
            let angle: f32 = rand::random_range(0.0..std::f32::consts::TAU);
            let (dx, dy) = (angle.cos(), angle.sin());

            let mut ball = Ball {
                x: brick.x - dx * START_DISTANCE,
                y: brick.y - dy * START_DISTANCE,
                vx: dx * BALL_SPEED_CAP,
                vy: dy * BALL_SPEED_CAP,
                radius: BALL_RADIUS,
                attached: false,
            };

            // Enough ticks to cross well past the brick if it were never
            // caught at all.
            let ticks = ((START_DISTANCE * 3.0) / (BALL_SPEED_CAP * DT)).ceil() as u32 + 1;
            let mut hit = false;
            'ticking: for _ in 0..ticks {
                let substeps = substep_count(ball.speed(), ball.radius, DT);
                let sub_dt = DT / substeps as f32;
                for _ in 0..substeps {
                    ball.x += ball.vx * sub_dt;
                    ball.y += ball.vy * sub_dt;
                    if ball_hits_brick(&ball, &brick) {
                        hit = true;
                        break 'ticking;
                    }
                }
            }

            assert!(
                hit,
                "ball approaching from angle {angle} at {BALL_SPEED_CAP} px/s tunneled through a {}x{} brick uncaught",
                levels::BRICK_WIDTH,
                levels::BRICK_HEIGHT
            );
        }
    }

    // -- power-ups: drop chance -------------------------------------------

    #[test]
    fn brick_destruction_drops_one_of_the_three_powerups_about_15_percent_of_the_time() {
        let mut game = playing_game();
        let mut spawned = 0;
        for _ in 0..2000 {
            game.powerups.clear();
            game.maybe_drop_powerup(400.0, 300.0);
            if let Some(powerup) = game.powerups.first() {
                spawned += 1;
                assert!(matches!(
                    powerup.kind,
                    PowerUpKind::Widen | PowerUpKind::Slow | PowerUpKind::Multiball
                ));
            }
        }
        // Loose bounds around the spec'd 15% of 2000 (expect ~300, std
        // dev ~16) -- wide enough to never flake, tight enough to catch a
        // badly broken roll (e.g. always/never dropping).
        assert!(
            (200..=400).contains(&spawned),
            "expected roughly 15% of 2000 destroyed bricks to drop a power-up, got {spawned}"
        );
    }

    // -- power-ups: Widen (refresh, not stack) -----------------------------

    #[test]
    fn widen_powerup_grows_the_paddle_for_its_duration() {
        let mut game = playing_game();
        game.apply_powerup(PowerUpKind::Widen);

        game.tick(&Input::default(), DT);

        assert!((game.widen_timer - (WIDEN_DURATION_SECS - DT)).abs() < 1e-4);
        assert!((game.paddle.width - PADDLE_WIDTH * WIDEN_MULTIPLIER).abs() < 1e-4);
    }

    #[test]
    fn widen_timer_refreshes_on_recatch_instead_of_stacking() {
        let mut game = playing_game();
        game.apply_powerup(PowerUpKind::Widen);
        game.widen_timer = 2.0; // simulate it almost expired

        game.apply_powerup(PowerUpKind::Widen); // caught again

        assert_eq!(
            game.widen_timer, WIDEN_DURATION_SECS,
            "re-catching resets the timer back to full duration instead of adding to it"
        );
    }

    #[test]
    fn widen_effect_ends_and_paddle_returns_to_base_width_once_the_timer_expires() {
        let mut game = playing_game();
        game.apply_powerup(PowerUpKind::Widen);

        let ticks = (WIDEN_DURATION_SECS / DT).ceil() as u32 + 1;
        for _ in 0..ticks {
            game.tick(&Input::default(), DT);
        }

        assert_eq!(game.widen_timer, 0.0);
        assert_eq!(game.paddle.width, PADDLE_WIDTH);
    }

    // -- power-ups: Slow (once, floored at launch speed) -------------------

    #[test]
    fn slow_powerup_reduces_ball_speed_by_30_percent_once() {
        let mut game = playing_game();
        game.ball.attached = false;
        game.ball.vx = BALL_SPEED_CAP;
        game.ball.vy = 0.0;

        game.apply_powerup(PowerUpKind::Slow);

        assert!((game.ball.speed() - BALL_SPEED_CAP * SLOW_MULTIPLIER).abs() < 1e-2);
    }

    #[test]
    fn slow_powerup_never_drops_the_ball_below_launch_speed() {
        let mut game = playing_game();
        game.ball.attached = false;
        game.ball.vx = BALL_LAUNCH_SPEED; // already at the floor
        game.ball.vy = 0.0;

        game.apply_powerup(PowerUpKind::Slow);

        assert!((game.ball.speed() - BALL_LAUNCH_SPEED).abs() < 1e-2);
    }

    // -- power-ups: Multiball / last-ball-loses-life rule -------------------

    #[test]
    fn multiball_powerup_spawns_two_extra_balls_at_the_current_balls_position() {
        let mut game = playing_game();
        game.ball.attached = false;
        game.ball.x = 300.0;
        game.ball.y = 200.0;
        game.ball.vx = 100.0;
        game.ball.vy = -200.0;

        game.apply_powerup(PowerUpKind::Multiball);

        assert_eq!(game.extra_balls.len(), 2);
        for extra in &game.extra_balls {
            assert!((extra.x - 300.0).abs() < 1e-4);
            assert!((extra.y - 200.0).abs() < 1e-4);
            assert!(!extra.attached);
        }
    }

    #[test]
    fn a_life_is_lost_only_when_the_last_ball_of_a_multiball_fleet_drops() {
        let mut game = playing_game();
        game.ball.attached = false;
        game.ball.vx = 0.0;
        game.ball.vy = 100.0;
        game.apply_powerup(PowerUpKind::Multiball); // 3 balls total now

        assert_eq!(game.lives, 3);

        // Drop the main ball and one extra past the bottom edge; the
        // second extra survives safely in the middle of the field.
        game.ball.y = PLAYFIELD_HEIGHT + game.ball.radius + 1.0;
        game.extra_balls[0].y = PLAYFIELD_HEIGHT + game.ball.radius + 1.0;
        game.extra_balls[0].vy = 100.0;
        game.extra_balls[1].y = 300.0;
        game.extra_balls[1].vy = 0.0;
        game.extra_balls[1].vx = 0.0;

        game.tick(&Input::default(), DT);

        assert_eq!(
            game.lives, 3,
            "a life survives while any ball is still in play"
        );
        assert_eq!(game.state, GameState::Playing);
        assert!(
            !game.ball.attached,
            "the surviving ball is promoted into the main slot"
        );
        assert!(game.extra_balls.is_empty());

        // Now drop that last remaining ball too.
        game.ball.y = PLAYFIELD_HEIGHT + game.ball.radius + 1.0;
        game.ball.vy = 100.0;
        game.tick(&Input::default(), DT);

        assert_eq!(game.lives, 2, "life is spent only once the LAST ball drops");
        assert!(game.ball.attached, "a fresh ball re-attaches to the paddle");
    }

    // -- power-ups: catch / miss integration --------------------------------

    #[test]
    fn falling_powerup_caught_by_the_paddle_applies_its_effect_and_fires_an_event() {
        let mut game = playing_game();
        game.ball.attached = false;
        game.ball.y = 50.0; // parked away from the paddle/bricks
        game.ball.vx = 0.0;
        game.ball.vy = 0.0;
        game.powerups.push(PowerUp {
            x: game.paddle.x,
            y: game.paddle.top(),
            kind: PowerUpKind::Widen,
        });

        game.tick(&Input::default(), DT);

        assert!(
            game.powerups.is_empty(),
            "caught power-up is removed from the falling list"
        );
        assert!(game
            .events
            .contains(&GameEvent::PowerUpCaught(PowerUpKind::Widen)));
        assert!(game.widen_timer > 0.0);
    }

    #[test]
    fn falling_powerup_that_reaches_the_bottom_uncaught_is_simply_removed() {
        let mut game = playing_game();
        game.ball.attached = false;
        game.ball.y = 50.0;
        game.ball.vx = 0.0;
        game.ball.vy = 0.0;
        game.powerups.push(PowerUp {
            x: 10.0, // far from the paddle
            y: PLAYFIELD_HEIGHT + 1.0,
            kind: PowerUpKind::Slow,
        });

        game.tick(&Input::default(), DT);

        assert!(game.powerups.is_empty());
        assert!(!game
            .events
            .contains(&GameEvent::PowerUpCaught(PowerUpKind::Slow)));
    }
}
