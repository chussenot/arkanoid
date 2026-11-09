// A cheap, stateless hash: procedural variation without a texture.
fn hash11(x: f32) -> f32 {
    return fract(sin(x * 127.1) * 43758.5453);
}
