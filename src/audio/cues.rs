//! The cue bank: every sound BRYX can make, as the PCM `build.rs` rendered
//! from `audio/cues/*.abc` with the Electric Pulse engine. The bytes stay
//! where the binary put them: unsigned 8-bit mono, read and converted one
//! sample at a time by the mixer, so 33 music tracks cost no memory beyond
//! the binary itself.

/// The rate `build.rs` renders cues at (its `SAMPLE_RATE`).
pub const CUE_SAMPLE_RATE: f32 = 22_050.0;

/// Level tracks in the score (`audio/compose.py`), one per campaign level.
pub const LEVEL_TRACKS: u8 = 32;

/// One sound. Every variant but `Level` names an `audio/cues/*.abc` file
/// (`file`); `Level(n)` is `music_NN.abc`, the track for level `n`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cue {
    // Effects
    Paddle,
    Brick,
    Armor,
    Capsule,
    Tiny,
    OneUp,
    Laser,
    SuperLaser,
    Asteroid,
    Explosion,
    BallLost,
    Launch,
    Warp,
    Select,
    Pause,
    // Stingers
    LifeLost,
    LevelClear,
    GameOver,
    Victory,
    // Music loops
    MusicMenu,
    /// The track for level 1..=[`LEVEL_TRACKS`].
    Level(u8),
}

impl Cue {
    /// Every cue that is not a level track.
    #[cfg(test)]
    pub const NAMED: [Cue; 20] = [
        Cue::Paddle,
        Cue::Brick,
        Cue::Armor,
        Cue::Capsule,
        Cue::Tiny,
        Cue::OneUp,
        Cue::Laser,
        Cue::SuperLaser,
        Cue::Asteroid,
        Cue::Explosion,
        Cue::BallLost,
        Cue::Launch,
        Cue::Warp,
        Cue::Select,
        Cue::Pause,
        Cue::LifeLost,
        Cue::LevelClear,
        Cue::GameOver,
        Cue::Victory,
        Cue::MusicMenu,
    ];

    /// The cue's file stem; `None` for a level track (see [`pcm`]).
    pub fn file(self) -> Option<&'static str> {
        Some(match self {
            Cue::Paddle => "paddle",
            Cue::Brick => "brick",
            Cue::Armor => "armor",
            Cue::Capsule => "capsule",
            Cue::Tiny => "tiny",
            Cue::OneUp => "oneup",
            Cue::Laser => "laser",
            Cue::SuperLaser => "superlaser",
            Cue::Asteroid => "asteroid",
            Cue::Explosion => "explosion",
            Cue::BallLost => "ball_lost",
            Cue::Launch => "launch",
            Cue::Warp => "warp",
            Cue::Select => "select",
            Cue::Pause => "pause",
            Cue::LifeLost => "life_lost",
            Cue::LevelClear => "level_clear",
            Cue::GameOver => "game_over",
            Cue::Victory => "victory",
            Cue::MusicMenu => "music_menu",
            Cue::Level(_) => return None,
        })
    }

    /// Music loops play whole, start to end and round again.
    #[cfg(test)]
    pub fn is_music(self) -> bool {
        matches!(self, Cue::MusicMenu | Cue::Level(_))
    }
}

#[cfg(feature = "audio")]
mod rendered {
    include!(concat!(env!("OUT_DIR"), "/cues.rs"));
}

/// The embedded PCM for `cue`: unsigned 8-bit mono at [`CUE_SAMPLE_RATE`],
/// effects already trimmed of trailing silence by `build.rs`. Empty
/// without the `audio` feature, or for a level past the score.
pub fn pcm(cue: Cue) -> &'static [u8] {
    #[cfg(feature = "audio")]
    {
        match cue {
            Cue::Level(n) => rendered::LEVEL_MUSIC
                .get(usize::from(n).wrapping_sub(1))
                .copied()
                .unwrap_or(&[]),
            named => rendered::CUES
                .iter()
                .find(|(name, _)| Some(*name) == named.file())
                .map(|(_, pcm)| *pcm)
                .unwrap_or(&[]),
        }
    }
    #[cfg(not(feature = "audio"))]
    {
        let _ = cue;
        &[]
    }
}

/// One of the engine's samples (unsigned, centered at 128) as `-1.0..1.0`.
#[inline]
pub fn sample(byte: u8) -> f32 {
    (f32::from(byte) - 128.0) / 128.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_are_centered() {
        assert_eq!(sample(128), 0.0);
        assert_eq!(sample(0), -1.0);
        assert!(sample(255) > 0.99);
    }

    #[cfg(feature = "audio")]
    fn every_cue() -> impl Iterator<Item = Cue> {
        Cue::NAMED
            .into_iter()
            .chain((1..=LEVEL_TRACKS).map(Cue::Level))
    }

    #[cfg(feature = "audio")]
    #[test]
    fn every_cue_was_rendered_and_is_audible() {
        for cue in every_cue() {
            let pcm = pcm(cue);
            assert!(!pcm.is_empty(), "{cue:?} has no render");
            assert!(pcm.iter().any(|b| b.abs_diff(128) > 8), "{cue:?} is silent");
        }
        assert_eq!(rendered::LEVEL_MUSIC.len(), usize::from(LEVEL_TRACKS));
        assert!(pcm(Cue::Level(0)).is_empty() && pcm(Cue::Level(33)).is_empty());
    }

    #[cfg(feature = "audio")]
    #[test]
    fn effects_are_trimmed_short_and_music_loops_are_long() {
        for cue in every_cue() {
            let pcm = pcm(cue);
            let secs = pcm.len() as f32 / CUE_SAMPLE_RATE;
            if cue.is_music() {
                assert!(secs > 20.0, "{cue:?}: {secs:.1}s");
            } else {
                assert!(secs < 8.0, "{cue:?}: {secs:.1}s");
                assert!(pcm.last().unwrap().abs_diff(128) > 1, "{cue:?} not trimmed");
            }
        }
    }

    /// The score is mastered (`audio/compose.py`): no level track clips,
    /// so no level is ever louder than the limiter expects.
    #[cfg(feature = "audio")]
    #[test]
    fn no_music_track_clips() {
        for n in 1..=LEVEL_TRACKS {
            let pcm = pcm(Cue::Level(n));
            let hot = pcm.iter().filter(|&&b| b == 0 || b == 255).count();
            assert_eq!(hot, 0, "level {n} clips");
        }
    }
}
