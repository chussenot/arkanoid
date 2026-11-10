//! The sound director: what plays, where, at what pitch, from the
//! simulation's events and state. Pure, like `hud::layout`: game in,
//! mixer commands out, tested without a device. It reads the game the way
//! the renderers do and never writes to it.
//!
//! The design (docs/design/audio.md):
//! - **The field plays in key.** The music is in E minor; each brick broken
//!   in a rally is the next note up the E minor pentatonic scale, with the
//!   playing track's own seventh (D sharp in harmonic minor), back to the
//!   root at each paddle hit. A good rally plays a phrase over the beat and
//!   tops out on the tonic.
//! - **Sound sits where it happens.** Every effect is panned to its x on
//!   the field, in both renderers.
//! - **The paddle tightens with the ball.** Its knock rises with the ball's
//!   speed, so the rally's pace is heard as well as seen.
//! - **Music follows the campaign.** Every level has its own track;
//!   stingers mark the moments and duck the loop under them.
//! - **One thing at a time.** A level's track starts at its first serve,
//!   after the serve's whip; a new track never starts under a fanfare, a
//!   warp or the serve (the mixer holds it, `Cue::holds_music`). A warp is
//!   the transition, so it plays without the level-clear fanfare, and
//!   leaving an end screen cuts its stinger.

use crate::cadence;
use crate::events::{GameEvent, PowerUpKind, Striker};
use crate::game::TICK_RATE;
use crate::game::{Game, GameState, GunKind, LevelSetKind, PLAYFIELD_WIDTH};
use crate::levels::BrickKind;
use crate::rally::Rally;

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

/// The mode a level track is written in (`audio/compose.py` names it in
/// each track; `build.rs` reads it into [`LEVEL_MODES`]). Every one is
/// over E.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Dorian,
    Aeolian,
    Phrygian,
    HarmonicMinor,
}

include!(concat!(env!("OUT_DIR"), "/track_modes.rs"));

impl Mode {
    /// The mode's seven degrees, in semitones above E.
    const fn scale(self) -> [u8; 7] {
        match self {
            Mode::Dorian => [0, 2, 3, 5, 7, 9, 10],
            Mode::Aeolian => [0, 2, 3, 5, 7, 8, 10],
            Mode::Phrygian => [0, 1, 3, 5, 7, 8, 10],
            Mode::HarmonicMinor => [0, 2, 3, 5, 7, 8, 11],
        }
    }

    /// The rally's notes over this mode, in semitones above the brick
    /// cue's E5: the E minor pentatonic with the mode's own seventh (D in
    /// three modes, D sharp in harmonic minor, which never plays a D), up
    /// to the tonic two octaves up, so the step a long rally holds is the
    /// home note, never a passing one.
    const fn rally(self) -> [f32; crate::rally::STEPS] {
        let seventh = self.scale()[6] as f32;
        [0.0, 3.0, 5.0, 7.0, seventh, 12.0, 15.0, 17.0, 19.0, 24.0]
    }
}
/// Brick sounds per tick at most: an explosion breaking eight bricks at
/// once is one chord, not eight pops stacked into noise.
const MAX_BRICK_SOUNDS_PER_TICK: usize = 3;
/// Notes waiting on a blast's front, at most: two blasts' worth of
/// neighbors and cracks.
const MAX_PENDING: usize = 24;
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
    /// place on the scale, counted as the effects layer counts it.
    rally: Rally,
    /// This tick's breaks as (event index, step), reused tick to tick.
    steps: Vec<(usize, usize)>,
    /// This tick's landing delays by event index (`cadence.rs`), reused.
    delays: Vec<Option<f32>>,
    /// Notes waiting for a blast's front to reach their brick, with the
    /// ticks left until they sound. At most `MAX_PENDING`.
    pending: Vec<(u32, Command)>,
    ball_attached: bool,
    /// The level whose first serve has been heard: its music plays from
    /// there. Cleared when a run ends, so a Continue waits for its serve.
    served_level: Option<usize>,
}

impl Director {
    /// A director whose levelset playlist is shuffled by `seed`.
    pub fn new(seed: u64) -> Self {
        Self {
            playlist: shuffled_playlist(seed),
            last_state: None,
            last_music: None,
            rally: Rally::default(),
            steps: Vec::new(),
            delays: Vec::new(),
            pending: Vec::with_capacity(MAX_PENDING),
            ball_attached: false,
            served_level: None,
        }
    }

    /// Commands for one simulation tick: its events, and any change of
    /// state or music since the last tick.
    pub fn observe(&mut self, game: &Game, events: &[GameEvent]) -> Vec<Command> {
        let mut out = Vec::new();
        let mut steps = std::mem::take(&mut self.steps);
        steps.clear();
        self.rally
            .observe(game, events, |i, step| steps.push((i, step)));
        let mut delays = std::mem::take(&mut self.delays);
        delays.clear();
        delays.resize(events.len(), None);
        cadence::landing_delays(events, |i, d| delays[i] = Some(d));
        self.transitions(game, &mut out);
        let released = self.release(game, &mut out);
        self.events(game, events, &steps, &delays, released, &mut out);
        self.steps = steps;
        self.delays = delays;
        self.launch(game, &mut out);
        let mut music = music_for(game, &self.playlist);
        let in_play = matches!(game.state, GameState::Playing | GameState::Paused);
        if in_play && self.served_level != Some(game.level) {
            // Before the level's first serve, the field waits in silence.
            music = None;
        }
        if self.last_music != Some(music) {
            out.push(Command::Music(music));
            self.last_music = Some(music);
        }
        out
    }

    /// The mode of the track the player is hearing, which the rally plays
    /// in. The music this tick has not changed yet: the last brick of a
    /// level is heard over that level's track, though `game.level` has
    /// already moved on. Aeolian, the score's home, when no track plays.
    fn mode_playing(&self) -> Mode {
        match self.last_music {
            Some(Some(Cue::Level(track))) => LEVEL_MODES[usize::from(track) - 1],
            _ => Mode::Aeolian,
        }
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
                    // The player has moved past the end screen: its stinger
                    // stops rather than play on under the new run.
                    out.push(Command::CutHeld);
                    out.push(play(Cue::Select, 0.55, PLAYFIELD_WIDTH / 2.0));
                    out.push(Command::MusicLevel(1.0));
                }
                // Q, from the pause or an end screen, back to the menu: the
                // same clean cut, and the pause's dimming lifted for the
                // menu's own music.
                (_, GameState::Menu) => {
                    out.push(Command::CutHeld);
                    out.push(play(Cue::Select, 0.5, PLAYFIELD_WIDTH / 2.0));
                    out.push(Command::MusicLevel(1.0));
                }
                _ => {}
            }
            if matches!(
                state,
                GameState::Menu | GameState::GameOver | GameState::Victory
            ) {
                self.served_level = None;
            }
        }
        self.last_state = Some(state);
    }

    fn events(
        &mut self,
        game: &Game,
        events: &[GameEvent],
        steps: &[(usize, usize)],
        delays: &[Option<f32>],
        released: usize,
        out: &mut Vec<Command>,
    ) {
        let paddle_x = game.paddle.x;
        let has = |want: &GameEvent| events.iter().any(|e| e == want);
        let run_over = has(&GameEvent::GameOver);
        let won = has(&GameEvent::Victory);
        let warped = has(&GameEvent::Warped);
        let mut bricks = released;
        let scale = self.mode_playing().rally();
        // Several balls can blow up in one tick: one blast is heard.
        let mut ball_blast = false;
        // Likewise one knock on steel a tick, however many balls bounce.
        let mut steel = false;
        for (i, event) in events.iter().enumerate() {
            match *event {
                GameEvent::BrickDestroyedAt { x, .. } => {
                    let step = steps.iter().find(|&&(j, _)| j == i).map_or(0, |&(_, s)| s);
                    let degree = scale[step];
                    let note = pitched(Cue::Brick, 0.6, x, semitones(degree));
                    match delays[i].unwrap_or(0.0) {
                        // A struck brick: at once, a few per tick at most.
                        d if d <= 0.0 => {
                            if bricks < MAX_BRICK_SOUNDS_PER_TICK {
                                out.push(note);
                            }
                            bricks += 1;
                        }
                        // A blast's neighbor: when the front reaches it.
                        d => self.land(d, note, out),
                    }
                }
                // An armored brick cracking, whatever cracked it: a ball, a
                // bolt, or a blast's front when it arrives.
                GameEvent::BrickStruck {
                    rect,
                    kind: BrickKind::Armored,
                    hits_left,
                    ..
                } if hits_left > 0 => {
                    let crack = play(Cue::Armor, 0.5, rect[0]);
                    self.land(delays[i].unwrap_or(0.0), crack, out);
                }
                // A ball turned back by an indestructible brick: a dull knock,
                // quiet, since walls of them turn balls back all the time.
                // A bolt's own sound already covers its hit.
                GameEvent::BrickStruck {
                    rect,
                    kind: BrickKind::Indestructible,
                    by: Striker::Ball | Striker::MetalBall | Striker::ExplosiveBall,
                    ..
                } if !steel => {
                    steel = true;
                    out.push(play(Cue::Steel, 0.35, rect[0]));
                }
                GameEvent::PaddleHit => {
                    // 300 px/s at launch to the 700 cap: up a minor third.
                    let ball = &game.ball;
                    let speed = ball.vx.hypot(ball.vy);
                    let t = ((speed - 300.0) / 400.0).clamp(0.0, 1.0);
                    out.push(pitched(Cue::Paddle, 0.65, paddle_x, semitones(3.0 * t)));
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
                // Quieter than an asteroid's: an explosive ball can blow up
                // every second for fifteen seconds, under the brick notes.
                GameEvent::BallExploded { x, .. } if !ball_blast => {
                    ball_blast = true;
                    out.push(play(Cue::Explosion, 0.4, x));
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
                    // The gate opens in the right wall: heard there.
                    PowerUpKind::Break => play(Cue::Gate, 0.6, PLAYFIELD_WIDTH),
                    _ => play(Cue::Capsule, 0.5, paddle_x),
                }),
                // A power about to run out, heard where it lives: on the
                // ball for a ball power, on the paddle for the rest.
                GameEvent::PowerUpExpiring(kind) => {
                    let x = match kind {
                        PowerUpKind::ExplosiveBall | PowerUpKind::MetalBall => game.ball.x,
                        _ => paddle_x,
                    };
                    out.push(play(Cue::Expiring, 0.6, x));
                }
                GameEvent::Warped => out.push(play(Cue::Warp, 0.6, PLAYFIELD_WIDTH)),
                GameEvent::BallLost if !has(&GameEvent::LifeLost) => {
                    out.push(play(Cue::BallLost, 0.45, paddle_x));
                }
                GameEvent::LifeLost if !run_over => {
                    out.push(stinger(Cue::LifeLost, 0.7, 1.6));
                }
                // A warp skips the level: its whoosh is the transition, and
                // the fanfare is for a level cleared brick by brick.
                GameEvent::LevelCleared if !won && !warped => {
                    out.push(stinger(Cue::LevelClear, 0.7, 2.7));
                }
                // The run's music fades out under these; the duck keeps its
                // tail from sounding over their first notes.
                GameEvent::GameOver => out.push(stinger(Cue::GameOver, 0.8, 1.5)),
                GameEvent::Victory => out.push(stinger(Cue::Victory, 0.8, 1.5)),
                _ => {}
            }
        }
    }

    /// The hit that cracks an armored brick: a brick that was intact last
    /// tick, still standing, now on its last hit. Re-read from scratch on a
    /// new level, so a fresh grid never sounds.
    /// Sounds the notes whose blast front reached their brick this tick,
    /// and returns how many brick notes that was (they count against the
    /// tick's `MAX_BRICK_SOUNDS_PER_TICK`). Leaving play drops them: a
    /// pause or a run's end is not the moment for a late crack.
    fn release(&mut self, game: &Game, out: &mut Vec<Command>) -> usize {
        if game.state != GameState::Playing {
            self.pending.clear();
            return 0;
        }
        let mut bricks = 0;
        let mut i = 0;
        while i < self.pending.len() {
            self.pending[i].0 = self.pending[i].0.saturating_sub(1);
            if self.pending[i].0 > 0 {
                i += 1;
                continue;
            }
            let (_, command) = self.pending.swap_remove(i);
            let brick = matches!(
                command,
                Command::Play {
                    cue: Cue::Brick,
                    ..
                }
            );
            if brick {
                if bricks >= MAX_BRICK_SOUNDS_PER_TICK {
                    continue;
                }
                bricks += 1;
            }
            out.push(command);
        }
        bricks
    }

    /// Plays `command` now, or after `delay` seconds (a blast's front on
    /// its way to the brick): the same moment the effects draw it.
    fn land(&mut self, delay: f32, command: Command, out: &mut Vec<Command>) {
        let ticks = (delay * TICK_RATE).round() as u32;
        if ticks == 0 {
            out.push(command);
        } else if self.pending.len() < MAX_PENDING {
            self.pending.push((ticks, command));
        }
    }

    /// The serve: the ball leaving the paddle at a launch, not a glue
    /// release mid-rally.
    fn launch(&mut self, game: &Game, out: &mut Vec<Command>) {
        let attached = game.ball.attached;
        if self.ball_attached && !attached && game.state == GameState::Playing {
            out.push(play(Cue::Launch, 0.4, game.paddle.x));
            self.served_level = Some(game.level);
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
        assert_eq!(next, vec![semitones(Mode::Aeolian.rally()[8])]);
    }

    /// Over whatever track plays, every note a rally can reach is in that
    /// track's mode: the rally always harmonizes (docs/design/music.md).
    #[test]
    fn every_rally_note_is_in_its_tracks_mode() {
        assert_eq!(LEVEL_MODES.len(), usize::from(LEVEL_TRACKS));
        for (track, mode) in LEVEL_MODES.iter().enumerate() {
            for note in mode.rally() {
                let degree = (note as u8) % 12;
                assert!(
                    mode.scale().contains(&degree),
                    "track {}: {note} semitones is not in {mode:?}",
                    track + 1
                );
            }
        }
    }

    /// The harmonic-minor tracks are the ones docs/design/music.md names;
    /// a recomposed track that changes mode changes this test with it.
    #[test]
    fn the_track_modes_are_read_from_the_score() {
        let harmonic: Vec<usize> = (1..=LEVEL_MODES.len())
            .filter(|&n| LEVEL_MODES[n - 1] == Mode::HarmonicMinor)
            .collect();
        assert_eq!(harmonic, [16, 19, 24, 29, 32]);
    }

    /// A campaign game on `level`, its first serve heard, so its track
    /// plays.
    fn serving(level: usize) -> (Director, Game) {
        let (mut director, mut game) = playing();
        game.level = level;
        director.served_level = Some(level);
        director.observe(&game, &[]);
        assert_eq!(director.last_music, Some(Some(Cue::Level(level as u8))));
        (director, game)
    }

    fn rally_of_ten(director: &mut Director, game: &Game) -> Vec<f32> {
        (0..10)
            .flat_map(|_| pitches(&director.observe(game, &[brick_at(400.0)])))
            .collect()
    }

    #[test]
    fn a_rally_over_harmonic_minor_plays_d_sharp_and_tops_out_on_the_tonic() {
        let (mut director, game) = serving(16);
        let heard = rally_of_ten(&mut director, &game);
        assert_eq!(heard[4], semitones(11.0), "D sharp, not D");
        assert_eq!(heard[9], semitones(24.0), "E, two octaves up");
        let more = pitches(&director.observe(&game, &[brick_at(400.0)]));
        assert_eq!(more, vec![semitones(24.0)], "a long rally holds the tonic");
    }

    #[test]
    fn a_rally_over_a_track_with_d_keeps_its_d() {
        let (mut director, game) = serving(1);
        assert_eq!(LEVEL_MODES[0], Mode::Dorian);
        let heard = rally_of_ten(&mut director, &game);
        assert_eq!(heard[4], semitones(10.0));
        assert_eq!(heard[9], semitones(24.0));
    }

    /// The last brick of a level is heard over that level's track: the
    /// simulation has already moved to the next level, the music has not.
    #[test]
    fn the_rally_follows_the_track_heard_not_the_level_loaded() {
        let (mut director, mut game) = serving(16);
        for _ in 0..4 {
            director.observe(&game, &[brick_at(400.0)]);
        }
        // Level 16's last brick is the rally's fifth: the tick that breaks
        // it has already loaded level 17 (Aeolian, with its D).
        game.level = 17;
        let last = pitches(&director.observe(&game, &[brick_at(400.0), E::LevelCleared]));
        assert_eq!(last, vec![semitones(11.0)], "D sharp, over Bridges");
    }

    #[test]
    fn ball_blasts_in_one_tick_are_heard_once_and_under_an_asteroids() {
        let (mut director, game) = playing();
        let out = director.observe(
            &game,
            &[
                E::BallExploded { x: 100.0, y: 100.0 },
                E::BallExploded { x: 500.0, y: 100.0 },
                E::AsteroidExploded { x: 300.0, y: 100.0 },
            ],
        );
        let gains: Vec<f32> = out
            .iter()
            .filter_map(|c| match c {
                Command::Play {
                    cue: Cue::Explosion,
                    gain,
                    ..
                } => Some(*gain),
                _ => None,
            })
            .collect();
        assert_eq!(gains.len(), 2, "one ball blast, one asteroid: {out:?}");
        assert!(gains[0] < gains[1], "the ball's is the quieter");
    }

    #[test]
    fn leaving_for_the_menu_cuts_the_stinger_and_lifts_the_pause_dimming() {
        for from in [GameState::Paused, GameState::GameOver, GameState::Victory] {
            let (mut director, mut game) = playing();
            game.state = from;
            director.observe(&game, &[]);
            game.state = GameState::Menu;
            let out = director.observe(&game, &[]);
            assert!(out.contains(&Command::CutHeld), "{from:?}: {out:?}");
            assert!(out.contains(&Command::MusicLevel(1.0)), "{from:?}");
            assert_eq!(music(&out), Some(Some(Cue::MusicMenu)), "{from:?}");
        }
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

    fn steel(x: f32, by: Striker) -> E {
        E::BrickStruck {
            rect: [x, 100.0, 52.0, 22.0],
            at: [x, 111.0],
            normal: [0.0, 1.0],
            kind: BrickKind::Indestructible,
            hits_left: 0,
            by,
        }
    }

    fn panned(commands: &[Command]) -> Vec<(Cue, f32)> {
        commands
            .iter()
            .filter_map(|c| match *c {
                Command::Play { cue, pan, .. } => Some((cue, pan)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_ball_on_steel_knocks_once_a_tick_and_a_bolt_does_not() {
        let (mut director, game) = playing();
        let out = director.observe(
            &game,
            &[
                steel(100.0, Striker::Ball),
                steel(700.0, Striker::MetalBall),
            ],
        );
        assert_eq!(panned(&out), [(Cue::Steel, pan_for(100.0))]);
        let bolt = director.observe(&game, &[steel(100.0, Striker::Laser)]);
        assert!(panned(&bolt).iter().all(|&(c, _)| c != Cue::Steel));
    }

    #[test]
    fn catching_break_opens_the_gate_in_the_right_wall() {
        let (mut director, game) = playing();
        let out = director.observe(&game, &[E::PowerUpCaught(PowerUpKind::Break)]);
        assert_eq!(panned(&out), [(Cue::Gate, pan_for(PLAYFIELD_WIDTH))]);
    }

    #[test]
    fn a_power_running_out_is_heard_where_it_lives() {
        let (mut director, mut game) = playing();
        game.paddle.x = 200.0;
        game.ball.x = 600.0;
        let paddle = director.observe(&game, &[E::PowerUpExpiring(PowerUpKind::Widen)]);
        assert_eq!(panned(&paddle), [(Cue::Expiring, pan_for(200.0))]);
        let ball = director.observe(&game, &[E::PowerUpExpiring(PowerUpKind::MetalBall)]);
        assert_eq!(panned(&ball), [(Cue::Expiring, pan_for(600.0))]);
    }

    fn crack(x: f32, by: crate::events::Striker) -> E {
        E::BrickStruck {
            rect: [x, 100.0, 52.0, 22.0],
            at: [x, 117.0],
            normal: [0.0, 1.0],
            kind: BrickKind::Armored,
            hits_left: 1,
            by,
        }
    }

    #[test]
    fn cracking_an_armored_brick_is_heard_whatever_cracked_it() {
        let (mut director, game) = playing();
        let ball = director.observe(&game, &[crack(200.0, crate::events::Striker::Ball)]);
        assert_eq!(cues(&ball), vec![Cue::Armor]);
        let laser = director.observe(&game, &[crack(200.0, crate::events::Striker::Laser)]);
        assert_eq!(cues(&laser), vec![Cue::Armor]);
        // A level loading new armor is not a crack: no event, no sound.
        assert!(cues(&director.observe(&game, &[])).is_empty());
    }

    #[test]
    fn a_blasts_neighbors_are_heard_as_its_front_reaches_them() {
        let (mut director, game) = playing();
        // The struck brick at the blast's center, a neighbor 52 px over and
        // an armored brick 104 px over that the blast cracks.
        let tick = [
            E::BallExploded { x: 400.0, y: 100.0 },
            brick_at(400.0),
            crack(504.0, crate::events::Striker::Blast),
        ];
        let neighbor = E::BrickDestroyedAt {
            x: 452.0,
            y: 100.0,
            width: 52.0,
            height: 22.0,
            kind: BrickKind::Normal,
        };
        let first = director.observe(&game, &[tick[0], tick[1], neighbor, tick[2]]);
        assert_eq!(
            cues(&first),
            vec![Cue::Explosion, Cue::Brick],
            "the blast and the brick it centers on, at once"
        );
        // 52 px at the front's speed, then 104 px: in ticks.
        let due = |d: f32| (d / cadence::BLAST_FRONT_SPEED * TICK_RATE).round() as usize;
        let mut heard = Vec::new();
        for tick in 1..=due(110.0) {
            for cue in cues(&director.observe(&game, &[])) {
                heard.push((tick, cue));
            }
        }
        assert_eq!(
            heard,
            vec![(due(52.0), Cue::Brick), (due(104.0), Cue::Armor)]
        );
    }

    #[test]
    fn a_pause_drops_the_notes_still_on_their_way() {
        let (mut director, mut game) = playing();
        let neighbor = E::BrickDestroyedAt {
            x: 452.0,
            y: 100.0,
            width: 52.0,
            height: 22.0,
            kind: BrickKind::Normal,
        };
        director.observe(&game, &[E::BallExploded { x: 400.0, y: 100.0 }, neighbor]);
        game.state = GameState::Paused;
        director.observe(&game, &[]);
        game.state = GameState::Playing;
        for _ in 0..60 {
            let out = director.observe(&game, &[]);
            assert!(!cues(&out).contains(&Cue::Brick), "{out:?}");
        }
    }

    fn music(commands: &[Command]) -> Option<Option<Cue>> {
        commands.iter().find_map(|c| match c {
            Command::Music(m) => Some(*m),
            _ => None,
        })
    }

    #[test]
    fn a_levels_music_starts_at_its_first_serve() {
        let (mut director, mut game) = playing();
        game.ball.attached = true;
        director.observe(&game, &[]);
        assert!(
            director.last_music == Some(None),
            "silence while the ball waits"
        );
        game.ball.attached = false;
        let out = director.observe(&game, &[]);
        assert_eq!(cues(&out), vec![Cue::Launch]);
        assert_eq!(music(&out), Some(Some(Cue::Level(game.level as u8))));

        // A lost life re-attaches the ball: the music plays on.
        game.ball.attached = true;
        assert_eq!(music(&director.observe(&game, &[E::LifeLost])), None);

        // The next level waits for its own serve.
        game.level += 1;
        assert_eq!(
            music(&director.observe(&game, &[E::LevelCleared])),
            Some(None)
        );
        game.ball.attached = false;
        assert_eq!(
            music(&director.observe(&game, &[])),
            Some(Some(Cue::Level(game.level as u8)))
        );
    }

    #[test]
    fn a_warp_plays_the_warp_without_the_fanfare() {
        let (mut director, game) = playing();
        let out = director.observe(&game, &[E::Warped, E::LevelCleared]);
        assert_eq!(cues(&out), vec![Cue::Warp]);
    }

    #[test]
    fn leaving_an_end_screen_cuts_its_stinger_and_waits_for_the_serve() {
        let (mut director, mut game) = playing();
        game.ball.attached = false;
        director.observe(&game, &[]);
        game.state = GameState::GameOver;
        let out = director.observe(&game, &[E::GameOver]);
        assert!(matches!(
            out.iter().find(|c| matches!(c, Command::Play { .. })),
            Some(Command::Play { cue: Cue::GameOver, duck, .. }) if *duck > 0.0
        ));
        game.state = GameState::Playing;
        game.ball.attached = true;
        let out = director.observe(&game, &[]);
        assert!(out.contains(&Command::CutHeld));
        assert!(
            director.last_music == Some(None),
            "the continued run waits for its serve"
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
