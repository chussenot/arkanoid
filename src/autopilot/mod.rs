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

use crate::events::GameEvent;
use crate::game::{Ball, Game, GameState, Input, Paddle, POWERUP_FALL_SPEED};
use board::{Board, LANES};

/// Lowest confidence at which a model's lane is used over the heuristic.
/// TypeSafe's Choice confidence is `(p_max − 1/n) / (1 − 1/n)`: over five
/// lanes 0.3 means the top lane holds at least 44% of the probability,
/// more than twice an even split.
pub const MIN_CONFIDENCE: f64 = 0.3;

/// How long a serve waits for the model's first answer before launching on
/// the heuristic lane anyway. Hosted p50 is about 250 ms.
const SERVE_WAIT_SECS: f32 = 1.5;

/// Paddle hits in a row without a brick broken after which a model's lane
/// is overridden: the shot is looping (typically between indestructible
/// bricks and the paddle) and the heuristic rotation breaks the loop.
const LOOP_OVERRIDE_RALLIES: u32 = 4;

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

        let balls = std::iter::once(&game.ball).chain(&game.extra_balls);
        let Some(hit) = pilot::next_intercept(balls, paddle) else {
            return;
        };
        let line_y = pilot::contact_line(paddle, &game.ball);
        let safe_x = pilot::paddle_x_for(hit.x, 0.0, paddle);
        let offset = pilot::aim_offset(hit.x, line_y, self.aim_point(&board));
        let aimed_x = pilot::paddle_x_for(hit.x, offset, paddle);
        let mut target_x = if pilot::reachable(paddle.x, aimed_x, hit.time) {
            aimed_x
        } else {
            safe_x
        };

        // A power-up is worth a detour only if the paddle can make it back
        // in time for the ball; which power-up is worth more is moot, all
        // three only help.
        let catch = game
            .powerups
            .iter()
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

        (input.left, input.right) = pilot::steer(paddle.x, target_x, dt);
    }

    /// The point a shot is aimed at: the plan's lane target, nudged
    /// sideways while shots are not breaking anything so a looping
    /// trajectory does not repeat exactly.
    fn aim_point(&self, board: &Board) -> (f32, f32) {
        let (x, y) = board.lanes[self.plan.lane].target;
        if self.fruitless < 2 {
            return (x, y);
        }
        let nudge = ((self.fruitless * 23) % 40) as f32 - 20.0;
        (x + nudge, y)
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

    /// The built-in levels the autopilot is held to: 1 and 2. Level 3's
    /// breakable bricks sit inside a closed box of indestructible ones (top
    /// and bottom rows all `X`, `X` at both ends of every row between), so
    /// no shot can reach them, piloted or not.
    fn reached_level_3(game: &Game) -> bool {
        game.level >= 3
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

    /// Game time allowed for built-in levels 1 and 2 together: six minutes,
    /// against the 200 to 250 s the heuristic takes on the seeds below.
    const TWO_LEVELS: u32 = 120 * 360;

    #[test]
    fn the_heuristic_alone_clears_the_first_two_built_in_levels() {
        for seed in [1, 7, 42] {
            let mut game = Game::with_seed(seed);
            let mut autopilot = Autopilot::new(None, 1);
            play(&mut game, &mut autopilot, TWO_LEVELS, reached_level_3);
            assert!(
                reached_level_3(&game) && game.state == GameState::Playing,
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
        play(&mut game, &mut autopilot, TWO_LEVELS, |_| false);
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
        play(&mut game, &mut autopilot, TWO_LEVELS, reached_level_3);
        assert!(reached_level_3(&game), "{:?}", autopilot.stats());
        let stats = autopilot.stats();
        assert!(stats.accepted > 0, "{stats:?}");
        assert!(stats.empty_lane > 0, "the far-left lane empties: {stats:?}");
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
        play(&mut game, &mut autopilot, TWO_LEVELS / 2, |_| false);
        let stats = autopilot.stats();
        assert_eq!(stats.accepted, 0);
        assert!(stats.low_confidence > 0);
        assert_eq!(autopilot.plan.source, Source::Heuristic);
    }

    #[test]
    fn a_silent_model_delays_each_serve_but_never_stops_the_game() {
        let mut game = Game::with_seed(5);
        let mut autopilot = Autopilot::new(Some(Box::new(Silent)), 1);
        play(&mut game, &mut autopilot, TWO_LEVELS, reached_level_3);
        assert!(reached_level_3(&game), "{:?}", autopilot.stats());
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
            play(&mut game, &mut autopilot, TWO_LEVELS / 2, |_| false);
            autopilot.stats().clone()
        };
        let every_rally = run(1);
        let every_fourth = run(4);
        assert!(every_rally.requests > every_fourth.requests * 2);
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
