// The effects' geometry (`fx/geometry.rs`): triangles in field pixels,
// premultiplied linear color whose rgb may exceed 1.0 (emissive light,
// which the bloom picks up). One fragment shader, two projections.
import package::fx::params::{FIELD, FxView};

@group(0) @binding(0) var<uniform> view: FxView;

struct VertexIn {
    @location(0) position: vec2f,
    @location(1) height: f32,
    @location(2) color: vec4f,
}

struct VertexOut {
    @builtin(position) clip: vec4f,
    @location(0) color: vec4f,
}

// 2D: field pixels straight through an orthographic `view_proj`.
@vertex
fn vs_field(v: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip = view.view_proj * vec4f(v.position, 0.0, 1.0);
    out.color = v.color;
    return out;
}

// 3D: the field's x/y become the board's x/z (centered, as
// `render3d::world_xz` maps them) and `height` lifts the shape off it, so
// an effect lies on the surface it happened on: a brick's top, the floor.
@vertex
fn vs_world(v: VertexIn) -> VertexOut {
    let world = vec3f(v.position.x - FIELD.x * 0.5, v.height, v.position.y - FIELD.y * 0.5);
    var out: VertexOut;
    out.clip = view.view_proj * vec4f(world, 1.0);
    out.color = v.color;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4f {
    return in.color;
}
