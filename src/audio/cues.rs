//! The cue bank: every sound BRYX can make, as the PCM `build.rs` rendered
//! from `audio/cues/*.abc` with BRYX's fork of the Electric Pulse engine.
//! The bytes stay where the binary put them: 8-bit mu-law mono (`mulaw`),
//! read and decoded one sample at a time by the mixer, so 33 music tracks
//! cost no memory beyond the binary itself.

use super::mulaw::MULAW_TO_F32;

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
    Steel,
    Gate,
    Expiring,
    // Stingers
    LifeLost,
    LevelClear,
    /// An act's last level cleared (`movement.rs`): the level fanfare,
    /// grown by a bar.
    ActClear,
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
    pub const NAMED: [Cue; 24] = [
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
        Cue::Steel,
        Cue::Gate,
        Cue::Expiring,
        Cue::LifeLost,
        Cue::LevelClear,
        Cue::ActClear,
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
            Cue::Steel => "steel",
            Cue::Gate => "gate",
            Cue::Expiring => "expiring",
            Cue::LifeLost => "life_lost",
            Cue::LevelClear => "level_clear",
            Cue::ActClear => "act_clear",
            Cue::GameOver => "game_over",
            Cue::Victory => "victory",
            Cue::MusicMenu => "music_menu",
            Cue::Level(_) => return None,
        })
    }

    /// Whether new music waits for this cue to finish: the stingers that
    /// end a level or a run, and the sounds that start play (the serve, the
    /// warp). The mixer holds a music change until none of them sounds.
    pub fn holds_music(self) -> bool {
        matches!(
            self,
            Cue::LevelClear
                | Cue::ActClear
                | Cue::GameOver
                | Cue::Victory
                | Cue::Launch
                | Cue::Warp
        )
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
    include!(concat!(env!("OUT_DIR"), "/scores.rs"));
}

/// `music_01`..`music_32` without formatting a string: the audio thread
/// looks a track up by level, and must not allocate to do it.
const LEVEL_NAMES: [&str; LEVEL_TRACKS as usize] = [
    "music_01", "music_02", "music_03", "music_04", "music_05", "music_06", "music_07", "music_08",
    "music_09", "music_10", "music_11", "music_12", "music_13", "music_14", "music_15", "music_16",
    "music_17", "music_18", "music_19", "music_20", "music_21", "music_22", "music_23", "music_24",
    "music_25", "music_26", "music_27", "music_28", "music_29", "music_30", "music_31", "music_32",
];

impl Cue {
    /// The name a studio score for this cue has (`audio/score/<name>.score`).
    fn score_name(self) -> Option<&'static str> {
        match self {
            Cue::Level(n) => LEVEL_NAMES.get(usize::from(n).wrapping_sub(1)).copied(),
            named => named.file(),
        }
    }
}

/// The studio render for `cue`, QOA stereo (`src/audio/qoa.rs`), when
/// `audio/score/` has a score of that name; it replaces the ABC render.
/// `None` otherwise, and always without the `audio` feature.
pub fn studio(cue: Cue) -> Option<&'static [u8]> {
    #[cfg(feature = "audio")]
    {
        let name = cue.score_name()?;
        rendered::STUDIO
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, qoa)| *qoa)
    }
    #[cfg(not(feature = "audio"))]
    {
        let _ = cue.score_name();
        None
    }
}

/// The embedded PCM for `cue`: mu-law mono at [`CUE_SAMPLE_RATE`],
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

/// One embedded mu-law byte as a sample in `-1.0..1.0`.
#[inline]
pub fn sample(byte: u8) -> f32 {
    MULAW_TO_F32[usize::from(byte)]
}

/// The mu-law bytes for full scale, positive and negative: no level track
/// may reach them.
#[cfg(test)]
const FULL_SCALE: [u8; 2] = [0x80, 0x00];

#[cfg(test)]
mod tests {
    use super::*;

    use crate::audio::mulaw::{mulaw_decode, mulaw_encode};

    #[test]
    fn mulaw_is_centered_symmetric_and_close() {
        assert_eq!(mulaw_encode(0), 0xFF, "silence");
        assert_eq!(sample(0xFF), 0.0);
        assert!(sample(FULL_SCALE[0]) > 0.97 && sample(FULL_SCALE[1]) < -0.97);
        assert_eq!(mulaw_encode(i16::MAX), FULL_SCALE[0]);
        assert_eq!(mulaw_encode(i16::MIN), FULL_SCALE[1]);
        // Every 16-bit sample comes back within half its segment's step: the
        // error grows with the level, staying near 1/32 of it (~30 dB
        // under the signal at every level, not just loud ones).
        for s in (-32_768..=32_767).step_by(7) {
            let s = s as i16;
            let back = i32::from(mulaw_decode(mulaw_encode(s)));
            let err = (back - i32::from(s)).abs();
            assert!(
                err <= 8 + i32::from(s).abs().min(32_635) / 30,
                "{s} -> {back}"
            );
            assert!(back.signum() * i32::from(s).signum() >= 0, "sign of {s}");
        }
    }

    /// A cue's studio render, decoded, if it has one.
    #[cfg(feature = "audio")]
    fn studio_frames(cue: Cue) -> Option<Vec<[i16; 2]>> {
        let mut q = crate::audio::qoa::QoaStream::new(studio(cue)?).unwrap();
        Some(std::iter::from_fn(|| q.next_frame()).collect())
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
            // A studio render replaces the ABC one, which is then not
            // embedded at all: one render per cue, never both.
            if let Some(frames) = studio_frames(cue) {
                assert!(pcm(cue).is_empty(), "{cue:?} embeds both renders");
                assert!(
                    frames.iter().any(|f| f[0].abs().max(f[1].abs()) > 2_048),
                    "{cue:?} is silent"
                );
                continue;
            }
            let pcm = pcm(cue);
            assert!(!pcm.is_empty(), "{cue:?} has no render");
            assert!(
                pcm.iter().any(|&b| sample(b).abs() > 0.0625),
                "{cue:?} is silent"
            );
        }
        assert_eq!(rendered::LEVEL_MUSIC.len(), usize::from(LEVEL_TRACKS));
        assert!(pcm(Cue::Level(0)).is_empty() && pcm(Cue::Level(33)).is_empty());
    }

    #[cfg(feature = "audio")]
    #[test]
    fn effects_are_trimmed_short_and_music_loops_are_long() {
        for cue in every_cue() {
            if let Some(frames) = studio_frames(cue) {
                let secs = frames.len() as f32 / 48_000.0;
                if cue.is_music() {
                    assert!(secs > 20.0, "{cue:?}: {secs:.1}s");
                } else {
                    assert!(secs < 9.0, "{cue:?}: {secs:.1}s");
                }
                continue;
            }
            let pcm = pcm(cue);
            let secs = pcm.len() as f32 / CUE_SAMPLE_RATE;
            if cue.is_music() {
                assert!(secs > 20.0, "{cue:?}: {secs:.1}s");
            } else {
                assert!(secs < 8.0, "{cue:?}: {secs:.1}s");
                let last = sample(*pcm.last().unwrap()).abs();
                assert!(last > 0.5 / 128.0, "{cue:?} not trimmed");
            }
        }
    }

    /// Every studio score is named after a cue (a typo would leave the
    /// ABC render playing, silently) and was written as 48 kHz stereo QOA.
    #[cfg(feature = "audio")]
    #[test]
    fn every_studio_score_replaces_a_cue_and_is_48_khz_stereo_qoa() {
        for (name, bytes) in rendered::STUDIO {
            let cue = every_cue().find(|c| c.score_name() == Some(*name));
            assert!(cue.is_some(), "audio/score/{name}.score names no cue");
            assert_eq!(studio(cue.unwrap()), Some(*bytes));
            let qoa = crate::audio::qoa::QoaStream::new(bytes)
                .unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_eq!((qoa.channels(), qoa.sample_rate()), (2, 48_000), "{name}");
        }
        assert_eq!(studio(Cue::Level(0)), None);
        assert_eq!(studio(Cue::Level(33)), None);
    }

    /// The score is mastered (`audio/compose.py`): no level track clips,
    /// so no level is ever louder than the limiter expects.
    #[cfg(feature = "audio")]
    #[test]
    fn no_music_track_clips() {
        for n in 1..=LEVEL_TRACKS {
            if let Some(frames) = studio_frames(Cue::Level(n)) {
                // The studio's limiter holds -1 dBFS; QOA may overshoot
                // by its error, never to full scale.
                let hot = frames
                    .iter()
                    .flatten()
                    .filter(|s| s.unsigned_abs() >= 32_700)
                    .count();
                assert_eq!(hot, 0, "level {n} clips");
                continue;
            }
            let pcm = pcm(Cue::Level(n));
            let hot = pcm.iter().filter(|b| FULL_SCALE.contains(b)).count();
            assert_eq!(hot, 0, "level {n} clips");
        }
    }
}
