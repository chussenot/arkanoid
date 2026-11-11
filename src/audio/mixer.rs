//! BRYX's mixer: runs inside the audio callback, so it allocates nothing
//! per sample and blocks on nothing. It plays three kinds of sound:
//! - **effects**: one-shot voices with a gain, a stereo position and a
//!   pitch (playback rate, so one rendered brick note plays a whole scale);
//! - **stingers**: effects that also *duck* the music while they play;
//! - **music**: one looping track at a time, crossfaded (equal power) on
//!   change. A new track waits for the sounds that hold the music
//!   (`Cue::holds_music`): it starts when the fanfare, the serve or the warp
//!   has finished, never under it. A track is QOA stereo, decoded a slice
//!   at a time as it plays (`qoa.rs`) and resampled to the device's rate
//!   with a cubic.
//!
//! It is pure arithmetic over the embedded cues, unit-tested without a device;
//! `output.rs` only feeds it buffers.

use super::cues::{pcm, sample, studio, Cue, CUE_SAMPLE_RATE};
use super::qoa::QoaStream;

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
    /// Fade out, in a moment, the sounds that hold the music: a stinger the
    /// player has already moved past (Continue during Game Over).
    CutHeld,
}

/// Voices that can sound at once; the oldest is stolen past this.
const MAX_VOICES: usize = 32;
/// The music's base level under the effects.
const MUSIC_GAIN: f32 = 0.52;
/// The music's level while a stinger plays.
const DUCK_LEVEL: f32 = 0.25;
/// Crossfade between two music loops, and the music's fade in/out.
const CROSSFADE_SECS: f32 = 1.2;
/// How fast ducking releases once the stinger is done.
const DUCK_RELEASE_SECS: f32 = 0.5;
/// Overall output level, under the soft limiter.
const MASTER: f32 = 1.5;
/// How quickly `CutHeld` silences a held sound: fast, but not a click.
const CUT_SECS: f32 = 0.08;

struct Voice {
    source: Source,
    left: f32,
    right: f32,
    /// New music waits while this voice sounds (`Cue::holds_music`).
    holds_music: bool,
    /// 1 while playing; falls to 0 at `fade_step` a frame once cut.
    fade: f32,
    fade_step: f32,
}

struct Loop {
    cue: Cue,
    /// The track's stream; `None` (silence) for a cue with no render, as
    /// in a build without the `audio` feature.
    stream: Option<StudioLoop>,
    /// 0..1 crossfade position, moving toward `target`; the gain is its
    /// equal-power curve (`crossfade_gain`).
    level: f32,
    target: f32,
}

/// What an effect voice plays.
// A studio stream is a few hundred bytes (its decoder's state) against a
// slice and a position; boxing it would allocate in the audio callback,
// where the mixer must not, so a voice carries it inline.
#[allow(clippy::large_enum_variant)]
enum Source {
    /// An ABC or studio effect: mu-law mono, read in place at `step`
    /// cue samples per output frame (its pitch).
    Mulaw {
        data: &'static [u8],
        pos: f64,
        step: f64,
    },
    /// A studio stinger: QOA stereo, played once.
    Studio(StudioLoop),
}

impl Source {
    fn finished(&self) -> bool {
        match self {
            Source::Mulaw { data, pos, .. } => *pos >= data.len() as f64,
            Source::Studio(stream) => stream.finished(),
        }
    }
}

/// A studio loop: the QOA stream decoded as it plays and resampled with a
/// Catmull-Rom cubic from its rate (48 kHz) to the device's. No allocation:
/// the decoder reads the embedded bytes, and the cubic keeps four frames.
pub struct StudioLoop {
    qoa: QoaStream<'static>,
    /// Four source frames; the output lies between `hist[1]` and `hist[2]`.
    hist: [[f32; 2]; 4],
    /// Position between `hist[1]` and `hist[2]`, 0..1.
    frac: f64,
    /// Source frames per output frame.
    step: f64,
    /// A loop wraps to its start; a one-shot stops at its end.
    looping: bool,
    /// A one-shot has run out of frames.
    done: bool,
}

impl StudioLoop {
    /// `None` for bytes that are not a QOA stream (`build.rs` only embeds
    /// what the studio wrote, so in practice: never).
    pub fn new(bytes: &'static [u8], output_rate: f32, looping: bool) -> Option<Self> {
        let qoa = QoaStream::new(bytes).ok()?;
        let step = f64::from(qoa.sample_rate()) / f64::from(output_rate.max(1.0));
        let mut stream = Self {
            qoa,
            hist: [[0.0; 2]; 4],
            frac: 0.0,
            step,
            looping,
            done: false,
        };
        // Prime so the first output is the first frame: [0, f0, f1, f2].
        for i in 1..4 {
            stream.hist[i] = stream.pull();
        }
        Some(stream)
    }

    /// The next source frame. A loop wraps to its start at the end (the
    /// studio renders it with its tails already wrapped, `loop on`, so the
    /// seam is part of the music); a one-shot is silent past it.
    fn pull(&mut self) -> [f32; 2] {
        let frame = self.qoa.next_frame().or_else(|| {
            if !self.looping {
                self.done = true;
                return None;
            }
            self.qoa.rewind();
            self.qoa.next_frame()
        });
        frame.map_or([0.0; 2], |[l, r]| {
            [f32::from(l) / 32_768.0, f32::from(r) / 32_768.0]
        })
    }

    /// A one-shot that has played its last frame.
    pub fn finished(&self) -> bool {
        self.done
    }

    pub fn next(&mut self) -> (f32, f32) {
        let t = self.frac as f32;
        let [p0, p1, p2, p3] = self.hist;
        let out = (
            catmull_rom(p0[0], p1[0], p2[0], p3[0], t),
            catmull_rom(p0[1], p1[1], p2[1], p3[1], t),
        );
        self.frac += self.step;
        while self.frac >= 1.0 {
            self.frac -= 1.0;
            self.hist.copy_within(1..4, 0);
            self.hist[3] = self.pull();
        }
        out
    }
}

/// The Catmull-Rom spline through `p1` (t = 0) and `p2` (t = 1).
#[inline]
fn catmull_rom(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    p1 + 0.5
        * t
        * (p2 - p0 + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3 + t * (3.0 * (p1 - p2) + p3 - p0)))
}

/// The gain at crossfade position `level` (0..1): a quarter sine, so two
/// unrelated tracks crossing keep the loudness (cos² + sin² = 1) where a
/// linear fade dips 3 dB in the middle.
#[inline]
fn crossfade_gain(level: f32) -> f32 {
    (level.clamp(0.0, 1.0) * std::f32::consts::FRAC_PI_2).sin()
}

pub struct Mixer {
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
    /// A music change waiting for the held sounds to finish.
    pending: Option<Option<Cue>>,
}

/// Equal-power stereo gains for a pan in -1..1: a sound keeps its
/// loudness as it moves across the field.
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let angle = (pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    (angle.cos(), angle.sin())
}

/// Linear interpolation into `data` (the embedded mu-law bytes, read in
/// place) at fractional position `pos`. Past the end is silence.
fn sample_at(data: &[u8], pos: f64) -> f32 {
    let i = pos as usize;
    let frac = (pos - i as f64) as f32;
    let a = data.get(i).map_or(0.0, |&b| sample(b));
    let b = data.get(i + 1).map_or(0.0, |&b| sample(b));
    a + (b - a) * frac
}

impl Mixer {
    pub fn new(output_rate: u32) -> Self {
        Self {
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
            pending: None,
        }
    }

    /// Whether a sound that holds the music is still sounding.
    fn music_held(&self) -> bool {
        self.voices.iter().any(|v| v.holds_music && v.fade > 0.0)
    }

    /// Crossfade to `next` now.
    fn switch_music(&mut self, next: Option<Cue>) {
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
                    stream: studio(cue)
                        .and_then(|bytes| StudioLoop::new(bytes, self.output_rate, true)),
                    level: 0.0,
                    target: 1.0,
                },
            );
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
                let pitch = f64::from(pitch.max(0.05));
                // A stinger with a studio render plays it, in stereo; the
                // effects (and any cue without one) play mu-law.
                let source = studio(cue)
                    .and_then(|bytes| StudioLoop::new(bytes, self.output_rate, false))
                    .map_or(
                        Source::Mulaw {
                            data: pcm(cue),
                            pos: 0.0,
                            step: self.rate_ratio * pitch,
                        },
                        |mut stream| {
                            stream.step *= pitch;
                            Source::Studio(stream)
                        },
                    );
                self.voices.push(Voice {
                    source,
                    left: gain * l,
                    right: gain * r,
                    holds_music: cue.holds_music(),
                    fade: 1.0,
                    fade_step: 0.0,
                });
            }
            Command::Music(next) => {
                if next.is_some() && self.music_held() {
                    // The old track still fades out now; the new one waits
                    // for the held sounds to finish (`render`).
                    self.switch_music(None);
                    self.pending = Some(next);
                } else {
                    self.pending = None;
                    self.switch_music(next);
                }
            }
            Command::CutHeld => {
                let step = 1.0 / (CUT_SECS * self.output_rate);
                for voice in self.voices.iter_mut().filter(|v| v.holds_music) {
                    voice.fade_step = step;
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
                let (l, r) = match &mut voice.source {
                    Source::Mulaw { data, pos, step } => {
                        let s = sample_at(data, *pos);
                        *pos += *step;
                        (s, s)
                    }
                    Source::Studio(stream) => stream.next(),
                };
                left += l * voice.fade * voice.left;
                right += r * voice.fade * voice.right;
                voice.fade = (voice.fade - voice.fade_step).max(0.0);
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
                let gain = crossfade_gain(track.level) * music_gain;
                let (l, r) = track.stream.as_mut().map_or((0.0, 0.0), StudioLoop::next);
                left += l * gain;
                right += r * gain;
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
        self.voices.retain(|v| !v.source.finished() && v.fade > 0.0);
        self.music.retain(|t| t.target > 0.0 || t.level > 0.0);
        if self.pending.is_some() && !self.music_held() {
            if let Some(next) = self.pending.take() {
                self.switch_music(next);
            }
        }
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
        let data = [0xFFu8, 0x80]; // mu-law silence, then full scale
        assert!(sample(0x80) > 0.97);
        assert!((sample_at(&data, 0.5) - sample(0x80) / 2.0).abs() < 1e-6);
        assert_eq!(sample_at(&data, 5.0), 0.0, "past the end is silence");
    }

    /// 12,000 stereo frames at 48 kHz, made by the reference encoder
    /// (`qoa.rs`'s tests check the decoder against it bit for bit).
    const REF_QOA: &[u8] = include_bytes!("testdata/ref.qoa");

    #[test]
    fn a_studio_loop_at_its_own_rate_plays_the_decoded_frames_and_wraps() {
        let mut stream = StudioLoop::new(REF_QOA, 48_000.0, true).unwrap();
        let mut reference = QoaStream::new(REF_QOA).unwrap();
        let len = reference.len() as usize;
        let decoded: Vec<[i16; 2]> = std::iter::from_fn(|| reference.next_frame()).collect();
        assert_eq!(decoded.len(), len);
        for i in 0..len + 2_500 {
            let [l, r] = decoded[i % len];
            let want = (f32::from(l) / 32_768.0, f32::from(r) / 32_768.0);
            assert_eq!(stream.next(), want, "frame {i}");
        }
        assert!(
            decoded.iter().any(|[l, r]| l != r),
            "the reference is stereo, and both sides come through"
        );
    }

    #[test]
    fn the_cubic_passes_through_its_points_and_follows_a_sine() {
        assert_eq!(catmull_rom(0.3, -0.2, 0.9, 0.1, 0.0), -0.2);
        assert!((catmull_rom(0.3, -0.2, 0.9, 0.1, 1.0) - 0.9).abs() < 1e-6);
        assert!(
            (catmull_rom(1.0, 2.0, 3.0, 4.0, 0.25) - 2.25).abs() < 1e-6,
            "exact on a line"
        );
        // A 1 kHz sine at 48 kHz, read back between its samples, as the
        // resampler does for a 44.1 kHz device: within 0.1% of full scale.
        let f = |n: f32| (n * std::f32::consts::TAU * 1_000.0 / 48_000.0).sin();
        let mut worst = 0.0f32;
        for k in 0..480 {
            let t = (k as f32 * 0.37).fract();
            let n = k as f32;
            let got = catmull_rom(f(n - 1.0), f(n), f(n + 1.0), f(n + 2.0), t);
            worst = worst.max((got - f(n + t)).abs());
        }
        assert!(worst < 1e-3, "{worst}");
    }

    #[test]
    fn a_studio_loop_on_a_44_1_khz_device_runs_at_the_right_speed() {
        // 44,100 output frames are one second: 48,000 source frames, so the
        // 12,000-frame reference has gone round four times. The loop's step
        // carries no drift: its position is a whole number of source frames
        // plus the fraction the rates leave.
        let mut stream = StudioLoop::new(REF_QOA, 44_100.0, true).unwrap();
        assert!((stream.step - 48_000.0 / 44_100.0).abs() < 1e-12);
        for _ in 0..44_100 {
            let (l, r) = stream.next();
            assert!(l.is_finite() && r.is_finite() && l.abs() <= 1.01 && r.abs() <= 1.01);
        }
        assert!(
            stream.frac < 1e-6 || stream.frac > 1.0 - 1e-6,
            "{}",
            stream.frac
        );
    }

    #[test]
    fn the_crossfade_keeps_the_loudness() {
        for i in 0..=20 {
            let x = i as f32 / 20.0;
            let power = crossfade_gain(x).powi(2) + crossfade_gain(1.0 - x).powi(2);
            assert!((power - 1.0).abs() < 1e-5, "at {x}: {power}");
        }
        assert_eq!(crossfade_gain(0.0), 0.0);
        assert!((crossfade_gain(1.0) - 1.0).abs() < 1e-6);
    }

    /// How long a cue plays at pitch 1: its studio render if it has one.
    fn cue_secs(cue: Cue) -> f64 {
        studio(cue).map_or_else(
            || pcm(cue).len() as f64 / f64::from(CUE_SAMPLE_RATE),
            |bytes| {
                let q = QoaStream::new(bytes).unwrap();
                f64::from(q.len()) / f64::from(q.sample_rate())
            },
        )
    }

    #[test]
    fn a_studio_one_shot_plays_once_and_frees_its_voice() {
        let mut mixer = Mixer::new(44_100);
        mixer.voices.push(Voice {
            source: Source::Studio(StudioLoop::new(REF_QOA, 44_100.0, false).unwrap()),
            left: 0.7,
            right: 0.7,
            holds_music: true,
            fade: 1.0,
            fade_step: 0.0,
        });
        // The reference is 12,000 frames at 48 kHz: a quarter second.
        let mut out = vec![0.0f32; 2 * 4_410];
        mixer.render(&mut out, 2);
        assert!(out.iter().any(|s| s.abs() > 0.01), "audible");
        assert!(mixer.music_held(), "and holding the music while it sounds");
        for _ in 0..2 {
            mixer.render(&mut out, 2);
        }
        assert_eq!(mixer.voice_count(), 0, "gone after its last frame");
        assert!(!mixer.music_held());
        mixer.render(&mut out, 2);
        assert!(out.iter().all(|s| *s == 0.0), "and silent after it");
    }

    /// The mixer's contract: nothing on the audio callback's path touches
    /// the heap. A busy stretch of play (music starting, crossfading and
    /// waiting on a stinger, more bricks than there are voices, a cut, the
    /// sound modes) makes not one allocator call.
    #[cfg(feature = "audio")]
    #[test]
    fn the_callback_path_never_touches_the_heap() {
        use crate::audio::heap;
        let mut mixer = Mixer::new(44_100);
        let mut out = vec![0.0f32; 2 * 512];
        let before = heap::calls();
        mixer.apply(Command::Music(Some(Cue::MusicMenu)));
        for block in 0..600 {
            match block {
                40 => mixer.apply(Command::Music(Some(Cue::Level(1)))),
                60 => mixer.apply(Command::Music(Some(Cue::Level(2)))),
                100..=160 => mixer.apply(Command::Play {
                    cue: Cue::Brick,
                    gain: 0.6,
                    pan: -0.3,
                    pitch: 1.5,
                    duck: 0.0,
                }),
                200 => mixer.apply(Command::Play {
                    cue: Cue::LevelClear,
                    gain: 0.7,
                    pan: 0.0,
                    pitch: 1.0,
                    duck: 2.7,
                }),
                210 => mixer.apply(Command::Music(Some(Cue::Level(3)))),
                260 => mixer.apply(Command::CutHeld),
                300 => mixer.apply(Command::MusicLevel(0.3)),
                340 => mixer.apply(Command::Enable {
                    effects: false,
                    music: true,
                }),
                380 => mixer.apply(Command::Enable {
                    effects: true,
                    music: true,
                }),
                420 => mixer.apply(Command::Music(None)),
                _ => {}
            }
            mixer.render(&mut out, 2);
        }
        assert_eq!(
            heap::calls(),
            before,
            "allocator calls on the callback's path"
        );
        assert_eq!(mixer.voice_count(), 0);
    }

    /// A measurement, not a check: what a second of the busiest mix costs.
    /// `cargo test --release -- --ignored callback_cost --nocapture`
    #[cfg(feature = "audio")]
    #[test]
    #[ignore = "a measurement; run it in release with --nocapture"]
    fn callback_cost() {
        let mut mixer = Mixer::new(48_000);
        let mut out = vec![0.0f32; 2 * 480];
        mixer.apply(Command::Music(Some(Cue::Level(29))));
        mixer.render(&mut out, 2);
        let mut times = Vec::with_capacity(1_000);
        let start = std::time::Instant::now();
        let blocks = 1_000; // ten seconds of 10 ms buffers
        for block in 0..blocks {
            if block % 120 == 0 {
                // Two tracks crossfading, most of the time.
                mixer.apply(Command::Music(Some(Cue::Level(
                    25 + (block / 120 % 8) as u8,
                ))));
            }
            // Every voice busy: explosions, the longest effect.
            while mixer.voice_count() < MAX_VOICES {
                mixer.apply(Command::Play {
                    cue: Cue::Explosion,
                    gain: 0.1,
                    pan: 0.0,
                    pitch: 0.5,
                    duck: 0.0,
                });
            }
            let t = std::time::Instant::now();
            mixer.render(&mut out, 2);
            times.push(t.elapsed().as_secs_f64() * 1e6);
        }
        let total = start.elapsed();
        times.sort_by(f64::total_cmp);
        // The work per buffer is fixed, so the median is the cost; the tail
        // is whatever else the machine was doing.
        println!(
            "10 s of audio (2 QOA streams, {MAX_VOICES} voices): {:.2}% of real time; per 10 ms buffer: median {:.0} us, p99 {:.0} us, worst {:.0} us",
            total.as_secs_f64() * 10.0,
            times[times.len() / 2],
            times[times.len() * 99 / 100],
            times[times.len() - 1]
        );
    }

    #[test]
    fn a_studio_track_plays_in_stereo_through_the_mixer() {
        let mut mixer = Mixer::new(44_100);
        mixer.music.push(Loop {
            cue: Cue::MusicMenu,
            stream: Some(StudioLoop::new(REF_QOA, 44_100.0, true).unwrap()),
            level: 1.0,
            target: 1.0,
        });
        let mut out = vec![0.0f32; 2 * 44_100];
        mixer.render(&mut out, 2);
        assert!(out.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
        assert!(out.iter().any(|s| s.abs() > 0.01), "audible");
        assert!(
            out.chunks(2).any(|f| (f[0] - f[1]).abs() > 1e-3),
            "left and right differ"
        );
    }

    #[cfg(feature = "audio")]
    #[test]
    fn an_effect_plays_through_and_frees_its_voice() {
        let mut mixer = Mixer::new(44_100);
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
            let mut mixer = Mixer::new(22_050);
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
        let mut mixer = Mixer::new(44_100);
        for _ in 0..(MAX_VOICES + 10) {
            mixer.apply(play(Cue::Explosion));
        }
        assert_eq!(mixer.voice_count(), MAX_VOICES);
    }

    #[cfg(feature = "audio")]
    #[test]
    fn music_switches_by_crossfade_and_a_repeat_request_is_ignored() {
        let mut mixer = Mixer::new(22_050);
        mixer.apply(Command::Music(Some(Cue::Level(1))));
        mixer.apply(Command::Music(Some(Cue::Level(1))));
        assert_eq!(mixer.music.len(), 1, "same track: no restart");
        let mut out = vec![0.0f32; 2 * 22_050];
        mixer.render(&mut out, 2);
        mixer.apply(Command::Music(Some(Cue::Level(2))));
        assert_eq!(mixer.music_track(), Some(Cue::Level(2)));
        assert_eq!(mixer.music.len(), 2, "the old loop fades out");
        mixer.render(&mut out, 2);
        mixer.render(&mut out, 2);
        assert_eq!(mixer.music.len(), 1, "and is gone after the crossfade");
        mixer.apply(Command::Music(None));
        assert_eq!(mixer.music_track(), None);
    }

    #[cfg(feature = "audio")]
    #[test]
    fn new_music_waits_for_the_fanfare_to_finish() {
        let mut mixer = Mixer::new(22_050);
        mixer.apply(Command::Music(Some(Cue::Level(1))));
        let mut out = vec![0.0f32; 2 * 2_205];
        mixer.render(&mut out, 2);
        mixer.apply(Command::Play {
            cue: Cue::LevelClear,
            gain: 1.0,
            pan: 0.0,
            pitch: 1.0,
            duck: 2.7,
        });
        mixer.apply(Command::Music(Some(Cue::Level(2))));
        assert_eq!(
            mixer.music_track(),
            None,
            "the old track fades, the new one waits"
        );
        let fanfare = cue_secs(Cue::LevelClear);
        let mut played = 0.0;
        while mixer.music_track().is_none() {
            mixer.render(&mut out, 2);
            played += 0.1;
            assert!(played < fanfare + 0.5, "the music never came back");
        }
        assert!(
            played + 0.1 >= fanfare,
            "started {played:.1}s in, under a {fanfare:.1}s fanfare"
        );
        assert_eq!(mixer.music_track(), Some(Cue::Level(2)));
        // And from its start: under 0.2 s in.
        let stream = mixer.music[0].stream.as_ref().unwrap();
        let secs = f64::from(stream.qoa.position()) / f64::from(stream.qoa.sample_rate());
        assert!(secs < 0.2, "{secs:.2}s in");
    }

    #[cfg(feature = "audio")]
    #[test]
    fn cutting_held_sounds_fades_them_out_and_frees_the_music() {
        let mut mixer = Mixer::new(22_050);
        mixer.apply(play(Cue::GameOver));
        mixer.apply(play(Cue::Brick));
        mixer.apply(Command::Music(Some(Cue::Level(1))));
        assert_eq!(mixer.music_track(), None, "held under Game Over");
        mixer.apply(Command::CutHeld);
        let mut out = vec![0.0f32; 2 * 4_410];
        mixer.render(&mut out, 2);
        assert_eq!(
            mixer.voice_count(),
            0,
            "cut in 80 ms; the brick ran out too"
        );
        assert_eq!(mixer.music_track(), Some(Cue::Level(1)));
    }

    #[cfg(feature = "audio")]
    #[test]
    fn effects_off_silences_effects_and_keeps_music() {
        let mut mixer = Mixer::new(22_050);
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
