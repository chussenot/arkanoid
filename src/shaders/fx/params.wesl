// Uniform layouts shared by the effects' shaders and `src/fx/gpu.rs`.
// The Rust side mirrors these field for field (`#[repr(C)]`); a test in
// `fx/gpu.rs` checks the sizes agree.

// The fixed logical playfield (spec: 800x600), in field pixels.
const FIELD: vec2f = vec2f(800.0, 600.0);

// How many shockwaves can distort the scene at once.
const MAX_PULSES: u32 = 8u;

// Effect geometry's projection: field pixels to clip space in 2D, the
// camera's view-projection in 3D.
struct FxView {
    view_proj: mat4x4f,
}

// One shockwave, in the scene texture's UV space.
struct Pulse {
    // x, y: center; z: radius; w: half-width of the distorted band.
    // Radius and width are in UV units of the texture's height.
    ring: vec4f,
    // x: displacement strength; y: light on the wavefront; z, w: unused.
    gain: vec4f,
}

struct Post {
    // The field's rect in UV (min x, min y, max x, max y): distortion and
    // bloom stay inside it, the HUD band never moves.
    field: vec4f,
    // x: bloom threshold; y: its soft knee; z: bloom strength; w: 1 when
    // bloom is on.
    bloom: vec4f,
    // x: active pulses; y: texture aspect (width / height); z, w: unused.
    misc: vec4f,
    pulses: array<Pulse, MAX_PULSES>,
}
