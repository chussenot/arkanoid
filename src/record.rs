//! Records, kept on disk between sessions: each progression's best score
//! and best rally. One small text file per progression, so a campaign run
//! never competes with a `--classic` one, a levelset's with another's, or
//! a rally-scored run (`--rally-score`, another scale) with a plain one.
//!
//! A loaded levelset is known by its levels, not its path: [`set_hash`]
//! hashes the bricks it lays out, so the same set fetched anywhere shares
//! its records, and a set whose levels change starts fresh. Shuffle draws
//! new levels every run, so all its runs share one record.
//!
//! I/O lives here, beside `main.rs`, never in `game.rs`: the simulation
//! stays pure. Every failure (no home directory, read-only disk, a corrupt
//! file) degrades to "no record yet", with one warning on a failed save;
//! the game never stops over a record file.
//!
//! Where: `$BRYX_DATA_DIR` if set (tests, portable installs), else the
//! platform's per-user data directory: `$XDG_DATA_HOME/bryx` or
//! `~/.local/share/bryx` on Linux, `~/Library/Application Support/bryx` on
//! macOS, `%APPDATA%\bryx` on Windows. Nothing is ever sent anywhere.

use std::path::PathBuf;

use crate::events::PowerUpKind;
use crate::levels::BrickKind;
use crate::levelset::{AuthoredDrop, ExternalLevel};

/// Which levels a run plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Progression {
    Campaign,
    Classic,
    Shuffle,
    /// A loaded levelset, by [`set_hash`].
    Set(u64),
}

/// Everything a run's records are kept apart by: its levels, and the rules
/// that change what a score means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RecordKey {
    pub progression: Progression,
    /// A loaded set or Shuffle under v1's rules (`--classic --levelset`).
    pub classic_rules: bool,
    /// Rally scoring (`--rally-score`): several times the plain scale.
    pub rally_score: bool,
}

impl RecordKey {
    /// The record's file name. The campaign and Classic keep the names
    /// they had when a record was one score.
    fn file_name(&self) -> String {
        let mut name = match self.progression {
            Progression::Campaign => "best-campaign".to_owned(),
            Progression::Classic => "best-classic".to_owned(),
            Progression::Shuffle => "best-shuffle".to_owned(),
            Progression::Set(hash) => format!("best-set-{hash:016x}"),
        };
        if self.classic_rules {
            name.push_str("-v1");
        }
        if self.rally_score {
            name.push_str("-rally");
        }
        name + ".txt"
    }
}

/// A progression's records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Best {
    pub score: u32,
    pub rally: u32,
}

impl Best {
    /// The record after a run that scored `score` with a longest rally of
    /// `rally`: each field the better of the two.
    pub fn with(self, score: u32, rally: u32) -> Self {
        Self {
            score: self.score.max(score),
            rally: self.rally.max(rally),
        }
    }

    /// The file's text: the score alone on the first line (as a record was
    /// before rallies were kept), then the rally.
    fn to_text(self) -> String {
        format!("{}\nrally {}\n", self.score, self.rally)
    }

    /// Reads [`Best::to_text`], or the older score-only file. Anything
    /// unreadable is no record.
    fn parse(text: &str) -> Self {
        let mut lines = text.lines().map(str::trim);
        let score = lines.next().and_then(|l| l.parse().ok()).unwrap_or(0);
        let rally = lines
            .find_map(|l| l.strip_prefix("rally "))
            .and_then(|n| n.trim().parse().ok())
            .unwrap_or(0);
        Self { score, rally }
    }
}

/// A levelset's identity: a 64-bit FNV-1a hash of the bricks its levels
/// lay out (position, kind and authored drop, level by level), not of its
/// path or its names. Stable across runs, builds and platforms.
pub fn set_hash(levels: &[ExternalLevel]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for &b in bytes {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for level in levels {
        feed(&(level.bricks.len() as u32).to_le_bytes());
        for brick in &level.bricks {
            feed(&brick.x.to_bits().to_le_bytes());
            feed(&brick.y.to_bits().to_le_bytes());
            feed(&[match brick.kind {
                BrickKind::Normal => 1,
                BrickKind::Armored => 2,
                BrickKind::Indestructible => 3,
            }]);
            feed(&match brick.powerup {
                None => [0, 0],
                Some(AuthoredDrop::Random) => [1, 0],
                Some(AuthoredDrop::RandomHelpful) => [2, 0],
                Some(AuthoredDrop::Kind(kind)) => [3, powerup_code(kind)],
            });
        }
    }
    hash
}

/// A fixed byte per power-up for [`set_hash`]: written out, so reordering
/// the enum never changes a set's identity.
fn powerup_code(kind: PowerUpKind) -> u8 {
    match kind {
        PowerUpKind::Widen => 1,
        PowerUpKind::Slow => 2,
        PowerUpKind::Multiball => 3,
        PowerUpKind::Glue => 4,
        PowerUpKind::Laser => 5,
        PowerUpKind::SuperLaser => 6,
        PowerUpKind::Tiny => 7,
        PowerUpKind::Player => 8,
        PowerUpKind::Break => 9,
        PowerUpKind::ExplosiveBall => 10,
        PowerUpKind::MetalBall => 11,
    }
}

fn data_dir() -> Option<PathBuf> {
    let env = |key: &str| std::env::var_os(key).filter(|v| !v.is_empty());
    if let Some(dir) = env("BRYX_DATA_DIR") {
        return Some(PathBuf::from(dir));
    }
    let base = if cfg!(target_os = "windows") {
        PathBuf::from(env("APPDATA")?)
    } else if cfg!(target_os = "macos") {
        PathBuf::from(env("HOME")?).join("Library/Application Support")
    } else {
        env("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| Some(PathBuf::from(env("HOME")?).join(".local/share")))?
    };
    Some(base.join("bryx"))
}

/// The stored records for `key`; zeros when there are none yet.
pub fn load(key: &RecordKey) -> Best {
    data_dir()
        .and_then(|dir| std::fs::read_to_string(dir.join(key.file_name())).ok())
        .map_or_else(Best::default, |text| Best::parse(&text))
}

/// Stores `best` as `key`'s records. Best effort: a failure is reported on
/// stderr and otherwise ignored.
pub fn save(key: &RecordKey, best: Best) {
    let Some(path) = data_dir().map(|dir| dir.join(key.file_name())) else {
        return;
    };
    let result = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&path, best.to_text()));
    if let Err(err) = result {
        eprintln!(
            "warning: could not save the records to {} ({err})",
            path.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::levelset::ExternalBrickSpawn;

    fn key(progression: Progression) -> RecordKey {
        RecordKey {
            progression,
            classic_rules: false,
            rally_score: false,
        }
    }

    /// One test touches the environment, so the cases run in sequence
    /// instead of racing on `BRYX_DATA_DIR`.
    #[test]
    fn records_round_trip_per_key_and_survive_old_and_corrupt_files() {
        let dir = std::env::temp_dir().join(format!("bryx-record-test-{}", std::process::id()));
        // SAFETY (edition 2021: `set_var` is safe): no other test reads
        // `BRYX_DATA_DIR`.
        std::env::set_var("BRYX_DATA_DIR", &dir);

        let campaign = key(Progression::Campaign);
        assert_eq!(load(&campaign), Best::default(), "no file yet");
        save(
            &campaign,
            Best {
                score: 12_340,
                rally: 23,
            },
        );
        assert_eq!(
            load(&campaign),
            Best {
                score: 12_340,
                rally: 23
            }
        );

        let rally = RecordKey {
            rally_score: true,
            ..campaign
        };
        assert_eq!(load(&rally), Best::default(), "rally scoring kept apart");
        assert_eq!(load(&key(Progression::Classic)), Best::default());
        assert_eq!(load(&key(Progression::Set(7))), Best::default());

        // A record from before rallies were kept: the score alone.
        std::fs::write(dir.join("best-classic.txt"), "4560\n").unwrap();
        assert_eq!(
            load(&key(Progression::Classic)),
            Best {
                score: 4560,
                rally: 0
            }
        );
        std::fs::write(dir.join("best-classic.txt"), "not a number").unwrap();
        assert_eq!(load(&key(Progression::Classic)), Best::default());

        std::env::remove_var("BRYX_DATA_DIR");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn every_key_has_its_own_file_and_the_old_names_stay() {
        let names: Vec<String> = [
            key(Progression::Campaign),
            key(Progression::Classic),
            key(Progression::Shuffle),
            key(Progression::Set(0xabc)),
            RecordKey {
                classic_rules: true,
                ..key(Progression::Set(0xabc))
            },
            RecordKey {
                rally_score: true,
                ..key(Progression::Campaign)
            },
        ]
        .iter()
        .map(RecordKey::file_name)
        .collect();
        assert_eq!(names[0], "best-campaign.txt");
        assert_eq!(names[1], "best-classic.txt");
        assert_eq!(names[3], "best-set-0000000000000abc.txt");
        assert_eq!(names[4], "best-set-0000000000000abc-v1.txt");
        assert_eq!(names[5], "best-campaign-rally.txt");
        let unique: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(unique.len(), names.len());
    }

    #[test]
    fn a_record_keeps_the_better_of_each() {
        let best = Best {
            score: 100,
            rally: 9,
        };
        assert_eq!(
            best.with(80, 12),
            Best {
                score: 100,
                rally: 12
            }
        );
        assert_eq!(
            best.with(120, 3),
            Best {
                score: 120,
                rally: 9
            }
        );
    }

    fn level(name: &str, bricks: &[(f32, BrickKind)]) -> ExternalLevel {
        ExternalLevel {
            author: "someone".into(),
            name: name.into(),
            bricks: bricks
                .iter()
                .map(|&(x, kind)| ExternalBrickSpawn {
                    x,
                    y: 0.0,
                    kind,
                    powerup: None,
                })
                .collect(),
        }
    }

    /// A set is its levels: renaming it keeps its records, changing a brick
    /// starts new ones. The value is pinned, so a change to the hash (which
    /// would orphan every stored levelset record) fails here first.
    #[test]
    fn a_set_is_known_by_its_bricks_not_its_names() {
        let a = [level(
            "One",
            &[(0.0, BrickKind::Normal), (52.0, BrickKind::Armored)],
        )];
        let renamed = [level(
            "Uno",
            &[(0.0, BrickKind::Normal), (52.0, BrickKind::Armored)],
        )];
        let changed = [level(
            "One",
            &[(0.0, BrickKind::Normal), (52.0, BrickKind::Normal)],
        )];
        assert_eq!(set_hash(&a), set_hash(&renamed));
        assert_ne!(set_hash(&a), set_hash(&changed));
        assert_eq!(set_hash(&a), 0xfb9f_9c70_1554_f3ca);
    }
}
