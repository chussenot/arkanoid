//! The cue bank: every sound BRYX can make, decoded from the PCM `build.rs`
//! rendered from `audio/cues/*.abc` with the Electric Pulse engine.

use std::sync::Arc;

/// The rate `build.rs` renders cues at (its `SAMPLE_RATE`).
pub const CUE_SAMPLE_RATE: f32 = 22_050.0;

/// One sound, by name. A variant's `file` must match an `audio/cues/*.abc`
/// file stem; `every_cue_was_rendered` holds the two together.
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
    MusicAct1,
    MusicAct2,
    MusicAct3,
    MusicAct4,
}

impl Cue {
    pub const ALL: [Cue; 24] = [
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
        Cue::MusicAct1,
        Cue::MusicAct2,
        Cue::MusicAct3,
        Cue::MusicAct4,
    ];

    pub fn file(self) -> &'static str {
        match self {
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
            Cue::MusicAct1 => "music_act1",
            Cue::MusicAct2 => "music_act2",
            Cue::MusicAct3 => "music_act3",
            Cue::MusicAct4 => "music_act4",
        }
    }

    /// Music loops play whole, start to end and round again; everything
    /// else is trimmed of the silence the engine renders after it.
    pub fn is_music(self) -> bool {
        matches!(
            self,
            Cue::MusicMenu | Cue::MusicAct1 | Cue::MusicAct2 | Cue::MusicAct3 | Cue::MusicAct4
        )
    }
}

#[cfg(feature = "audio")]
mod rendered {
    include!(concat!(env!("OUT_DIR"), "/cues.rs"));
}

/// The embedded PCM for `cue`: unsigned 8-bit mono at
/// [`CUE_SAMPLE_RATE`]. Empty without the `audio` feature.
fn raw(cue: Cue) -> &'static [u8] {
    #[cfg(feature = "audio")]
    {
        rendered::CUES
            .iter()
            .find(|(name, _)| *name == cue.file())
            .map(|(_, pcm)| *pcm)
            .unwrap_or(&[])
    }
    #[cfg(not(feature = "audio"))]
    {
        let _ = cue;
        &[]
    }
}

/// The engine's unsigned 8-bit samples (centered at 128) as `-1.0..1.0`,
/// trailing near-silence trimmed for effects so a voice frees itself as
/// soon as its sound ends.
pub fn decode(cue: Cue, pcm: &[u8]) -> Arc<[f32]> {
    let mut end = pcm.len();
    if !cue.is_music() {
        while end > 0 && pcm[end - 1].abs_diff(128) <= 1 {
            end -= 1;
        }
    }
    pcm[..end]
        .iter()
        .map(|&b| (f32::from(b) - 128.0) / 128.0)
        .collect()
}

/// Every cue, decoded once, indexed by `Cue as usize`.
pub struct Bank {
    samples: Vec<Arc<[f32]>>,
}

impl Bank {
    pub fn load() -> Self {
        Self {
            samples: Cue::ALL.iter().map(|&c| decode(c, raw(c))).collect(),
        }
    }

    pub fn get(&self, cue: Cue) -> Arc<[f32]> {
        Arc::clone(&self.samples[cue as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cue_all_is_in_declaration_order() {
        for (i, cue) in Cue::ALL.iter().enumerate() {
            assert_eq!(*cue as usize, i, "{cue:?}");
        }
    }

    #[test]
    fn decoding_centers_and_trims_effects_but_not_music() {
        let pcm = [128u8, 255, 0, 128, 129, 128];
        let fx = decode(Cue::Brick, &pcm);
        assert_eq!(fx.len(), 3, "trailing silence trimmed");
        assert_eq!(fx[0], 0.0);
        assert!(fx[1] > 0.99 && fx[2] == -1.0);
        assert_eq!(decode(Cue::MusicAct1, &pcm).len(), pcm.len());
    }

    #[cfg(feature = "audio")]
    #[test]
    fn every_cue_was_rendered() {
        for cue in Cue::ALL {
            let pcm = raw(cue);
            assert!(
                !pcm.is_empty(),
                "{} has no audio/cues/{0}.abc render",
                cue.file()
            );
            assert!(
                pcm.iter().any(|b| b.abs_diff(128) > 8),
                "{} rendered silent",
                cue.file()
            );
        }
        assert_eq!(rendered::CUES.len(), Cue::ALL.len(), "an .abc no Cue plays");
    }

    #[cfg(feature = "audio")]
    #[test]
    fn effects_stay_short_and_music_loops_are_long() {
        let bank = Bank::load();
        for cue in Cue::ALL {
            let secs = bank.get(cue).len() as f32 / CUE_SAMPLE_RATE;
            if cue.is_music() {
                assert!(secs > 8.0, "{cue:?}: {secs:.2}s loop");
            } else {
                assert!(secs < 8.0, "{cue:?}: {secs:.2}s");
            }
        }
    }
}
