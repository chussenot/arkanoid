//! The playtest log (`--playtest-log <file>`): one CSV row per level
//! played, for comparing human sessions (`docs/playtest.md`). It records
//! what the review's hypotheses need: how long a level took, how long its
//! last bricks were hunted, lives lost, the longest rally, bricks broken,
//! capsules caught and the points scored.
//!
//! Pure, like the renderers' view of the game: [`PlaytestLog::observe`]
//! reads each tick's state and events and returns the rows of the levels
//! that ended. `main.rs` appends them to the file. Nothing is ever sent
//! anywhere.

use crate::events::{GameEvent, PowerUpKind};
use crate::game::{Game, GameState, TICK_RATE};
use crate::levels::BrickKind;

/// The share of a level's breakable bricks left when its "hunt" starts:
/// the last tenth, as the director reviews and the pacing probe measure it.
const HUNT_SHARE: usize = 10;

/// How a level ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Its last brick broke.
    Cleared,
    /// Left through the Break gate.
    Warped,
    /// The set's last level cleared.
    Victory,
    /// Its last life lost.
    GameOver,
    /// Left for the menu, or the window closed, mid-level.
    Abandoned,
}

impl Outcome {
    fn label(self) -> &'static str {
        match self {
            Outcome::Cleared => "cleared",
            Outcome::Warped => "warped",
            Outcome::Victory => "victory",
            Outcome::GameOver => "game_over",
            Outcome::Abandoned => "abandoned",
        }
    }
}

/// One level played.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub session: u64,
    pub run: u32,
    pub player: &'static str,
    pub mode: String,
    pub rally_score: bool,
    pub level: usize,
    pub name: String,
    pub outcome: Outcome,
    /// Seconds in play: not paused.
    pub secs: f32,
    /// Seconds from the last tenth of its breakable bricks to its end.
    pub hunt_secs: f32,
    pub lives_lost: u32,
    pub longest_rally: u32,
    pub bricks: u32,
    pub capsules: u32,
    /// The capsules caught, by letter (`palette::capsule_letter`), in
    /// order: "WMS".
    pub capsule_letters: String,
    pub score: u32,
}

impl Row {
    pub const HEADER: &'static str = "session,run,player,mode,rally_score,level,name,outcome,\
        secs,hunt_secs,lives_lost,longest_rally,bricks,capsules,capsule_letters,score";

    /// The row as one CSV line (RFC 4180 quoting for the free-text fields).
    pub fn csv(&self) -> String {
        format!(
            "{},{},{},{},{},{},{},{},{:.1},{:.1},{},{},{},{},{},{}",
            self.session,
            self.run,
            self.player,
            quoted(&self.mode),
            self.rally_score,
            self.level,
            quoted(&self.name),
            self.outcome.label(),
            self.secs,
            self.hunt_secs,
            self.lives_lost,
            self.longest_rally,
            self.bricks,
            self.capsules,
            self.capsule_letters,
            self.score,
        )
    }
}

fn quoted(field: &str) -> String {
    if field.contains([',', '"', '\n']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_owned()
    }
}

/// The level being played, as far as it has gone.
#[derive(Debug, Clone)]
struct Level {
    level: usize,
    name: String,
    start_ticks: u64,
    start_score: u32,
    breakable: usize,
    hunt_from: Option<u64>,
    lives_lost: u32,
    rally: u32,
    longest_rally: u32,
    bricks: u32,
    caught: Vec<PowerUpKind>,
}

/// Follows a session's runs level by level.
#[derive(Debug)]
pub struct PlaytestLog {
    session: u64,
    player: &'static str,
    mode: String,
    rally_score: bool,
    run: u32,
    level: Option<Level>,
    last_state: GameState,
}

fn breakable(game: &Game) -> usize {
    game.bricks
        .iter()
        .filter(|b| b.kind != BrickKind::Indestructible)
        .count()
}

impl PlaytestLog {
    /// A log for one session (`session` tells sessions apart in a shared
    /// file); `autopilot` marks the machine's runs as such.
    pub fn new(session: u64, autopilot: bool) -> Self {
        Self {
            session,
            player: if autopilot { "autopilot" } else { "human" },
            mode: String::new(),
            rally_score: false,
            run: 0,
            level: None,
            last_state: GameState::Menu,
        }
    }

    /// What the next runs play (the picker's mode, `menu::Menu::label`)
    /// and whether they are rally-scored.
    pub fn set_mode(&mut self, mode: String, rally_score: bool) {
        self.mode = mode;
        self.rally_score = rally_score;
    }

    fn start(&mut self, game: &Game) {
        self.level = Some(Level {
            level: game.level,
            name: game.level_name().unwrap_or_default().to_owned(),
            start_ticks: game.run_ticks,
            start_score: game.score,
            breakable: breakable(game),
            hunt_from: None,
            lives_lost: 0,
            rally: 0,
            longest_rally: 0,
            bricks: 0,
            caught: Vec::new(),
        });
    }

    fn finish(&mut self, game: &Game, outcome: Outcome) -> Option<Row> {
        let level = self.level.take()?;
        let secs = |ticks: u64| ticks as f32 / TICK_RATE;
        Some(Row {
            session: self.session,
            run: self.run,
            player: self.player,
            mode: self.mode.clone(),
            rally_score: self.rally_score,
            level: level.level,
            name: level.name,
            outcome,
            secs: secs(game.run_ticks.saturating_sub(level.start_ticks)),
            hunt_secs: level
                .hunt_from
                .map_or(0.0, |from| secs(game.run_ticks.saturating_sub(from))),
            lives_lost: level.lives_lost,
            longest_rally: level.longest_rally,
            bricks: level.bricks,
            capsules: level.caught.len() as u32,
            capsule_letters: level
                .caught
                .iter()
                .map(|&k| crate::palette::capsule_letter(k))
                .collect(),
            score: game.score.saturating_sub(level.start_score),
        })
    }

    /// One tick, after `Game::tick` and before its events are drained:
    /// the rows of any level that ended in it.
    pub fn observe(&mut self, game: &Game, events: &[GameEvent]) -> Vec<Row> {
        let mut rows = Vec::new();
        let state = game.state;
        // A run starts (from the menu, or a Continue or Play again).
        if state == GameState::Playing
            && matches!(
                self.last_state,
                GameState::Menu | GameState::GameOver | GameState::Victory
            )
        {
            self.run += 1;
            self.start(game);
        }
        self.last_state = state;

        let has = |want: &GameEvent| events.contains(want);
        if let Some(level) = &mut self.level {
            // The rally from the events, in order: the game resets its own
            // count on the tick a level clears, before this sees it.
            for event in events {
                match event {
                    GameEvent::BrickDestroyed => {
                        level.bricks += 1;
                        level.rally += 1;
                        level.longest_rally = level.longest_rally.max(level.rally);
                    }
                    GameEvent::PaddleHit => level.rally = 0,
                    GameEvent::LifeLost => {
                        level.lives_lost += 1;
                        level.rally = 0;
                    }
                    GameEvent::PowerUpCaught(kind) => level.caught.push(*kind),
                    _ => {}
                }
            }
            // The hunt starts when the last tenth is left. On the tick a
            // level clears the field already holds the next one's bricks.
            if level.hunt_from.is_none()
                && !has(&GameEvent::LevelCleared)
                && breakable(game) * HUNT_SHARE <= level.breakable
            {
                level.hunt_from = Some(game.run_ticks);
            }
        }

        if has(&GameEvent::Victory) {
            rows.extend(self.finish(game, Outcome::Victory));
        } else if has(&GameEvent::GameOver) {
            rows.extend(self.finish(game, Outcome::GameOver));
        } else if has(&GameEvent::LevelCleared) {
            let outcome = if has(&GameEvent::Warped) {
                Outcome::Warped
            } else {
                Outcome::Cleared
            };
            rows.extend(self.finish(game, outcome));
            self.start(game);
        }
        rows
    }

    /// The level in play, if any, ended without finishing: the player left
    /// for the menu or closed the window.
    pub fn abandon(&mut self, game: &Game) -> Option<Row> {
        self.last_state = GameState::Menu;
        self.finish(game, Outcome::Abandoned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{Features, Input};

    const DT: f32 = 1.0 / TICK_RATE;

    /// Ticks `game` with the autopilot until `done`, feeding `log`, and
    /// returns its rows.
    fn play(
        game: &mut Game,
        log: &mut PlaytestLog,
        max_secs: u32,
        done: impl Fn(&[Row]) -> bool,
    ) -> Vec<Row> {
        let mut pilot = crate::autopilot::Autopilot::new(None, 1);
        let mut rows = Vec::new();
        for _ in 0..max_secs * TICK_RATE as u32 {
            let input = pilot.input(game, DT);
            game.tick(&input, DT);
            pilot.observe(game);
            rows.extend(log.observe(game, &game.events));
            game.events.clear();
            if done(&rows) {
                break;
            }
        }
        rows
    }

    #[test]
    fn a_cleared_level_is_one_row_with_its_story() {
        // No Break capsule, so the level ends on its last brick.
        let mut game = Game::with_seed(5).with_campaign().with_features(Features {
            warp: false,
            ..Features::ALL
        });
        let mut log = PlaytestLog::new(42, true);
        log.set_mode("campaign".into(), false);
        let rows = play(&mut game, &mut log, 300, |rows| !rows.is_empty());
        let row = &rows[0];
        assert_eq!((row.run, row.level, row.outcome), (1, 1, Outcome::Cleared));
        assert_eq!(row.name, "Warm-up");
        assert_eq!(row.bricks, 36, "every brick of Warm-up");
        assert!(row.secs > 1.0 && row.hunt_secs <= row.secs, "{row:?}");
        assert!(row.longest_rally >= 1 && row.score > 0, "{row:?}");
        assert_eq!(row.capsules as usize, row.capsule_letters.len());
        assert_eq!(game.level, 2, "the next level is being logged");
    }

    #[test]
    fn a_game_over_and_a_quit_end_their_level_and_a_continue_is_a_new_run() {
        let mut game = Game::with_seed(1).with_campaign();
        let mut log = PlaytestLog::new(1, false);
        game.state = GameState::Playing;
        log.observe(&game, &[]);
        game.lives = 1;
        game.ball.attached = false;
        game.ball.y = 700.0;
        game.tick(&Input::default(), DT);
        let rows = log.observe(&game, &game.events);
        game.events.clear();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].outcome, rows[0].lives_lost),
            (Outcome::GameOver, 1)
        );

        // Space continues: a second run, then Q mid-level.
        game.tick(
            &Input {
                space: true,
                ..Input::default()
            },
            DT,
        );
        assert!(log.observe(&game, &game.events).is_empty());
        game.events.clear();
        let quit = log.abandon(&game).expect("a level was in play");
        assert_eq!((quit.run, quit.outcome), (2, Outcome::Abandoned));
        assert!(log.abandon(&game).is_none(), "nothing left in play");
    }

    #[test]
    fn a_row_is_one_csv_line_with_the_headers_fields() {
        let row = Row {
            session: 7,
            run: 2,
            player: "human",
            mode: "levelset:Bricks, \"Vol 2\"".into(),
            rally_score: true,
            level: 3,
            name: "Twin towers".into(),
            outcome: Outcome::Warped,
            secs: 61.25,
            hunt_secs: 9.0,
            lives_lost: 1,
            longest_rally: 12,
            bricks: 40,
            capsules: 3,
            capsule_letters: "WBM".into(),
            score: 2160,
        };
        let line = row.csv();
        assert_eq!(
            line,
            "7,2,human,\"levelset:Bricks, \"\"Vol 2\"\"\",true,3,Twin towers,warped,\
             61.2,9.0,1,12,40,3,WBM,2160"
        );
        assert_eq!(Row::HEADER.split(',').count(), 16);
    }
}
