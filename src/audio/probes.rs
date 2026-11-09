//! Checks on the engine fork (`audio/engine/`, see its PROVENANCE.md),
//! made on what it actually renders: each probe in `audio/probes/` is one
//! feature, rendered by `build.rs`, and each test here measures the
//! property that feature exists for. A change to the engine that breaks a
//! feature fails here, in CI, without anyone having to listen.

mod rendered {
    include!(concat!(env!("OUT_DIR"), "/probes.rs"));
}

const RATE: f64 = 22_050.0;
/// One sixteenth-note step at the probes' 120 BPM.
const STEP: f64 = RATE * 0.125;

fn probe(name: &str) -> Vec<f64> {
    let (_, bytes) = rendered::PROBES
        .iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("no probe {name}"));
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&b| f64::from(i16::from_le_bytes(b)))
        .collect()
}

/// Steps `from..to` of a probe.
fn steps(x: &[f64], from: f64, to: f64) -> &[f64] {
    &x[(from * STEP) as usize..((to * STEP) as usize).min(x.len())]
}

fn rms(x: &[f64]) -> f64 {
    (x.iter().map(|v| v * v).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

fn zero_crossings(x: &[f64]) -> usize {
    x.windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count()
}

/// The amplitude of one frequency in `x` (the Goertzel algorithm).
fn tone(x: &[f64], hz: f64) -> f64 {
    let c = 2.0 * (2.0 * std::f64::consts::PI * hz / RATE).cos();
    let (mut s1, mut s2) = (0.0, 0.0);
    for &v in x {
        let s0 = v + c * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    (s1 * s1 + s2 * s2 - c * s1 * s2).max(0.0).sqrt() / x.len() as f64
}

/// How much of a signal is in its fastest changes: falls as it darkens.
fn brightness(x: &[f64]) -> f64 {
    let diff: Vec<f64> = x.windows(2).map(|w| w[1] - w[0]).collect();
    rms(&diff) / rms(x)
}

#[test]
fn held_notes_finish_their_attack() {
    // A 300 ms attack under a two-second note: struck every step, it never
    // gets past half its level; held, it does.
    let held = rms(steps(&probe("hold"), 8.0, 16.0));
    let struck = rms(steps(&probe("struck"), 8.0, 16.0));
    assert!(held > 3.0 * struck, "held {held:.0}, struck {struck:.0}");
}

#[test]
fn dynamics_scale_the_note() {
    let v = probe("velocity");
    let ratio = rms(steps(&v, 17.0, 23.0)) / rms(steps(&v, 1.0, 7.0));
    // !f! is velocity 104, !p! is 56.
    assert!((ratio - 104.0 / 56.0).abs() < 0.08, "f/p {ratio:.3}");
}

#[test]
fn only_marked_notes_are_accented() {
    let a = probe("accents");
    let level: Vec<f64> = (0..4)
        .map(|i| {
            rms(steps(
                &a,
                4.0 * f64::from(i) + 1.0,
                4.0 * f64::from(i) + 3.0,
            ))
        })
        .collect();
    assert!((level[1] / level[0] - 1.5).abs() < 0.05, "{level:?}");
    assert!(
        (level[2] - level[0]).abs() < 1.0 && (level[3] - level[0]).abs() < 1.0,
        "{level:?}"
    );
}

#[test]
fn a_chord_sounds_its_three_notes() {
    let c = probe("chord");
    let x = steps(&c, 0.0, 16.0);
    let (e, g, b) = (tone(x, 329.63), tone(x, 392.0), tone(x, 493.88));
    let off = tone(x, 293.66).max(tone(x, 440.0)).max(tone(x, 523.25));
    for (name, level) in [("E", e), ("G", g), ("B", b)] {
        assert!(
            level > 50.0 * off.max(1.0),
            "{name} {level:.0}, off-chord {off:.0}"
        );
    }
    assert!(
        e.max(g).max(b) / e.min(g).min(b) < 1.1,
        "E {e:.0} G {g:.0} B {b:.0}"
    );
}

#[test]
fn the_kick_sweeps_down_to_its_note() {
    let k = probe("kick");
    let start = zero_crossings(&k[..441]); // the first 20 ms
    let settled = zero_crossings(&k[2205..2646]); // 100..120 ms, at E2
    assert!(start >= 2 * settled, "start {start}, settled {settled}");
    assert!(
        (3..=5).contains(&settled),
        "E2 crosses about 3.3 times in 20 ms: {settled}"
    );
}

#[test]
fn chip_noise_follows_the_note_and_metal_rings() {
    let n = probe("noise");
    let (low, high, metal) = (
        steps(&n, 1.0, 7.0),
        steps(&n, 17.0, 23.0),
        steps(&n, 33.0, 39.0),
    );
    let rate = |x: &[f64]| zero_crossings(x) as f64 / x.len() as f64;
    assert!(
        rate(high) > 5.0 * rate(low),
        "low {:.3}, high {:.3}",
        rate(low),
        rate(high)
    );
    // The metal LFSR's short loop repeats: the signal matches itself one
    // loop later. The long LFSR does not, within the window.
    let repeats = |x: &[f64]| {
        let x = &x[..6000];
        let energy: f64 = x.iter().map(|v| v * v).sum();
        (20..1500)
            .map(|lag| x.iter().zip(&x[lag..]).map(|(a, b)| a * b).sum::<f64>() / energy)
            .fold(f64::MIN, f64::max)
    };
    assert!(repeats(metal) > 0.8, "metal {:.2}", repeats(metal));
    assert!(repeats(high) < 0.3, "long {:.2}", repeats(high));
}

#[test]
fn a_ramp_closes_the_filter_over_the_note() {
    let r = probe("ramp");
    let (open, closed) = (
        brightness(steps(&r, 0.0, 2.0)),
        brightness(steps(&r, 13.0, 15.0)),
    );
    assert!(closed < 0.75 * open, "open {open:.3}, closed {closed:.3}");
}

#[test]
fn the_keyed_sidechain_ducks_its_bus_on_the_kick() {
    let d = probe("duck");
    let (after, before) = (rms(steps(&d, 8.0, 8.4)), rms(steps(&d, 6.5, 8.0)));
    assert!(
        after < 0.5 * before,
        "after the kick {after:.0}, before {before:.0}"
    );
}

#[test]
fn a_quiet_voice_keeps_its_shape() {
    // 8-bit synthesis drew this triangle with about 20 levels; the 16-bit
    // mix draws it smoothly.
    let q = probe("quiet");
    let mut levels: Vec<i64> = steps(&q, 0.0, 8.0).iter().map(|&v| v as i64).collect();
    levels.sort_unstable();
    levels.dedup();
    assert!(levels.len() > 500, "{} levels", levels.len());
}
