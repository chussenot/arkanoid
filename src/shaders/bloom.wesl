// Selective bloom, as a dual-filter chain: a bright-pass downsample to
// quarter size, two more downsamples, then upsamples added back on the
// way up (`fx/gpu.rs` runs the passes). Only light above 1.0 enters it.
import package::fx::fullscreen::{fullscreen, FsOut};
import package::fx::color::{bright_pass, karis_weight};
import package::fx::params::Post;

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var linear_clamp: sampler;
@group(0) @binding(2) var<uniform> post: Post;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> FsOut {
    return fullscreen(index);
}

fn texel() -> vec2f {
    return 1.0 / vec2f(textureDimensions(source));
}

fn tap(uv: vec2f) -> vec3f {
    return textureSampleLevel(source, linear_clamp, uv, 0.0).rgb;
}

// Full size to quarter: four bilinear taps one texel off center, each
// averaging a 2x2 block, so together they cover the 4x4 block one output
// texel stands for. Each is bright-passed and Karis-weighted. Outside the
// field nothing blooms.
@fragment
fn fs_prefilter(in: FsOut) -> @location(0) vec4f {
    let inside = all(in.uv >= post.field.xy) && all(in.uv <= post.field.zw);
    if (!inside) {
        return vec4f(0.0);
    }
    let t = texel();
    var sum = vec3f(0.0);
    var weight = 0.0;
    for (var i = 0; i < 4; i++) {
        let o = vec2f(f32(i & 1) * 2.0 - 1.0, f32((i >> 1) & 1) * 2.0 - 1.0) * t;
        let c = bright_pass(tap(in.uv + o), post.bloom.x, post.bloom.y);
        let w = karis_weight(c);
        sum += c * w;
        weight += w;
    }
    return vec4f(sum / max(weight, 1e-4), 1.0);
}

// Quarter to eighth to sixteenth: the dual filter's downsample.
@fragment
fn fs_down(in: FsOut) -> @location(0) vec4f {
    let t = texel() * 0.5;
    var c = tap(in.uv) * 4.0;
    c += tap(in.uv + vec2f(-t.x, -t.y));
    c += tap(in.uv + vec2f(t.x, -t.y));
    c += tap(in.uv + vec2f(-t.x, t.y));
    c += tap(in.uv + vec2f(t.x, t.y));
    return vec4f(c / 8.0, 1.0);
}

// Back up one size: the dual filter's tent upsample, added onto the
// target's own downsample by the pipeline's additive blend.
@fragment
fn fs_up(in: FsOut) -> @location(0) vec4f {
    let t = texel() * 0.5;
    var c = tap(in.uv + vec2f(-2.0 * t.x, 0.0));
    c += tap(in.uv + vec2f(2.0 * t.x, 0.0));
    c += tap(in.uv + vec2f(0.0, -2.0 * t.y));
    c += tap(in.uv + vec2f(0.0, 2.0 * t.y));
    c += tap(in.uv + vec2f(-t.x, -t.y)) * 2.0;
    c += tap(in.uv + vec2f(t.x, -t.y)) * 2.0;
    c += tap(in.uv + vec2f(-t.x, t.y)) * 2.0;
    c += tap(in.uv + vec2f(t.x, t.y)) * 2.0;
    return vec4f(c / 12.0, 1.0);
}
