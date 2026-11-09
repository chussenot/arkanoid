//! Autopilot: plays the game by producing the same [`Input`] a keyboard
//! would, one fixed tick at a time.
//!
//! Two layers, split by what each is good at:
//!
//! - **Strategy** (which lane of the board to attack next) is a judgment.
//!   It is asked of a [`Strategist`] at every serve and every
//!   `consult_every` paddle hits; with the `typesafe` feature that is a
//!   TypeSafe System One model through the `judgment` crate
//!   (`advisor.rs`). The answer arrives asynchronously, a few hundred
//!   milliseconds later, while the ball is still on its way up.
//! - **Piloting** (where the ball will meet the paddle, which paddle offset
//!   sends it at the chosen lane, catching a power-up when it is safe) is
//!   geometry the simulation already defines, so it is computed every tick
//!   (`pilot.rs`) and never waits on the network.
//!
//! A model answer is only a preference: it is used when it arrives in time,
//! names a lane with bricks left in it, and is confident enough
//! ([`MIN_CONFIDENCE`]); otherwise the deterministic lane pick
//! ([`Board::heuristic_lane`]) stands. The game therefore plays the same
//! with no key, no network or a slow model; the model can only change
//! where the shots go.
//!
//! Lives in its own module, outside `game.rs`, so the simulation keeps no
//! dependency on it: the autopilot reads `Game` and writes `Input`, exactly
//! like `main.rs`'s keyboard handling.

#[cfg(feature = "typesafe")]
pub mod advisor;
pub mod board;
pub mod pilot;

use std::fmt;
use std::time::Duration;

use crate::events::{GameEvent, PowerUpKind};
use crate::game::{
    forecast_crossing, Ball, Forecast, Game, GameState, Input, Paddle, PADDLE_SPEED,
    PLAYFIELD_WIDTH, POWERUP_FALL_SPEED,
};
use board::{Board, LANES};

/// Lowest confidence at which a model's lane is used over the heuristic.
/// TypeSafe's Choice confidence is `(p_max − 1/n) / (1 − 1/n)`: over five
/// lanes 0.3 means the top lane holds at least 44% of the probability,
/// more than twice an even split.
pub const MIN_CONFIDENCE: f64 = 0.3;

/// How long a serve waits for the model's first answer before launching on
/// the heuristic lane anyway. Hosted p50 is about 250 ms.
const SERVE_WAIT_SECS: f32 = 1.5;

/// Paddle hits in a row without a brick broken after which the shot counts
/// as looping (typically between indestructible bricks and the paddle):
/// the model's lane is overridden, and its answers are set aside, until a
/// brick breaks, while the heuristic rotation looks for a way through.
/// Setting answers aside, not only the current lane, is what stops a model
/// that keeps naming the same unreachable lane from re-entering the loop on
/// its very next answer.
const LOOP_OVERRIDE_RALLIES: u32 = 4;

/// Spare time the autopilot wants, beyond the slide itself, before it
/// heads out through an open Break gate.
const WARP_MARGIN_SECS: f32 = 0.2;

/// How far ahead a forecast looks, in ticks (three seconds). A ball still
/// up among the bricks past that gets the straight-line guess instead.
const FORECAST_TICKS: u32 = 360;

/// Shortens a power-up catch's return trip estimate: a rising ball's
/// predicted landing is coarse until it has bounced off whatever it hits.
const CATCH_RETURN_MARGIN: f32 = 0.6;

/// Lane names, in lane order: the keys a model chooses between.
pub const LANE_NAMES: [&str; LANES] = ["far_left", "left", "center", "right", "far_right"];

/// What the strategist is told about the game at a decision point.
// Only the TypeSafe advisor reads every field; without the feature the
// strategists are the tests' stand-ins, which read none.
#[cfg_attr(not(feature = "typesafe"), allow(dead_code))]
#[derive(Debug, Clone, PartialEq)]
pub struct Situation {
    pub board: Board,
    /// Balls in flight (empty at a serve).
    pub balls: Vec<Ball>,
    pub paddle: Paddle,
    pub lives: u32,
    pub level: usize,
    pub rallies_without_progress: u32,
    /// Asteroid centers.
    pub asteroids: Vec<(f32, f32)>,
}

/// A strategist's answer: the lane to attack and how sure it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Advice {
    pub lane: usize,
    pub confidence: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// A strategist's reply to request `id`.
#[derive(Debug)]
pub struct Reply {
    pub id: u64,
    pub outcome: Result<Advice, String>,
    pub latency: Duration,
}

/// Something that answers "which lane next" for a [`Situation`]. Requests
/// are fire-and-forget; replies are collected by [`Strategist::poll`], so an
/// implementation that talks to a network never blocks a tick.
pub trait Strategist {
    /// Shown in logs and the session summary.
    fn describe(&self) -> String;
    fn request(&mut self, id: u64, situation: Situation);
    fn poll(&mut self) -> Option<Reply>;
}

/// Where the current lane came from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Source {
    Heuristic,
    Model,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Plan {
    lane: usize,
    source: Source,
}

/// Counters for the session summary.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Stats {
    pub rallies: u32,
    pub requests: u32,
    pub accepted: u32,
    pub low_confidence: u32,
    /// Named a lane with nothing left to break.
    pub empty_lane: u32,
    /// Arrived after a newer request was sent.
    pub late: u32,
    pub errors: u32,
    pub loop_overrides: u32,
    pub latency_total: Duration,
    pub latency_max: Duration,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl fmt::Display for Stats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let answered = self.accepted + self.low_confidence + self.empty_lane + self.late;
        let mean_ms = if answered > 0 {
            self.latency_total.as_millis() / u128::from(answered)
        } else {
            0
        };
        write!(
            f,
            "{} rallies, {} consultations: {} used, {} below confidence {MIN_CONFIDENCE}, \
             {} empty lane, {} late, {} failed, {} loop overrides; latency mean {mean_ms} ms, \
             max {} ms; tokens {} in / {} out",
            self.rallies,
            self.requests,
            self.accepted,
            self.low_confidence,
            self.empty_lane,
            self.late,
            self.errors,
            self.loop_overrides,
            self.latency_max.as_millis(),
            self.input_tokens,
            self.output_tokens,
        )
    }
}

/// Where a serve stands.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Serve {
    /// Ball in flight.
    InPlay,
    /// Ball on the paddle; seconds spent waiting so far.
    Waiting(f32),
}

pub struct Autopilot {
    strategist: Option<Box<dyn Strategist>>,
    consult_every: u32,
    plan: Plan,
    /// The request whose reply would still be used.
    pending: Option<u64>,
    next_id: u64,
    serve: Serve,
    /// Paddle hits since a brick last broke.
    fruitless: u32,
    /// Paddle hits since the strategist was last asked.
    since_consulted: u32,
    log: bool,
    stats: Stats,
    /// Space as held on the last tick. Releasing a glued ball takes a
    /// press, so a release is a false-to-true edge, never a held key.
    space_held: bool,
    /// One forecast per ball in the fleet (`game.ball` first), replayed a
    /// tick at a time while the ball stays on it, so a new one is computed
    /// only when something the forecast left out happens: the paddle, a
    /// new asteroid, another ball breaking a brick first.
    forecasts: Vec<Option<Tracked>>,
}

/// A forecast and how many ticks of it have played out.
#[derive(Debug, Clone)]
struct Tracked {
    forecast: Forecast,
    step: usize,
}

impl Autopilot {
    /// An autopilot consulting `strategist` at every serve and every
    /// `consult_every` paddle hits (at least 1); `None` plays on the
    /// heuristic alone.
    pub fn new(strategist: Option<Box<dyn Strategist>>, consult_every: u32) -> Self {
        Self {
            strategist,
            consult_every: consult_every.max(1),
            plan: Plan {
                lane: LANES / 2,
                source: Source::Heuristic,
            },
            pending: None,
            next_id: 0,
            serve: Serve::InPlay,
            fruitless: 0,
            since_consulted: 0,
            log: false,
            stats: Stats::default(),
            space_held: false,
            forecasts: Vec::new(),
        }
    }

    /// Print one line per decision to stdout.
    pub fn with_log(mut self, log: bool) -> Self {
        self.log = log;
        self
    }

    pub fn describe(&self) -> String {
        match &self.strategist {
            Some(s) => format!(
                "{}, consulted every {} rally(ies)",
                s.describe(),
                self.consult_every
            ),
            None => "heuristic only".to_owned(),
        }
    }

    pub fn stats(&self) -> &Stats {
        &self.stats
    }

    /// The keys to hold for the next tick of `game`.
    pub fn input(&mut self, game: &Game, dt: f32) -> Input {
        self.collect_replies(game);
        let mut input = Input::default();
        match game.state {
            GameState::Menu => input.space = true,
            GameState::Playing => self.pilot(game, dt, &mut input),
            GameState::Paused | GameState::GameOver | GameState::Victory => {}
        }
        self.space_held = input.space;
        input
    }

    /// Reads the events of the tick that just ran (call before they are
    /// drained).
    pub fn observe(&mut self, game: &Game) {
        for event in &game.events {
            match event {
                GameEvent::PaddleHit => {
                    self.stats.rallies += 1;
                    self.fruitless += 1;
                    self.since_consulted += 1;
                    self.replan(game);
                }
                GameEvent::BrickDestroyed => self.fruitless = 0,
                GameEvent::LevelCleared => self.fruitless = 0,
                _ => {}
            }
        }
    }

    fn pilot(&mut self, game: &Game, dt: f32, input: &mut Input) {
        let paddle = &game.paddle;
        let board = Board::from_game(game);

        if game.ball.attached {
            let waited = match self.serve {
                Serve::InPlay => {
                    // A new serve: start of the game, a level, or a life.
                    self.fruitless = 0;
                    self.since_consulted = 0;
                    self.plan = self.heuristic_plan(&board);
                    self.consult(game, board.clone());
                    0.0
                }
                Serve::Waiting(waited) => waited,
            };
            self.serve = Serve::Waiting(waited + dt);

            let target = self.aim_point(&board);
            let serve_x = pilot::serve_x_for(target, &game.ball, paddle);
            (input.left, input.right) = pilot::steer(paddle.x, serve_x, dt);
            let in_position = !input.left && !input.right;
            let answer_settled = self.pending.is_none() || waited >= SERVE_WAIT_SECS;
            input.space = in_position && answer_settled;
            return;
        }
        self.serve = Serve::InPlay;

        // A glued ball leaves with the velocity its paddle hit gave it, from
        // wherever the paddle carries it: line it up with the target, then
        // press Space. Not while another ball is in flight, though: the
        // paddle is needed under that one, so the glued ball goes at once.
        let fleet = || std::iter::once(&game.ball).chain(&game.extra_balls);
        if let Some(ball) = fleet().find(|b| b.glued.is_some()) {
            let offset = ball.glued.map_or(0.0, |g| g.offset);
            let others_in_flight = fleet().any(|b| b.glued.is_none() && !b.attached);
            let release_x = pilot::release_x_for(self.aim_point(&board), ball, offset, paddle);
            let ready = match release_x {
                Some(x) if !others_in_flight => {
                    (input.left, input.right) = pilot::steer(paddle.x, x, dt);
                    !input.left && !input.right
                }
                _ => true,
            };
            // A press is an edge: let go for a tick if Space is still down.
            input.space = ready && !self.space_held;
            if !others_in_flight {
                return;
            }
        }

        let Some(hit) = self.next_intercept(game, dt) else {
            return;
        };
        let line_y = pilot::contact_line(paddle, &game.ball);
        let safe_x = pilot::paddle_x_for(hit.x, 0.0, paddle);
        let offset = pilot::aim_offset(hit.x, line_y, self.aim_point(&board));
        let aimed_x = pilot::paddle_x_for(hit.x, offset, paddle);
        let mut target_x = if hit.exact && pilot::reachable(paddle.x, aimed_x, hit.time) {
            aimed_x
        } else {
            safe_x
        };

        // A power-up is worth a detour only if the paddle can make it back
        // in time for the ball. Which helpful one is worth more is moot;
        // Tiny, the one that hurts, is never chased.
        let catch = game
            .powerups
            .iter()
            .filter(|p| p.kind != PowerUpKind::Tiny)
            .filter_map(|p| {
                let time = (paddle.top() - p.y) / POWERUP_FALL_SPEED;
                let detour_ok = time >= 0.0
                    && time < hit.time
                    && pilot::reachable(paddle.x, p.x, time)
                    && pilot::reachable(p.x, target_x, (hit.time - time) * CATCH_RETURN_MARGIN);
                detour_ok.then_some((time, p.x))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, x)) = catch {
            target_x = x;
        }

        // An open Break gate skips the rest of the level: worth more than
        // any catch, as long as the paddle is all the way through before
        // the next ball reaches it (with a margin for the deadband).
        if game.gate_open {
            let through_x = PLAYFIELD_WIDTH + paddle.width / 2.0;
            let time = (through_x - paddle.x) / PADDLE_SPEED;
            if time + WARP_MARGIN_SECS < hit.time {
                target_x = through_x + PADDLE_SPEED * dt;
            }
        }

        (input.left, input.right) = pilot::steer(paddle.x, target_x, dt);

        // A gun fires while Space is held, and every bolt can only help.
        // A glued ball's release owns Space while there is one (above).
        if game.gun.is_some() && fleet().all(|b| b.glued.is_none()) {
            input.space = true;
        }
    }

    /// The ball the paddle must deal with next, the one reaching its
    /// contact line soonest, and where and when it gets there: from an exact
    /// forecast when one reaches the line within `FORECAST_TICKS`, the
    /// straight-line guess otherwise. Every other ball in a multiball is on
    /// its own; the game only spends a life when the last one drops.
    fn next_intercept(&mut self, game: &Game, dt: f32) -> Option<pilot::Intercept> {
        let fleet: Vec<&Ball> = std::iter::once(&game.ball)
            .chain(&game.extra_balls)
            .collect();
        self.forecasts.resize(fleet.len(), None);
        let mut best: Option<pilot::Intercept> = None;
        for (ball, tracked) in fleet.iter().zip(self.forecasts.iter_mut()) {
            if ball.attached || ball.glued.is_some() {
                *tracked = None;
                continue;
            }
            let line_y = pilot::contact_line(&game.paddle, ball);
            let on_track = tracked.as_mut().is_some_and(|t| {
                t.step += 1;
                t.forecast
                    .path
                    .get(t.step)
                    .is_some_and(|&(x, y)| (x - ball.x).abs() < 1e-3 && (y - ball.y).abs() < 1e-3)
            });
            if !on_track {
                *tracked = forecast_crossing(
                    ball,
                    &game.bricks,
                    &game.asteroids,
                    line_y,
                    dt,
                    FORECAST_TICKS,
                )
                .map(|forecast| Tracked { forecast, step: 0 });
            }
            let hit = match tracked {
                Some(t) => pilot::Intercept {
                    x: t.forecast.x,
                    time: (t.forecast.ticks as usize).saturating_sub(t.step) as f32 * dt,
                    exact: true,
                },
                None => match pilot::predict_intercept(ball, line_y) {
                    Some(guess) => guess,
                    None => continue,
                },
            };
            if best.is_none_or(|b| hit.time < b.time) {
                best = Some(hit);
            }
        }
        best
    }

    /// The point a shot is aimed at: the plan's lane target, nudged
    /// sideways while shots are not breaking anything so a looping
    /// trajectory does not repeat exactly. Once the shot counts as looping
    /// (`LOOP_OVERRIDE_RALLIES`), the nudge becomes a sweep of the whole
    /// field: the bricks left at that point are often reachable only by a
    /// bank shot no lane target describes, and an aim this precise would
    /// otherwise replay the same few trajectories forever.
    fn aim_point(&self, board: &Board) -> (f32, f32) {
        let (x, y) = board.lanes[self.plan.lane].target;
        if self.fruitless < 2 {
            return (x, y);
        }
        if self.fruitless < LOOP_OVERRIDE_RALLIES {
            let nudge = ((self.fruitless * 23) % 40) as f32 - 20.0;
            return (x + nudge, y);
        }
        // Golden-ratio steps: successive shots spread evenly over the
        // width and never fall back into a short cycle.
        let fraction = (self.fruitless as f32 * 0.618_034).fract();
        (40.0 + fraction * (crate::game::PLAYFIELD_WIDTH - 80.0), y)
    }

    /// After a paddle hit: keep, replace or re-ask for the lane.
    fn replan(&mut self, game: &Game) {
        let board = Board::from_game(game);
        let looping = self.fruitless >= LOOP_OVERRIDE_RALLIES;
        let lane_spent = board.lanes[self.plan.lane].destructible == 0;
        if looping && self.plan.source == Source::Model {
            self.stats.loop_overrides += 1;
        }
        if looping || lane_spent || self.plan.source == Source::Heuristic {
            self.plan = self.heuristic_plan(&board);
        }
        if self.since_consulted >= self.consult_every {
            self.since_consulted = 0;
            self.consult(game, board);
        }
    }

    fn heuristic_plan(&self, board: &Board) -> Plan {
        // Every fruitless rally past the first steps one lane down the
        // ranking.
        let skip = self.fruitless.saturating_sub(1) as usize;
        Plan {
            lane: board.heuristic_lane(skip),
            source: Source::Heuristic,
        }
    }

    fn consult(&mut self, game: &Game, board: Board) {
        let Some(strategist) = &mut self.strategist else {
            return;
        };
        let id = self.next_id;
        self.next_id += 1;
        self.pending = Some(id);
        self.stats.requests += 1;
        strategist.request(
            id,
            Situation {
                board,
                balls: std::iter::once(&game.ball)
                    .chain(&game.extra_balls)
                    .filter(|b| !b.attached)
                    .copied()
                    .collect(),
                paddle: game.paddle,
                lives: game.lives,
                level: game.level,
                rallies_without_progress: self.fruitless,
                asteroids: game.asteroids.iter().map(|a| (a.x, a.y)).collect(),
            },
        );
    }

    fn collect_replies(&mut self, game: &Game) {
        let Some(strategist) = &mut self.strategist else {
            return;
        };
        while let Some(reply) = strategist.poll() {
            let advice = match reply.outcome {
                Ok(advice) => advice,
                Err(error) => {
                    self.stats.errors += 1;
                    if self.pending == Some(reply.id) {
                        self.pending = None;
                    }
                    eprintln!("autopilot: strategist failed, playing on the heuristic: {error}");
                    continue;
                }
            };
            self.stats.latency_total += reply.latency;
            self.stats.latency_max = self.stats.latency_max.max(reply.latency);
            self.stats.input_tokens += advice.input_tokens;
            self.stats.output_tokens += advice.output_tokens;

            let verdict = if self.pending != Some(reply.id) {
                self.stats.late += 1;
                "late, ignored"
            } else if self.fruitless >= LOOP_OVERRIDE_RALLIES {
                self.stats.loop_overrides += 1;
                "shots are looping, heuristic kept until a brick breaks"
            } else if advice.confidence < MIN_CONFIDENCE {
                self.stats.low_confidence += 1;
                "below the confidence bar, heuristic kept"
            } else if Board::from_game(game).lanes[advice.lane].destructible == 0 {
                self.stats.empty_lane += 1;
                "no bricks left there, heuristic kept"
            } else {
                self.stats.accepted += 1;
                self.plan = Plan {
                    lane: advice.lane,
                    source: Source::Model,
                };
                "used"
            };
            if self.pending == Some(reply.id) {
                self.pending = None;
            }
            if self.log {
                println!(
                    "autopilot: #{} level {} -> {} (confidence {:.2}, {} ms): {verdict}",
                    reply.id,
                    game.level,
                    LANE_NAMES[advice.lane],
                    advice.confidence,
                    reply.latency.as_millis(),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::GLUE_HOLD_SECS;
    use std::collections::VecDeque;

    const DT: f32 = 1.0 / 120.0;

    /// Plays `game` with `autopilot` until `done`, a terminal state, or
    /// `max_ticks`.
    fn play(
        game: &mut Game,
        autopilot: &mut Autopilot,
        max_ticks: u32,
        done: impl Fn(&Game) -> bool,
    ) {
        for _ in 0..max_ticks {
            let input = autopilot.input(game, DT);
            game.tick(&input, DT);
            autopilot.observe(game);
            game.events.clear();
            if done(game) || matches!(game.state, GameState::Victory | GameState::GameOver) {
                return;
            }
        }
    }

    fn won(game: &Game) -> bool {
        game.state == GameState::Victory
    }

    /// Answers every request at once with a fixed lane, so a test runs
    /// the model path deterministically.
    struct Scripted {
        lane: usize,
        confidence: f64,
        replies: VecDeque<Reply>,
    }

    impl Strategist for Scripted {
        fn describe(&self) -> String {
            "scripted".to_owned()
        }
        fn request(&mut self, id: u64, _situation: Situation) {
            self.replies.push_back(Reply {
                id,
                outcome: Ok(Advice {
                    lane: self.lane,
                    confidence: self.confidence,
                    input_tokens: 0,
                    output_tokens: 0,
                }),
                latency: Duration::ZERO,
            });
        }
        fn poll(&mut self) -> Option<Reply> {
            self.replies.pop_front()
        }
    }

    /// Never answers: a model that is down, or slower than the game.
    struct Silent;

    impl Strategist for Silent {
        fn describe(&self) -> String {
            "silent".to_owned()
        }
        fn request(&mut self, _id: u64, _situation: Situation) {}
        fn poll(&mut self) -> Option<Reply> {
            None
        }
    }

    /// Game time allowed for the three built-in levels: fifteen minutes,
    /// against the 348 to 626 s the heuristic takes across seeds (level 3,
    /// entered through one four-brick gate, is most of it).
    const ALL_LEVELS: u32 = 120 * 900;

    #[test]
    fn the_heuristic_alone_wins_the_built_in_levels() {
        for seed in [1, 7, 42] {
            let mut game = Game::with_seed(seed);
            let mut autopilot = Autopilot::new(None, 1);
            play(&mut game, &mut autopilot, ALL_LEVELS, won);
            assert!(
                won(&game),
                "seed {seed}: {:?} at level {}, {} bricks left, {} lives, {:?}",
                game.state,
                game.level,
                game.bricks.len(),
                game.lives,
                autopilot.stats()
            );
        }
    }

    #[test]
    fn the_heuristic_alone_wins_a_whole_level_set() {
        let set = crate::levelset::load_file(std::path::Path::new("tests/fixtures/homemade.lbl"))
            .expect("fixture");
        let mut game = Game::with_seed(2);
        game.load_external_levels(set.levels);
        let mut autopilot = Autopilot::new(None, 1);
        play(&mut game, &mut autopilot, ALL_LEVELS, |_| false);
        assert_eq!(
            game.state,
            GameState::Victory,
            "level {}, {} bricks left, {:?}",
            game.level,
            game.bricks.len(),
            autopilot.stats()
        );
    }

    #[test]
    fn a_confident_model_lane_is_used_until_its_bricks_are_gone() {
        let mut game = Game::with_seed(3);
        let strategist = Scripted {
            lane: 0,
            confidence: 0.9,
            replies: VecDeque::new(),
        };
        let mut autopilot = Autopilot::new(Some(Box::new(strategist)), 1);
        play(&mut game, &mut autopilot, ALL_LEVELS, won);
        assert!(won(&game), "{:?}", autopilot.stats());
        let stats = autopilot.stats();
        assert!(stats.accepted > 0, "{stats:?}");
        assert!(stats.empty_lane > 0, "the far-left lane empties: {stats:?}");
        // On level 3 the far-left bricks sit behind indestructible ones,
        // reachable only through the gate: a model that keeps naming that
        // lane loops until its answers are set aside.
        assert!(stats.loop_overrides > 0, "{stats:?}");
    }

    #[test]
    fn a_model_below_the_confidence_bar_is_never_used() {
        let mut game = Game::with_seed(3);
        let strategist = Scripted {
            lane: 0,
            confidence: MIN_CONFIDENCE / 2.0,
            replies: VecDeque::new(),
        };
        let mut autopilot = Autopilot::new(Some(Box::new(strategist)), 1);
        play(&mut game, &mut autopilot, ALL_LEVELS / 2, |_| false);
        let stats = autopilot.stats();
        assert_eq!(stats.accepted, 0);
        assert!(stats.low_confidence > 0);
        assert_eq!(autopilot.plan.source, Source::Heuristic);
    }

    #[test]
    fn a_silent_model_delays_each_serve_but_never_stops_the_game() {
        let mut game = Game::with_seed(5);
        let mut autopilot = Autopilot::new(Some(Box::new(Silent)), 1);
        play(&mut game, &mut autopilot, ALL_LEVELS, won);
        assert!(won(&game), "{:?}", autopilot.stats());
        assert_eq!(autopilot.stats().accepted, 0);
    }

    #[test]
    fn consult_every_spaces_out_the_consultations() {
        let run = |every| {
            let mut game = Game::with_seed(9);
            let strategist = Scripted {
                lane: 2,
                confidence: 0.9,
                replies: VecDeque::new(),
            };
            let mut autopilot = Autopilot::new(Some(Box::new(strategist)), every);
            play(&mut game, &mut autopilot, ALL_LEVELS / 2, |_| false);
            autopilot.stats().clone()
        };
        let every_rally = run(1);
        let every_fourth = run(4);
        assert!(every_rally.requests > every_fourth.requests * 2);
    }

    #[test]
    fn the_autopilot_aims_and_releases_glued_balls_itself() {
        let mut game = Game::with_seed(1);
        game.state = GameState::Playing;
        let mut autopilot = Autopilot::new(None, 1);
        let mut releases = Vec::new();
        for _ in 0..ALL_LEVELS {
            game.glue_timer = 10.0; // Glue all game long.
            let held = game.ball.glued.map(|g| g.held);
            let input = autopilot.input(&game, DT);
            game.tick(&input, DT);
            autopilot.observe(&game);
            game.events.clear();
            if let (Some(held), None) = (held, game.ball.glued) {
                releases.push(held);
            }
            if releases.len() >= 20 || game.state != GameState::Playing {
                break;
            }
        }
        assert!(releases.len() >= 20, "only {} releases", releases.len());
        let pressed = releases
            .iter()
            .filter(|&&h| h < GLUE_HOLD_SECS - DT)
            .count();
        assert!(
            pressed * 10 >= releases.len() * 9,
            "most releases are Space presses, not the hold limit: {releases:?}"
        );
    }

    #[test]
    fn the_autopilot_fires_a_gun_and_leaves_a_tiny_capsule_alone() {
        let mut game = Game::with_seed(4);
        game.state = GameState::Playing;
        let mut autopilot = Autopilot::new(None, 1);
        // Serve, then give the paddle a laser.
        for _ in 0..240 {
            let input = autopilot.input(&game, DT);
            game.tick(&input, DT);
            autopilot.observe(&game);
            game.events.clear();
        }
        assert!(!game.ball.attached);
        game.gun = Some(crate::game::Gun {
            kind: crate::game::GunKind::Laser,
            timer: 10.0,
        });
        assert!(autopilot.input(&game, DT).space, "holds Space to fire");

        // A capsule falling beside the paddle with time to spare (a slow
        // ball high up, one brick left in a far corner): a Widen is worth
        // the detour, a Tiny is treated as if it were not there.
        let steer_with = |powerups: Vec<crate::game::PowerUp>| {
            let mut probe = Game::with_seed(4).with_features(crate::game::Features::V1);
            probe.state = GameState::Playing;
            probe.paddle.x = 400.0;
            probe.ball.attached = false;
            (probe.ball.x, probe.ball.y, probe.ball.vx, probe.ball.vy) =
                (400.0, 300.0, 10.0, -150.0);
            probe.bricks.retain(|b| b.x < 100.0 && b.y < 80.0);
            probe.powerups = powerups;
            let input = Autopilot::new(None, 1).input(&probe, DT);
            (input.left, input.right)
        };
        let capsule = |kind| crate::game::PowerUp {
            x: 200.0,
            y: 300.0,
            kind,
        };
        let none = steer_with(vec![]);
        let widen = steer_with(vec![capsule(PowerUpKind::Widen)]);
        let tiny = steer_with(vec![capsule(PowerUpKind::Tiny)]);
        assert_eq!(widen, (true, false), "goes for a Widen");
        assert_ne!(
            none,
            (true, false),
            "the ball alone would not take it there"
        );
        assert_eq!(tiny, none, "ignores a Tiny");
    }

    #[test]
    fn the_autopilot_takes_an_open_gate_when_it_has_the_time_and_not_otherwise() {
        let setup = |ball_y: f32, vy: f32| {
            let mut game = Game::with_seed(4).with_features(crate::game::Features::V1);
            game.state = GameState::Playing;
            game.paddle.x = 300.0;
            game.ball.attached = false;
            (game.ball.x, game.ball.y, game.ball.vx, game.ball.vy) = (300.0, ball_y, 0.0, vy);
            game.gate_open = true;
            game
        };

        // A ball high up and rising: plenty of time. The autopilot drives
        // out through the gate and the level is left.
        let mut game = setup(300.0, -150.0);
        let mut autopilot = Autopilot::new(None, 1);
        for _ in 0..240 {
            let input = autopilot.input(&game, DT);
            game.tick(&input, DT);
            autopilot.observe(&game);
            game.events.clear();
            if game.level == 2 {
                break;
            }
        }
        assert_eq!(game.level, 2, "warped");

        // A ball about to reach the paddle: it stays under the ball.
        let game = setup(500.0, 300.0);
        let input = Autopilot::new(None, 1).input(&game, DT);
        assert!(!input.right, "no dash for the gate with the ball arriving");
    }

    #[test]
    fn the_autopilot_starts_the_game_from_the_menu() {
        let mut game = Game::with_seed(1);
        let mut autopilot = Autopilot::new(None, 1);
        assert!(autopilot.input(&game, DT).space);
        game.tick(&autopilot.input(&game, DT), DT);
        assert_eq!(game.state, GameState::Playing);
    }
}

/// Not part of `cargo test`: plays every feature set over twenty seeds and
/// prints wins, lives lost and game time, the numbers in docs/autopilot.md.
/// `cargo test --release -- --ignored --nocapture benchmark_`
#[cfg(test)]
mod benchmark {
    use super::*;
    use crate::game::Features;

    #[test]
    #[ignore = "a benchmark: about thirty seconds in release"]
    fn benchmark_feature_sets() {
        let sets = [
            ("v1", Features::V1),
            (
                "glue",
                Features {
                    glue: true,
                    ..Features::V1
                },
            ),
            (
                "asteroids",
                Features {
                    asteroids: true,
                    ..Features::V1
                },
            ),
            (
                "lasers",
                Features {
                    lasers: true,
                    ..Features::V1
                },
            ),
            (
                "tiny",
                Features {
                    tiny: true,
                    ..Features::V1
                },
            ),
            (
                "player, break",
                Features {
                    player: true,
                    warp: true,
                    ..Features::V1
                },
            ),
            ("all", Features::ALL),
        ];
        for (name, features) in sets {
            let (mut wins, mut lives_lost, mut total, mut longest) = (0, 0, 0, 0);
            for seed in 1..=20 {
                let mut game = Game::with_seed(seed).with_features(features);
                let mut autopilot = Autopilot::new(None, 1);
                let mut ticks = 0u32;
                while ticks < 120 * 1800
                    && !matches!(game.state, GameState::Victory | GameState::GameOver)
                {
                    let input = autopilot.input(&game, 1.0 / 120.0);
                    game.tick(&input, 1.0 / 120.0);
                    autopilot.observe(&game);
                    // Counted from the events: Player can add lives, so
                    // the count left says nothing about the count lost.
                    lives_lost += game
                        .events
                        .iter()
                        .filter(|e| **e == GameEvent::LifeLost)
                        .count();
                    game.events.clear();
                    ticks += 1;
                }
                if game.state == GameState::Victory {
                    wins += 1;
                    total += ticks / 120;
                    longest = longest.max(ticks / 120);
                } else {
                    println!(
                        "  {name} seed {seed}: {:?} at level {}",
                        game.state, game.level
                    );
                }
            }
            let mean = total.checked_div(wins).unwrap_or(0);
            println!(
                "{name}: {wins}/20 won, {lives_lost} lives lost, won in {mean} s on average, {longest} s at most"
            );
        }
    }
}
