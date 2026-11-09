// The scene to the surface: shockwave displacement, then bloom added,
// then light above 1.0 spilled to white (`fx/gpu.rs`). The HUD is drawn
// after this, on the surface, so no effect ever touches its text.
import package::fx::fullscreen::{fullscreen, FsOut};
import package::fx::color::spill;
import package::fx::distort::shockwave;
import package::fx::params::Post;

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var bloom: texture_2d<f32>;
@group(0) @binding(2) var linear_clamp: sampler;
@group(0) @binding(3) var<uniform> post: Post;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> FsOut {
    return fullscreen(index);
}

@fragment
fn fs_main(in: FsOut) -> @location(0) vec4f {
    let inside = all(in.uv >= post.field.xy) && all(in.uv <= post.field.zw);
    var offset = vec2f(0.0);
    var light = 0.0;
    if (inside) {
        let count = u32(post.misc.x);
        for (var i = 0u; i < count; i++) {
            let wave = shockwave(in.uv, post.pulses[i], post.misc.y);
            offset += wave.xy;
            light += wave.z;
        }
    }
    // Never sample across the field's edge: the band stays put.
    let uv = clamp(in.uv + offset, post.field.xy, post.field.zw);
    let src = select(in.uv, uv, inside);
    var c = textureSampleLevel(scene, linear_clamp, src, 0.0).rgb;
    if (inside && post.bloom.w > 0.5) {
        c += textureSampleLevel(bloom, linear_clamp, src, 0.0).rgb * post.bloom.z;
    }
    c += vec3f(light);
    return vec4f(spill(c), 1.0);
}
