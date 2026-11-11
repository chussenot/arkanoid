//! The timing sound and light must share, in one place, so a note and the
//! effect it belongs with cannot drift apart (docs/design/kinetic-fx.md,
//! "Timing").
//!
//! - **The blast front.** An explosive ball's or an asteroid's blast
//!   reaches the bricks around it at [`BLAST_FRONT_SPEED`]: each brick it
//!   breaks or cracks lands when the front gets there. The effects draw it
//!   then, and the sound director plays its note then
//!   ([`landing_delays`]).
//! - **The fanfares' meter.** The level-clear sweep crosses the field in
//!   one bar of the fanfare and lands on its next downbeat; the warp's, in
//!   one bar of its whoosh. Bar lengths come from the cues' own ABC
//!   headers, read by build.rs, so rewriting a cue moves the sweep with it.
//!
//! Pure, like `rally.rs`: it reads events, never changes anything.

use crate::events::{GameEvent, Striker};

/// The blast's front, in field pixels per second: a brick 52 px from the
/// center lands an eighth of a second after the blast.
pub const BLAST_FRONT_SPEED: f32 = 420.0;

/// A cue's meter: how long one bar lasts, and how many beats it holds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Meter {
    pub bar: f64,
    pub beats: u32,
}

impl Meter {
    /// One bar, in seconds.
    pub fn bar_secs(self) -> f32 {
        self.bar as f32
    }

    /// One beat, in seconds.
    pub fn beat_secs(self) -> f32 {
        (self.bar / f64::from(self.beats.max(1))) as f32
    }
}

include!(concat!(env!("OUT_DIR"), "/cue_meters.rs"));

/// When each landing in one tick's `events` happens, in seconds after the
/// tick: `each(index, delay)` for every `BrickDestroyedAt` and every
/// blast's `BrickStruck`.
///
/// A brick a striker touched lands at once: the `BrickStruck` that broke it
/// comes first in the tick, for the same brick. A brick a blast reached
/// lands when the front gets there: the distance from the tick's latest
/// blast to the brick, over [`BLAST_FRONT_SPEED`]. Anything else (a bolt's
/// break, no blast) lands at once.
pub fn landing_delays(events: &[GameEvent], mut each: impl FnMut(usize, f32)) {
    // The brick the last breaking contact touched: its break follows.
    let mut touched: Option<[f32; 2]> = None;
    let mut blast: Option<[f32; 2]> = None;
    let front = |center: Option<[f32; 2]>, x: f32, y: f32| {
        center.map_or(0.0, |c| (x - c[0]).hypot(y - c[1]) / BLAST_FRONT_SPEED)
    };
    for (i, event) in events.iter().enumerate() {
        match *event {
            GameEvent::BallExploded { x, y } | GameEvent::AsteroidExploded { x, y } => {
                blast = Some([x, y]);
            }
            GameEvent::BrickStruck {
                rect,
                by: Striker::Blast,
                ..
            } => each(i, front(blast, rect[0], rect[1])),
            GameEvent::BrickStruck {
                rect, hits_left: 0, ..
            } => touched = Some([rect[0], rect[1]]),
            GameEvent::BrickDestroyedAt { x, y, .. } => {
                if touched == Some([x, y]) {
                    touched = None;
                    each(i, 0.0);
                } else {
                    each(i, front(blast, x, y));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::levels::BrickKind;

    fn broke(x: f32) -> GameEvent {
        GameEvent::BrickDestroyedAt {
            x,
            y: 100.0,
            width: 52.0,
            height: 22.0,
            kind: BrickKind::Normal,
        }
    }

    fn struck(x: f32, hits_left: u8, by: Striker) -> GameEvent {
        GameEvent::BrickStruck {
            rect: [x, 100.0, 52.0, 22.0],
            at: [x, 117.0],
            normal: [0.0, 1.0],
            kind: BrickKind::Armored,
            hits_left,
            by,
        }
    }

    fn delays(events: &[GameEvent]) -> Vec<(usize, f32)> {
        let mut out = Vec::new();
        landing_delays(events, |i, d| out.push((i, d)));
        out
    }

    #[test]
    fn the_struck_brick_lands_at_once_its_neighbors_as_the_front_arrives() {
        let events = [
            struck(400.0, 0, Striker::ExplosiveBall),
            GameEvent::BallExploded { x: 400.0, y: 100.0 },
            struck(504.0, 1, Striker::Blast),
            broke(400.0),
            broke(452.0),
        ];
        let d = delays(&events);
        assert_eq!(d.len(), 3);
        assert_eq!(d[0].0, 2);
        assert!(
            (d[0].1 - 104.0 / BLAST_FRONT_SPEED).abs() < 1e-6,
            "crack two over"
        );
        assert_eq!(d[1], (3, 0.0), "the struck brick");
        assert!(
            (d[2].1 - 52.0 / BLAST_FRONT_SPEED).abs() < 1e-6,
            "next door"
        );
    }

    #[test]
    fn without_a_blast_everything_lands_at_once() {
        let events = [struck(100.0, 0, Striker::Laser), broke(100.0), broke(300.0)];
        assert_eq!(delays(&events), [(1, 0.0), (2, 0.0)]);
    }

    #[test]
    fn the_fanfares_meters_come_from_their_cues() {
        // level_clear.score: tempo 160, in four. sfx/warp.score: `# meter 1 4`.
        assert!((LEVEL_CLEAR.bar_secs() - 1.5).abs() < 1e-6);
        assert!((LEVEL_CLEAR.beat_secs() - 0.375).abs() < 1e-6);
        assert!((WARP.bar_secs() - 1.0).abs() < 1e-6);
    }

    /// An act's end plays `act_clear` where a level's plays `level_clear`,
    /// and the level-end sweep keeps `LEVEL_CLEAR`'s beat for both: the two
    /// fanfares must share their tempo (both are in four).
    #[test]
    fn the_act_fanfare_keeps_the_level_fanfares_beat() {
        let tempo = |score: &str| {
            score
                .lines()
                .find_map(|l| l.strip_prefix("tempo 0 "))
                .map(|v| v.trim().to_owned())
        };
        let level = tempo(include_str!("../audio/score/level_clear.score"));
        let act = tempo(include_str!("../audio/score/act_clear.score"));
        assert!(level.is_some(), "level_clear.score has a tempo");
        assert_eq!(level, act);
    }
}
