//! Sound (docs/design/audio.md).
//!
//! - `cues`: the sounds, rendered at build time from `audio/cues/*.abc` by
//!   BRYX's fork of the Electric Pulse engine (`build.rs`), stored as
//!   mu-law (`mulaw`);
//! - `director`: which sound plays when, where and at what pitch: pure,
//!   tested;
//! - `mixer`: effects, stingers and music into one stereo stream: pure,
//!   tested;
//! - `output`: the `cpal` device stream that runs the mixer.
//!
//! Like the renderers, sound only *reads* the game: `main.rs` hands each
//! tick's events over before draining them (events.rs's contract), and
//! nothing here can change what happens.

// Without the `audio` feature there is no device to feed, so the bank and
// the mixer are only exercised by their tests.
#[cfg_attr(not(feature = "audio"), allow(dead_code))]
mod cues;
#[cfg_attr(not(feature = "audio"), allow(dead_code))]
mod director;
#[cfg_attr(not(feature = "audio"), allow(dead_code))]
mod mixer;
mod mulaw;
#[cfg(feature = "audio")]
mod output;
#[cfg(all(test, feature = "audio"))]
mod probes;

pub use director::SoundMode;

use crate::events::GameEvent;
use crate::game::Game;

pub struct Audio {
    director: director::Director,
    mode: SoundMode,
    #[cfg(feature = "audio")]
    output: Option<output::Output>,
}

impl Audio {
    /// Opens the audio device. Without one, or without the `audio`
    /// feature, every call is a no-op and the game plays silent.
    pub fn new(mode: SoundMode) -> Self {
        #[cfg(feature = "audio")]
        let output = match output::Output::open() {
            Ok(output) => {
                output.send(mode.command());
                Some(output)
            }
            Err(err) => {
                eprintln!("audio: {err}; playing without sound");
                None
            }
        };
        Self {
            // Seeds the levelset playlist: a fresh order each session.
            director: director::Director::new(rand::random()),
            mode,
            #[cfg(feature = "audio")]
            output,
        }
    }

    /// Hands one simulation tick to the sound director.
    pub fn observe(&mut self, game: &Game, events: &[GameEvent]) {
        let commands = self.director.observe(game, events);
        #[cfg(feature = "audio")]
        if let Some(output) = &self.output {
            for command in commands {
                output.send(command);
            }
        }
        #[cfg(not(feature = "audio"))]
        drop(commands);
    }

    /// M: all sound, effects only, off, and round again.
    pub fn cycle_mode(&mut self) {
        self.mode = self.mode.next();
        #[cfg(feature = "audio")]
        if let Some(output) = &self.output {
            output.send(self.mode.command());
        }
    }

    /// What the HUD shows for the M key; `None` when this build or this
    /// machine has no sound to set.
    pub fn label(&self) -> Option<&'static str> {
        #[cfg(feature = "audio")]
        if self.output.is_some() {
            return Some(self.mode.label());
        }
        None
    }
}
