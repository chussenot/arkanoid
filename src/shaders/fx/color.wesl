// Light and color helpers for linear-light, HDR effect color.

// Rec. 709 luminance of a linear color.
fn luminance(c: vec3f) -> f32 {
    return dot(c, vec3f(0.2126, 0.7152, 0.0722));
}

// The bloom's bright-pass: only light above `threshold` passes, eased in
// over `knee` so nothing pops on or off. Palette colors never exceed 1.0,
// so with a threshold of 1.0 only emissive effect light blooms: the
// selection is by construction, not by mask.
fn bright_pass(c: vec3f, threshold: f32, knee: f32) -> vec3f {
    let brightness = max(c.r, max(c.g, c.b));
    var soft = clamp(brightness - threshold + knee, 0.0, 2.0 * knee);
    soft = soft * soft / (4.0 * knee + 1e-4);
    let contribution = max(soft, brightness - threshold) / max(brightness, 1e-4);
    return c * contribution;
}

// Writes HDR light to the 0..1 surface without clipping its hue: below
// 1.0 a color is untouched (the palette stays exact); above, the excess
// spills into the other channels, so a hot amber core turns white-hot
// instead of flattening to saturated orange. Light that stacks saturates
// to white, never past it.
fn spill(c: vec3f) -> vec3f {
    let peak = max(c.r, max(c.g, c.b));
    let excess = max(peak - 1.0, 0.0);
    return min(c + vec3f(excess * 0.6), vec3f(1.0));
}

// Karis weight: one over luma, so a single very bright texel cannot
// dominate a downsample and flicker as it moves.
fn karis_weight(c: vec3f) -> f32 {
    return 1.0 / (1.0 + luminance(c));
}
