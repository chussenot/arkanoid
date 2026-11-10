//! Checks on BRYX's studio (`audio/engine/studio*.c`,
//! docs/design/orchestral-score.md), made on what it actually renders, as
//! `probes.rs` does for the ABC engine: each `audio/probes/studio_*.score`
//! exercises one property, `build.rs` renders it to float stereo at 48 kHz,
//! and each test here measures that property. These check what can be
//! measured (pitch, layers, loops, decay, seating, ceiling, seams,
//! sidechain); whether the result is music is for ears, not tests.

mod rendered {
    include!(concat!(env!("OUT_DIR"), "/studio_probes.rs"));
}

const RATE: f64 = 48_000.0;

/// A probe's frames, `[left, right]`.
fn probe(name: &str) -> Vec<[f64; 2]> {
    let (_, bytes) = rendered::STUDIO_PROBES
        .iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("no studio probe {name}"));
    bytes
        .as_chunks::<8>()
        .0
        .iter()
        .map(|b| {
            let l = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            let r = f32::from_le_bytes([b[4], b[5], b[6], b[7]]);
            [f64::from(l), f64::from(r)]
        })
        .collect()
}

/// Seconds `from..to` of a probe, as one channel (`Some(0)` left,
/// `Some(1)` right) or the mid (`None`).
fn span(x: &[[f64; 2]], from: f64, to: f64, channel: Option<usize>) -> Vec<f64> {
    let a = (from * RATE) as usize;
    let b = ((to * RATE) as usize).min(x.len());
    x[a..b]
        .iter()
        .map(|f| channel.map_or((f[0] + f[1]) * 0.5, |c| f[c]))
        .collect()
}

fn rms(x: &[f64]) -> f64 {
    (x.iter().map(|v| v * v).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

fn db(v: f64) -> f64 {
    20.0 * v.max(1e-12).log10()
}

/// How much of a signal is in its fastest changes: rises as it brightens.
fn brightness(x: &[f64]) -> f64 {
    let diff: Vec<f64> = x.windows(2).map(|w| w[1] - w[0]).collect();
    rms(&diff) / rms(x)
}

/// The fundamental of `x` in Hz, 50..2000 Hz: the strongest
/// autocorrelation lag, refined by a parabola through its neighbours.
fn pitch(x: &[f64]) -> f64 {
    let (lo, hi) = ((RATE / 2_000.0) as usize, (RATE / 50.0) as usize);
    let ac = |lag: usize| -> f64 { x.iter().zip(&x[lag..]).map(|(a, b)| a * b).sum() };
    let best = (lo..hi).max_by(|&a, &b| ac(a).total_cmp(&ac(b))).unwrap();
    let (a, b, c) = (ac(best - 1), ac(best), ac(best + 1));
    let shift = 0.5 * (a - c) / (a - 2.0 * b + c);
    RATE / (best as f64 + shift)
}

fn cents(hz: f64, reference: f64) -> f64 {
    1_200.0 * (hz / reference).log2()
}

/// A band-pass (RBJ, constant peak gain) at `hz`.
fn band(x: &[f64], hz: f64, q: f64) -> Vec<f64> {
    let w = std::f64::consts::TAU * hz / RATE;
    let alpha = w.sin() / (2.0 * q);
    let (a0, a1, a2) = (1.0 + alpha, -2.0 * w.cos(), 1.0 - alpha);
    let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
    x.iter()
        .map(|&v| {
            let y = (alpha * v - alpha * x2 - a1 * y1 - a2 * y2) / a0;
            (x2, x1, y2, y1) = (x1, v, y1, y);
            y
        })
        .collect()
}

/// RT60 from the Schroeder integral's fall from -5 to -35 dB (T30).
fn rt60(x: &[f64]) -> f64 {
    let mut energy: Vec<f64> = x.iter().map(|v| v * v).collect();
    for i in (0..energy.len() - 1).rev() {
        energy[i] += energy[i + 1];
    }
    let level = |i: usize| 10.0 * (energy[i] / energy[0]).log10();
    let a = (0..energy.len()).find(|&i| level(i) < -5.0).unwrap();
    let b = (a..energy.len()).find(|&i| level(i) < -35.0).unwrap();
    (b - a) as f64 / RATE * 2.0
}

#[test]
fn notes_sound_at_their_pitch() {
    let x = probe("studio_tuning");
    // A sampled violin section: its zone, keycenter and measured `tune`.
    // Players breathe, so ±15 cents; a synth sine is exact.
    let violin = cents(pitch(&span(&x, 0.5, 1.0, None)), 440.0);
    assert!(violin.abs() < 15.0, "violin A4 is {violin:+.1} cents off");
    let sine = cents(pitch(&span(&x, 2.5, 3.0, None)), 440.0);
    assert!(sine.abs() < 1.0, "sine A4 is {sine:+.2} cents off");
}

#[test]
fn velocity_picks_a_louder_and_brighter_layer() {
    let x = probe("studio_layers");
    let soft = span(&x, 0.3, 1.6, None);
    let loud = span(&x, 2.3, 3.6, None);
    let louder = db(rms(&loud)) - db(rms(&soft));
    assert!(louder > 12.0, "f over p: {louder:.1} dB");
    assert!(
        brightness(&loud) > brightness(&soft) * 1.05,
        "the f layer is the brighter recording: {:.3} vs {:.3}",
        brightness(&loud),
        brightness(&soft)
    );
}

#[test]
fn a_held_note_outlasts_its_sample_without_a_gap() {
    let x = probe("studio_loop");
    let held = span(&x, 1.0, 9.5, None);
    let windows: Vec<f64> = held.chunks(4_800).map(|w| db(rms(w))).collect();
    let mut sorted = windows.clone();
    sorted.sort_by(f64::total_cmp);
    let median = sorted[sorted.len() / 2];
    for (i, level) in windows.iter().enumerate() {
        assert!(
            (level - median).abs() < 3.0,
            "at {:.1} s the note is {:+.1} dB off its level",
            1.0 + i as f64 * 0.1,
            level - median
        );
    }
}

#[test]
fn the_hall_decays_as_its_preset_says() {
    let x = probe("studio_reverb");
    let tail = span(&x, 0.05, 6.0, None);
    // The hall preset is 2.4 s; damping shortens the top, so measure the
    // body of the band and check the top is the shorter.
    let mid = rt60(&band(&tail, 500.0, 1.41));
    assert!((mid / 2.4 - 1.0).abs() < 0.15, "RT60 at 500 Hz: {mid:.2} s");
    let top = rt60(&band(&tail, 4_000.0, 1.41));
    assert!(top < mid, "highs die first: {top:.2} s at 4 kHz");
}

#[test]
fn the_orchestra_is_seated() {
    let x = probe("studio_seating");
    let side =
        |from, to| db(rms(&span(&x, from, to, Some(0)))) - db(rms(&span(&x, from, to, Some(1))));
    let violins = side(0.3, 1.4);
    let celli = side(2.3, 3.4);
    assert!(violins > 2.0, "violins sit left: L-R {violins:+.1} dB");
    assert!(celli < -2.0, "celli sit right: L-R {celli:+.1} dB");
}

#[test]
fn the_limiter_holds_its_ceiling() {
    let x = probe("studio_limiter");
    let peak = x
        .iter()
        .flat_map(|f| f.iter())
        .fold(0.0f64, |m, v| m.max(v.abs()));
    assert!(
        db(peak) <= -0.99,
        "peak {:.3} dBFS over a -1 dB ceiling",
        db(peak)
    );
    let level = db(rms(&span(&x, 0.5, 3.5, None)));
    assert!(
        level > -16.0,
        "and the tutti is loud, not squashed away: {level:.1} dB"
    );
}

#[test]
fn a_loop_starts_inside_its_ends_tail_without_a_seam() {
    let x = probe("studio_seam");
    // Nothing plays in the first second: what sounds is the end's reverb.
    let start = db(rms(&span(&x, 0.0, 0.25, None)));
    assert!(start > -30.0, "the tail crosses the seam: {start:.1} dB");
    for c in 0..2 {
        let s: Vec<f64> = x.iter().map(|f| f[c]).collect();
        let n = s.len();
        // The curvature across the wrap (last two frames into the first)
        // against the most anywhere inside: a seam would be a spike.
        let at_seam = (s[0] - 2.0 * s[n - 1] + s[n - 2]).abs();
        let inside = s
            .windows(3)
            .map(|w| (w[2] - 2.0 * w[1] + w[0]).abs())
            .fold(0.0, f64::max);
        assert!(
            at_seam < inside * 0.5,
            "channel {c}: {at_seam:.5} at the seam, {inside:.5} at most inside"
        );
    }
}

#[test]
fn a_keyed_compressor_ducks_the_pad_on_each_kick() {
    let x = probe("studio_sidechain");
    // The kick is low; differencing keeps the pad's saw and drops it.
    let pad = |from, to| {
        let s = span(&x, from, to, None);
        let d: Vec<f64> = s.windows(2).map(|w| w[1] - w[0]).collect();
        db(rms(&d))
    };
    for kick in [1.0, 2.0, 3.0] {
        let before = pad(kick - 0.15, kick - 0.01);
        let under = pad(kick + 0.02, kick + 0.08);
        let after = pad(kick + 0.8, kick + 0.98);
        assert!(
            before - under > 6.0,
            "kick at {kick} s ducks: {before:.1} -> {under:.1} dB"
        );
        assert!(
            (after - before).abs() < 3.0,
            "and the pad comes back: {after:.1} dB"
        );
    }
}
