//! Pure simulation: `struct Game`, `fn tick(&mut self, input: &Input, dt_fixed)`.
//!
//! No wgpu, winit, or I/O types belong in this module — it must stay
//! unit-testable headless. Populated starting at Milestone 1.

use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};

use crate::events::{GameEvent, PowerUpKind};
use crate::levels::{self, BrickKind};
use crate::levelset::{ExternalBrickSpawn, ExternalLevel, GRID_WIDTH};

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
/// `pub` for the autopilot (`src/autopilot/pilot.rs`), which plans paddle
/// moves against the same speed the simulation applies.
pub const PADDLE_SPEED: f32 = 520.0;
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
/// Most lives the Player power-up can bring the count to: the HUD's row of
/// life icons has room for this many.
pub const MAX_LIVES: u32 = 9;

/// Gap between the playfield's top edge and the brick grid, so the HUD's
/// level number (top-center, per spec) has room above the bricks.
const BRICK_GRID_MARGIN_TOP: f32 = 60.0;

// -- power-ups ------------------------------------------------------------

/// Chance that destroying a brick drops a power-up (spec: 15%).
const POWERUP_DROP_CHANCE: f64 = 0.15;
/// Fall speed for a dropped power-up (spec: 140 px/s).
/// `pub` for the autopilot, which times a catch against it.
pub const POWERUP_FALL_SPEED: f32 = 140.0;
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
/// How long Glue stays active once caught. Like Widen, re-catching it while
/// active resets the timer rather than adding to it.
const GLUE_DURATION_SECS: f32 = 15.0;
/// Longest a ball stays stuck to the paddle before it releases on its own,
/// so a held ball can never stall the game.
pub const GLUE_HOLD_SECS: f32 = 3.0;

/// Paddle width multiplier while Tiny is active, and for how long. Tiny and
/// Widen cancel each other: catching one ends the other, so the paddle is
/// only ever one size off.
const TINY_MULTIPLIER: f32 = 0.6;
const TINY_DURATION_SECS: f32 = 15.0;

/// Points for leaving a level through the Break gate.
const WARP_SCORE: u32 = 10_000;
/// The Break gate's span on the right wall: from this far above the
/// paddle's center to the bottom of the field, so the paddle meets it
/// where it already moves.
pub const GATE_REACH_ABOVE_PADDLE: f32 = 40.0;

// -- lasers ---------------------------------------------------------------

/// How long a gun (Laser or Super laser) lasts once caught. Catching either
/// while one is active replaces it, timer full again.
const GUN_DURATION_SECS: f32 = 15.0;
/// Laser: a pair of bolts from near the paddle's ends, each giving the
/// first brick it meets one hit, as a ball would.
const LASER_COOLDOWN_SECS: f32 = 0.3;
const LASER_SPEED: f32 = 800.0;
const LASER_HALF_SIZE: [f32; 2] = [2.0, 7.0];
/// Laser bolts leave this far in from each end of the paddle.
const LASER_INSET: f32 = 8.0;
/// Super laser: one wide bolt from the paddle's center that destroys every
/// destructible brick in its way, armored ones outright, until an
/// indestructible brick stops it.
const SUPER_LASER_COOLDOWN_SECS: f32 = 0.5;
const SUPER_LASER_SPEED: f32 = 1000.0;
const SUPER_LASER_HALF_SIZE: [f32; 2] = [5.0, 12.0];
/// Bolts in flight at once, at most: a bound, not a rate (the cooldown is).
const MAX_BOLTS: usize = 16;

// -- stalls ---------------------------------------------------------------

/// Seconds of flight without any ball touching the paddle, breaking or
/// denting a brick, or blowing up an asteroid, after which the shot counts
/// as stalled: a ball can settle into a closed orbit among indestructible
/// bricks (level 3's box is one place) that no input can reach.
const STALL_SECS: f32 = 10.0;
/// While stalled, each wall or indestructible-brick bounce turns the ball
/// by this much, alternating direction, until something gives.
const STALL_NUDGE_DEG: f32 = 7.0;
/// A nudge never leaves the ball flatter than this from the horizontal,
/// so it cannot turn a stall into a ball skimming sideways forever.
const STALL_MIN_ANGLE_DEG: f32 = 20.0;

// -- asteroids ------------------------------------------------------------

/// Asteroids alive at once, at most.
const MAX_ASTEROIDS: usize = 3;
pub const ASTEROID_RADIUS: f32 = 10.0;
/// Downward drift: slow enough that an asteroid crosses the field in about
/// fifteen seconds, spending most of it in the ball's lanes.
const ASTEROID_FALL_SPEED: f32 = 40.0;
/// Sideways speed: a base drift that flips at the walls, plus a wobble.
const ASTEROID_DRIFT_SPEED: f32 = 30.0;
const ASTEROID_WOBBLE_SPEED: f32 = 45.0;
/// Seconds of play (ball in flight) before a level's first asteroid, then
/// between spawns: uniformly in this range.
const ASTEROID_FIRST_SPAWN_SECS: f32 = 4.0;
const ASTEROID_SPAWN_SECS: std::ops::Range<f32> = 5.0..10.0;
/// Points for blowing one up, with the ball or the paddle.
const ASTEROID_SCORE: u32 = 100;
/// An asteroid's explosion hits every brick within this distance of its
/// center (closest point of the brick): a little under a brick's width,
/// so a blast takes out a small cluster, never a whole row.
pub const ASTEROID_BLAST_RADIUS: f32 = 40.0;

/// Gameplay added after the v1 spec. Each can be switched off, so v1's
/// rules (and its pinned replay hash) stay reproducible: `--classic` plays
/// with none of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Features {
    /// The Glue power-up: a ball hitting the paddle sticks to it until
    /// Space releases it, aimed by where it landed.
    pub glue: bool,
    /// Asteroids drifting in from the top, blowing up when a ball or the
    /// paddle hits them.
    pub asteroids: bool,
    /// The Laser and Super laser power-ups: the paddle fires bolts.
    pub lasers: bool,
    /// The Tiny power-up, a bad one: the paddle shrinks.
    pub tiny: bool,
    /// The Player power-up: an extra life.
    pub player: bool,
    /// The Break power-up: a gate opens in the right wall, and sliding the
    /// paddle through it skips to the next level.
    pub warp: bool,
}

impl Features {
    pub const ALL: Self = Self {
        glue: true,
        asteroids: true,
        lasers: true,
        tiny: true,
        player: true,
        warp: true,
    };
    pub const V1: Self = Self {
        glue: false,
        asteroids: false,
        lasers: false,
        tiny: false,
        player: false,
        warp: false,
    };
}

impl Default for Features {
    fn default() -> Self {
        Self::ALL
    }
}

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

    /// `pub` for the autopilot: the line a ball's center crosses at contact
    /// is `top() - radius`.
    pub fn top(&self) -> f32 {
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
    /// Stuck to the paddle by Glue. Unlike `attached` (the serve), the ball
    /// keeps the exit velocity its paddle hit gave it and leaves with it on
    /// release; the rest of the fleet keeps moving meanwhile.
    pub glued: Option<Glued>,
}

/// A ball held by Glue.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glued {
    /// Ball center minus paddle center, in pixels, where it landed.
    pub offset: f32,
    /// Seconds held so far (released at `GLUE_HOLD_SECS`).
    pub held: f32,
}

/// Which gun the paddle carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GunKind {
    Laser,
    SuperLaser,
}

/// An active gun and the seconds it has left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gun {
    pub kind: GunKind,
    pub timer: f32,
}

/// A bolt fired by the paddle's gun, flying straight up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bolt {
    /// Center.
    pub x: f32,
    pub y: f32,
    pub half_size: [f32; 2],
    pub kind: GunKind,
}

impl Bolt {
    fn touches_brick(&self, brick: &Brick) -> bool {
        (self.x - brick.x).abs() <= self.half_size[0] + brick.width / 2.0
            && (self.y - brick.y).abs() <= self.half_size[1] + brick.height / 2.0
    }

    fn touches_asteroid(&self, asteroid: &Asteroid) -> bool {
        let dx = asteroid.x
            - asteroid
                .x
                .clamp(self.x - self.half_size[0], self.x + self.half_size[0]);
        let dy = asteroid.y
            - asteroid
                .y
                .clamp(self.y - self.half_size[1], self.y + self.half_size[1]);
        dx * dx + dy * dy <= asteroid.radius * asteroid.radius
    }
}

/// An asteroid: drifts in from the top through the bricks, and blows up
/// when a ball (which bounces off it) or the paddle touches it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Asteroid {
    pub x: f32,
    pub y: f32,
    /// Velocity over the last tick, for the autopilot and renderers.
    pub vx: f32,
    pub vy: f32,
    pub radius: f32,
    /// Base sideways drift, flipped by the walls.
    drift: f32,
    /// Wobble phase (radians) and rate (radians per second).
    phase: f32,
    wobble_rate: f32,
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
            glued: None,
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

/// One already-typed-and-positioned brick spawn -> a full `Brick` (row ->
/// score, kind -> hit count, grid-local pixels -> playfield pixels via
/// `offset_x`). Shared by the built-in-levels path (`build_bricks`) and
/// the externally-loaded one (`build_external_bricks`, arkanoid-v2-a5) so
/// this mapping only lives in one place.
fn brick_from_spawn(x: f32, y: f32, kind: BrickKind, offset_x: f32) -> Brick {
    let row = (y / levels::BRICK_HEIGHT).round() as usize;
    let hits_remaining = match kind {
        BrickKind::Normal => 1,
        BrickKind::Armored => 2,
        BrickKind::Indestructible => 0,
    };
    let score = match kind {
        BrickKind::Indestructible => 0,
        _ => score_for_row(row),
    };
    Brick {
        x: x + offset_x + levels::BRICK_WIDTH / 2.0,
        y: y + BRICK_GRID_MARGIN_TOP + levels::BRICK_HEIGHT / 2.0,
        width: levels::BRICK_WIDTH,
        height: levels::BRICK_HEIGHT,
        kind,
        hits_remaining,
        score,
    }
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
        .map(|spawn| brick_from_spawn(spawn.x, spawn.y, spawn.kind, offset_x))
        .collect()
}

/// Same brick-building contract as `build_bricks`, for one already-parsed
/// external level's spawns (arkanoid-v2-a5). External grids are always
/// `levelset::GRID_WIDTH` (14) columns wide per docs/levelset-format.md --
/// unlike the built-in levels, which can vary -- so centering doesn't need
/// to inspect the spawns themselves.
fn build_external_bricks(spawns: &[ExternalBrickSpawn]) -> Vec<Brick> {
    let offset_x = (PLAYFIELD_WIDTH - GRID_WIDTH as f32 * levels::BRICK_WIDTH) / 2.0;
    spawns
        .iter()
        .map(|spawn| brick_from_spawn(spawn.x, spawn.y, spawn.kind, offset_x))
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
///
/// `pub` so the autopilot inverts the exact mapping the simulation applies
/// rather than a copy of it that could drift.
pub fn offset_to_exit_angle_deg(offset_norm: f32) -> f32 {
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
    /// Externally-loaded levels to progress through instead of the
    /// built-in `levels::LEVELS` (arkanoid-v2-a5's level-select-per-loaded-
    /// set). `None` -- the default `new`/`with_seed` leave it at -- keeps
    /// the original built-in-3-levels behavior byte-for-byte, which is
    /// what keeps `deterministic_replay_matches_the_pinned_v1_baseline`
    /// green (that test never calls `load_external_levels`). Set via
    /// `load_external_levels`, e.g. with a loaded set's levels in file
    /// order, or with `levelset::random10`'s output for the RANDOM10 menu
    /// mode -- either way this field doesn't care how its order was
    /// chosen, only that it plays them in the order given.
    external_levels: Option<Vec<ExternalLevel>>,
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
    /// Edge-detects `Input::space` for releasing a glued ball, the way
    /// `pause_was_held` does for Pause: held Space serves (v1), a press
    /// releases.
    space_was_held: bool,
    /// Seconds left on an active Glue; 0 means inactive.
    pub glue_timer: f32,
    /// Seconds left on an active Tiny; 0 means inactive.
    pub tiny_timer: f32,
    /// The paddle's gun, while one is active.
    pub gun: Option<Gun>,
    /// Seconds until the gun may fire again.
    gun_cooldown: f32,
    /// Bolts in flight.
    pub bolts: Vec<Bolt>,
    /// The Break gate in the right wall is open: the paddle may slide out
    /// through it, which skips to the next level. Open until it is used, a
    /// life is lost or the level ends.
    pub gate_open: bool,
    /// Asteroids currently drifting.
    pub asteroids: Vec<Asteroid>,
    /// Seconds of play until the next asteroid may spawn.
    asteroid_timer: f32,
    /// Which post-v1 additions are on.
    pub features: Features,
    /// Seconds of flight since the last progress (see `STALL_SECS`).
    stall_secs: f32,
    /// Stall nudges so far; its parity alternates their direction.
    stall_nudges: u32,
    /// Source of randomness for the one non-deterministic simulation
    /// decision (`maybe_drop_powerup`'s drop chance/kind). `new()` seeds
    /// from OS entropy for normal play; `with_seed` seeds deterministically
    /// so a fixed seed + scripted input reproduces the exact same run --
    /// see the `deterministic_replay` test module for the property this
    /// exists to support.
    rng: StdRng,
    /// Asteroid spawns and wobble draw from their own stream, so turning
    /// asteroids on or off leaves every power-up drop where it was.
    asteroid_rng: StdRng,
}

impl Default for Game {
    fn default() -> Self {
        Self::new()
    }
}

impl Game {
    pub fn new() -> Self {
        Self::new_with_rng(
            StdRng::from_rng(&mut rand::rng()),
            StdRng::from_rng(&mut rand::rng()),
        )
    }

    /// Plays with only `features` of the post-v1 additions.
    pub fn with_features(mut self, features: Features) -> Self {
        self.features = features;
        self
    }

    /// Same as `new()`, but with a deterministic RNG seed instead of OS
    /// entropy -- fixed seed + the same scripted `Input` sequence then
    /// always produces the same sequence of states (including power-up
    /// drops), which is what makes the replay-hash property meaningful.
    /// Test-only: this binary crate has no external API, so an unused
    /// `pub` item still reads as dead code outside `#[cfg(test)]`.
    #[cfg(test)]
    pub fn with_seed(seed: u64) -> Self {
        Self::new_with_rng(
            StdRng::seed_from_u64(seed),
            StdRng::seed_from_u64(seed ^ 0x9E37_79B9_7F4A_7C15),
        )
    }

    /// Switches level progression to an externally-loaded level set
    /// (arkanoid-v2-a5's "level-select per loaded set"), replacing the
    /// built-in 3-level progression with `levels` in the order given --
    /// e.g. file order to play a whole set, or `levelset::random10`'s
    /// output for the RANDOM10 menu mode; this method itself doesn't care
    /// which. Resets to level 1 of the new set with a fresh, re-attached
    /// ball, mirroring the reset `advance_level`/`lose_life` already do
    /// between levels. A no-op on an empty `levels` (nothing to play --
    /// keeps whatever progression, built-in or a previously loaded set,
    /// was already active), so a caller never has to special-case an
    /// empty load.
    ///
    /// Wired in from `main.rs` (arkanoid-v2-a8's integration job): a
    /// successfully-resolved `--levelset` load is fed straight in here
    /// before the event loop starts driving frames.
    pub fn load_external_levels(&mut self, levels: Vec<ExternalLevel>) {
        if levels.is_empty() {
            return;
        }
        self.external_levels = Some(levels);
        self.level = 1;
        self.bricks = self.current_level_bricks();
        self.ball = Ball::attached_to(&self.paddle);
        self.extra_balls.clear();
        self.powerups.clear();
        self.reset_for_serve();
    }

    /// Bricks for whatever `self.level` currently refers to: the loaded
    /// external set if `load_external_levels` switched to one, else the
    /// built-in `levels::LEVELS` (unchanged default path).
    fn current_level_bricks(&self) -> Vec<Brick> {
        match &self.external_levels {
            Some(levels) => build_external_bricks(&levels[self.level - 1].bricks),
            None => build_bricks(self.level - 1),
        }
    }

    /// Number of levels in whatever progression is currently active --
    /// what `advance_level` compares `self.level` against to decide
    /// "load the next level" vs. "Victory".
    fn level_count(&self) -> usize {
        match &self.external_levels {
            Some(levels) => levels.len(),
            None => levels::LEVELS.len(),
        }
    }

    fn new_with_rng(rng: StdRng, asteroid_rng: StdRng) -> Self {
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
            external_levels: None,
            powerups: Vec::new(),
            widen_timer: 0.0,
            pause_was_held: false,
            space_was_held: false,
            glue_timer: 0.0,
            tiny_timer: 0.0,
            gun: None,
            gun_cooldown: 0.0,
            bolts: Vec::new(),
            gate_open: false,
            asteroids: Vec::new(),
            asteroid_timer: ASTEROID_FIRST_SPAWN_SECS,
            features: Features::default(),
            stall_secs: 0.0,
            stall_nudges: 0,
            rng,
            asteroid_rng,
        }
    }

    /// Advance the simulation by one fixed timestep. Dispatches on
    /// `state` first -- the explicit state machine the spec asks for
    /// (`Menu -> Playing <-> Paused -> GameOver/Victory`), rather than a
    /// scatter of booleans layered on top of a single "is playing" path.
    pub fn tick(&mut self, input: &Input, dt_fixed: f32) {
        let pause_pressed = input.pause && !self.pause_was_held;
        self.pause_was_held = input.pause;
        let space_pressed = input.space && !self.space_was_held;
        self.space_was_held = input.space;

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
                self.tick_playing(input, space_pressed, dt_fixed);
            }
        }
    }

    /// The actual gameplay tick, run only while `state == Playing`.
    fn tick_playing(&mut self, input: &Input, space_pressed: bool, dt_fixed: f32) {
        if self.widen_timer > 0.0 {
            self.widen_timer = (self.widen_timer - dt_fixed).max(0.0);
        }
        if self.glue_timer > 0.0 {
            self.glue_timer = (self.glue_timer - dt_fixed).max(0.0);
        }
        if self.tiny_timer > 0.0 {
            self.tiny_timer = (self.tiny_timer - dt_fixed).max(0.0);
        }
        if let Some(gun) = &mut self.gun {
            gun.timer -= dt_fixed;
            if gun.timer <= 0.0 {
                self.gun = None;
            }
        }
        self.gun_cooldown = (self.gun_cooldown - dt_fixed).max(0.0);
        self.paddle.width = if self.widen_timer > 0.0 {
            PADDLE_WIDTH * WIDEN_MULTIPLIER
        } else if self.tiny_timer > 0.0 {
            PADDLE_WIDTH * TINY_MULTIPLIER
        } else {
            PADDLE_WIDTH
        };

        self.move_paddle(input, dt_fixed);
        if self.gate_open && self.paddle.left() >= PLAYFIELD_WIDTH {
            self.warp();
            return;
        }

        if self.ball.attached {
            // Ride the paddle until launched.
            self.ball.x = self.paddle.x;
            self.ball.y = self.paddle.top() - self.ball.radius;
            if input.space {
                self.launch_ball();
            }
            return;
        }

        self.stall_secs += dt_fixed;
        self.hold_glued_balls(space_pressed, dt_fixed);
        if input.space {
            self.fire_gun();
        }
        self.advance_balls(dt_fixed);
        self.update_powerups(dt_fixed);
        if self.state == GameState::Playing && !self.ball.attached {
            self.update_bolts(dt_fixed);
        }
        if self.state == GameState::Playing && !self.ball.attached {
            self.update_asteroids(dt_fixed);
        }
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
        if ball.glued.is_some() {
            return false; // Rides the paddle (`hold_glued_balls`).
        }
        ball.x += ball.vx * sub_dt;
        ball.y += ball.vy * sub_dt;

        let before = (ball.vx, ball.vy);
        resolve_wall_collisions(ball);
        let off_a_wall = (ball.vx, ball.vy) != before;
        let off_indestructible = self.resolve_brick_collisions(ball);
        if (off_a_wall || off_indestructible) && self.stall_secs >= STALL_SECS {
            self.nudge_stalled(ball);
        }
        self.resolve_asteroid_collision(ball);
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
        // An open Break gate lets the paddle slide out through the right
        // wall, all the way.
        let right = if self.gate_open {
            PLAYFIELD_WIDTH + half
        } else {
            PLAYFIELD_WIDTH - half
        };
        self.paddle.x = self.paddle.x.clamp(half, right);
    }

    /// The paddle has slid all the way out through the Break gate: the
    /// level is left, with a bonus, as if it had been cleared.
    fn warp(&mut self) {
        self.score += WARP_SCORE;
        self.events.push(GameEvent::Warped);
        self.paddle.x = PLAYFIELD_WIDTH / 2.0;
        self.extra_balls.clear();
        self.advance_level();
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
    fn resolve_paddle_collision(&mut self, ball: &mut Ball) {
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
        self.events.push(GameEvent::PaddleHit);
        self.stall_secs = 0.0;
        if self.features.glue && self.glue_timer > 0.0 {
            ball.glued = Some(Glued {
                offset: ball.x - self.paddle.x,
                held: 0.0,
            });
        }
    }

    /// AABB-vs-circle collision against every brick, resolved against the
    /// nearest face. At most one brick is resolved per call (per substep,
    /// per ball) -- two bricks occupying the same few square pixels of
    /// ball travel within one substep isn't a real scenario at these
    /// speeds/sizes.
    ///
    /// Returns whether the ball bounced off an indestructible brick (the
    /// one kind of brick hit that is not progress; see `STALL_SECS`).
    fn resolve_brick_collisions(&mut self, ball: &mut Ball) -> bool {
        let Some(index) = self
            .bricks
            .iter()
            .position(|brick| ball_hits_brick(ball, brick))
        else {
            return false;
        };

        reflect_off_brick_face(ball, &self.bricks[index]);

        if self.bricks[index].kind == BrickKind::Indestructible {
            return true;
        }
        self.stall_secs = 0.0;

        self.bricks[index].hits_remaining = self.bricks[index].hits_remaining.saturating_sub(1);
        if self.bricks[index].hits_remaining > 0 {
            // Armored brick's first hit: state changed (2 -> 1 hit left),
            // not destroyed yet -- no score, no event.
            return false;
        }

        let brick = self.bricks.remove(index);
        self.on_brick_destroyed(brick);
        false
    }

    /// Scores a destroyed brick, announces it and rolls its power-up drop,
    /// whatever destroyed it (a ball, or an asteroid's blast).
    fn on_brick_destroyed(&mut self, brick: Brick) {
        self.score += brick.score;
        self.events.push(GameEvent::BrickDestroyed);
        self.events.push(GameEvent::BrickDestroyedAt {
            x: brick.x,
            y: brick.y,
            width: brick.width,
            height: brick.height,
            kind: brick.kind,
        });
        self.maybe_drop_powerup(brick.x, brick.y);
    }

    /// Spec: 15% of destroyed bricks drop one of the three power-ups,
    /// uniformly at random, falling from where the brick was.
    fn maybe_drop_powerup(&mut self, x: f32, y: f32) {
        if !self.rng.random_bool(POWERUP_DROP_CHANCE) {
            return;
        }
        let mut kinds = vec![
            PowerUpKind::Widen,
            PowerUpKind::Slow,
            PowerUpKind::Multiball,
        ];
        if self.features.glue {
            kinds.push(PowerUpKind::Glue);
        }
        if self.features.lasers {
            kinds.extend([PowerUpKind::Laser, PowerUpKind::SuperLaser]);
        }
        if self.features.tiny {
            kinds.push(PowerUpKind::Tiny);
        }
        if self.features.player {
            kinds.push(PowerUpKind::Player);
        }
        if self.features.warp {
            kinds.push(PowerUpKind::Break);
        }
        // An `i32` range, as v1 drew it: the same draw over 0..3 keeps
        // `Features::V1` runs drop for drop identical to v1.
        let kind = kinds[self.rng.random_range(0..kinds.len() as i32) as usize];
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

        if self.level >= self.level_count() {
            self.state = GameState::Victory;
            self.events.push(GameEvent::Victory);
            return;
        }

        self.level += 1;
        self.bricks = self.current_level_bricks();
        self.ball = Ball::attached_to(&self.paddle);
        self.extra_balls.clear();
        self.powerups.clear();
        self.reset_for_serve();
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
        self.reset_for_serve();

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
                self.tiny_timer = 0.0;
            }
            PowerUpKind::Tiny => {
                self.tiny_timer = TINY_DURATION_SECS;
                self.widen_timer = 0.0;
            }
            PowerUpKind::Player => {
                self.lives = (self.lives + 1).min(MAX_LIVES);
            }
            PowerUpKind::Break => {
                self.gate_open = true;
            }
            PowerUpKind::Laser => {
                self.gun = Some(Gun {
                    kind: GunKind::Laser,
                    timer: GUN_DURATION_SECS,
                });
            }
            PowerUpKind::SuperLaser => {
                self.gun = Some(Gun {
                    kind: GunKind::SuperLaser,
                    timer: GUN_DURATION_SECS,
                });
            }
            PowerUpKind::Glue => {
                self.glue_timer = GLUE_DURATION_SECS;
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

    /// Glued balls ride the paddle at their offset until Space is pressed,
    /// Glue runs out, or they have been held `GLUE_HOLD_SECS`; then they
    /// leave with the exit velocity their paddle hit already gave them.
    fn hold_glued_balls(&mut self, space_pressed: bool, dt_fixed: f32) {
        let release_all = space_pressed || self.glue_timer <= 0.0;
        let paddle = self.paddle;
        for ball in std::iter::once(&mut self.ball).chain(self.extra_balls.iter_mut()) {
            let Some(glued) = &mut ball.glued else {
                continue;
            };
            glued.held += dt_fixed;
            if release_all || glued.held >= GLUE_HOLD_SECS {
                ball.glued = None;
                continue;
            }
            let half = paddle.width / 2.0;
            ball.x = (paddle.x + glued.offset.clamp(-half, half))
                .clamp(ball.radius, PLAYFIELD_WIDTH - ball.radius);
            ball.y = paddle.top() - ball.radius;
        }
    }

    /// Fires the paddle's gun if it has one and has cooled down: a Laser
    /// pair from near the paddle's ends, or one Super laser bolt from its
    /// center. Holding Space fires at the cooldown's rate.
    fn fire_gun(&mut self) {
        let Some(gun) = self.gun else {
            return;
        };
        if self.gun_cooldown > 0.0 {
            return;
        }
        let paddle = self.paddle;
        let (xs, half_size, cooldown): (Vec<f32>, _, _) = match gun.kind {
            GunKind::Laser => {
                let reach = paddle.width / 2.0 - LASER_INSET;
                (
                    vec![paddle.x - reach, paddle.x + reach],
                    LASER_HALF_SIZE,
                    LASER_COOLDOWN_SECS,
                )
            }
            GunKind::SuperLaser => (
                vec![paddle.x],
                SUPER_LASER_HALF_SIZE,
                SUPER_LASER_COOLDOWN_SECS,
            ),
        };
        for x in xs {
            if self.bolts.len() < MAX_BOLTS {
                self.bolts.push(Bolt {
                    x,
                    y: paddle.top() - half_size[1],
                    half_size,
                    kind: gun.kind,
                });
            }
        }
        self.gun_cooldown = cooldown;
        self.events.push(GameEvent::GunFired(gun.kind));
    }

    /// Moves every bolt up one tick and resolves what it hits. A Laser bolt
    /// gives the first brick it meets one hit and is spent; a Super laser
    /// bolt destroys every destructible brick it meets and flies on. Either
    /// stops at an indestructible brick and blows up an asteroid. One past
    /// the top just leaves. A bolt moves at most 17 px a tick, under a
    /// brick's 22 px height, so none can skip one.
    fn update_bolts(&mut self, dt_fixed: f32) {
        let mut bolts = std::mem::take(&mut self.bolts);
        bolts.retain_mut(|bolt| {
            let speed = match bolt.kind {
                GunKind::Laser => LASER_SPEED,
                GunKind::SuperLaser => SUPER_LASER_SPEED,
            };
            bolt.y -= speed * dt_fixed;
            if let Some(index) = self.asteroids.iter().position(|a| bolt.touches_asteroid(a)) {
                let asteroid = self.asteroids.remove(index);
                self.explode_asteroid(asteroid.x, asteroid.y);
                return bolt.kind == GunKind::SuperLaser;
            }
            while let Some(index) = self.bricks.iter().position(|b| bolt.touches_brick(b)) {
                if self.bricks[index].kind == BrickKind::Indestructible {
                    return false;
                }
                self.stall_secs = 0.0;
                let brick = &mut self.bricks[index];
                brick.hits_remaining = match bolt.kind {
                    GunKind::Laser => brick.hits_remaining.saturating_sub(1),
                    GunKind::SuperLaser => 0,
                };
                if brick.hits_remaining == 0 {
                    let brick = self.bricks.remove(index);
                    self.on_brick_destroyed(brick);
                }
                if bolt.kind == GunKind::Laser {
                    return false;
                }
            }
            bolt.y + bolt.half_size[1] > 0.0
        });
        self.bolts = bolts;
        if self.destructible_bricks_remaining() == 0 {
            self.advance_level();
        }
    }

    /// A fresh serve (a level start, a lost life, a loaded set) starts
    /// clean: no asteroid, no bolt, no open gate, and the countdown to the
    /// first asteroid restarted.
    fn reset_for_serve(&mut self) {
        self.stall_secs = 0.0;
        self.gate_open = false;
        self.bolts.clear();
        self.asteroids.clear();
        self.asteroid_timer = ASTEROID_FIRST_SPAWN_SECS;
    }

    /// Spawns, moves and retires asteroids for one tick of play. Paddle
    /// contact blows one up; one past the paddle just leaves.
    fn update_asteroids(&mut self, dt_fixed: f32) {
        if !self.features.asteroids {
            return;
        }
        self.asteroid_timer -= dt_fixed;
        if self.asteroid_timer <= 0.0 {
            self.asteroid_timer = self.asteroid_rng.random_range(ASTEROID_SPAWN_SECS);
            if self.asteroids.len() < MAX_ASTEROIDS {
                let x = self
                    .asteroid_rng
                    .random_range(ASTEROID_RADIUS * 4.0..PLAYFIELD_WIDTH - ASTEROID_RADIUS * 4.0);
                let drift = if self.asteroid_rng.random_bool(0.5) {
                    ASTEROID_DRIFT_SPEED
                } else {
                    -ASTEROID_DRIFT_SPEED
                };
                self.asteroids.push(Asteroid {
                    x,
                    y: -ASTEROID_RADIUS,
                    vx: drift,
                    vy: ASTEROID_FALL_SPEED,
                    radius: ASTEROID_RADIUS,
                    drift,
                    phase: self.asteroid_rng.random_range(0.0..std::f32::consts::TAU),
                    wobble_rate: self.asteroid_rng.random_range(1.0..2.5),
                });
                self.events.push(GameEvent::AsteroidSpawned);
            }
        }

        let paddle = self.paddle;
        let mut asteroids = std::mem::take(&mut self.asteroids);
        asteroids.retain_mut(|a| {
            a.advance(dt_fixed);
            if circle_touches_rect(a.x, a.y, a.radius, &paddle) {
                self.explode_asteroid(a.x, a.y);
                return false;
            }
            a.y - a.radius <= paddle.bottom()
        });
        self.asteroids = asteroids;
        // A paddle-triggered blast can take the level's last bricks; a
        // ball-triggered one is caught by `advance_balls`' own check.
        if self.destructible_bricks_remaining() == 0 {
            self.advance_level();
        }
    }

    /// A ball touching an asteroid blows it up and bounces off it like off
    /// a round obstacle: its velocity is mirrored about the line between
    /// the two centers, its speed unchanged.
    fn resolve_asteroid_collision(&mut self, ball: &mut Ball) {
        let Some(index) = self
            .asteroids
            .iter()
            .position(|a| ball_touches_asteroid(ball, a))
        else {
            return;
        };
        let asteroid = self.asteroids.remove(index);
        bounce_off_asteroid(ball, &asteroid);
        self.explode_asteroid(asteroid.x, asteroid.y);
    }

    /// An asteroid blows up at (`x`, `y`): it scores, and its blast hits
    /// the bricks around it (see `blast_bricks`).
    fn explode_asteroid(&mut self, x: f32, y: f32) {
        self.stall_secs = 0.0;
        self.score += ASTEROID_SCORE;
        self.events.push(GameEvent::AsteroidExploded { x, y });
        for brick in blast_bricks(&mut self.bricks, x, y) {
            self.on_brick_destroyed(brick);
        }
    }

    /// Turns a stalled ball by `STALL_NUDGE_DEG`, alternating direction,
    /// or the other way if that would leave it flatter than
    /// `STALL_MIN_ANGLE_DEG`. Speed is unchanged.
    fn nudge_stalled(&mut self, ball: &mut Ball) {
        self.stall_nudges += 1;
        let sign = if self.stall_nudges.is_multiple_of(2) {
            1.0
        } else {
            -1.0
        };
        let min_vy = ball.speed() * STALL_MIN_ANGLE_DEG.to_radians().sin();
        let (vx, vy) = rotate_velocity(ball.vx, ball.vy, sign * STALL_NUDGE_DEG);
        (ball.vx, ball.vy) = if vy.abs() >= min_vy {
            (vx, vy)
        } else {
            rotate_velocity(ball.vx, ball.vy, -sign * STALL_NUDGE_DEG)
        };
    }
}

impl Asteroid {
    /// One tick of drift: down, sideways with a wobble, the sideways drift
    /// (and the wobble) flipping at a wall.
    fn advance(&mut self, dt_fixed: f32) {
        self.phase += self.wobble_rate * dt_fixed;
        self.vx = self.drift + ASTEROID_WOBBLE_SPEED * self.phase.sin();
        self.vy = ASTEROID_FALL_SPEED;
        self.x += self.vx * dt_fixed;
        self.y += self.vy * dt_fixed;
        if self.x < self.radius || self.x > PLAYFIELD_WIDTH - self.radius {
            self.x = self.x.clamp(self.radius, PLAYFIELD_WIDTH - self.radius);
            self.drift = -self.drift;
            self.phase += std::f32::consts::PI; // Flip the wobble too.
        }
    }
}

/// An asteroid's blast at (`x`, `y`): every destructible brick within
/// `ASTEROID_BLAST_RADIUS` takes one hit, the hit a ball would give it (a
/// normal brick breaks, an armored one loses one of its two). Returns the
/// bricks it destroyed, already removed from `bricks`.
fn blast_bricks(bricks: &mut Vec<Brick>, x: f32, y: f32) -> Vec<Brick> {
    let in_range = |brick: &Brick| {
        let dx = x - x.clamp(brick.left(), brick.right());
        let dy = y - y.clamp(brick.top(), brick.bottom());
        dx * dx + dy * dy <= ASTEROID_BLAST_RADIUS * ASTEROID_BLAST_RADIUS
    };
    let mut destroyed = Vec::new();
    bricks.retain_mut(|brick| {
        if brick.kind == BrickKind::Indestructible || !in_range(brick) {
            return true;
        }
        brick.hits_remaining = brick.hits_remaining.saturating_sub(1);
        if brick.hits_remaining > 0 {
            return true;
        }
        destroyed.push(*brick);
        false
    });
    destroyed
}

fn ball_touches_asteroid(ball: &Ball, asteroid: &Asteroid) -> bool {
    let (dx, dy) = (ball.x - asteroid.x, ball.y - asteroid.y);
    let reach = ball.radius + asteroid.radius;
    dx * dx + dy * dy <= reach * reach
}

/// A ball bounces off an asteroid like off a round obstacle: its velocity
/// mirrored about the line between the two centers, its speed unchanged,
/// and it is pushed back out to touching distance.
fn bounce_off_asteroid(ball: &mut Ball, asteroid: &Asteroid) {
    let (dx, dy) = (ball.x - asteroid.x, ball.y - asteroid.y);
    let distance = (dx * dx + dy * dy).sqrt();
    if distance <= f32::EPSILON {
        return;
    }
    let (nx, ny) = (dx / distance, dy / distance);
    let along = ball.vx * nx + ball.vy * ny;
    if along < 0.0 {
        ball.vx -= 2.0 * along * nx;
        ball.vy -= 2.0 * along * ny;
    }
    let reach = ball.radius + asteroid.radius;
    ball.x = asteroid.x + nx * reach;
    ball.y = asteroid.y + ny * reach;
}

/// Where a ball will next reach a line on its way down, simulated tick by
/// tick with the game's own rules: walls, bricks (which break, or lose a
/// hit, as they would), and asteroids (which drift and blow up as they
/// would). See [`forecast_crossing`].
#[derive(Debug, Clone, PartialEq)]
pub struct Forecast {
    /// Ball center where it reaches the line.
    pub x: f32,
    /// Ticks from now until it does.
    pub ticks: u32,
    /// Ball center after each tick, from now (`path[0]` is where it is) up
    /// to the crossing. A caller replays this to tell when the ball has
    /// left the forecast (a paddle hit, a new asteroid) and a new one is
    /// needed.
    pub path: Vec<(f32, f32)>,
}

/// [`Forecast`] for `ball` against `bricks` and `asteroids`, down to
/// `line_y` (the paddle's contact line; the paddle itself is left out, as
/// is anything random: no asteroid spawns, no level clear). `None` if the
/// ball is resting on the paddle, held by Glue, or still above the line
/// after `max_ticks`.
///
/// It runs the same per-substep sequence `step_ball` does, through the
/// same functions, so it reproduces the game exactly for as long as
/// nothing it leaves out happens.
pub fn forecast_crossing(
    ball: &Ball,
    bricks: &[Brick],
    asteroids: &[Asteroid],
    line_y: f32,
    dt_fixed: f32,
    max_ticks: u32,
) -> Option<Forecast> {
    if ball.attached || ball.glued.is_some() {
        return None;
    }
    let mut ball = *ball;
    let mut bricks = bricks.to_vec();
    let mut asteroids = asteroids.to_vec();
    let mut path = Vec::with_capacity(max_ticks as usize + 1);
    path.push((ball.x, ball.y));
    for tick in 1..=max_ticks {
        let substeps = substep_count(ball.speed(), BALL_RADIUS, dt_fixed);
        let sub_dt = dt_fixed / substeps as f32;
        for _ in 0..substeps {
            ball.x += ball.vx * sub_dt;
            ball.y += ball.vy * sub_dt;
            resolve_wall_collisions(&mut ball);
            if let Some(index) = bricks.iter().position(|b| ball_hits_brick(&ball, b)) {
                reflect_off_brick_face(&mut ball, &bricks[index]);
                let brick = &mut bricks[index];
                if brick.kind != BrickKind::Indestructible {
                    brick.hits_remaining = brick.hits_remaining.saturating_sub(1);
                    if brick.hits_remaining == 0 {
                        bricks.remove(index);
                    }
                }
            }
            if let Some(index) = asteroids
                .iter()
                .position(|a| ball_touches_asteroid(&ball, a))
            {
                let asteroid = asteroids.remove(index);
                bounce_off_asteroid(&mut ball, &asteroid);
                blast_bricks(&mut bricks, asteroid.x, asteroid.y);
            }
            if ball.vy > 0.0 && ball.y >= line_y {
                path.push((ball.x, ball.y));
                return Some(Forecast {
                    x: ball.x,
                    ticks: tick,
                    path,
                });
            }
        }
        for asteroid in &mut asteroids {
            asteroid.advance(dt_fixed);
        }
        path.push((ball.x, ball.y));
    }
    None
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

/// Circle vs the paddle's rect: the closest point on the rect within the
/// radius.
fn circle_touches_rect(x: f32, y: f32, radius: f32, paddle: &Paddle) -> bool {
    let dx = x - x.clamp(paddle.left(), paddle.right());
    let dy = y - y.clamp(paddle.top(), paddle.bottom());
    dx * dx + dy * dy <= radius * radius
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
            glued: None,
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

    // -- level load / progression: externally-loaded level sets (a5) ------

    fn external_level(name: &str, x: f32, kind: BrickKind) -> ExternalLevel {
        ExternalLevel {
            author: "test".to_string(),
            name: name.to_string(),
            bricks: vec![ExternalBrickSpawn {
                x,
                y: 0.0,
                kind,
                powerup: None,
            }],
        }
    }

    #[test]
    fn loading_an_external_set_switches_to_its_level_1_bricks() {
        let mut game = playing_game();

        game.load_external_levels(vec![
            external_level("One", 0.0, BrickKind::Normal),
            external_level("Two", 52.0, BrickKind::Armored),
        ]);

        assert_eq!(game.level, 1);
        assert_eq!(game.bricks.len(), 1);
        assert_eq!(game.bricks[0].kind, BrickKind::Normal);
        assert!(
            game.ball.attached,
            "loading a new set re-attaches the ball, like starting a level"
        );
    }

    #[test]
    fn loading_an_empty_external_set_is_a_no_op() {
        let mut game = playing_game();
        let bricks_before = game.bricks.clone();
        let level_before = game.level;

        game.load_external_levels(Vec::new());

        assert_eq!(game.level, level_before);
        assert_eq!(
            game.bricks, bricks_before,
            "an empty load must not disturb the built-in progression already in play"
        );
    }

    #[test]
    fn clearing_a_non_final_external_level_advances_to_the_next_one_in_the_set() {
        let mut game = playing_game();
        game.load_external_levels(vec![
            external_level("One", 0.0, BrickKind::Normal),
            external_level("Two", 52.0, BrickKind::Armored),
        ]);
        game.ball = approach_from_below(&game.bricks[0]);

        game.tick(&Input::default(), DT);

        assert_eq!(
            game.level, 2,
            "level 1 of the loaded set cleared -> its level 2"
        );
        assert_eq!(game.state, GameState::Playing);
        assert_eq!(game.bricks.len(), 1);
        assert_eq!(
            game.bricks[0].kind,
            BrickKind::Armored,
            "the loaded set's own level 2 bricks are loaded, not the built-in ones"
        );
    }

    #[test]
    fn clearing_an_external_sets_final_level_reaches_victory_by_its_own_length() {
        // A one-level external set: reaching Victory here (rather than
        // playing through the built-in levels::LEVELS.len() == 3) proves
        // `advance_level` compares against the loaded set's own length,
        // not the hardcoded built-in one.
        let mut game = playing_game();
        game.load_external_levels(vec![external_level("Only", 0.0, BrickKind::Normal)]);
        game.ball = approach_from_below(&game.bricks[0]);

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
                glued: None,
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
    fn brick_destruction_drops_a_powerup_about_15_percent_of_the_time() {
        let mut game = playing_game();
        let mut spawned = 0;
        let mut kinds = std::collections::HashMap::new();
        for _ in 0..2000 {
            game.powerups.clear();
            game.maybe_drop_powerup(400.0, 300.0);
            if let Some(powerup) = game.powerups.first() {
                spawned += 1;
                *kinds.entry(powerup.kind).or_insert(0) += 1;
            }
        }
        // Loose bounds around the spec'd 15% of 2000 (expect ~300, std
        // dev ~16) -- wide enough to never flake, tight enough to catch a
        // badly broken roll (e.g. always/never dropping).
        assert!(
            (200..=400).contains(&spawned),
            "expected roughly 15% of 2000 destroyed bricks to drop a power-up, got {spawned}"
        );
        // Nine kinds, evenly: about 33 each out of ~300, std dev ~5.5.
        assert_eq!(kinds.len(), 9, "{kinds:?}");
        assert!(
            kinds.values().all(|&n| (8..=70).contains(&n)),
            "{kinds:?} out of {spawned}"
        );
    }

    #[test]
    fn v1_rules_never_drop_glue() {
        let mut game = playing_game().with_features(Features::V1);
        for _ in 0..2000 {
            game.powerups.clear();
            game.maybe_drop_powerup(400.0, 300.0);
            if let Some(powerup) = game.powerups.first() {
                assert!(matches!(
                    powerup.kind,
                    PowerUpKind::Widen | PowerUpKind::Slow | PowerUpKind::Multiball
                ));
            }
        }
    }

    // -- power-ups: Glue ----------------------------------------------------

    /// A playing game with Glue active and its ball falling straight onto
    /// the paddle, `offset` px off center, one tick from contact.
    fn glued_drop(offset: f32) -> Game {
        let mut game = playing_game();
        game.apply_powerup(PowerUpKind::Glue);
        game.ball = Ball {
            x: game.paddle.x + offset,
            y: game.paddle.top() - BALL_RADIUS - 1.0,
            vx: 0.0,
            vy: 300.0,
            radius: BALL_RADIUS,
            attached: false,
            glued: None,
        };
        game.tick(&Input::default(), DT);
        game
    }

    #[test]
    fn with_glue_a_paddle_hit_sticks_the_ball_where_it_landed() {
        let game = glued_drop(20.0);
        let glued = game.ball.glued.expect("the ball sticks");
        assert!((glued.offset - 20.0).abs() < 1.0);
        assert!(game.events.contains(&GameEvent::PaddleHit));
    }

    #[test]
    fn a_glued_ball_rides_the_paddle_and_leaves_on_a_space_press_at_its_offset_angle() {
        let mut game = glued_drop(25.0);
        let offset = game.ball.glued.unwrap().offset;
        for _ in 0..30 {
            game.tick(&held(|i| i.left = true), DT);
        }
        assert!(
            (game.ball.x - (game.paddle.x + offset)).abs() < 1e-3,
            "rides along"
        );
        assert!(game.ball.glued.is_some());

        game.tick(&held(|i| i.space = true), DT);
        assert!(game.ball.glued.is_none(), "a press releases it");
        let angle = (-game.ball.vy).atan2(game.ball.vx).to_degrees();
        let expected = offset_to_exit_angle_deg(offset / (game.paddle.width / 2.0));
        assert!((angle - expected).abs() < 0.5, "{angle} vs {expected}");
        assert!(game.ball.vy < 0.0);
    }

    #[test]
    fn holding_space_does_not_release_a_glued_ball_only_a_press_does() {
        let mut game = glued_drop(0.0);
        game.tick(&held(|i| i.space = true), DT); // the press: released
        assert!(game.ball.glued.is_none());

        let mut game = glued_drop(0.0);
        game.space_was_held = true; // Space was already down when it stuck
        game.tick(&held(|i| i.space = true), DT);
        assert!(game.ball.glued.is_some(), "a held key is not a press");
    }

    #[test]
    fn a_glued_ball_lets_go_by_itself_after_the_hold_limit() {
        let mut game = glued_drop(0.0);
        let ticks = (GLUE_HOLD_SECS / DT).ceil() as usize;
        for _ in 0..ticks - 2 {
            game.tick(&Input::default(), DT);
        }
        assert!(game.ball.glued.is_some());
        game.tick(&Input::default(), DT);
        game.tick(&Input::default(), DT);
        assert!(game.ball.glued.is_none());
    }

    #[test]
    fn glue_running_out_releases_a_held_ball_and_stops_catching() {
        let mut game = glued_drop(0.0);
        game.glue_timer = DT / 2.0;
        game.tick(&Input::default(), DT);
        assert!(game.ball.glued.is_none());
        assert_eq!(game.glue_timer, 0.0);
    }

    #[test]
    fn glue_timer_refreshes_on_recatch_instead_of_stacking() {
        let mut game = playing_game();
        game.apply_powerup(PowerUpKind::Glue);
        game.glue_timer = 2.0;
        game.apply_powerup(PowerUpKind::Glue);
        assert_eq!(game.glue_timer, GLUE_DURATION_SECS);
    }

    #[test]
    fn without_the_glue_feature_a_paddle_hit_never_sticks() {
        let mut game = playing_game().with_features(Features::V1);
        game.glue_timer = GLUE_DURATION_SECS; // even if set by hand
        game.ball = Ball {
            x: game.paddle.x,
            y: game.paddle.top() - BALL_RADIUS - 1.0,
            vx: 0.0,
            vy: 300.0,
            radius: BALL_RADIUS,
            attached: false,
            glued: None,
        };
        game.tick(&Input::default(), DT);
        assert!(game.ball.glued.is_none());
        assert!(game.ball.vy < 0.0);
    }

    // -- power-ups: Tiny -----------------------------------------------------

    #[test]
    fn tiny_shrinks_the_paddle_for_its_duration_then_lets_it_grow_back() {
        let mut game = playing_game();
        game.apply_powerup(PowerUpKind::Tiny);
        game.tick(&Input::default(), DT);
        assert!((game.paddle.width - PADDLE_WIDTH * TINY_MULTIPLIER).abs() < 1e-4);
        game.tiny_timer = DT / 2.0;
        game.tick(&Input::default(), DT);
        assert_eq!(game.paddle.width, PADDLE_WIDTH);
    }

    #[test]
    fn tiny_and_widen_cancel_each_other_the_last_caught_wins() {
        let mut game = playing_game();
        game.apply_powerup(PowerUpKind::Widen);
        game.apply_powerup(PowerUpKind::Tiny);
        game.tick(&Input::default(), DT);
        assert_eq!(game.widen_timer, 0.0);
        assert!((game.paddle.width - PADDLE_WIDTH * TINY_MULTIPLIER).abs() < 1e-4);

        game.apply_powerup(PowerUpKind::Widen);
        game.tick(&Input::default(), DT);
        assert_eq!(game.tiny_timer, 0.0);
        assert!((game.paddle.width - PADDLE_WIDTH * WIDEN_MULTIPLIER).abs() < 1e-4);
    }

    // -- power-ups: Laser and Super laser -------------------------------------

    /// A playing game with its ball in flight far from the bricks and the
    /// paddle, and `kind`'s gun on the paddle.
    fn armed(kind: PowerUpKind) -> Game {
        let mut game = playing_game().with_features(Features {
            asteroids: false,
            ..Features::ALL
        });
        game.ball.attached = false;
        game.ball.x = 20.0;
        game.ball.y = 400.0;
        game.ball.vx = 0.0;
        game.ball.vy = -1.0;
        game.apply_powerup(kind);
        game
    }

    fn fire(game: &mut Game) {
        game.tick(&held(|i| i.space = true), DT);
    }

    #[test]
    fn a_laser_fires_a_pair_from_near_the_paddle_ends_at_its_cooldown_while_space_is_held() {
        let mut game = armed(PowerUpKind::Laser);
        fire(&mut game);
        assert_eq!(game.bolts.len(), 2);
        let reach = game.paddle.width / 2.0 - LASER_INSET;
        let mut xs: Vec<f32> = game.bolts.iter().map(|b| b.x).collect();
        xs.sort_by(f32::total_cmp);
        assert!((xs[0] - (game.paddle.x - reach)).abs() < 1e-3);
        assert!((xs[1] - (game.paddle.x + reach)).abs() < 1e-3);

        for _ in 0..((LASER_COOLDOWN_SECS / DT) as usize - 2) {
            fire(&mut game);
        }
        assert_eq!(game.bolts.len(), 2, "cooling down");
        fire(&mut game);
        fire(&mut game);
        assert_eq!(game.bolts.len(), 4, "held Space fires again");
    }

    #[test]
    fn a_laser_bolt_gives_the_first_brick_it_meets_one_hit_and_is_spent() {
        let mut game = armed(PowerUpKind::Laser);
        let reach = game.paddle.width / 2.0 - LASER_INSET;
        let (left, right) = (game.paddle.x - reach, game.paddle.x + reach);
        game.bricks = vec![
            test_brick_at(left, 300.0, BrickKind::Normal, 1, 10),
            test_brick_at(left, 278.0, BrickKind::Normal, 1, 10),
            test_brick_at(right, 300.0, BrickKind::Armored, 2, 20),
            test_brick_at(20.0, 100.0, BrickKind::Normal, 1, 10),
        ];
        fire(&mut game);
        for _ in 0..60 {
            game.tick(&Input::default(), DT);
        }
        assert!(game.bolts.is_empty(), "both spent");
        assert_eq!(
            game.bricks.len(),
            3,
            "only the first brick on the left broke"
        );
        let armored = game
            .bricks
            .iter()
            .find(|b| b.kind == BrickKind::Armored)
            .unwrap();
        assert_eq!(armored.hits_remaining, 1, "armored: one hit");
        assert_eq!(game.score, 10);
    }

    #[test]
    fn a_super_laser_bolt_pierces_destructible_bricks_until_an_indestructible_one() {
        let mut game = armed(PowerUpKind::SuperLaser);
        let x = game.paddle.x;
        game.bricks = vec![
            test_brick_at(x, 300.0, BrickKind::Normal, 1, 10),
            test_brick_at(x, 278.0, BrickKind::Armored, 2, 20),
            test_brick_at(x, 256.0, BrickKind::Normal, 1, 30),
            test_brick_at(x, 234.0, BrickKind::Indestructible, 1, 0),
            test_brick_at(x, 212.0, BrickKind::Normal, 1, 40), // behind the wall
            test_brick_at(20.0, 100.0, BrickKind::Normal, 1, 10),
        ];
        fire(&mut game);
        assert_eq!(game.bolts.len(), 1, "one bolt, from the center");
        for _ in 0..60 {
            game.tick(&Input::default(), DT);
        }
        assert!(game.bolts.is_empty());
        assert_eq!(game.score, 10 + 20 + 30, "armored destroyed outright");
        assert_eq!(
            game.bricks.len(),
            3,
            "the wall, the brick behind it, the far one"
        );
    }

    #[test]
    fn a_bolt_blows_up_an_asteroid_in_its_way() {
        let mut game = armed(PowerUpKind::Laser);
        let reach = game.paddle.width / 2.0 - LASER_INSET;
        game.asteroids
            .push(test_asteroid(game.paddle.x + reach, 400.0));
        fire(&mut game);
        for _ in 0..30 {
            game.tick(&Input::default(), DT);
        }
        assert!(game.asteroids.is_empty());
        assert!(game.score >= ASTEROID_SCORE);
    }

    #[test]
    fn the_gun_does_not_fire_during_a_serve_and_goes_quiet_when_it_runs_out() {
        let mut game = playing_game();
        game.apply_powerup(PowerUpKind::Laser);
        fire(&mut game); // the serve: Space launches the ball
        assert!(game.bolts.is_empty(), "no bolt on the launching tick");

        let mut game = armed(PowerUpKind::Laser);
        game.gun.as_mut().unwrap().timer = DT / 2.0;
        fire(&mut game);
        assert!(game.gun.is_none());
        assert!(game.bolts.is_empty());
    }

    #[test]
    fn catching_either_gun_replaces_the_other_with_a_full_timer() {
        let mut game = armed(PowerUpKind::Laser);
        game.gun.as_mut().unwrap().timer = 1.0;
        game.apply_powerup(PowerUpKind::SuperLaser);
        assert_eq!(
            game.gun,
            Some(Gun {
                kind: GunKind::SuperLaser,
                timer: GUN_DURATION_SECS
            })
        );
    }

    #[test]
    fn a_laser_that_takes_the_last_brick_clears_the_level() {
        let mut game = armed(PowerUpKind::Laser);
        let reach = game.paddle.width / 2.0 - LASER_INSET;
        game.bricks = vec![test_brick_at(
            game.paddle.x + reach,
            300.0,
            BrickKind::Normal,
            1,
            10,
        )];
        fire(&mut game);
        for _ in 0..40 {
            game.tick(&Input::default(), DT);
            if game.level == 2 {
                break;
            }
        }
        assert_eq!(game.level, 2);
        assert!(game.bolts.is_empty(), "a new level starts with none");
    }

    #[test]
    fn without_the_lasers_and_tiny_features_none_of_them_ever_drops() {
        let mut game = playing_game().with_features(Features {
            glue: true,
            asteroids: true,
            ..Features::V1
        });
        for _ in 0..2000 {
            game.powerups.clear();
            game.maybe_drop_powerup(400.0, 300.0);
            if let Some(powerup) = game.powerups.first() {
                assert!(!matches!(
                    powerup.kind,
                    PowerUpKind::Laser | PowerUpKind::SuperLaser | PowerUpKind::Tiny
                ));
            }
        }
    }

    // -- power-ups: Player and Break -------------------------------------------

    #[test]
    fn player_gives_an_extra_life_up_to_the_cap() {
        let mut game = playing_game();
        game.apply_powerup(PowerUpKind::Player);
        assert_eq!(game.lives, STARTING_LIVES + 1);
        for _ in 0..20 {
            game.apply_powerup(PowerUpKind::Player);
        }
        assert_eq!(game.lives, MAX_LIVES);
    }

    /// A playing game with its ball high up and rising, out of the way.
    fn rallying() -> Game {
        let mut game = playing_game().with_features(Features {
            asteroids: false,
            ..Features::ALL
        });
        game.ball.attached = false;
        game.ball.x = 100.0;
        game.ball.y = 300.0;
        game.ball.vx = 0.0;
        game.ball.vy = -1.0;
        game
    }

    #[test]
    fn the_paddle_stops_at_the_right_wall_while_the_gate_is_closed() {
        let mut game = rallying();
        for _ in 0..240 {
            game.tick(&held(|i| i.right = true), DT);
        }
        assert!((game.paddle.right() - PLAYFIELD_WIDTH).abs() < 1e-3);
        assert_eq!(game.level, 1);
    }

    #[test]
    fn sliding_out_through_the_open_gate_skips_to_the_next_level_with_a_bonus() {
        let mut game = rallying();
        game.apply_powerup(PowerUpKind::Break);
        let score = game.score;
        for _ in 0..240 {
            game.tick(&held(|i| i.right = true), DT);
            if game.level == 2 {
                break;
            }
            assert!(game.paddle.left() < PLAYFIELD_WIDTH, "still sliding out");
        }
        assert_eq!(game.level, 2);
        assert_eq!(game.score, score + WARP_SCORE);
        assert!(game.events.contains(&GameEvent::Warped));
        assert!(!game.gate_open, "a new level starts with it shut");
        assert_eq!(game.paddle.x, PLAYFIELD_WIDTH / 2.0, "back in the middle");
        assert!(game.ball.attached && (game.ball.x - game.paddle.x).abs() < 1e-3);
    }

    #[test]
    fn warping_out_of_the_last_level_is_victory() {
        let mut game = rallying();
        game.level = levels::LEVELS.len();
        game.apply_powerup(PowerUpKind::Break);
        for _ in 0..240 {
            game.tick(&held(|i| i.right = true), DT);
        }
        assert_eq!(game.state, GameState::Victory);
    }

    #[test]
    fn a_lost_life_shuts_the_gate() {
        let mut game = rallying();
        game.apply_powerup(PowerUpKind::Break);
        game.lose_life();
        assert!(!game.gate_open);
    }

    #[test]
    fn without_the_player_and_warp_features_neither_ever_drops() {
        let mut game = playing_game().with_features(Features {
            player: false,
            warp: false,
            ..Features::ALL
        });
        for _ in 0..3000 {
            game.powerups.clear();
            game.maybe_drop_powerup(400.0, 300.0);
            if let Some(powerup) = game.powerups.first() {
                assert!(!matches!(
                    powerup.kind,
                    PowerUpKind::Player | PowerUpKind::Break
                ));
            }
        }
    }

    // -- asteroids ----------------------------------------------------------

    /// A playing game with its ball launched and kept in flight high up,
    /// away from the paddle and the bricks' reach, for `seconds`.
    fn fly(game: &mut Game, seconds: f32) {
        game.ball.attached = false;
        for _ in 0..(seconds / DT) as usize {
            game.ball.x = 400.0;
            game.ball.y = 300.0;
            game.ball.vx = 0.0;
            game.ball.vy = -1.0;
            game.tick(&Input::default(), DT);
        }
    }

    fn test_asteroid(x: f32, y: f32) -> Asteroid {
        Asteroid {
            x,
            y,
            vx: 0.0,
            vy: ASTEROID_FALL_SPEED,
            radius: ASTEROID_RADIUS,
            drift: 0.0,
            phase: 0.0,
            wobble_rate: 0.0,
        }
    }

    #[test]
    fn asteroids_start_coming_once_the_ball_is_in_flight_and_never_on_the_serve() {
        let mut game = Game::with_seed(7);
        game.state = GameState::Playing;
        for _ in 0..(ASTEROID_FIRST_SPAWN_SECS * 3.0 / DT) as usize {
            game.tick(&Input::default(), DT); // ball still on the paddle
        }
        assert!(game.asteroids.is_empty(), "nothing during a serve");

        fly(&mut game, ASTEROID_FIRST_SPAWN_SECS + 0.1);
        assert_eq!(game.asteroids.len(), 1);
        assert!(
            game.asteroids[0].y < ASTEROID_RADIUS * 2.0,
            "enters at the top"
        );
    }

    #[test]
    fn there_are_never_more_than_three_asteroids() {
        let mut game = Game::with_seed(9);
        game.state = GameState::Playing;
        fly(&mut game, 12.0);
        let mut most = 0;
        for _ in 0..60 {
            fly(&mut game, 1.0);
            // Keep them up high so none leaves the field.
            for asteroid in &mut game.asteroids {
                asteroid.y = 200.0;
            }
            most = most.max(game.asteroids.len());
        }
        assert_eq!(most, MAX_ASTEROIDS);
    }

    #[test]
    fn asteroids_drift_down_and_bounce_off_the_side_walls() {
        let mut asteroid = test_asteroid(ASTEROID_RADIUS + 1.0, 100.0);
        asteroid.drift = -ASTEROID_DRIFT_SPEED;
        for _ in 0..60 {
            asteroid.advance(DT);
            assert!(asteroid.x >= ASTEROID_RADIUS);
        }
        assert!(asteroid.drift > 0.0, "the wall flipped its drift");
        assert!(asteroid.y > 100.0 + ASTEROID_FALL_SPEED * 0.4);
    }

    #[test]
    fn a_ball_blows_up_an_asteroid_and_bounces_off_it_at_the_same_speed() {
        let mut game = playing_game();
        game.bricks.clear();
        game.bricks
            .push(test_brick_at(60.0, 70.0, BrickKind::Normal, 1, 10));
        game.ball = Ball {
            x: 400.0,
            y: 280.0,
            vx: 0.0,
            vy: 300.0,
            radius: BALL_RADIUS,
            attached: false,
            glued: None,
        };
        game.asteroids.push(test_asteroid(404.0, 300.0));
        let score = game.score;
        for _ in 0..10 {
            game.tick(&Input::default(), DT);
            if game.asteroids.is_empty() {
                break;
            }
        }
        assert!(game.asteroids.is_empty(), "blown up");
        assert_eq!(game.score, score + ASTEROID_SCORE);
        assert!(game.ball.vy < 0.0, "bounced back up");
        assert!(game.ball.vx < 0.0, "off the asteroid's left side");
        assert!((game.ball.speed() - 300.0).abs() < 1e-2);
        assert!(game
            .events
            .iter()
            .any(|e| matches!(e, GameEvent::AsteroidExploded { .. })));
    }

    #[test]
    fn an_asteroid_blast_hits_every_destructible_brick_in_range_once() {
        let mut game = playing_game();
        game.bricks = vec![
            test_brick_at(400.0, 300.0, BrickKind::Normal, 1, 10), // under it
            test_brick_at(452.0, 300.0, BrickKind::Normal, 1, 20), // next door
            test_brick_at(400.0, 322.0, BrickKind::Armored, 2, 30), // below
            test_brick_at(348.0, 300.0, BrickKind::Indestructible, 1, 0),
            test_brick_at(600.0, 300.0, BrickKind::Normal, 1, 40), // too far
        ];
        game.explode_asteroid(400.0, 300.0);

        let at = |x: f32, y: f32| game.bricks.iter().find(|b| b.x == x && b.y == y);
        assert!(
            at(400.0, 300.0).is_none() && at(452.0, 300.0).is_none(),
            "in range: gone"
        );
        assert_eq!(
            at(400.0, 322.0).map(|b| b.hits_remaining),
            Some(1),
            "armored: one hit"
        );
        assert!(at(348.0, 300.0).is_some(), "indestructible: untouched");
        assert!(at(600.0, 300.0).is_some(), "out of range: untouched");
        assert_eq!(game.score, ASTEROID_SCORE + 10 + 20);
        let destroyed = game
            .events
            .iter()
            .filter(|e| matches!(e, GameEvent::BrickDestroyedAt { .. }))
            .count();
        assert_eq!(
            destroyed, 2,
            "each destroyed brick is announced, for the renderers"
        );
    }

    #[test]
    fn a_blast_that_takes_the_last_bricks_clears_the_level() {
        let mut game = playing_game();
        game.ball.attached = false;
        game.ball.x = 100.0;
        game.ball.y = 300.0;
        game.ball.vy = -300.0;
        let paddle = game.paddle;
        // The level's only brick, right above where the paddle meets an
        // asteroid: the paddle blows it up, the blast takes the brick.
        game.bricks = vec![test_brick_at(
            paddle.x,
            paddle.top() - ASTEROID_RADIUS * 2.0 - 11.0,
            BrickKind::Normal,
            1,
            10,
        )];
        game.asteroids.push(test_asteroid(
            paddle.x,
            paddle.top() - ASTEROID_RADIUS - 0.1,
        ));
        game.tick(&Input::default(), DT);
        assert!(game.events.contains(&GameEvent::LevelCleared));
        assert_eq!(game.level, 2);
    }

    #[test]
    fn the_forecast_reproduces_a_blast_the_ball_sets_off() {
        // A ball blowing up an asteroid among bricks, then coming down:
        // the forecast must break the same bricks the game does to land
        // where the game's ball lands.
        let mut game = playing_game().with_features(Features::ALL);
        game.asteroid_timer = f32::INFINITY; // no new asteroid mid-test
        game.ball = Ball {
            x: 300.0,
            y: 250.0,
            vx: 150.0,
            vy: -260.0,
            radius: BALL_RADIUS,
            attached: false,
            glued: None,
        };
        game.asteroids = vec![test_asteroid(360.0, 160.0)];
        game.paddle.x = 60.0;
        let line_y = game.paddle.top() - BALL_RADIUS;
        let forecast =
            forecast_crossing(&game.ball, &game.bricks, &game.asteroids, line_y, DT, 600)
                .expect("it comes down within five seconds");
        let bricks_before = game.bricks.len();
        for step in 1..=forecast.ticks as usize {
            game.tick(&Input::default(), DT);
            if step < forecast.ticks as usize {
                let (x, y) = forecast.path[step];
                assert!(
                    (x - game.ball.x).abs() < 1e-3 && (y - game.ball.y).abs() < 1e-3,
                    "tick {step}: forecast ({x}, {y}), game ({}, {})",
                    game.ball.x,
                    game.ball.y
                );
            }
        }
        assert!(
            game.bricks.len() + 2 <= bricks_before,
            "the blast broke bricks"
        );
    }

    #[test]
    fn the_paddle_blows_up_an_asteroid_it_touches() {
        let mut game = playing_game();
        game.ball.attached = false;
        game.ball.x = 100.0;
        game.ball.y = 300.0;
        game.ball.vy = -300.0;
        let paddle = game.paddle;
        game.asteroids.push(test_asteroid(
            paddle.x,
            paddle.top() - ASTEROID_RADIUS - 0.1,
        ));
        game.tick(&Input::default(), DT);
        assert!(game.asteroids.is_empty());
        assert_eq!(game.score, ASTEROID_SCORE);
        assert_eq!(game.lives, STARTING_LIVES, "no harm done");
    }

    #[test]
    fn an_asteroid_past_the_paddle_just_leaves() {
        let mut game = playing_game();
        game.ball.attached = false;
        game.ball.x = 100.0;
        game.ball.y = 300.0;
        game.ball.vy = -300.0;
        game.asteroids.push(test_asteroid(
            700.0,
            game.paddle.bottom() + ASTEROID_RADIUS - 0.1,
        ));
        game.tick(&Input::default(), DT);
        assert!(game.asteroids.is_empty());
        assert_eq!(game.score, 0, "no points for one that got away");
    }

    #[test]
    fn a_lost_life_clears_the_asteroids() {
        let mut game = playing_game();
        game.asteroids.push(test_asteroid(400.0, 300.0));
        game.lose_life();
        assert!(game.asteroids.is_empty());
    }

    #[test]
    fn without_the_asteroids_feature_none_ever_comes() {
        let mut game = Game::with_seed(7).with_features(Features::V1);
        game.state = GameState::Playing;
        fly(&mut game, 30.0);
        assert!(game.asteroids.is_empty());
    }

    // -- stalls -------------------------------------------------------------

    #[test]
    fn a_ball_with_no_progress_for_ten_seconds_is_turned_on_its_next_bounce() {
        // No asteroids: one drifting by would deflect the ball first.
        let mut game = playing_game().with_features(Features {
            asteroids: false,
            ..Features::ALL
        });
        // Straight up and down between the ceiling and an indestructible
        // brick: a closed orbit no paddle can reach.
        game.bricks = vec![test_brick_at(400.0, 300.0, BrickKind::Indestructible, 1, 0)];
        game.bricks
            .push(test_brick_at(60.0, 70.0, BrickKind::Normal, 1, 10));
        game.ball = Ball {
            x: 400.0,
            y: 200.0,
            vx: 0.0,
            vy: -300.0,
            radius: BALL_RADIUS,
            attached: false,
            glued: None,
        };
        for _ in 0..((STALL_SECS - 0.5) / DT) as usize {
            game.tick(&Input::default(), DT);
        }
        assert_eq!(game.ball.vx, 0.0, "untouched before the stall limit");
        for _ in 0..(2.0 / DT) as usize {
            game.tick(&Input::default(), DT);
        }
        assert!(game.ball.vx != 0.0, "turned once stalled");
        let angle = (game.ball.vy.abs()).atan2(game.ball.vx.abs()).to_degrees();
        assert!(angle >= STALL_MIN_ANGLE_DEG);
        assert!((game.ball.speed() - 300.0).abs() < 1e-2, "speed kept");
    }

    #[test]
    fn hitting_the_paddle_is_progress_and_restarts_the_stall_clock() {
        let mut game = playing_game();
        game.ball = Ball {
            x: game.paddle.x,
            y: game.paddle.top() - BALL_RADIUS - 1.0,
            vx: 0.0,
            vy: 300.0,
            radius: BALL_RADIUS,
            attached: false,
            glued: None,
        };
        game.stall_secs = STALL_SECS - 1.0;
        game.tick(&Input::default(), DT);
        assert!(game.events.contains(&GameEvent::PaddleHit));
        assert_eq!(game.stall_secs, 0.0);
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

    // -- deterministic replay ---------------------------------------------
    //
    // v2's cross-workstream invariant: "a fixed seed + scripted input
    // produces the same state hash before and after every workstream" is
    // the contract that a workstream changed presentation, not physics.
    // These two tests are that contract, pinned against *this* commit's
    // simulation -- levels/textures/3D-presentation work must never touch
    // game.rs in a way that moves the pinned hash below.

    /// Hashes the physics-relevant fields of `Game` -- everything the
    /// simulation actually computes, not `rng`'s internal state or the
    /// per-tick `events` scratch buffer (draining events is a rendering
    /// concern, not part of "the same game state"). f32 fields hash by
    /// bit pattern (f32 itself isn't Hash/Eq, because of NaN) and enums
    /// hash by their Debug string -- the simplest thing that can't
    /// silently drift if a variant gets renamed.
    fn state_hash(game: &Game) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        fn hash_ball(h: &mut impl Hasher, ball: &Ball) {
            h.write_u32(ball.x.to_bits());
            h.write_u32(ball.y.to_bits());
            h.write_u32(ball.vx.to_bits());
            h.write_u32(ball.vy.to_bits());
            h.write_u32(ball.radius.to_bits());
            ball.attached.hash(h);
        }

        let mut h = DefaultHasher::new();
        h.write_u32(game.paddle.x.to_bits());
        h.write_u32(game.paddle.y.to_bits());
        h.write_u32(game.paddle.width.to_bits());
        h.write_u32(game.paddle.height.to_bits());
        hash_ball(&mut h, &game.ball);
        game.extra_balls.len().hash(&mut h);
        for ball in &game.extra_balls {
            hash_ball(&mut h, ball);
        }
        game.lives.hash(&mut h);
        format!("{:?}", game.state).hash(&mut h);
        game.bricks.len().hash(&mut h);
        for brick in &game.bricks {
            h.write_u32(brick.x.to_bits());
            h.write_u32(brick.y.to_bits());
            h.write_u32(brick.width.to_bits());
            h.write_u32(brick.height.to_bits());
            format!("{:?}", brick.kind).hash(&mut h);
            brick.hits_remaining.hash(&mut h);
            brick.score.hash(&mut h);
        }
        game.score.hash(&mut h);
        game.level.hash(&mut h);
        game.powerups.len().hash(&mut h);
        for powerup in &game.powerups {
            h.write_u32(powerup.x.to_bits());
            h.write_u32(powerup.y.to_bits());
            format!("{:?}", powerup.kind).hash(&mut h);
        }
        h.write_u32(game.widen_timer.to_bits());
        h.finish()
    }

    /// The scripted run pinned as the frozen-physics baseline: a fixed
    /// seed, launch the ball, then hold the paddle still through several
    /// seconds of bounces/brick hits/life losses -- long enough to
    /// exercise `maybe_drop_powerup`'s RNG draw at least once, so the
    /// property actually covers the one non-deterministic decision
    /// `game.rs` makes. Both tests below replay this same script.
    fn run_scripted_replay(seed: u64) -> u64 {
        run_scripted_replay_with(seed, Features::V1)
    }

    /// The same script with any `features`: v1's rules for the pinned
    /// baseline, every addition for the determinism check below.
    fn run_scripted_replay_with(seed: u64, features: Features) -> u64 {
        let mut game = Game::with_seed(seed).with_features(features);
        game.state = GameState::Playing;
        game.tick(&held(|i| i.space = true), DT);
        for _ in 0..(120 * 25) {
            game.tick(&Input::default(), DT);
        }
        state_hash(&game)
    }

    #[test]
    fn deterministic_replay_same_seed_and_script_always_matches() {
        assert_eq!(
            run_scripted_replay(42),
            run_scripted_replay(42),
            "same seed + same scripted input must reproduce the exact same state"
        );
    }

    #[test]
    fn deterministic_replay_with_every_addition_also_matches() {
        assert_eq!(
            run_scripted_replay_with(42, Features::ALL),
            run_scripted_replay_with(42, Features::ALL),
        );
    }

    #[test]
    fn deterministic_replay_matches_the_pinned_v1_baseline() {
        // Pinned against the pre-v2 simulation (see docs/pact-audit.md /
        // the v2 spec for why). A failure here means something in
        // game.rs's *physics* changed -- v2's workstreams are supposed to
        // leave this frozen; re-pin only after confirming the change was
        // an intentional, spec-approved game.rs edit, not an accident.
        const V1_BASELINE_HASH: u64 = 16597883421283269447;
        assert_eq!(run_scripted_replay(42), V1_BASELINE_HASH);
    }
}
