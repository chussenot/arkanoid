import package::fx::params::Pulse;
import package::fx::noise::hash11;

// One shockwave at `uv`: the displacement it applies (xy) and the light on
// its wavefront (z).
//
// The displacement is a refracting band, not a smear. Across the band the
// offset follows x * exp(-x^2) (the derivative of a Gaussian), so the
// scene is pushed outward just ahead of the wave and pulled in just behind
// it, like a lens ring, and is untouched at its crest and away from it.
// `aspect` keeps the ring round on a wide target.
//
// Pixels farther than three band widths from the ring are rejected before
// any transcendental: most of the screen pays one length() per pulse.
fn shockwave(uv: vec2f, p: Pulse, aspect: f32) -> vec3f {
    let d = (uv - p.ring.xy) * vec2f(aspect, 1.0);
    let dist = length(d);
    let width = max(p.ring.w, 1e-4);
    let x = (dist - p.ring.z) / width;
    if (abs(x) > 3.0 || dist < 1e-5) {
        return vec3f(0.0);
    }
    // A little irregularity along the ring, so it reads as a pressure
    // front rather than a drawn circle.
    let angle = atan2(d.y, d.x);
    let wobble = 0.85 + 0.3 * hash11(floor(angle * 6.0));
    let bell = exp(-x * x);
    let push = p.gain.x * wobble * x * bell;
    let offset = (d / dist) * push / vec2f(aspect, 1.0);
    // The crest's light: a thinner band than the displacement's.
    let crest = exp(-4.0 * x * x);
    return vec3f(offset, p.gain.y * crest);
}
