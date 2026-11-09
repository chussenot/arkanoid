//! BRYX's mixer: runs inside the audio callback, so it allocates nothing
//! per sample and blocks on nothing. It plays three kinds of sound:
//! - **effects**: one-shot voices with a gain, a stereo position and a
//!   pitch (playback rate, so one rendered brick note plays a whole scale);
//! - **stingers**: effects that also *duck* the music while they play;
//! - **music**: one looping track at a time, crossfaded on change.
//!
//! It is pure arithmetic over the cue bank, unit-tested without a device;
//! `output.rs` only feeds it buffers.

use std::sync::Arc;

use super::cues::{Bank, Cue, CUE_SAMPLE_RATE};

/// What the sound director asks the mixer to do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
    /// One-shot sound. `pan` -1 (left) to 1 (right); `pitch` a playback
    /// rate (1 = as rendered); `duck` > 0 lowers the music for that many
    /// seconds (stingers).
    Play {
        cue: Cue,
        gain: f32,
        pan: f32,
        pitch: f32,
        duck: f32,
    },
    /// Switch the music to this loop, or to silence, crossfading.
    Music(Option<Cue>),
    /// The music's level from the game's state: 1 in play, low in pause.
    MusicLevel(f32),
    /// The player's sound setting: effects on/off, music on/off.
    Enable { effects: bool, music: bool },
}

/// Voices that can sound at once; the oldest is stolen past this.
const MAX_VOICES: usize = 32;
/// The music's base level under the effects.
const MUSIC_GAIN: f32 = 0.36;
/// The music's level while a stinger plays.
const DUCK_LEVEL: f32 = 0.25;
/// Crossfade between two music loops, and the music's fade in/out.
const CROSSFADE_SECS: f32 = 1.2;
/// How fast ducking releases once the stinger is done.
const DUCK_RELEASE_SECS: f32 = 0.5;
/// Overall output level, under the soft limiter.
const MASTER: f32 = 1.5;

struct Voice {
    data: Arc<[f32]>,
    pos: f64,
    step: f64,
    left: f32,
    right: f32,
}

struct Loop {
    cue: Cue,
    data: Arc<[f32]>,
    pos: f64,
    /// 0..1 crossfade level, moving toward `target`.
    level: f32,
    target: f32,
}

pub struct Mixer {
    bank: Bank,
    /// Cue samples per output frame at pitch 1.
    rate_ratio: f64,
    output_rate: f32,
    voices: Vec<Voice>,
    /// The playing loop first; at most one fading one after it.
    music: Vec<Loop>,
    music_level: f32,
    music_level_now: f32,
    duck_secs: f32,
    duck_now: f32,
    effects_on: bool,
    music_on: bool,
}

/// Equal-power stereo gains for a pan in -1..1: a sound keeps its
/// loudness as it moves across the field.
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let angle = (pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    (angle.cos(), angle.sin())
}

/// Linear interpolation into `data` at fractional position `pos`.
fn sample_at(data: &[f32], pos: f64) -> f32 {
    let i = pos as usize;
    let frac = (pos - i as f64) as f32;
    let a = data.get(i).copied().unwrap_or(0.0);
    let b = data.get(i + 1).copied().unwrap_or(0.0);
    a + (b - a) * frac
}

impl Mixer {
    pub fn new(bank: Bank, output_rate: u32) -> Self {
        Self {
            bank,
            rate_ratio: f64::from(CUE_SAMPLE_RATE) / f64::from(output_rate.max(1)),
            output_rate: output_rate.max(1) as f32,
            voices: Vec::with_capacity(MAX_VOICES),
            music: Vec::with_capacity(2),
            music_level: 1.0,
            music_level_now: 1.0,
            duck_secs: 0.0,
            duck_now: 1.0,
            effects_on: true,
            music_on: true,
        }
    }

    pub fn apply(&mut self, command: Command) {
        match command {
            Command::Play {
                cue,
                gain,
                pan,
                pitch,
                duck,
            } => {
                if !self.effects_on {
                    return;
                }
                if duck > 0.0 {
                    self.duck_secs = self.duck_secs.max(duck);
                }
                if self.voices.len() >= MAX_VOICES {
                    self.voices.remove(0);
                }
                let (l, r) = pan_gains(pan);
                self.voices.push(Voice {
                    data: self.bank.get(cue),
                    pos: 0.0,
                    step: self.rate_ratio * f64::from(pitch.max(0.05)),
                    left: gain * l,
                    right: gain * r,
                });
            }
            Command::Music(next) => {
                let current = self.music.first().filter(|l| l.target > 0.0).map(|l| l.cue);
                if current == next {
                    return;
                }
                for playing in &mut self.music {
                    playing.target = 0.0;
                }
                self.music.retain(|l| l.level > 0.0);
                if self.music.len() > 1 {
                    // Never more than one loop fading out: drop the quietest.
                    self.music.truncate(1);
                }
                if let Some(cue) = next {
                    self.music.insert(
                        0,
                        Loop {
                            cue,
                            data: self.bank.get(cue),
                            pos: 0.0,
                            level: 0.0,
                            target: 1.0,
                        },
                    );
                }
            }
            Command::MusicLevel(level) => self.music_level = level.clamp(0.0, 1.0),
            Command::Enable { effects, music } => {
                self.effects_on = effects;
                self.music_on = music;
                if !effects {
                    self.voices.clear();
                }
            }
        }
    }

    /// Fills `out` (interleaved, `channels` per frame) with the mix.
    pub fn render(&mut self, out: &mut [f32], channels: usize) {
        let channels = channels.max(1);
        let dt = 1.0 / self.output_rate;
        let fade_step = dt / CROSSFADE_SECS;
        for frame in out.chunks_mut(channels) {
            let (mut left, mut right) = (0.0f32, 0.0f32);

            for voice in &mut self.voices {
                let s = sample_at(&voice.data, voice.pos);
                left += s * voice.left;
                right += s * voice.right;
                voice.pos += voice.step;
            }

            // Music: level eases toward its target (pause, mode), the duck
            // holds while a stinger plays and releases after.
            let music_target = if self.music_on { self.music_level } else { 0.0 };
            self.music_level_now += (music_target - self.music_level_now) * (dt * 6.0).min(1.0);
            if self.duck_secs > 0.0 {
                self.duck_secs -= dt;
                self.duck_now += (DUCK_LEVEL - self.duck_now) * (dt * 30.0).min(1.0);
            } else {
                self.duck_now += (1.0 - self.duck_now) * (dt / DUCK_RELEASE_SECS * 4.0).min(1.0);
            }
            let music_gain = MUSIC_GAIN * self.music_level_now * self.duck_now;
            for track in &mut self.music {
                track.level = if track.level < track.target {
                    (track.level + fade_step).min(track.target)
                } else {
                    (track.level - fade_step).max(track.target)
                };
                if track.data.is_empty() {
                    continue;
                }
                let s = sample_at(&track.data, track.pos) * track.level * music_gain;
                left += s;
                right += s;
                track.pos += self.rate_ratio;
                if track.pos >= track.data.len() as f64 {
                    track.pos -= track.data.len() as f64;
                }
            }

            // Soft limit: transparent at normal levels, never a hard clip
            // when a dozen bricks break on the same frame.
            let (l, r) = ((left * MASTER).tanh(), (right * MASTER).tanh());
            match frame {
                [mono] => *mono = (l + r) * 0.5,
                [a, b, rest @ ..] => {
                    *a = l;
                    *b = r;
                    rest.fill(0.0);
                }
                [] => {}
            }
        }
        self.voices.retain(|v| v.pos < v.data.len() as f64);
        self.music.retain(|t| t.target > 0.0 || t.level > 0.0);
    }

    /// Voices currently sounding (tests and diagnostics).
    #[cfg(test)]
    pub fn voice_count(&self) -> usize {
        self.voices.len()
    }

    /// The loop the music is heading to.
    #[cfg(test)]
    pub fn music_track(&self) -> Option<Cue> {
        self.music.first().filter(|l| l.target > 0.0).map(|l| l.cue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play(cue: Cue) -> Command {
        Command::Play {
            cue,
            gain: 1.0,
            pan: 0.0,
            pitch: 1.0,
            duck: 0.0,
        }
    }

    #[test]
    fn equal_power_pan_keeps_loudness() {
        for pan in [-1.0, -0.5, 0.0, 0.3, 1.0] {
            let (l, r) = pan_gains(pan);
            assert!((l * l + r * r - 1.0).abs() < 1e-5, "pan {pan}");
        }
        let (l, r) = pan_gains(-1.0);
        assert!(l > 0.99 && r < 0.01, "hard left");
    }

    #[test]
    fn interpolation_reads_between_samples() {
        let data = [0.0, 1.0];
        assert!((sample_at(&data, 0.5) - 0.5).abs() < 1e-6);
        assert_eq!(sample_at(&data, 5.0), 0.0, "past the end is silence");
    }

    #[cfg(feature = "audio")]
    #[test]
    fn an_effect_plays_through_and_frees_its_voice() {
        let mut mixer = Mixer::new(Bank::load(), 44_100);
        mixer.apply(play(Cue::Brick));
        assert_eq!(mixer.voice_count(), 1);
        let mut out = vec![0.0f32; 2 * 4_096];
        mixer.render(&mut out, 2);
        assert!(out.iter().any(|s| s.abs() > 0.05), "audible");
        assert!(out.iter().all(|s| s.abs() <= 1.0), "limited");
        for _ in 0..40 {
            mixer.render(&mut out, 2);
        }
        assert_eq!(mixer.voice_count(), 0, "a finished voice is freed");
    }

    #[cfg(feature = "audio")]
    #[test]
    fn a_doubled_pitch_finishes_in_half_the_time() {
        let frames_until_silent = |pitch: f32| {
            let mut mixer = Mixer::new(Bank::load(), 22_050);
            mixer.apply(Command::Play {
                cue: Cue::Brick,
                gain: 1.0,
                pan: 0.0,
                pitch,
                duck: 0.0,
            });
            let mut frames = 0;
            let mut out = [0.0f32; 2 * 64];
            while mixer.voice_count() > 0 {
                mixer.render(&mut out, 2);
                frames += 64;
            }
            frames
        };
        let normal = frames_until_silent(1.0) as f32;
        let double = frames_until_silent(2.0) as f32;
        assert!((double / normal - 0.5).abs() < 0.05, "{double} vs {normal}");
    }

    #[cfg(feature = "audio")]
    #[test]
    fn voices_are_capped_by_stealing_the_oldest() {
        let mut mixer = Mixer::new(Bank::load(), 44_100);
        for _ in 0..(MAX_VOICES + 10) {
            mixer.apply(play(Cue::Explosion));
        }
        assert_eq!(mixer.voice_count(), MAX_VOICES);
    }

    #[cfg(feature = "audio")]
    #[test]
    fn music_switches_by_crossfade_and_a_repeat_request_is_ignored() {
        let mut mixer = Mixer::new(Bank::load(), 22_050);
        mixer.apply(Command::Music(Some(Cue::MusicAct1)));
        mixer.apply(Command::Music(Some(Cue::MusicAct1)));
        assert_eq!(mixer.music.len(), 1, "same track: no restart");
        let mut out = vec![0.0f32; 2 * 22_050];
        mixer.render(&mut out, 2);
        mixer.apply(Command::Music(Some(Cue::MusicAct2)));
        assert_eq!(mixer.music_track(), Some(Cue::MusicAct2));
        assert_eq!(mixer.music.len(), 2, "the old loop fades out");
        mixer.render(&mut out, 2);
        mixer.render(&mut out, 2);
        assert_eq!(mixer.music.len(), 1, "and is gone after the crossfade");
        mixer.apply(Command::Music(None));
        assert_eq!(mixer.music_track(), None);
    }

    #[cfg(feature = "audio")]
    #[test]
    fn effects_off_silences_effects_and_keeps_music() {
        let mut mixer = Mixer::new(Bank::load(), 22_050);
        mixer.apply(Command::Enable {
            effects: false,
            music: true,
        });
        mixer.apply(play(Cue::Brick));
        assert_eq!(mixer.voice_count(), 0);
        mixer.apply(Command::Music(Some(Cue::MusicMenu)));
        let mut out = vec![0.0f32; 2 * 22_050];
        mixer.render(&mut out, 2);
        assert!(out.iter().any(|s| s.abs() > 0.01), "music still plays");
    }
}
