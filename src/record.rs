//! The best score, kept on disk between sessions. One small text file per
//! built-in progression, so a campaign score never competes with a
//! `--classic` one. Loaded levelsets keep no record: a set is any file
//! from anywhere, and a best score keyed on a path would mislead.
//!
//! I/O lives here, beside `main.rs`, never in `game.rs`: the simulation
//! stays pure. Every failure (no home directory, read-only disk, a corrupt
//! file) degrades to "no record yet", with one warning on a failed save;
//! the game never stops over a score file.
//!
//! Where: `$BRYX_DATA_DIR` if set (tests, portable installs), else the
//! platform's per-user data directory: `$XDG_DATA_HOME/bryx` or
//! `~/.local/share/bryx` on Linux, `~/Library/Application Support/bryx` on
//! macOS, `%APPDATA%\bryx` on Windows. Nothing is ever sent anywhere.

use std::path::PathBuf;

use crate::game::LevelSetKind;

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

/// The record file for a progression, `None` for loaded levelsets.
fn path(set: LevelSetKind) -> Option<PathBuf> {
    let name = match set {
        LevelSetKind::Campaign => "best-campaign.txt",
        LevelSetKind::Classic => "best-classic.txt",
        LevelSetKind::Loaded => return None,
    };
    Some(data_dir()?.join(name))
}

/// The stored best score, 0 when there is none yet; `None` when this
/// progression keeps no record.
pub fn load(set: LevelSetKind) -> Option<u32> {
    let path = path(set)?;
    Some(
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or(0),
    )
}

/// Stores `score` as the new best. Best effort: a failure is reported on
/// stderr and otherwise ignored.
pub fn save(set: LevelSetKind, score: u32) {
    let Some(path) = path(set) else {
        return;
    };
    let result = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&path, format!("{score}\n")));
    if let Err(err) = result {
        eprintln!(
            "warning: could not save the best score to {} ({err})",
            path.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test touches the environment, so the cases run in sequence
    /// instead of racing on `BRYX_DATA_DIR`.
    #[test]
    fn records_round_trip_per_progression_and_survive_a_corrupt_file() {
        let dir = std::env::temp_dir().join(format!("bryx-record-test-{}", std::process::id()));
        // SAFETY (edition 2021: `set_var` is safe): no other test reads
        // `BRYX_DATA_DIR`.
        std::env::set_var("BRYX_DATA_DIR", &dir);

        assert_eq!(load(LevelSetKind::Campaign), Some(0), "no file yet");
        save(LevelSetKind::Campaign, 12_340);
        assert_eq!(load(LevelSetKind::Campaign), Some(12_340));
        assert_eq!(load(LevelSetKind::Classic), Some(0), "kept apart");
        assert_eq!(load(LevelSetKind::Loaded), None, "levelsets keep none");

        std::fs::write(dir.join("best-classic.txt"), "not a number").unwrap();
        assert_eq!(load(LevelSetKind::Classic), Some(0));

        std::env::remove_var("BRYX_DATA_DIR");
        std::fs::remove_dir_all(&dir).ok();
    }
}
