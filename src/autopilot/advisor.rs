//! The autopilot's strategist backed by a TypeSafe System One model, through
//! the `judgment` crate (feature `typesafe`).
//!
//! One atomic question per decision, a Choice over the five lanes: "which
//! lane should the next shot attack". Everything the model would otherwise
//! have to count or compute (bricks per lane, which are exposed, where the
//! balls are) is computed by `board.rs` and sent as the state; the model
//! weighs it, and `mod.rs` decides whether its answer is used.
//!
//! `judgment` is async and the game loop is not, so the client runs on a
//! worker thread with its own single-threaded tokio runtime ([`Background`]):
//! the game sends a request and polls for the reply every tick, and never
//! waits. [`Blocking`] answers in place instead, for headless runs where
//! the simulation can wait for the model.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use judgment::{options, Choice, Client, Handle, Options, Questions, RetryPolicy, SystemOne};
use serde_json::{json, Value};
use tokio::runtime::Runtime;

use super::board::LANE_WIDTH;
use super::{Advice, Reply, Situation, Strategist, LANE_NAMES};

options! {
    /// The five lanes of `board.rs`, left to right.
    pub enum LaneKey {
        FarLeft = "far_left" => "Aim the next shot at the leftmost fifth of the field, x 0 to 160",
        Left = "left" => "Aim the next shot at x 160 to 320",
        Center = "center" => "Aim the next shot at the middle fifth, x 320 to 480",
        Right = "right" => "Aim the next shot at x 480 to 640",
        FarRight = "far_right" => "Aim the next shot at the rightmost fifth, x 640 to 800",
    }
}

const INSTRUCTIONS: &str = "You are choosing where the next shot goes in a game of \
Arkanoid, to clear every breakable brick in as few paddle hits as possible without \
losing the ball. `map` is the brick wall, rows top to bottom, one character per 50 px \
column: '.' empty, '1' a brick one hit from breaking, '2' an armored brick two hits from \
breaking, 'X' indestructible. `lanes` splits the 800 px field into five lanes and counts, \
for each, the breakable bricks left (`destructible`), those a straight shot from the \
paddle reaches first (`exposed`), and the indestructible ones that only deflect the \
ball. `balls` and `paddle` are positions in pixels, y pointing down. \
`rallies_without_progress` counts paddle hits since a brick last broke: a high value \
means the current aim is looping and a different lane is needed. A lane with no \
destructible bricks is never a good answer. Which lane should the next shot attack?";

/// A late answer is a useless one: the ball is back at the paddle in about
/// a second at top speed, so one quick retry at most, and a hard deadline.
const ATTEMPT_TIMEOUT: Duration = Duration::from_millis(1500);
const DEADLINE: Duration = Duration::from_millis(2500);

/// The question set and the handle that reads its answer.
pub struct Advisor {
    questions: Questions,
    lane: Handle<Choice<LaneKey>>,
}

impl Advisor {
    pub fn new() -> judgment::Result<Self> {
        let mut questions = Questions::new();
        let lane = questions.choice::<LaneKey>("next_lane", INSTRUCTIONS)?;
        Ok(Self { questions, lane })
    }

    /// The state sent with the question. Rounded to whole pixels: the model
    /// has no use for sub-pixel precision, and every digit is a token.
    pub fn state(situation: &Situation) -> Value {
        let lanes: Vec<Value> = situation
            .board
            .lanes
            .iter()
            .zip(LANE_NAMES)
            .enumerate()
            .map(|(index, (lane, name))| {
                json!({
                    "lane": name,
                    "x_from": (index as f32 * LANE_WIDTH).round(),
                    "x_to": ((index + 1) as f32 * LANE_WIDTH).round(),
                    "destructible": lane.destructible,
                    "exposed": lane.exposed,
                    "indestructible": lane.indestructible,
                })
            })
            .collect();
        let balls: Vec<Value> = situation
            .balls
            .iter()
            .map(|b| {
                json!({
                    "x": b.x.round(), "y": b.y.round(),
                    "vx": b.vx.round(), "vy": b.vy.round(),
                })
            })
            .collect();
        json!({
            "map": situation.board.map,
            "lanes": lanes,
            "balls": balls,
            "paddle": {
                "x": situation.paddle.x.round(),
                "width": situation.paddle.width.round(),
            },
            "lives": situation.lives,
            "level": situation.level,
            "rallies_without_progress": situation.rallies_without_progress,
        })
    }

    /// Ask `backend` for the next lane.
    pub async fn advise<B: SystemOne + ?Sized>(
        &self,
        backend: &B,
        model: &str,
        situation: &Situation,
    ) -> judgment::Result<Advice> {
        let response = backend
            .answer(&Self::state(situation), model, &self.questions)
            .await?;
        let choice = response.get(&self.lane)?;
        // `Options::ALL` is declaration order, which is lane order.
        let lane = LaneKey::ALL
            .iter()
            .position(|k| *k == choice.chosen)
            .unwrap_or(LANE_NAMES.len() / 2);
        Ok(Advice {
            lane,
            confidence: choice.confidence.value(),
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
        })
    }
}

/// The hosted client, configured for a game: `TYPESAFE_API_KEY` (required),
/// `TYPESAFE_BASE_URL` for any server that speaks the same wire, short
/// timeouts and at most one retry.
pub fn client_from_env() -> judgment::Result<Client> {
    let mut builder = Client::builder()
        .timeout(ATTEMPT_TIMEOUT)
        .retry(RetryPolicy {
            max_retries: 1,
            backoff_initial: Duration::from_millis(100),
            budget: Some(DEADLINE),
            ..RetryPolicy::default()
        });
    if let Some(url) = env_nonblank("TYPESAFE_BASE_URL") {
        builder = builder.base_url(url);
    }
    builder.build()
}

/// `TYPESAFE_MODEL`, as `judgment`'s own examples read it, else `jev-latest`.
pub fn model_from_env() -> String {
    env_nonblank("TYPESAFE_MODEL").unwrap_or_else(|| "jev-latest".to_owned())
}

fn env_nonblank(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn runtime() -> std::io::Result<Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
}

/// Asks the model on a worker thread; the game polls for replies.
pub struct Background {
    jobs: Sender<(u64, Situation)>,
    replies: Receiver<Reply>,
    describe: String,
}

impl Background {
    pub fn spawn<B: SystemOne + 'static>(backend: B, model: String) -> std::io::Result<Self> {
        let advisor = Advisor::new().map_err(std::io::Error::other)?;
        let runtime = runtime()?;
        let (jobs, job_rx) = mpsc::channel::<(u64, Situation)>();
        let (reply_tx, replies) = mpsc::channel();
        let describe = format!("TypeSafe System One ({model})");
        thread::Builder::new()
            .name("autopilot-advisor".to_owned())
            .spawn(move || {
                while let Ok(mut job) = job_rx.recv() {
                    // Only the newest situation is worth an answer.
                    while let Ok(newer) = job_rx.try_recv() {
                        job = newer;
                    }
                    let (id, situation) = job;
                    let started = Instant::now();
                    let outcome = runtime.block_on(async {
                        match tokio::time::timeout(
                            DEADLINE,
                            advisor.advise(&backend, &model, &situation),
                        )
                        .await
                        {
                            Ok(Ok(advice)) => Ok(advice),
                            Ok(Err(error)) => Err(error.to_string()),
                            Err(_) => Err(format!("no answer within {DEADLINE:?}")),
                        }
                    });
                    let reply = Reply {
                        id,
                        outcome,
                        latency: started.elapsed(),
                    };
                    if reply_tx.send(reply).is_err() {
                        break; // The game has gone.
                    }
                }
            })?;
        Ok(Self {
            jobs,
            replies,
            describe,
        })
    }
}

impl Strategist for Background {
    fn describe(&self) -> String {
        self.describe.clone()
    }

    fn request(&mut self, id: u64, situation: Situation) {
        // A send only fails once the worker has exited, which it does only
        // when the game drops this; nothing to report then.
        let _ = self.jobs.send((id, situation));
    }

    fn poll(&mut self) -> Option<Reply> {
        self.replies.try_recv().ok()
    }
}

/// Asks the model in place, blocking the caller: game time stands still
/// while the model thinks. For headless runs that measure the strategy
/// rather than the latency.
#[cfg_attr(not(test), allow(dead_code))]
pub struct Blocking<B> {
    backend: B,
    model: String,
    advisor: Advisor,
    runtime: Runtime,
    reply: Option<Reply>,
}

#[cfg_attr(not(test), allow(dead_code))]
impl<B: SystemOne> Blocking<B> {
    pub fn new(backend: B, model: impl Into<String>) -> std::io::Result<Self> {
        Ok(Self {
            backend,
            model: model.into(),
            advisor: Advisor::new().map_err(std::io::Error::other)?,
            runtime: runtime()?,
            reply: None,
        })
    }
}

impl<B: SystemOne> Strategist for Blocking<B> {
    fn describe(&self) -> String {
        format!("System One ({}), blocking", self.model)
    }

    fn request(&mut self, id: u64, situation: Situation) {
        let started = Instant::now();
        let outcome = self
            .runtime
            .block_on(self.advisor.advise(&self.backend, &self.model, &situation))
            .map_err(|e| e.to_string());
        self.reply = Some(Reply {
            id,
            outcome,
            latency: started.elapsed(),
        });
    }

    fn poll(&mut self) -> Option<Reply> {
        self.reply.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autopilot::{Autopilot, MIN_CONFIDENCE};
    use crate::game::{Game, GameState};
    use judgment::Fake;

    const DT: f32 = 1.0 / 120.0;

    fn situation() -> Situation {
        let game = Game::new();
        Situation {
            board: crate::autopilot::board::Board::from_game(&game),
            balls: Vec::new(),
            paddle: game.paddle,
            lives: game.lives,
            level: game.level,
            rallies_without_progress: 0,
        }
    }

    #[test]
    fn the_question_is_one_choice_over_the_five_lanes_in_order() {
        let advisor = Advisor::new().unwrap();
        let questions: Vec<_> = advisor.questions.iter().collect();
        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].0, "next_lane");
        let wire: Vec<_> = LaneKey::ALL.iter().map(|k| k.key()).collect();
        assert_eq!(wire, LANE_NAMES, "the model sees the lanes left to right");
    }

    #[test]
    fn the_state_carries_the_map_and_the_counts_the_model_is_not_asked_for() {
        let state = Advisor::state(&situation());
        assert!(!state["map"].as_array().unwrap().is_empty());
        assert_eq!(state["lanes"].as_array().unwrap().len(), 5);
        assert_eq!(state["lanes"][0]["lane"], "far_left");
        assert_eq!(state["lanes"][4]["x_to"], 800.0);
        assert_eq!(state["lives"], 3);
    }

    #[test]
    fn a_fake_backend_answer_is_read_back_as_lane_and_confidence() {
        let backend = Fake::new()
            .choice(
                "next_lane",
                [
                    ("far_left", 0.05),
                    ("left", 0.05),
                    ("center", 0.1),
                    ("right", 0.7),
                    ("far_right", 0.1),
                ],
                0.62,
            )
            .unwrap();
        let mut strategist = Blocking::new(backend, "fake").unwrap();
        strategist.request(7, situation());
        let reply = strategist.poll().unwrap();
        assert_eq!(reply.id, 7);
        let advice = reply.outcome.unwrap();
        assert_eq!(advice.lane, 3);
        assert!((advice.confidence - 0.62).abs() < 1e-9);
    }

    /// Waits for the worker thread's reply in real time.
    fn wait_for(strategist: &mut Background, seconds: u64) -> Option<Reply> {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        while Instant::now() < deadline {
            if let Some(reply) = strategist.poll() {
                return Some(reply);
            }
            thread::sleep(Duration::from_millis(5));
        }
        None
    }

    #[test]
    fn the_background_worker_answers_without_blocking_the_caller() {
        let backend = Fake::new()
            .choice("next_lane", [("far_left", 0.9), ("left", 0.1)], 0.8)
            .unwrap();
        let mut strategist = Background::spawn(backend, "fake".to_owned()).unwrap();
        assert!(
            strategist.poll().is_none(),
            "nothing asked, nothing answered"
        );
        strategist.request(3, situation());
        let reply = wait_for(&mut strategist, 5).expect("a reply from the worker");
        assert_eq!(reply.id, 3);
        assert_eq!(reply.outcome.unwrap().lane, 0);
    }

    /// One request through the worker thread against the real API: the
    /// path the windowed game uses. Run by hand like the test below.
    #[test]
    #[ignore = "calls the TypeSafe API; needs TYPESAFE_API_KEY"]
    fn live_background_worker_answers_within_a_rally() {
        let client = client_from_env().expect("TYPESAFE_API_KEY");
        let mut strategist = Background::spawn(client, model_from_env()).unwrap();
        strategist.request(1, situation());
        let reply = wait_for(&mut strategist, 5).expect("a reply within 5 s");
        let advice = reply.outcome.expect("an answer");
        println!(
            "live worker: lane {} at confidence {:.2} in {} ms",
            LANE_NAMES[advice.lane],
            advice.confidence,
            reply.latency.as_millis()
        );
        assert!(reply.latency < DEADLINE);
    }

    #[test]
    fn an_answer_off_the_offered_lanes_is_an_error_not_a_default() {
        // `Response::verify` rejects an option the question did not offer;
        // the autopilot then plays on the heuristic.
        let backend = Fake::new().choice("next_lane", [("up", 1.0)], 1.0).unwrap();
        let mut strategist = Blocking::new(backend, "fake").unwrap();
        strategist.request(1, situation());
        assert!(strategist.poll().unwrap().outcome.is_err());
    }

    #[test]
    fn the_autopilot_wins_with_a_fake_model_through_the_real_advisor() {
        let backend = Fake::new()
            .choice(
                "next_lane",
                [
                    ("far_left", 0.1),
                    ("left", 0.1),
                    ("center", 0.6),
                    ("right", 0.1),
                    ("far_right", 0.1),
                ],
                0.5,
            )
            .unwrap();
        const { assert!(0.5 >= MIN_CONFIDENCE) };
        let strategist = Blocking::new(backend, "fake").unwrap();
        let mut autopilot = Autopilot::new(Some(Box::new(strategist)), 1);
        let mut game = Game::with_seed(11);
        // Built-in levels 1 and 2 (level 3 cannot be cleared, see
        // `autopilot::tests::reached_level_3`).
        for _ in 0..120 * 360 {
            let input = autopilot.input(&game, DT);
            game.tick(&input, DT);
            autopilot.observe(&game);
            game.events.clear();
            if game.level >= 3 || game.state != GameState::Playing {
                break;
            }
        }
        assert_eq!(game.level, 3, "{:?}", autopilot.stats());
        assert_eq!(game.state, GameState::Playing);
        assert!(autopilot.stats().accepted > 0);
    }

    /// Plays built-in levels 1 and 2 against the real model, game time
    /// frozen while it answers. Needs `TYPESAFE_API_KEY`; run by hand:
    /// `cargo test --features typesafe -- --ignored --nocapture live_`.
    #[test]
    #[ignore = "calls the TypeSafe API; needs TYPESAFE_API_KEY"]
    fn live_model_plays_the_built_in_levels() {
        let client = client_from_env().expect("TYPESAFE_API_KEY");
        let strategist = Blocking::new(client, model_from_env()).unwrap();
        let mut autopilot = Autopilot::new(Some(Box::new(strategist)), 1).with_log(true);
        let mut game = Game::with_seed(42);
        let mut ticks = 0;
        while ticks < 120 * 360 && game.level < 3 && game.state != GameState::GameOver {
            let input = autopilot.input(&game, DT);
            game.tick(&input, DT);
            autopilot.observe(&game);
            game.events.clear();
            ticks += 1;
        }
        println!(
            "live: {:?} at level {} after {:.1} s of game time, {} lives left, score {}; {}",
            game.state,
            game.level,
            ticks as f32 * DT,
            game.lives,
            game.score,
            autopilot.stats()
        );
        assert_eq!(game.level, 3);
    }
}
