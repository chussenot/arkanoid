//! The sound director: what plays, where, at what pitch, from the
//! simulation's events and state. Pure, like `hud::layout`: game in,
//! mixer commands out, tested without a device. It reads the game the way
//! the renderers do and never writes to it.
//!
//! The design (docs/design/audio.md):
//! - **The field plays in key.** The music is in E minor; each brick broken
//!   in a rally is the next note up the E minor pentatonic scale, back to
//!   the root at each paddle hit. A good rally plays a phrase over the beat.
//! - **Sound sits where it happens.** Every effect is panned to its x on
//!   the field, in both renderers.
//! - **The paddle tightens with the ball.** Its knock rises with the ball's
//!   speed, so the rally's pace is heard as well as seen.
//! - **Music follows the campaign.** One loop per act, faster and fuller
//!   act by act; stingers mark the moments and duck the loop under them.

use crate::events::{GameEvent, PowerUpKind};
use crate::game::{Game, GameState, GunKind, LevelSetKind, PLAYFIELD_WIDTH};
use crate::levels::BrickKind;

use super::cues::{Cue, LEVEL_TRACKS};
use super::mixer::Command;

/// The player's sound setting, cycled with M.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundMode {
    All,
    EffectsOnly,
    Off,
}

impl SoundMode {
    pub fn next(self) -> Self {
        match self {
            SoundMode::All => SoundMode::EffectsOnly,
            SoundMode::EffectsOnly => SoundMode::Off,
            SoundMode::Off => SoundMode::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SoundMode::All => "Sound: on",
            SoundMode::EffectsOnly => "Sound: effects only",
            SoundMode::Off => "Sound: off",
        }
    }

    /// `BRYX_SOUND=effects|off` (anything else, or unset, is on).
    pub fn from_env() -> Self {
        match std::env::var("BRYX_SOUND").as_deref() {
            Ok("effects") => SoundMode::EffectsOnly,
            Ok("off") => SoundMode::Off,
            _ => SoundMode::All,
        }
    }

    pub fn command(self) -> Command {
        Command::Enable {
            effects: self != SoundMode::Off,
            music: self == SoundMode::All,
        }
    }
}

/// The E minor pentatonic scale, in semitones above the brick cue's E5.
const PENTATONIC: [f32; 10] = [0.0, 3.0, 5.0, 7.0, 10.0, 12.0, 15.0, 17.0, 19.0, 22.0];
/// Brick sounds per tick at most: an explosion breaking eight bricks at
/// once is one chord, not eight pops stacked into noise.
const MAX_BRICK_SOUNDS_PER_TICK: usize = 3;
/// Stereo width: how far to the sides the field's edges sit.
const PAN_WIDTH: f32 = 0.75;
/// The music's level while paused.
const PAUSED_MUSIC: f32 = 0.3;

fn semitones(n: f32) -> f32 {
    2f32.powf(n / 12.0)
}

/// Field x (0..800) to a pan position.
pub fn pan_for(x: f32) -> f32 {
    ((x / PLAYFIELD_WIDTH) * 2.0 - 1.0).clamp(-1.0, 1.0) * PAN_WIDTH
}

/// The music for a state and level: the menu theme, the level's own
/// track, or silence after a run ends (its stinger plays alone).
///
/// The campaign and `--classic` play each level's track (`audio/compose.py`
/// wrote one per campaign level). A loaded levelset's levels have no track
/// of their own, so they walk `playlist`, a shuffle of all 32 drawn once
/// per session: every track before any repeats, a different order each
/// time the game starts.
pub fn music_for(game: &Game, playlist: &[u8]) -> Option<Cue> {
    match game.state {
        GameState::Menu => Some(Cue::MusicMenu),
        GameState::Playing | GameState::Paused => {
            let track = match game.level_set() {
                LevelSetKind::Loaded => playlist[(game.level - 1) % playlist.len()],
                _ => (game.level.clamp(1, usize::from(LEVEL_TRACKS))) as u8,
            };
            Some(Cue::Level(track))
        }
        GameState::GameOver | GameState::Victory => None,
    }
}

/// The tracks 1..=32 in an order fixed by `seed` (Fisher-Yates over a
/// xorshift stream: deterministic, so tests can name the order).
pub fn shuffled_playlist(seed: u64) -> Vec<u8> {
    let mut order: Vec<u8> = (1..=LEVEL_TRACKS).collect();
    // Spread the seed over all 64 bits (and keep xorshift off zero).
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    for i in (1..order.len()).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        order.swap(i, (state % (i as u64 + 1)) as usize);
    }
    order
}

fn play(cue: Cue, gain: f32, x: f32) -> Command {
    pitched(cue, gain, x, 1.0)
}

fn pitched(cue: Cue, gain: f32, x: f32, pitch: f32) -> Command {
    Command::Play {
        cue,
        gain,
        pan: pan_for(x),
        pitch,
        duck: 0.0,
    }
}

fn stinger(cue: Cue, gain: f32, duck: f32) -> Command {
    Command::Play {
        cue,
        gain,
        pan: 0.0,
        pitch: 1.0,
        duck,
    }
}

#[derive(Debug)]
pub struct Director {
    /// The order loaded levelsets play the score in (`music_for`).
    playlist: Vec<u8>,
    last_state: Option<GameState>,
    last_music: Option<Option<Cue>>,
    /// Bricks broken since the ball last left the paddle: the rally's
    /// place on the scale.
    combo: usize,
    /// Armored bricks still on their first hit, by position, to hear the
    /// hit that cracks one (no event carries it).
    intact_armor: Vec<(f32, f32)>,
    level: usize,
    ball_attached: bool,
}

impl Director {
    /// A director whose levelset playlist is shuffled by `seed`.
    pub fn new(seed: u64) -> Self {
        Self {
            playlist: shuffled_playlist(seed),
            last_state: None,
            last_music: None,
            combo: 0,
            intact_armor: Vec::new(),
            level: 0,
            ball_attached: false,
        }
    }

    /// Commands for one simulation tick: its events, and any change of
    /// state or music since the last tick.
    pub fn observe(&mut self, game: &Game, events: &[GameEvent]) -> Vec<Command> {
        let mut out = Vec::new();
        self.transitions(game, &mut out);
        self.events(game, events, &mut out);
        self.armor(game, &mut out);
        self.launch(game, &mut out);
        let music = music_for(game, &self.playlist);
        if self.last_music != Some(music) {
            out.push(Command::Music(music));
            self.last_music = Some(music);
        }
        out
    }

    fn transitions(&mut self, game: &Game, out: &mut Vec<Command>) {
        let state = game.state;
        if let Some(last) = self.last_state.filter(|&s| s != state) {
            match (last, state) {
                (GameState::Playing, GameState::Paused) => {
                    out.push(play(Cue::Pause, 0.5, PLAYFIELD_WIDTH / 2.0));
                    out.push(Command::MusicLevel(PAUSED_MUSIC));
                }
                (GameState::Paused, GameState::Playing) => {
                    out.push(play(Cue::Select, 0.5, PLAYFIELD_WIDTH / 2.0));
                    out.push(Command::MusicLevel(1.0));
                }
                (
                    GameState::Menu | GameState::GameOver | GameState::Victory,
                    GameState::Playing,
                ) => {
                    out.push(play(Cue::Select, 0.55, PLAYFIELD_WIDTH / 2.0));
                    out.push(Command::MusicLevel(1.0));
                    self.combo = 0;
                }
                _ => {}
            }
        }
        self.last_state = Some(state);
    }

    fn events(&mut self, game: &Game, events: &[GameEvent], out: &mut Vec<Command>) {
        let paddle_x = game.paddle.x;
        let has = |want: &GameEvent| events.iter().any(|e| e == want);
        let run_over = has(&GameEvent::GameOver);
        let won = has(&GameEvent::Victory);
        let mut bricks = 0;
        for event in events {
            match *event {
                GameEvent::BrickDestroyedAt { x, .. } => {
                    let degree = PENTATONIC[self.combo.min(PENTATONIC.len() - 1)];
                    self.combo += 1;
                    if bricks < MAX_BRICK_SOUNDS_PER_TICK {
                        out.push(pitched(Cue::Brick, 0.6, x, semitones(degree)));
                    }
                    bricks += 1;
                }
                GameEvent::PaddleHit => {
                    // 300 px/s at launch to the 700 cap: up a minor third.
                    let ball = &game.ball;
                    let speed = ball.vx.hypot(ball.vy);
                    let t = ((speed - 300.0) / 400.0).clamp(0.0, 1.0);
                    out.push(pitched(Cue::Paddle, 0.65, paddle_x, semitones(3.0 * t)));
                    self.combo = 0;
                }
                GameEvent::GunFired(GunKind::Laser) => {
                    out.push(play(Cue::Laser, 0.28, paddle_x));
                }
                GameEvent::GunFired(GunKind::SuperLaser) => {
                    out.push(play(Cue::SuperLaser, 0.36, paddle_x));
                }
                GameEvent::AsteroidSpawned => {
                    let x = game.asteroids.last().map_or(paddle_x, |a| a.x);
                    out.push(play(Cue::Asteroid, 0.5, x));
                }
                GameEvent::AsteroidExploded { x, .. } => {
                    out.push(play(Cue::Explosion, 0.7, x));
                }
                GameEvent::PowerUpCaught(kind) => out.push(match kind {
                    PowerUpKind::Tiny => play(Cue::Tiny, 0.55, paddle_x),
                    PowerUpKind::Player => Command::Play {
                        cue: Cue::OneUp,
                        gain: 0.6,
                        pan: pan_for(paddle_x),
                        pitch: 1.0,
                        duck: 0.6,
                    },
                    _ => play(Cue::Capsule, 0.5, paddle_x),
                }),
                GameEvent::Warped => out.push(play(Cue::Warp, 0.6, PLAYFIELD_WIDTH)),
                GameEvent::BallLost if !has(&GameEvent::LifeLost) => {
                    out.push(play(Cue::BallLost, 0.45, paddle_x));
                }
                GameEvent::LifeLost if !run_over => {
                    out.push(stinger(Cue::LifeLost, 0.7, 1.6));
                    self.combo = 0;
                }
                GameEvent::LevelCleared if !won => {
                    out.push(stinger(Cue::LevelClear, 0.7, 2.7));
                    self.combo = 0;
                }
                GameEvent::GameOver => out.push(stinger(Cue::GameOver, 0.8, 0.0)),
                GameEvent::Victory => out.push(stinger(Cue::Victory, 0.8, 0.0)),
                _ => {}
            }
        }
    }

    /// The hit that cracks an armored brick: a brick that was intact last
    /// tick, still standing, now on its last hit. Re-read from scratch on a
    /// new level, so a fresh grid never sounds.
    fn armor(&mut self, game: &Game, out: &mut Vec<Command>) {
        let intact: Vec<(f32, f32)> = game
            .bricks
            .iter()
            .filter(|b| b.kind == BrickKind::Armored && b.hits_remaining >= 2)
            .map(|b| (b.x, b.y))
            .collect();
        if game.level == self.level {
            for b in &game.bricks {
                if b.kind == BrickKind::Armored
                    && b.hits_remaining == 1
                    && self.intact_armor.contains(&(b.x, b.y))
                {
                    out.push(play(Cue::Armor, 0.5, b.x));
                }
            }
        }
        self.intact_armor = intact;
        self.level = game.level;
    }

    /// The serve: the ball leaving the paddle at a launch, not a glue
    /// release mid-rally.
    fn launch(&mut self, game: &Game, out: &mut Vec<Command>) {
        let attached = game.ball.attached;
        if self.ball_attached && !attached && game.state == GameState::Playing {
            out.push(play(Cue::Launch, 0.4, game.paddle.x));
            self.combo = 0;
        }
        self.ball_attached = attached;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::GameEvent as E;

    fn cues(commands: &[Command]) -> Vec<Cue> {
        commands
            .iter()
            .filter_map(|c| match c {
                Command::Play { cue, .. } => Some(*cue),
                _ => None,
            })
            .collect()
    }

    fn pitches(commands: &[Command]) -> Vec<f32> {
        commands
            .iter()
            .filter_map(|c| match c {
                Command::Play {
                    cue: Cue::Brick,
                    pitch,
                    ..
                } => Some(*pitch),
                _ => None,
            })
            .collect()
    }

    fn brick_at(x: f32) -> E {
        E::BrickDestroyedAt {
            x,
            y: 100.0,
            width: 52.0,
            height: 22.0,
            kind: BrickKind::Normal,
        }
    }

    fn playing() -> (Director, Game) {
        let mut game = Game::new().with_campaign();
        game.state = GameState::Playing;
        let mut director = Director::new(1);
        director.observe(&game, &[]);
        (director, game)
    }

    #[test]
    fn a_rally_climbs_the_pentatonic_and_the_paddle_resets_it() {
        let (mut director, game) = playing();
        let mut heard = Vec::new();
        for _ in 0..4 {
            heard.extend(pitches(&director.observe(&game, &[brick_at(400.0)])));
        }
        let expected: Vec<f32> = [0.0, 3.0, 5.0, 7.0].iter().map(|&n| semitones(n)).collect();
        assert_eq!(heard, expected, "E, G, A, B");
        director.observe(&game, &[E::PaddleHit]);
        let after = pitches(&director.observe(&game, &[brick_at(400.0)]));
        assert_eq!(after, vec![1.0], "back to the root");
    }

    #[test]
    fn many_bricks_in_one_tick_are_capped_but_still_climb() {
        let (mut director, game) = playing();
        let burst: Vec<E> = (0..8).map(|i| brick_at(100.0 + 50.0 * i as f32)).collect();
        let out = director.observe(&game, &burst);
        assert_eq!(pitches(&out).len(), MAX_BRICK_SOUNDS_PER_TICK);
        let next = pitches(&director.observe(&game, &[brick_at(400.0)]));
        assert_eq!(next, vec![semitones(PENTATONIC[8])]);
    }

    #[test]
    fn effects_are_panned_to_where_they_happen() {
        let (mut director, game) = playing();
        let out = director.observe(&game, &[brick_at(0.0)]);
        let Some(Command::Play { pan, .. }) = out.first() else {
            panic!("{out:?}")
        };
        assert!((pan + PAN_WIDTH).abs() < 1e-5, "far left");
        assert_eq!(pan_for(PLAYFIELD_WIDTH / 2.0), 0.0);
    }

    #[test]
    fn the_paddle_knock_rises_with_ball_speed() {
        let (mut director, mut game) = playing();
        let knock = |director: &mut Director, game: &Game| match director
            .observe(game, &[E::PaddleHit])
            .into_iter()
            .find(|c| {
                matches!(
                    c,
                    Command::Play {
                        cue: Cue::Paddle,
                        ..
                    }
                )
            }) {
            Some(Command::Play { pitch, .. }) => pitch,
            other => panic!("{other:?}"),
        };
        game.ball.vx = 0.0;
        game.ball.vy = -300.0;
        let slow = knock(&mut director, &game);
        game.ball.vy = -700.0;
        let fast = knock(&mut director, &game);
        assert_eq!(slow, 1.0);
        assert!((fast - semitones(3.0)).abs() < 1e-5);
    }

    #[test]
    fn each_level_plays_its_own_track() {
        let playlist = shuffled_playlist(9);
        let mut game = Game::new().with_campaign();
        assert_eq!(music_for(&game, &playlist), Some(Cue::MusicMenu));
        game.state = GameState::Playing;
        for level in [1, 9, 17, 32] {
            game.level = level;
            assert_eq!(music_for(&game, &playlist), Some(Cue::Level(level as u8)));
        }
        game.state = GameState::GameOver;
        assert_eq!(music_for(&game, &playlist), None);
        let mut classic = Game::new();
        classic.state = GameState::Playing;
        classic.level = 3;
        assert_eq!(music_for(&classic, &playlist), Some(Cue::Level(3)));
    }

    #[test]
    fn a_levelset_walks_a_shuffle_of_the_whole_score() {
        let playlist = shuffled_playlist(42);
        let mut sorted = playlist.clone();
        sorted.sort_unstable();
        assert_eq!(
            sorted,
            (1..=LEVEL_TRACKS).collect::<Vec<_>>(),
            "each track once"
        );
        assert_ne!(
            playlist,
            shuffled_playlist(43),
            "the seed changes the order"
        );
        assert_ne!(playlist, sorted, "and it is shuffled");

        let mut game = Game::new();
        let level = crate::levelset::ExternalLevel {
            author: String::new(),
            name: String::new(),
            bricks: Vec::new(),
        };
        game.load_external_levels(vec![level; 40]);
        game.state = GameState::Playing;
        let heard: Vec<_> = (1..=33)
            .map(|level| {
                game.level = level;
                music_for(&game, &playlist)
            })
            .collect();
        assert_eq!(heard[0], Some(Cue::Level(playlist[0])));
        assert_eq!(heard[32], heard[0], "wraps after the whole score");
    }

    #[test]
    fn the_music_command_is_sent_once_per_change() {
        let mut game = Game::new().with_campaign();
        let mut director = Director::new(1);
        let music = |out: &[Command]| {
            out.iter()
                .filter(|c| matches!(c, Command::Music(_)))
                .count()
        };
        assert_eq!(music(&director.observe(&game, &[])), 1);
        assert_eq!(music(&director.observe(&game, &[])), 0);
        game.state = GameState::Playing;
        assert_eq!(music(&director.observe(&game, &[])), 1);
    }

    #[test]
    fn pause_dims_the_music_and_resume_restores_it() {
        let (mut director, mut game) = playing();
        game.state = GameState::Paused;
        let out = director.observe(&game, &[]);
        assert!(out.contains(&Command::MusicLevel(PAUSED_MUSIC)));
        assert_eq!(cues(&out), vec![Cue::Pause]);
        game.state = GameState::Playing;
        assert!(director
            .observe(&game, &[])
            .contains(&Command::MusicLevel(1.0)));
    }

    #[test]
    fn the_last_life_plays_game_over_not_life_lost() {
        let (mut director, game) = playing();
        let out = director.observe(&game, &[E::BallLost, E::LifeLost, E::GameOver]);
        assert_eq!(cues(&out), vec![Cue::GameOver]);
        let out = director.observe(&game, &[E::BallLost, E::LifeLost]);
        assert_eq!(cues(&out), vec![Cue::LifeLost]);
        let out = director.observe(&game, &[E::BallLost]);
        assert_eq!(cues(&out), vec![Cue::BallLost], "one ball of a multiball");
    }

    #[test]
    fn stingers_duck_the_music() {
        let (mut director, game) = playing();
        let out = director.observe(&game, &[E::LevelCleared]);
        assert!(matches!(
            out.first(),
            Some(Command::Play { cue: Cue::LevelClear, duck, .. }) if *duck > 0.0
        ));
        let out = director.observe(&game, &[E::LevelCleared, E::Victory]);
        assert_eq!(
            cues(&out),
            vec![Cue::Victory],
            "the last level: victory only"
        );
    }

    #[test]
    fn cracking_an_armored_brick_is_heard_and_a_new_level_is_not() {
        let (mut director, mut game) = playing();
        game.level = 3;
        game.bricks = vec![crate::game::Brick {
            x: 200.0,
            y: 80.0,
            width: 52.0,
            height: 22.0,
            kind: BrickKind::Armored,
            hits_remaining: 2,
            score: 10,
        }];
        director.observe(&game, &[]);
        game.bricks[0].hits_remaining = 1;
        assert_eq!(cues(&director.observe(&game, &[])), vec![Cue::Armor]);
        assert!(cues(&director.observe(&game, &[])).is_empty(), "once");
        game.level = 4;
        game.bricks[0].hits_remaining = 2;
        director.observe(&game, &[]);
        game.level = 5;
        game.bricks[0].hits_remaining = 1;
        assert!(
            cues(&director.observe(&game, &[])).is_empty(),
            "level change"
        );
    }

    #[test]
    fn sound_modes_cycle_and_map_to_the_mixer() {
        assert_eq!(SoundMode::All.next(), SoundMode::EffectsOnly);
        assert_eq!(SoundMode::Off.next(), SoundMode::All);
        assert_eq!(
            SoundMode::EffectsOnly.command(),
            Command::Enable {
                effects: true,
                music: false
            }
        );
    }
}
