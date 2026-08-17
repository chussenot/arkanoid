//! Sibling 3D renderer (arkanoid-v2-c1): a perspective-camera, depth-tested
//! pipeline that draws the same `Game` state as `render.rs`'s classic
//! instanced-quad 2D renderer, but as instanced cubes (bricks/paddle/walls)
//! and a sphere (ball(s)), lit by one Blinn-Phong shader (key light + fill
//! ambient). Selected via `--renderer 3d` (see `cli.rs`); `render.rs` is
//! untouched and stays the default renderer for the rest of this epic.
//!
//! Scope note: HUD text and the menu/pause/game-over/victory overlay are
//! still out of scope here (a later bead's concern). What *is* drawn --
//! paddle, ball(s), bricks, falling power-ups, and three static boundary
//! walls -- is interpolated between ticks exactly like the classic
//! renderer. Brick texturing (arkanoid-v2-c4) and juice (arkanoid-v2-c3:
//! hit-flash, destroy-tumble, ball trail, screen shake, power-up spin --
//! see the "-- juice --" section below) are both now implemented.
//!
//! World space: playfield x/y (pixels, y-down, per `game.rs`) map onto this
//! renderer's ground plane as world X/Z, centered so world (0, 0, 0) is the
//! playfield's center; world Y is "up", used only to extrude otherwise-flat
//! 2D shapes into slabs the perspective camera and Blinn-Phong shader below
//! have something to shade. Nothing about *where* an entity is on the board
//! changes -- it's `render.rs`'s own layout, just given a third dimension.

mod atlas;

use std::collections::VecDeque;
use std::f32::consts::PI;
use std::mem::size_of;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::time::Instant;

use glam::camera::rh::{proj::directx, view};
use glam::{Quat, Vec3};
use wgpu::util::DeviceExt;
use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::events::{GameEvent, PowerUpKind};
use crate::game::{Ball, Brick, Game, Paddle, PLAYFIELD_HEIGHT, PLAYFIELD_WIDTH};
use crate::levels::BrickKind;
use crate::render::RenderState;
use atlas::{Atlas, SpriteId};

// -- entity extrusion / placement constants --------------------------------

/// Vertical (world-Y) thickness given to an otherwise-flat brick so it
/// reads as a slab under the tilted camera.
const BRICK_THICKNESS: f32 = 22.0;
const PADDLE_THICKNESS: f32 = 18.0;
/// Three static boundary walls (left/right/top) frame the board -- the
/// mesh/instance this bead's description asks for beyond bricks/paddle,
/// even though `game.rs` has no wall *entities* of its own (its walls are
/// just the playfield edges `resolve_wall_collisions` bounces off).
const WALL_THICKNESS: f32 = 20.0;
const WALL_HEIGHT: f32 = 46.0;
/// Matches `render.rs`'s `POWERUP_HALF_SIZE` (private to that module, so
/// re-declared here rather than shared cross-module for one constant).
const POWERUP_HALF: f32 = 9.0;

const MAX_BRICKS: usize = 14 * 8;
const MAX_POWERUPS: usize = 16;
const MAX_EXTRA_BALLS: usize = 16;
const WALL_COUNT: usize = 3;
/// Upper bound on simultaneous destroy-tumble ghosts (see `TumbleGhost`).
/// Raised from `render.rs`'s original 8 to this bead's (arkanoid-v2-c5)
/// named stress scenario -- "full 14x8 board + multiball + 20 tumbling
/// corpses" -- since a multiball fleet catching up `MAX_TICKS_PER_FRAME`
/// (10) queued ticks in one frame can plausibly destroy well past 8 bricks
/// before a single render(); this is still headroom for a worst case, not
/// a real gameplay limit.
const MAX_DESTROY_GHOSTS: usize = 20;
/// Fading sphere instances behind the ball (see `push_trail`) -- matches
/// `render.rs`'s own `TRAIL_LEN` (spec: "a 4-quad fading trail").
const TRAIL_LEN: usize = 4;
const MAX_CUBE_INSTANCES: usize =
    1 /* paddle */ + MAX_BRICKS + MAX_POWERUPS + WALL_COUNT + MAX_DESTROY_GHOSTS;
const MAX_SPHERE_INSTANCES: usize = 1 /* main ball */ + MAX_EXTRA_BALLS + TRAIL_LEN;

/// Maps a playfield-space (x right, y down, pixels) position to this
/// renderer's world-space X/Z ground-plane coordinates.
fn world_xz(px: f32, py: f32) -> (f32, f32) {
    (px - PLAYFIELD_WIDTH / 2.0, py - PLAYFIELD_HEIGHT / 2.0)
}

// -- colors -----------------------------------------------------------------
//
// Same palette `render.rs` uses (its constants are private to that module,
// so duplicated here rather than exposed cross-module for one caller each).

const PADDLE_COLOR: [f32; 4] = [0.80, 0.85, 0.95, 1.0];
const BALL_COLOR: [f32; 4] = [1.0, 0.78, 0.2, 1.0];
const NORMAL_BRICK_COLOR: [f32; 4] = [0.85, 0.25, 0.25, 1.0];
const ARMORED_BRICK_COLOR: [f32; 4] = [0.30, 0.45, 0.68, 1.0];
const ARMORED_BRICK_COLOR_HIT: [f32; 4] = [0.90, 0.60, 0.15, 1.0];
const INDESTRUCTIBLE_BRICK_COLOR: [f32; 4] = [0.25, 0.25, 0.28, 1.0];
const WALL_COLOR: [f32; 4] = [0.16, 0.18, 0.24, 1.0];
const POWERUP_WIDEN_COLOR: [f32; 4] = [0.35, 0.85, 0.40, 1.0];
const POWERUP_SLOW_COLOR: [f32; 4] = [0.35, 0.55, 0.95, 1.0];
const POWERUP_MULTIBALL_COLOR: [f32; 4] = [0.80, 0.35, 0.90, 1.0];

/// Shared by `brick_color` (a standing brick) and `TumbleGhost::spawn` (a
/// just-destroyed one, which by the time `BrickDestroyedAt` fires is
/// already gone from `Game::bricks` -- see the module doc comment -- so it
/// has no live `&Brick` to read a color off of, only the event's copied
/// fields).
fn brick_color_for(kind: BrickKind, hits_remaining: u8) -> [f32; 4] {
    match kind {
        BrickKind::Normal => NORMAL_BRICK_COLOR,
        BrickKind::Armored if hits_remaining >= 2 => ARMORED_BRICK_COLOR,
        BrickKind::Armored => ARMORED_BRICK_COLOR_HIT,
        BrickKind::Indestructible => INDESTRUCTIBLE_BRICK_COLOR,
    }
}

fn brick_color(brick: &Brick) -> [f32; 4] {
    brick_color_for(brick.kind, brick.hits_remaining)
}

/// Which atlas sprite a brick's front face samples -- same kind/hits-
/// remaining split `brick_color` uses for its own two armored colors, so
/// the textured front face and the flat side faces always agree on which
/// visual state a brick is in.
fn sprite_for_brick(brick: &Brick) -> SpriteId {
    match brick.kind {
        BrickKind::Normal => SpriteId::BrickNormal,
        BrickKind::Armored if brick.hits_remaining >= 2 => SpriteId::BrickArmoredIntact,
        BrickKind::Armored => SpriteId::BrickArmoredHit,
        BrickKind::Indestructible => SpriteId::BrickIndestructible,
    }
}

fn powerup_color(kind: PowerUpKind) -> [f32; 4] {
    match kind {
        PowerUpKind::Widen => POWERUP_WIDEN_COLOR,
        PowerUpKind::Slow => POWERUP_SLOW_COLOR,
        PowerUpKind::Multiball => POWERUP_MULTIBALL_COLOR,
    }
}

// -- camera -------------------------------------------------------------
//
// Fixed cinematic angle: high above the board, tilted toward the paddle
// side just enough to read as 3D without losing the top-down legibility
// the spec asks for ("the playfield must still read like 2D"). Eye/target
// never move -- only the projection's aspect ratio changes, on resize.
// Values were chosen (and checked by hand against the view/projection
// math) so the full 800x600 board -- including the raised walls -- stays
// within the view frustum with margin to spare; there's no automated check
// for this since it's a property of fixed constants, not runtime logic.

const CAMERA_EYE: Vec3 = Vec3::new(0.0, 900.0, 560.0);
const CAMERA_TARGET: Vec3 = Vec3::new(0.0, 0.0, 0.0);
/// 50 degrees in radians -- `f32::to_radians` isn't `const fn`, so this is
/// spelled out rather than computed.
const FOV_Y_RADIANS: f32 = 0.872_665;
const NEAR: f32 = 10.0;
const FAR: f32 = 2000.0;

/// Builds the combined view*projection matrix for `aspect` (surface
/// width/height), as a flat column-major array ready for the `Camera`
/// uniform buffer -- `glam::Mat4::to_cols_array` already produces that
/// layout, and `rh::proj::directx` (NDC Z in `[0, 1]`, unlike the OpenGL
/// convention's `[-1, 1]`) already targets wgpu's clip-space depth range,
/// which is the whole reason this module reaches for glam instead of
/// hand-rolling the matrix.
/// `eye`/`target` are `CAMERA_EYE`/`CAMERA_TARGET` plus this frame's screen-
/// shake offset (arkanoid-v2-c3, see `next_shake_remaining`) -- both nudged
/// by the same vector so the shake is a pure camera-rig translation, not a
/// look-direction change.
fn camera_view_proj(aspect: f32, eye: Vec3, target: Vec3) -> [f32; 16] {
    let view_mat = view::look_at_mat4(eye, target, Vec3::Y);
    let proj = directx::perspective(FOV_Y_RADIANS, aspect, NEAR, FAR);
    (proj * view_mat).to_cols_array()
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    view_proj: [f32; 16],
    /// xyz used; w is padding so the struct matches WGSL's `vec4<f32>`
    /// alignment for the uniform-buffer member.
    eye_pos: [f32; 4],
}

// -- mesh data ----------------------------------------------------------

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex3D {
    position: [f32; 3],
    normal: [f32; 3],
    /// Local 0..1 quad UV, meaningful only on the +Z (front) face -- the
    /// only face this pipeline ever textures (bricks' front face, see
    /// `Instance3D::textured`). Every other face carries `[0.0, 0.0]`
    /// stub coordinates since nothing ever samples them.
    uv: [f32; 2],
}

/// Identity quaternion (x, y, z, w) -- every instance that doesn't spin or
/// tumble carries this in `Instance3D::rotation`.
const ROTATION_IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// Per-instance transform + color for one cube or sphere: a translation
/// (`center`), axis-aligned `scale`, and now (arkanoid-v2-c3) a `rotation`
/// quaternion -- the "extend this format" upgrade this struct's previous
/// doc comment flagged, needed for power-up spin and the brick
/// destroy-tumble effect (see the module doc comment's "-- juice --"
/// section). `scale` is still applied in object space *before* `rotation`
/// (`center + rotation * (vert.position * scale)` in `SHADER_SRC`'s
/// `vs_main`), so the normal transform stays exact for every shape this
/// pipeline draws: for `M = R * S`, `transpose(inverse(M)) = R *
/// inverse(S)` (S diagonal, R orthogonal) -- i.e. the same "normal /
/// scale" rescale this module always did, just also rotated by the same
/// quaternion as the position, which is the "real normal matrix" upgrade
/// promised there.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Instance3D {
    center: [f32; 3],
    scale: [f32; 3],
    color: [f32; 4],
    /// Atlas UV rect (`u0, v0, u1, v1`) this instance's front face
    /// samples, or `[0.0; 4]` (zero-area -- see `textured_brick_instance`
    /// et al.) for every instance that isn't a textured brick, which the
    /// fragment shader reads as "use flat `color` on every face instead."
    uv_rect: [f32; 4],
    rotation: [f32; 4],
    /// Added straight to the shader's lit color (see `SHADER_SRC`'s
    /// `fs_main`), bypassing the Blinn-Phong lighting term -- an unlit
    /// brightness boost used for the brick hit-flash effect so it reads
    /// clearly regardless of the light's angle on that face. `0.0` for
    /// every instance that isn't mid-flash.
    emissive: f32,
}

impl Instance3D {
    fn new(center: [f32; 3], scale: [f32; 3], color: [f32; 4]) -> Self {
        Self {
            center,
            scale,
            color,
            uv_rect: [0.0; 4],
            rotation: ROTATION_IDENTITY,
            emissive: 0.0,
        }
    }

    /// Same as `new`, but with a nonzero-area atlas UV rect so the
    /// shader's front face samples `uv_rect` instead of `color` (see
    /// `Vertex3D::uv`'s doc comment and `SHADER_SRC`'s `fs_main`).
    fn textured(center: [f32; 3], scale: [f32; 3], color: [f32; 4], uv: atlas::UvRect) -> Self {
        Self {
            uv_rect: [uv.u0, uv.v0, uv.u1, uv.v1],
            ..Self::new(center, scale, color)
        }
    }

    fn with_rotation(mut self, rotation: [f32; 4]) -> Self {
        self.rotation = rotation;
        self
    }

    fn with_emissive(mut self, emissive: f32) -> Self {
        self.emissive = emissive;
        self
    }
}

/// Unit cube (-1..1 each axis), 24 vertices (4 per face) so each face keeps
/// its own flat normal -- the whole point of drawing boxes instead of
/// smooth-shaded shapes here. Hardcoded like `render.rs`'s `QUAD_VERTICES`:
/// small, fixed data with no reason to generate it at runtime.
const CUBE_VERTICES: [Vertex3D; 24] = [
    // +X (untextured -- stub uv, see `Vertex3D::uv`'s doc comment)
    Vertex3D {
        position: [1.0, -1.0, -1.0],
        normal: [1.0, 0.0, 0.0],
        uv: [0.0, 0.0],
    },
    Vertex3D {
        position: [1.0, 1.0, -1.0],
        normal: [1.0, 0.0, 0.0],
        uv: [0.0, 0.0],
    },
    Vertex3D {
        position: [1.0, 1.0, 1.0],
        normal: [1.0, 0.0, 0.0],
        uv: [0.0, 0.0],
    },
    Vertex3D {
        position: [1.0, -1.0, 1.0],
        normal: [1.0, 0.0, 0.0],
        uv: [0.0, 0.0],
    },
    // -X (untextured)
    Vertex3D {
        position: [-1.0, -1.0, 1.0],
        normal: [-1.0, 0.0, 0.0],
        uv: [0.0, 0.0],
    },
    Vertex3D {
        position: [-1.0, 1.0, 1.0],
        normal: [-1.0, 0.0, 0.0],
        uv: [0.0, 0.0],
    },
    Vertex3D {
        position: [-1.0, 1.0, -1.0],
        normal: [-1.0, 0.0, 0.0],
        uv: [0.0, 0.0],
    },
    Vertex3D {
        position: [-1.0, -1.0, -1.0],
        normal: [-1.0, 0.0, 0.0],
        uv: [0.0, 0.0],
    },
    // +Y (untextured)
    Vertex3D {
        position: [-1.0, 1.0, -1.0],
        normal: [0.0, 1.0, 0.0],
        uv: [0.0, 0.0],
    },
    Vertex3D {
        position: [-1.0, 1.0, 1.0],
        normal: [0.0, 1.0, 0.0],
        uv: [0.0, 0.0],
    },
    Vertex3D {
        position: [1.0, 1.0, 1.0],
        normal: [0.0, 1.0, 0.0],
        uv: [0.0, 0.0],
    },
    Vertex3D {
        position: [1.0, 1.0, -1.0],
        normal: [0.0, 1.0, 0.0],
        uv: [0.0, 0.0],
    },
    // -Y (untextured)
    Vertex3D {
        position: [-1.0, -1.0, 1.0],
        normal: [0.0, -1.0, 0.0],
        uv: [0.0, 0.0],
    },
    Vertex3D {
        position: [-1.0, -1.0, -1.0],
        normal: [0.0, -1.0, 0.0],
        uv: [0.0, 0.0],
    },
    Vertex3D {
        position: [1.0, -1.0, -1.0],
        normal: [0.0, -1.0, 0.0],
        uv: [0.0, 0.0],
    },
    Vertex3D {
        position: [1.0, -1.0, 1.0],
        normal: [0.0, -1.0, 0.0],
        uv: [0.0, 0.0],
    },
    // +Z (toward the camera/paddle side -- the only face this pipeline
    // ever textures; see `Instance3D::textured` and `SHADER_SRC`'s
    // `fs_main`). uv is a standard y-down quad mapping: local -Y (bottom)
    // is v=1, local +Y (top) is v=0.
    Vertex3D {
        position: [-1.0, -1.0, 1.0],
        normal: [0.0, 0.0, 1.0],
        uv: [0.0, 1.0],
    },
    Vertex3D {
        position: [1.0, -1.0, 1.0],
        normal: [0.0, 0.0, 1.0],
        uv: [1.0, 1.0],
    },
    Vertex3D {
        position: [1.0, 1.0, 1.0],
        normal: [0.0, 0.0, 1.0],
        uv: [1.0, 0.0],
    },
    Vertex3D {
        position: [-1.0, 1.0, 1.0],
        normal: [0.0, 0.0, 1.0],
        uv: [0.0, 0.0],
    },
    // -Z (far side, away from the camera -- untextured)
    Vertex3D {
        position: [1.0, -1.0, -1.0],
        normal: [0.0, 0.0, -1.0],
        uv: [0.0, 0.0],
    },
    Vertex3D {
        position: [-1.0, -1.0, -1.0],
        normal: [0.0, 0.0, -1.0],
        uv: [0.0, 0.0],
    },
    Vertex3D {
        position: [-1.0, 1.0, -1.0],
        normal: [0.0, 0.0, -1.0],
        uv: [0.0, 0.0],
    },
    Vertex3D {
        position: [1.0, 1.0, -1.0],
        normal: [0.0, 0.0, -1.0],
        uv: [0.0, 0.0],
    },
];

/// Two triangles per face, in the same vertex order `CUBE_VERTICES` lays
/// its four-vertex faces out in.
const CUBE_INDICES: [u16; 36] = [
    0, 1, 2, 0, 2, 3, // +X
    4, 5, 6, 4, 6, 7, // -X
    8, 9, 10, 8, 10, 11, // +Y
    12, 13, 14, 12, 14, 15, // -Y
    16, 17, 18, 16, 18, 19, // +Z
    20, 21, 22, 20, 22, 23, // -Z
];

/// Resolution of the generated UV sphere used for every ball. Coarse on
/// purpose -- balls are small on screen, and this is an untextured flat-
/// palette pipeline (per this bead's acceptance criteria), so extra
/// polygons would only cost fill rate for no visible benefit.
const SPHERE_STACKS: u32 = 8;
const SPHERE_SECTORS: u32 = 12;

/// Builds a unit UV sphere (radius 1, centered at the origin) as an
/// indexed mesh: `stacks` latitude rings from -90 to 90 degrees, `sectors`
/// longitude divisions each. Generated at runtime (unlike the cube) since
/// hand-writing this many vertices would be unreadable; the ball is the
/// only shape that needs it. Standard UV-sphere construction -- see e.g.
/// http://www.songho.ca/opengl/gl_sphere.html for the same algorithm.
fn build_sphere_mesh(stacks: u32, sectors: u32) -> (Vec<Vertex3D>, Vec<u16>) {
    let mut vertices = Vec::with_capacity(((stacks + 1) * (sectors + 1)) as usize);
    for i in 0..=stacks {
        let phi = PI * (i as f32 / stacks as f32) - PI / 2.0;
        let (sin_phi, cos_phi) = phi.sin_cos();
        for j in 0..=sectors {
            let theta = 2.0 * PI * (j as f32 / sectors as f32);
            let (sin_theta, cos_theta) = theta.sin_cos();
            let x = cos_phi * cos_theta;
            let y = sin_phi;
            let z = cos_phi * sin_theta;
            // Unit sphere: the surface normal at a point IS that point.
            // Balls are never textured (see `Vertex3D::uv`'s doc comment
            // -- only bricks' +Z face is), so `uv` is an unused stub here.
            vertices.push(Vertex3D {
                position: [x, y, z],
                normal: [x, y, z],
                uv: [0.0, 0.0],
            });
        }
    }

    let mut indices = Vec::with_capacity((stacks * sectors * 6) as usize);
    for i in 0..stacks {
        for j in 0..sectors {
            let a = i * (sectors + 1) + j;
            let b = a + sectors + 1;
            // Degenerate (zero-area) triangles at the poles are harmless
            // and standard for this construction -- not worth special-
            // casing away.
            indices.push(a as u16);
            indices.push(b as u16);
            indices.push(a as u16 + 1);
            indices.push(a as u16 + 1);
            indices.push(b as u16);
            indices.push(b as u16 + 1);
        }
    }
    (vertices, indices)
}

const VERTEX_ATTRS: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 5 => Float32x2];
const INSTANCE_ATTRS: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
    2 => Float32x3, // center
    3 => Float32x3, // scale
    4 => Float32x4, // color
    6 => Float32x4, // uv_rect
    7 => Float32x4, // rotation
    8 => Float32,   // emissive
];

const SHADER_SRC: &str = r#"
struct Camera {
    view_proj: mat4x4<f32>,
    eye_pos: vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;

// Brick front-face atlas (see `Renderer3D::new`'s atlas/texture setup and
// `render3d::atlas`'s module doc comment for where these pixels come
// from). Nothing else this pipeline draws samples this texture -- see
// `fs_main`'s `has_sprite` check below.
@group(1) @binding(0) var atlas_tex: texture_2d<f32>;
@group(1) @binding(1) var atlas_samp: sampler;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(5) uv: vec2<f32>,
};
struct InstanceInput {
    @location(2) center: vec3<f32>,
    @location(3) scale: vec3<f32>,
    @location(4) color: vec4<f32>,
    @location(6) uv_rect: vec4<f32>,
    @location(7) rotation: vec4<f32>,
    @location(8) emissive: f32,
};
struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) uv_rect: vec4<f32>,
    @location(5) emissive: f32,
};

// Standard quaternion*vector rotation (q assumed unit-length, which every
// `Instance3D::rotation` this module ever writes is -- see `TumbleGhost`'s
// and the power-up spin's `Quat` constructors, both of which only ever
// build a unit quaternion). See `Instance3D`'s doc comment for why this is
// also the correct *normal* transform here, not just the position one.
fn quat_rotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let axis = q.xyz;
    return v + 2.0 * cross(axis, cross(axis, v) + q.w * v);
}

// Key light direction (points FROM the surface TOWARD the light) and fill
// ambient -- the "one Blinn-Phong shader (key light + fill ambient)" this
// bead's description asks for. Fixed constants, not a uniform: the light
// never moves, same reasoning `render.rs`'s own shader gives for
// hardcoding PLAYFIELD_WIDTH/HEIGHT instead of a bind group.
const LIGHT_DIR: vec3<f32> = vec3<f32>(0.35, 0.82, 0.28);
const AMBIENT: f32 = 0.32;
const SPEC_STRENGTH: f32 = 0.35;
const SHININESS: f32 = 24.0;

@vertex
fn vs_main(vert: VertexInput, inst: InstanceInput) -> VertexOutput {
    let scaled = vert.position * inst.scale;
    let world_pos = inst.center + quat_rotate(inst.rotation, scaled);
    // See `Instance3D`'s doc comment: rescale by the (diagonal) scale's
    // reciprocal first -- exact for axis-aligned scaling -- then rotate by
    // the same quaternion as the position. Identity rotation (every
    // non-spinning, non-tumbling instance) makes this a no-op, so this is
    // a strict upgrade of the pre-c3 "normal / scale" transform, not a
    // behavior change for anything that doesn't rotate.
    let world_normal = normalize(quat_rotate(inst.rotation, vert.normal / inst.scale));

    var out: VertexOutput;
    out.clip_position = camera.view_proj * vec4<f32>(world_pos, 1.0);
    out.world_pos = world_pos;
    out.world_normal = world_normal;
    out.color = inst.color;
    out.uv = vert.uv;
    out.uv_rect = inst.uv_rect;
    out.emissive = inst.emissive;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let n = normalize(in.world_normal);
    let l = normalize(LIGHT_DIR);
    let v = normalize(camera.eye_pos.xyz - in.world_pos);
    let h = normalize(l + v);

    let diffuse = max(dot(n, l), 0.0);
    let spec = pow(max(dot(n, h), 0.0), SHININESS) * SPEC_STRENGTH;

    // Front-face-only texturing (bricks' +Z side -- see `Instance3D`'s
    // doc comment): a nonzero-area `uv_rect` means "this instance carries
    // an atlas sprite," and `n.z` close to 1 means "this fragment belongs
    // to that unrotated cube's +Z face" (no rotation ever happens here,
    // so object-space and world-space normals are identical). Every other
    // face/instance keeps the flat `color` albedo. `textureSampleLevel`
    // (not `textureSample`) is used deliberately: this atlas has no
    // mipmaps, and an explicit LOD sidesteps WGSL's derivative-uniformity
    // requirement for a texture sample inside this per-fragment branch.
    let has_sprite = (in.uv_rect.z - in.uv_rect.x) > 0.0;
    var albedo = in.color.rgb;
    if has_sprite && n.z > 0.9 {
        let atlas_uv = vec2<f32>(
            mix(in.uv_rect.x, in.uv_rect.z, in.uv.x),
            mix(in.uv_rect.y, in.uv_rect.w, in.uv.y)
        );
        albedo = textureSampleLevel(atlas_tex, atlas_samp, atlas_uv, 0.0).rgb;
    }

    // `in.emissive` (brick hit-flash -- see `Instance3D`'s doc comment)
    // adds flat brightness on top of the lit result, unaffected by `n`/`l`
    // so the flash reads the same regardless of which face or light angle.
    let lit = albedo * (AMBIENT + diffuse) + vec3<f32>(spec, spec, spec) + vec3<f32>(in.emissive);
    return vec4<f32>(lit, in.color.a);
}
"#;

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Blends `prev` toward `curr` by `alpha`, mirroring `render::RenderState`'s
/// own (private-to-that-module) interpolation -- including the ball-
/// respawn snap -- so this sibling renderer gets identical motion without
/// editing a file outside this bead's ownership. Small enough to duplicate
/// rather than loosen `render.rs`'s encapsulation for one caller.
fn interpolate(prev: &RenderState, curr: &RenderState, alpha: f32) -> RenderState {
    let alpha = alpha.clamp(0.0, 1.0);
    let just_respawned = !prev.ball.attached && curr.ball.attached;
    RenderState {
        paddle: Paddle {
            x: lerp(prev.paddle.x, curr.paddle.x, alpha),
            y: lerp(prev.paddle.y, curr.paddle.y, alpha),
            width: lerp(prev.paddle.width, curr.paddle.width, alpha),
            height: lerp(prev.paddle.height, curr.paddle.height, alpha),
        },
        ball: if just_respawned {
            curr.ball
        } else {
            Ball {
                x: lerp(prev.ball.x, curr.ball.x, alpha),
                y: lerp(prev.ball.y, curr.ball.y, alpha),
                vx: curr.ball.vx,
                vy: curr.ball.vy,
                radius: lerp(prev.ball.radius, curr.ball.radius, alpha),
                attached: curr.ball.attached,
            }
        },
    }
}

// -- juice: hit-flash, destroy-tumble, ball trail, screen shake, power-up
// spin (arkanoid-v2-c3) --------------------------------------------------
//
// Hit-flash and shake are frame-to-frame *diffs* against a snapshot kept on
// `Renderer3D`, same technique and same reasoning `render.rs`'s own (private
// -to-that-module) `bricks_just_hit`/`next_shake_remaining` use -- see that
// module's "-- juice --" comment: by the time `render()` runs, `main.rs`
// has already drained whatever events this frame's tick(s) pushed, and
// `hits_remaining` is fully present on `Game::bricks` every tick anyway, so
// no event is needed for either. Destroy-tumble is the one exception: a
// destroyed brick is already gone from `Game::bricks` by the time
// `BrickDestroyedAt` fires (see events.rs's doc comment), so it really is
// "spawned from the event" -- fed in via `Renderer3D::ingest_tick_events`,
// which `main.rs` must call once per tick, before draining `Game::events`.
// Ball trail and power-up spin need neither: a trail is just recent ball
// positions, and a spin is just wall-clock time -- both plain per-frame
// state on `Renderer3D`.

/// White-hot flash color for a brick that was just hit (surviving or not).
const HIT_FLASH_EMISSIVE: f32 = 0.9;

/// Alpha of the newest (closest to the ball) trail sphere; older ones fade
/// linearly toward 0 from there. Matches `render.rs`'s own constant.
const TRAIL_BASE_ALPHA: f32 = 0.5;

/// How long a destroyed brick's tumble-away ghost stays on screen (spec:
/// "the cube tumbles away with gravity for 0.5s").
const TUMBLE_DURATION_SECS: f32 = 0.5;
/// World-Y units/s^2 the ghost falls under -- tuned only so the fall reads
/// clearly within `TUMBLE_DURATION_SECS`, not against any real-world unit.
const TUMBLE_GRAVITY: f32 = 1400.0;
/// Initial upward launch speed (world-Y units/s) so a destroyed brick
/// visibly "pops" before gravity takes over, instead of only ever falling.
const TUMBLE_LAUNCH_SPEED: f32 = 220.0;
/// Radians/s each ghost spins around its own (per-ghost) axis.
const TUMBLE_SPIN_RATE: f32 = 6.0;

/// Radians/s a falling power-up capsule spins around world-Y (spec:
/// "power-up capsules rotate slowly").
const POWERUP_SPIN_RATE: f32 = 1.6;

/// Screen-space shake, camera-nudge flavor: `render.rs`'s 2D shake moves
/// quads by up to 3 world/screen px; this camera sits ~900 world units
/// back (see `CAMERA_EYE`), so a translation that small would be invisible
/// -- this is the same juice at a larger, camera-appropriate scale.
const CAMERA_SHAKE_MAX_OFFSET: f32 = 26.0;
const SHAKE_DURATION_SECS: f32 = 0.15;

/// Which currently-standing bricks just took a hit and survived it (e.g. an
/// armored brick's first hit): present in both snapshots at the same
/// position, with fewer hits left now than before. Duplicated from
/// `render.rs`'s own (private) function of the same name/behavior -- see
/// the module doc comment's established convention for why (`brick_color`,
/// `interpolate`, etc.).
fn bricks_just_hit(prev: &[Brick], curr: &[Brick]) -> Vec<(f32, f32)> {
    curr.iter()
        .filter(|b| {
            prev.iter()
                .any(|p| p.x == b.x && p.y == b.y && p.hits_remaining > b.hits_remaining)
        })
        .map(|b| (b.x, b.y))
        .collect()
}

/// Shake time remaining after `dt` seconds elapse, refreshed back to
/// `SHAKE_DURATION_SECS` when `score_increased`. Duplicated from
/// `render.rs`'s function of the same name/behavior.
fn next_shake_remaining(current: f32, dt: f32, score_increased: bool) -> f32 {
    let refreshed = if score_increased {
        SHAKE_DURATION_SECS
    } else {
        current
    };
    (refreshed - dt).max(0.0)
}

/// Ball trail history update for one frame: cleared while the ball is
/// parked on the paddle, otherwise the new position is appended and the
/// oldest one dropped past `TRAIL_LEN`. Duplicated from `render.rs`'s
/// function of the same name/behavior.
fn push_trail(trail: &mut VecDeque<(f32, f32)>, ball: &Ball) {
    if ball.attached {
        trail.clear();
        return;
    }
    trail.push_back((ball.x, ball.y));
    while trail.len() > TRAIL_LEN {
        trail.pop_front();
    }
}

/// Deterministic per-brick "randomness" for a tumble ghost's spin axis,
/// derived from its own spawn position rather than an RNG -- two bricks at
/// different positions tumble around visibly different axes, and the same
/// brick always tumbles the same way (see the `tumble_*` tests below,
/// which are exactly this bead's "headless determinism test given a
/// seeded event stream").
fn tumble_axis(x: f32, y: f32) -> Vec3 {
    Vec3::new(
        (x * 0.073).sin(),
        0.6 + 0.4 * (y * 0.051).cos(),
        (x * 0.037 + y * 0.029).cos(),
    )
    .normalize()
}

/// One destroyed brick's render-only "tumble away" physics (spec:
/// "render-side only, spawned from the event, no sim change"). Spawned by
/// `Renderer3D::ingest_tick_events` from a `GameEvent::BrickDestroyedAt`,
/// stepped forward by wall-clock `dt` in `Renderer3D::render`, and dropped
/// once `elapsed` passes `TUMBLE_DURATION_SECS`. Plain data plus pure
/// functions (`spawn`/`step`/`instance`) below, deliberately kept free of
/// any wgpu handle, so this is unit-testable without a GPU `Renderer3D`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct TumbleGhost {
    x: f32,
    z: f32,
    y_offset: f32,
    vy: f32,
    half_extent: [f32; 3],
    color: [f32; 4],
    axis: Vec3,
    spin_dir: f32,
    elapsed: f32,
}

impl TumbleGhost {
    fn spawn(x: f32, y: f32, width: f32, height: f32, kind: BrickKind) -> Self {
        let (wx, wz) = world_xz(x, y);
        Self {
            x: wx,
            z: wz,
            y_offset: 0.0,
            vy: TUMBLE_LAUNCH_SPEED,
            half_extent: [width / 2.0, BRICK_THICKNESS / 2.0, height / 2.0],
            // `hits_remaining: 0`: the brick's last-shown appearance before
            // dying (an armored brick's final hit already shows its "hit"
            // color -- see `brick_color_for` -- and every other kind has
            // only one color regardless of hit count).
            color: brick_color_for(kind, 0),
            axis: tumble_axis(x, y),
            spin_dir: if x >= 0.0 { 1.0 } else { -1.0 },
            elapsed: 0.0,
        }
    }

    /// Advances gravity/spin by `dt` seconds of wall-clock time.
    fn step(&mut self, dt: f32) {
        self.elapsed += dt;
        self.vy -= TUMBLE_GRAVITY * dt;
        self.y_offset += self.vy * dt;
    }

    fn alive(&self) -> bool {
        self.elapsed < TUMBLE_DURATION_SECS
    }

    fn rotation(&self) -> Quat {
        Quat::from_axis_angle(self.axis, self.spin_dir * self.elapsed * TUMBLE_SPIN_RATE)
    }

    /// Linear fade to fully transparent by the time the ghost expires, so
    /// it doesn't just vanish with a pop at exactly `TUMBLE_DURATION_SECS`.
    fn alpha_fade(&self) -> f32 {
        (1.0 - self.elapsed / TUMBLE_DURATION_SECS).clamp(0.0, 1.0)
    }

    fn instance(&self) -> Instance3D {
        let center = [self.x, BRICK_THICKNESS / 2.0 + self.y_offset, self.z];
        let mut color = self.color;
        color[3] *= self.alpha_fade();
        Instance3D::new(center, self.half_extent, color).with_rotation(self.rotation().to_array())
    }
}

/// Pure core of `Renderer3D::ingest_tick_events`: appends one fresh
/// `TumbleGhost` per `BrickDestroyedAt` in `events` (every other variant is
/// ignored -- this effect is the one juice effect that genuinely needs the
/// event, see the module doc comment), capped at `MAX_DESTROY_GHOSTS`.
/// Extracted as a free function, independent of any wgpu handle, so it's
/// unit-testable without constructing a GPU-backed `Renderer3D`.
fn ingest_brick_destroyed_events(ghosts: &mut Vec<TumbleGhost>, events: &[GameEvent]) {
    for event in events {
        if let GameEvent::BrickDestroyedAt {
            x,
            y,
            width,
            height,
            kind,
        } = *event
        {
            if ghosts.len() < MAX_DESTROY_GHOSTS {
                ghosts.push(TumbleGhost::spawn(x, y, width, height, kind));
            }
        }
    }
}

fn create_depth_texture(
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("render3d depth texture"),
        size: wgpu::Extent3d {
            width: config.width.max(1),
            height: config.height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// Owns the wgpu state for the 3D renderer: its own adapter-negotiated
/// device/queue and surface, entirely independent of `render::Renderer`
/// (see the module doc comment -- this is a sibling, not a replacement).
pub struct Renderer3D {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    depth_view: wgpu::TextureView,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    /// Brick front-face atlas sprite lookup (see `sprite_for_brick` and
    /// `atlas::Atlas::uv_rect`). The pixels themselves are uploaded once
    /// into `texture_bind_group`'s texture at construction time; this is
    /// kept around only for its cheap per-frame `uv_rect` arithmetic.
    atlas: Atlas,
    texture_bind_group: wgpu::BindGroup,
    cube_vertex_buffer: wgpu::Buffer,
    cube_index_buffer: wgpu::Buffer,
    cube_instance_buffer: wgpu::Buffer,
    sphere_vertex_buffer: wgpu::Buffer,
    sphere_index_buffer: wgpu::Buffer,
    sphere_index_count: u32,
    sphere_instance_buffer: wgpu::Buffer,
    // -- juice: hit-flash, destroy-tumble, ball trail, screen shake,
    // power-up spin -- see the module-level "-- juice --" comment above
    // `bricks_just_hit` for why this is diffed/timed here rather than read
    // off `Game::events` (except tumble, which really does need the event).
    /// Ball's last few interpolated positions, oldest-first; see
    /// `push_trail`.
    ball_trail: VecDeque<(f32, f32)>,
    /// Snapshot of `curr.bricks` as of the last `render()` call, diffed
    /// against this frame's by `bricks_just_hit`.
    prev_bricks: Vec<Brick>,
    /// `curr.level` as of the last `render()` call -- a change means the
    /// diff above would be comparing two different levels' grids, so it's
    /// skipped for that one frame, same reasoning `render.rs` documents.
    prev_level: usize,
    /// `curr.score` as of the last `render()` call -- see
    /// `next_shake_remaining`.
    prev_score: u32,
    /// Seconds of screen shake left to play; see `next_shake_remaining`.
    shake_remaining: f32,
    /// Wall-clock time of the last `render()` call, used to compute the
    /// real elapsed `dt` the shake timer, tumble ghosts, and power-up spin
    /// all advance by (frames don't map 1:1 to fixed simulation ticks).
    last_frame_instant: Instant,
    /// Currently tumbling destroy-ghosts; see `TumbleGhost` and
    /// `ingest_tick_events`.
    tumbling: Vec<TumbleGhost>,
    /// Accumulated power-up spin angle (radians, world-Y axis), wrapped to
    /// `[0, 2*PI)` each frame so it never grows unbounded over a long
    /// session.
    powerup_spin_angle: f32,
}

impl Renderer3D {
    /// Negotiates an adapter/device for `window` and configures its
    /// surface. Mirrors `render::Renderer::new`'s adapter/device/surface
    /// setup (duplicated rather than shared -- that function is private to
    /// its own module, and this renderer must stay independently
    /// constructible since only one of the two is ever selected at a
    /// time, per `--renderer`).
    pub fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window)
            .expect("failed to create wgpu surface");

        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("failed to find a compatible wgpu adapter");

        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("arkanoid render3d device"),
            ..Default::default()
        }))
        .expect("failed to request wgpu device");

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let present_mode = if caps.present_modes.contains(&wgpu::PresentMode::FifoRelaxed) {
            wgpu::PresentMode::FifoRelaxed
        } else {
            wgpu::PresentMode::Fifo
        };

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        let (_, depth_view) = create_depth_texture(&device, &config);

        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("render3d camera uniform"),
            size: size_of::<CameraUniform>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("render3d camera bind group layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("render3d camera bind group"),
            layout: &camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        // Brick front-face atlas -- see `render3d::atlas`'s module doc
        // comment for why this is a local copy of Workstream B's real
        // `epic/textures:src/assets.rs` recipe rather than an import of
        // it, and why that's still "B's atlas" rather than an invented
        // placeholder.
        eprintln!(
            "render3d: brick front faces textured via a LOCAL COPY of Workstream B's real \
             atlas recipe (epic/textures src/assets.rs, beads b1/b2) -- hand-synced pending \
             the cross-epic merge into master, see src/render3d/atlas.rs's doc comment. This \
             is B's real beveled-brick-panel recipe, not an invented flat placeholder."
        );
        let atlas = Atlas::procedural_placeholder();
        let atlas_texture_size = wgpu::Extent3d {
            width: atlas.width,
            height: atlas.height,
            depth_or_array_layers: 1,
        };
        let atlas_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("render3d brick atlas texture"),
            size: atlas_texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &atlas_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &atlas.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(atlas.width * 4),
                rows_per_image: Some(atlas.height),
            },
            atlas_texture_size,
        );
        let atlas_view = atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());
        // Nearest filtering: the atlas is small pixel-art-style sprites
        // sampled at roughly 1:1 on screen, not photographic detail --
        // bilinear would just blur the bevel/noise recipe's fine detail.
        let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("render3d brick atlas sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("render3d atlas texture bind group layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let texture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("render3d atlas texture bind group"),
            layout: &texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&atlas_sampler),
                },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("render3d blinn-phong shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SRC.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("render3d pipeline layout"),
            bind_group_layouts: &[
                Some(&camera_bind_group_layout),
                Some(&texture_bind_group_layout),
            ],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("render3d cube/sphere pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<Vertex3D>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &VERTEX_ATTRS,
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<Instance3D>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &INSTANCE_ATTRS,
                    }),
                ],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                // No culling: cube/sphere winding isn't worth chasing down
                // face-by-face for a handful of instances per frame (same
                // tradeoff `render.rs`'s own pipeline makes).
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    // Alpha blending (arkanoid-v2-c3): the ball trail and
                    // the destroy-tumble ghosts (see the "-- juice --"
                    // section) fade via `color.a` < 1. Every other
                    // instance keeps alpha 1.0, for which blending
                    // produces the same pixels `REPLACE` would -- same
                    // reasoning `render.rs`'s own pipeline gives for
                    // always blending.
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let cube_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("render3d cube vertices"),
            contents: bytemuck::cast_slice(&CUBE_VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let cube_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("render3d cube indices"),
            contents: bytemuck::cast_slice(&CUBE_INDICES),
            usage: wgpu::BufferUsages::INDEX,
        });
        let cube_instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("render3d cube instances"),
            size: (size_of::<Instance3D>() * MAX_CUBE_INSTANCES) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let (sphere_vertices, sphere_indices) = build_sphere_mesh(SPHERE_STACKS, SPHERE_SECTORS);
        let sphere_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("render3d sphere vertices"),
            contents: bytemuck::cast_slice(&sphere_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let sphere_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("render3d sphere indices"),
            contents: bytemuck::cast_slice(&sphere_indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let sphere_instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("render3d sphere instances"),
            size: (size_of::<Instance3D>() * MAX_SPHERE_INSTANCES) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            depth_view,
            camera_buffer,
            camera_bind_group,
            atlas,
            texture_bind_group,
            cube_vertex_buffer,
            cube_index_buffer,
            cube_instance_buffer,
            sphere_vertex_buffer,
            sphere_index_buffer,
            sphere_index_count: sphere_indices.len() as u32,
            sphere_instance_buffer,
            ball_trail: VecDeque::new(),
            prev_bricks: Vec::new(),
            prev_level: 0,
            prev_score: 0,
            shake_remaining: 0.0,
            last_frame_instant: Instant::now(),
            tumbling: Vec::new(),
            powerup_spin_angle: 0.0,
        }
    }

    /// Feeds one tick's `GameEvent`s into this renderer's own juice state
    /// -- currently only `BrickDestroyedAt`, for the destroy-tumble effect
    /// (see the module doc comment). Must be called once per tick, by the
    /// caller (`main.rs`'s fixed-timestep loop), *before* it drains
    /// `Game::events` for that tick -- `render()` alone runs too late, once
    /// per display frame rather than once per (up to `MAX_TICKS_PER_FRAME`)
    /// simulation tick, and after the events that fired are already gone.
    ///
    /// Thin wrapper over `ingest_brick_destroyed_events`, which does the
    /// actual work as a free function so it's unit-testable without a
    /// GPU-backed `Renderer3D` -- see the `tumble_ghosts_*` tests below,
    /// this bead's "headless determinism test given a seeded event
    /// stream".
    pub fn ingest_tick_events(&mut self, events: &[GameEvent]) {
        ingest_brick_destroyed_events(&mut self.tumbling, events);
    }

    /// Reconfigures the surface and depth buffer after a window resize.
    /// The depth texture must match the color target's size every frame,
    /// so it's rebuilt here alongside the surface -- unlike
    /// `render::Renderer::resize`, which has no depth buffer to keep in
    /// sync.
    pub fn resize(&mut self, new_size: PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }
        self.config.width = new_size.width;
        self.config.height = new_size.height;
        self.surface.configure(&self.device, &self.config);
        let (_, depth_view) = create_depth_texture(&self.device, &self.config);
        self.depth_view = depth_view;
    }

    /// Clears `view` and the depth buffer, then draws the paddle, ball(s),
    /// bricks, falling power-ups, and the three boundary walls as instances
    /// of one shared pipeline in two draw calls (cubes, then the sphere
    /// mesh), submitting the result to the queue. `prev`/`curr`/`alpha` are
    /// the same interpolation inputs `render::Renderer::render` takes --
    /// see `interpolate`.
    ///
    /// Split out of `render()` (which draws to the real swapchain surface)
    /// so `render_offscreen_for_bench` (arkanoid-v2-c5's perf harness, see
    /// `src/bin/bench_render3d.rs`) can drive the exact same drawing code
    /// against a plain offscreen texture instead -- measuring real render
    /// cost without a swapchain/compositor in the loop, whose present-mode
    /// pacing (vsync, unfocused-window throttling, etc.) has nothing to do
    /// with this renderer's own performance.
    fn render_into(
        &mut self,
        clear_color: wgpu::Color,
        prev: &RenderState,
        curr: &Game,
        alpha: f32,
        view: &wgpu::TextureView,
    ) {
        let drawn = interpolate(prev, &RenderState::from(curr), alpha);

        // -- juice bookkeeping: see the module-level "-- juice --" comment
        // above `bricks_just_hit` for why most of this is diffed/timed
        // here rather than read off `Game::events`.
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame_instant).as_secs_f32();
        self.last_frame_instant = now;

        self.powerup_spin_angle =
            (self.powerup_spin_angle + dt * POWERUP_SPIN_RATE).rem_euclid(2.0 * PI);
        let powerup_rotation = Quat::from_rotation_y(self.powerup_spin_angle).to_array();

        let score_increased = curr.score > self.prev_score;
        self.prev_score = curr.score;
        self.shake_remaining = next_shake_remaining(self.shake_remaining, dt, score_increased);
        let shake_offset = if self.shake_remaining > 0.0 {
            let amplitude = CAMERA_SHAKE_MAX_OFFSET * (self.shake_remaining / SHAKE_DURATION_SECS);
            Vec3::new(
                rand::random_range(-amplitude..=amplitude),
                0.0,
                rand::random_range(-amplitude..=amplitude),
            )
        } else {
            Vec3::ZERO
        };
        let eye = CAMERA_EYE + shake_offset;
        let target = CAMERA_TARGET + shake_offset;

        // Level change: `advance_level` reloads `bricks` wholesale, so the
        // diff below would misread every leftover brick as "just hit" --
        // skip it for this one frame instead (see `render.rs`'s own
        // `level_changed` handling for the same reasoning).
        let level_changed = curr.level != self.prev_level;
        self.prev_level = curr.level;
        let hit_flash = if level_changed {
            Vec::new()
        } else {
            bricks_just_hit(&self.prev_bricks, &curr.bricks)
        };
        self.prev_bricks.clear();
        self.prev_bricks.extend_from_slice(&curr.bricks);

        // Destroy-tumble ghosts: advance by wall-clock `dt`, drop any that
        // just expired. New ones arrive via `ingest_tick_events`, called by
        // the caller once per tick -- see that method's doc comment.
        for ghost in &mut self.tumbling {
            ghost.step(dt);
        }
        self.tumbling.retain(TumbleGhost::alive);

        let aspect = self.config.width as f32 / self.config.height as f32;
        let camera_uniform = CameraUniform {
            view_proj: camera_view_proj(aspect, eye, target),
            eye_pos: [eye.x, eye.y, eye.z, 0.0],
        };
        self.queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&camera_uniform));

        let mut cube_instances = Vec::with_capacity(
            1 + curr.bricks.len() + curr.powerups.len() + WALL_COUNT + self.tumbling.len(),
        );
        let mut sphere_instances =
            Vec::with_capacity(1 + curr.extra_balls.len() + self.ball_trail.len());

        // Paddle.
        {
            let (x, z) = world_xz(drawn.paddle.x, drawn.paddle.y);
            cube_instances.push(Instance3D::new(
                [x, PADDLE_THICKNESS / 2.0, z],
                [
                    drawn.paddle.width / 2.0,
                    PADDLE_THICKNESS / 2.0,
                    drawn.paddle.height / 2.0,
                ],
                PADDLE_COLOR,
            ));
        }

        // Ball's fading trail: up to `TRAIL_LEN` spheres at its last few
        // interpolated positions (oldest/faintest first), added *before*
        // the opaque ball sphere below so the ball itself always ends up
        // layered on top of its own trail. Uses the trail as it stood at
        // the *start* of this frame, then updates it (`push_trail`) after.
        let trail_len = self.ball_trail.len().max(1) as f32;
        sphere_instances.extend(self.ball_trail.iter().enumerate().map(|(i, &(x, y))| {
            let fade = (i + 1) as f32 / trail_len;
            let (wx, wz) = world_xz(x, y);
            let radius = drawn.ball.radius * (0.5 + 0.5 * fade);
            Instance3D::new(
                [wx, radius, wz],
                [radius; 3],
                [
                    BALL_COLOR[0],
                    BALL_COLOR[1],
                    BALL_COLOR[2],
                    TRAIL_BASE_ALPHA * fade,
                ],
            )
        }));
        push_trail(&mut self.ball_trail, &drawn.ball);

        // Main ball (interpolated) and Multiball's extra balls (read
        // straight off `curr`, same simplification `render.rs` makes for
        // its own extra-ball trail-less quads).
        {
            let (x, z) = world_xz(drawn.ball.x, drawn.ball.y);
            sphere_instances.push(Instance3D::new(
                [x, drawn.ball.radius, z],
                [drawn.ball.radius; 3],
                BALL_COLOR,
            ));
        }
        sphere_instances.extend(curr.extra_balls.iter().take(MAX_EXTRA_BALLS).map(|ball| {
            let (x, z) = world_xz(ball.x, ball.y);
            Instance3D::new([x, ball.radius, z], [ball.radius; 3], BALL_COLOR)
        }));

        // Bricks -- front face textured from `self.atlas` (see
        // `sprite_for_brick`/`Instance3D::textured`), side/top/bottom
        // faces stay `brick_color`'s flat palette (see `SHADER_SRC`'s
        // `fs_main`). A brick that was just hit and survived (e.g.
        // armored's first hit) gets an emissive white flash for this one
        // frame instead of `brick_color`'s ordinary lit palette (see
        // `hit_flash`). `.take(MAX_BRICKS)` is defensive only -- `levels.rs`'s
        // own tests already enforce every built-in level fits this cap.
        cube_instances.extend(curr.bricks.iter().take(MAX_BRICKS).map(|brick| {
            let (x, z) = world_xz(brick.x, brick.y);
            let uv = self.atlas.uv_rect(sprite_for_brick(brick));
            let emissive = if hit_flash.contains(&(brick.x, brick.y)) {
                HIT_FLASH_EMISSIVE
            } else {
                0.0
            };
            Instance3D::textured(
                [x, BRICK_THICKNESS / 2.0, z],
                [brick.width / 2.0, BRICK_THICKNESS / 2.0, brick.height / 2.0],
                brick_color(brick),
                uv,
            )
            .with_emissive(emissive)
        }));

        // Destroy-tumble ghosts: bricks destroyed recently enough that
        // they're still mid-tumble (spec: "the cube tumbles away with
        // gravity for 0.5s"). See `TumbleGhost` and `ingest_tick_events`.
        cube_instances.extend(self.tumbling.iter().map(TumbleGhost::instance));

        // Falling power-up capsules, colored by kind, slowly spinning
        // around world-Y (spec: "power-up capsules rotate slowly").
        cube_instances.extend(curr.powerups.iter().take(MAX_POWERUPS).map(|powerup| {
            let (x, z) = world_xz(powerup.x, powerup.y);
            Instance3D::new(
                [x, POWERUP_HALF, z],
                [POWERUP_HALF; 3],
                powerup_color(powerup.kind),
            )
            .with_rotation(powerup_rotation)
        }));

        // Three static boundary walls (left, right, top) -- see the
        // module-level constants' doc comments for why these exist despite
        // `game.rs` having no wall entities of its own.
        let half_w = PLAYFIELD_WIDTH / 2.0;
        let half_h = PLAYFIELD_HEIGHT / 2.0;
        cube_instances.push(Instance3D::new(
            [-half_w - WALL_THICKNESS / 2.0, WALL_HEIGHT / 2.0, 0.0],
            [WALL_THICKNESS / 2.0, WALL_HEIGHT / 2.0, half_h],
            WALL_COLOR,
        ));
        cube_instances.push(Instance3D::new(
            [half_w + WALL_THICKNESS / 2.0, WALL_HEIGHT / 2.0, 0.0],
            [WALL_THICKNESS / 2.0, WALL_HEIGHT / 2.0, half_h],
            WALL_COLOR,
        ));
        cube_instances.push(Instance3D::new(
            [0.0, WALL_HEIGHT / 2.0, -half_h - WALL_THICKNESS / 2.0],
            [half_w, WALL_HEIGHT / 2.0, WALL_THICKNESS / 2.0],
            WALL_COLOR,
        ));

        self.queue.write_buffer(
            &self.cube_instance_buffer,
            0,
            bytemuck::cast_slice(&cube_instances),
        );
        self.queue.write_buffer(
            &self.sphere_instance_buffer,
            0,
            bytemuck::cast_slice(&sphere_instances),
        );

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("render3d frame encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("render3d pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear_color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.camera_bind_group, &[]);
            pass.set_bind_group(1, &self.texture_bind_group, &[]);

            pass.set_vertex_buffer(0, self.cube_vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, self.cube_instance_buffer.slice(..));
            pass.set_index_buffer(self.cube_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(
                0..CUBE_INDICES.len() as u32,
                0,
                0..cube_instances.len() as u32,
            );

            pass.set_vertex_buffer(0, self.sphere_vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, self.sphere_instance_buffer.slice(..));
            pass.set_index_buffer(
                self.sphere_index_buffer.slice(..),
                wgpu::IndexFormat::Uint16,
            );
            pass.draw_indexed(
                0..self.sphere_index_count,
                0,
                0..sphere_instances.len() as u32,
            );
        }
        self.queue.submit(std::iter::once(encoder.finish()));
    }

    /// Acquires the swapchain's current frame and draws into it via
    /// `render_into`, then presents. This is the real, production render
    /// path -- see `render_into`'s doc comment for why the actual drawing
    /// is factored out of this method.
    pub fn render(
        &mut self,
        clear_color: wgpu::Color,
        prev: &RenderState,
        curr: &Game,
        alpha: f32,
    ) {
        let surface_texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => return,
        };
        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.render_into(clear_color, prev, curr, alpha, &view);
        self.queue.present(surface_texture);
    }

    /// Same drawing work as `render`, but into a throwaway offscreen
    /// texture instead of the swapchain, and blocking (via `device.poll`)
    /// until the GPU has actually finished the frame before returning.
    ///
    /// Exists for `src/bin/bench_render3d.rs` (arkanoid-v2-c5's "measure
    /// before optimizing" perf harness): a swapchain's present-mode pacing
    /// (vsync, or a desktop compositor throttling an unfocused/off-screen
    /// window to as little as 1 fps to save power -- both observed while
    /// building this bench) has nothing to do with this renderer's own
    /// per-frame cost, so timing real GPU+CPU work needs a path that never
    /// waits on a compositor at all.
    ///
    /// `#[allow(dead_code)]`: only that sibling `[[bin]]` target ever calls
    /// this -- the main `arkanoid` binary's own dead-code analysis has no
    /// visibility into a separate binary crate's call sites.
    #[allow(dead_code)]
    pub fn render_offscreen_for_bench(
        &mut self,
        clear_color: wgpu::Color,
        prev: &RenderState,
        curr: &Game,
        alpha: f32,
    ) {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("render3d offscreen bench target"),
            size: wgpu::Extent3d {
                width: self.config.width.max(1),
                height: self.config.height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.render_into(clear_color, prev, curr, alpha, &view);
        // Block until this frame's submitted work is actually done, so the
        // caller's wall-clock timing around this call reflects real GPU
        // completion time, not just how fast the CPU could enqueue it.
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll failed");
    }
}

/// Minimal single-purpose executor for wgpu's one-shot startup futures.
/// Copy of `render::block_on` (private to that module) -- see this
/// module's doc comment on why `Renderer3D` duplicates rather than shares
/// `render.rs`'s setup plumbing.
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    struct ThreadWaker(std::thread::Thread);

    impl Wake for ThreadWaker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }

    let mut future = std::pin::pin!(future);
    let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::park(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(paddle_x: f32, ball_x: f32, ball_y: f32, attached: bool) -> RenderState {
        RenderState {
            paddle: Paddle {
                x: paddle_x,
                y: 550.0,
                width: 100.0,
                height: 16.0,
            },
            ball: Ball {
                x: ball_x,
                y: ball_y,
                vx: 0.0,
                vy: 0.0,
                radius: 6.0,
                attached,
            },
        }
    }

    #[test]
    fn interpolate_at_zero_and_one_returns_the_endpoints_unchanged() {
        let prev = state(100.0, 100.0, 200.0, false);
        let curr = state(140.0, 150.0, 180.0, false);

        assert_eq!(interpolate(&prev, &curr, 0.0), prev);
        assert_eq!(interpolate(&prev, &curr, 1.0), curr);
    }

    #[test]
    fn interpolate_at_half_is_the_midpoint() {
        let prev = state(100.0, 100.0, 200.0, false);
        let curr = state(140.0, 150.0, 180.0, false);

        let mid = interpolate(&prev, &curr, 0.5);

        assert!((mid.paddle.x - 120.0).abs() < 1e-4);
        assert!((mid.ball.x - 125.0).abs() < 1e-4);
        assert!((mid.ball.y - 190.0).abs() < 1e-4);
    }

    #[test]
    fn interpolate_clamps_an_out_of_range_alpha() {
        let prev = state(100.0, 100.0, 200.0, false);
        let curr = state(140.0, 150.0, 180.0, false);

        assert_eq!(interpolate(&prev, &curr, -1.0), prev);
        assert_eq!(interpolate(&prev, &curr, 2.0), curr);
    }

    #[test]
    fn interpolate_snaps_the_ball_to_curr_on_respawn_instead_of_streaking_in() {
        let prev = state(400.0, 400.0, 650.0, false);
        let curr = state(400.0, 400.0, 100.0, true);

        let drawn = interpolate(&prev, &curr, 0.5);

        assert_eq!(drawn.ball, curr.ball, "no blending through the gap");
        assert!((drawn.paddle.x - 400.0).abs() < 1e-4);
    }

    #[test]
    fn world_xz_centers_the_playfield_on_the_origin() {
        assert_eq!(
            world_xz(PLAYFIELD_WIDTH / 2.0, PLAYFIELD_HEIGHT / 2.0),
            (0.0, 0.0)
        );
        assert_eq!(
            world_xz(0.0, 0.0),
            (-PLAYFIELD_WIDTH / 2.0, -PLAYFIELD_HEIGHT / 2.0)
        );
    }

    #[test]
    fn each_brick_kind_gets_its_own_distinct_color() {
        let brick_of = |kind, hits| Brick {
            x: 0.0,
            y: 0.0,
            width: 52.0,
            height: 22.0,
            kind,
            hits_remaining: hits,
            score: 10,
        };
        let normal = brick_color(&brick_of(BrickKind::Normal, 1));
        let armored = brick_color(&brick_of(BrickKind::Armored, 2));
        let indestructible = brick_color(&brick_of(BrickKind::Indestructible, 0));
        assert_ne!(normal, armored);
        assert_ne!(normal, indestructible);
        assert_ne!(armored, indestructible);
    }

    #[test]
    fn sprite_for_brick_agrees_with_brick_color_on_the_armored_hit_split() {
        let brick_of = |kind, hits| Brick {
            x: 0.0,
            y: 0.0,
            width: 52.0,
            height: 22.0,
            kind,
            hits_remaining: hits,
            score: 10,
        };
        // Every kind/hit combination gets its own sprite, and in
        // particular the armored intact/hit split -- the one place a
        // single `BrickKind` maps to two different visual states -- must
        // land on two distinct sprites, same as `brick_color`'s two
        // distinct colors for the same split.
        let normal = sprite_for_brick(&brick_of(BrickKind::Normal, 1));
        let armored_intact = sprite_for_brick(&brick_of(BrickKind::Armored, 2));
        let armored_hit = sprite_for_brick(&brick_of(BrickKind::Armored, 1));
        let indestructible = sprite_for_brick(&brick_of(BrickKind::Indestructible, 0));
        assert_eq!(normal, SpriteId::BrickNormal);
        assert_eq!(armored_intact, SpriteId::BrickArmoredIntact);
        assert_eq!(armored_hit, SpriteId::BrickArmoredHit);
        assert_eq!(indestructible, SpriteId::BrickIndestructible);
        assert_ne!(armored_intact, armored_hit);
    }

    #[test]
    fn textured_brick_instance_carries_a_nonzero_area_uv_rect() {
        let atlas = Atlas::procedural_placeholder();
        let brick = Brick {
            x: 0.0,
            y: 0.0,
            width: 52.0,
            height: 22.0,
            kind: BrickKind::Normal,
            hits_remaining: 1,
            score: 10,
        };
        let uv = atlas.uv_rect(sprite_for_brick(&brick));
        let instance = Instance3D::textured([0.0; 3], [1.0; 3], brick_color(&brick), uv);
        assert!(
            instance.uv_rect[2] > instance.uv_rect[0],
            "u1 must exceed u0"
        );
        assert!(
            instance.uv_rect[3] > instance.uv_rect[1],
            "v1 must exceed v0"
        );

        // A plain (non-textured) instance keeps the zero-area rect the
        // shader reads as "no sprite, use flat color everywhere."
        let flat = Instance3D::new([0.0; 3], [1.0; 3], PADDLE_COLOR);
        assert_eq!(flat.uv_rect, [0.0; 4]);
    }

    #[test]
    fn each_powerup_kind_gets_its_own_distinct_color() {
        let widen = powerup_color(PowerUpKind::Widen);
        let slow = powerup_color(PowerUpKind::Slow);
        let multiball = powerup_color(PowerUpKind::Multiball);
        assert_ne!(widen, slow);
        assert_ne!(widen, multiball);
        assert_ne!(slow, multiball);
    }

    #[test]
    fn sphere_mesh_indices_stay_in_bounds_of_its_own_vertex_buffer() {
        let (vertices, indices) = build_sphere_mesh(SPHERE_STACKS, SPHERE_SECTORS);
        assert!(!vertices.is_empty());
        assert!(!indices.is_empty());
        for &i in &indices {
            assert!(
                (i as usize) < vertices.len(),
                "index {i} out of bounds for {} vertices",
                vertices.len()
            );
        }
    }

    #[test]
    fn sphere_mesh_vertices_sit_on_the_unit_sphere() {
        let (vertices, _) = build_sphere_mesh(SPHERE_STACKS, SPHERE_SECTORS);
        for v in vertices {
            let len_sq = v.position[0] * v.position[0]
                + v.position[1] * v.position[1]
                + v.position[2] * v.position[2];
            assert!((len_sq - 1.0).abs() < 1e-4, "position not on unit sphere");
            assert_eq!(
                v.position, v.normal,
                "unit-sphere normal must equal position"
            );
        }
    }

    #[test]
    fn cube_indices_stay_in_bounds_of_the_cube_vertex_buffer() {
        for &i in &CUBE_INDICES {
            assert!((i as usize) < CUBE_VERTICES.len());
        }
    }

    // -- juice (arkanoid-v2-c3) ---------------------------------------

    fn brick_of(kind: BrickKind, hits: u8) -> Brick {
        Brick {
            x: 100.0,
            y: 200.0,
            width: 52.0,
            height: 22.0,
            kind,
            hits_remaining: hits,
            score: 20,
        }
    }

    #[test]
    fn new_instance_carries_identity_rotation_and_no_emissive() {
        let inst = Instance3D::new([0.0; 3], [1.0; 3], PADDLE_COLOR);
        assert_eq!(inst.rotation, ROTATION_IDENTITY);
        assert_eq!(inst.emissive, 0.0);
    }

    #[test]
    fn with_rotation_and_with_emissive_override_only_their_own_field() {
        let inst = Instance3D::new([0.0; 3], [1.0; 3], PADDLE_COLOR)
            .with_rotation([0.1, 0.2, 0.3, 0.9])
            .with_emissive(0.5);
        assert_eq!(inst.rotation, [0.1, 0.2, 0.3, 0.9]);
        assert_eq!(inst.emissive, 0.5);
        assert_eq!(inst.color, PADDLE_COLOR);
    }

    #[test]
    fn bricks_just_hit_flags_a_surviving_armored_brick_after_its_first_hit() {
        let prev = vec![brick_of(BrickKind::Armored, 2)];
        let curr = vec![brick_of(BrickKind::Armored, 1)];
        assert_eq!(bricks_just_hit(&prev, &curr), vec![(100.0, 200.0)]);
    }

    #[test]
    fn bricks_just_hit_ignores_bricks_whose_hit_count_is_unchanged() {
        let prev = vec![brick_of(BrickKind::Armored, 2)];
        let curr = vec![brick_of(BrickKind::Armored, 2)];
        assert!(bricks_just_hit(&prev, &curr).is_empty());
    }

    #[test]
    fn next_shake_remaining_refreshes_on_a_score_increase_and_decays_otherwise() {
        let refreshed = next_shake_remaining(0.0, 0.01, true);
        assert!((refreshed - (SHAKE_DURATION_SECS - 0.01)).abs() < 1e-6);

        let decayed = next_shake_remaining(0.05, 0.02, false);
        assert!((decayed - 0.03).abs() < 1e-6);

        let floored = next_shake_remaining(0.01, 0.5, false);
        assert_eq!(floored, 0.0);
    }

    #[test]
    fn push_trail_clears_while_attached_and_caps_length_once_launched() {
        let mut trail = VecDeque::new();
        let mut ball = Ball {
            x: 0.0,
            y: 0.0,
            vx: 0.0,
            vy: 0.0,
            radius: 6.0,
            attached: true,
        };
        push_trail(&mut trail, &ball);
        assert!(trail.is_empty(), "an attached ball leaves no trail");

        ball.attached = false;
        for i in 0..(TRAIL_LEN + 3) {
            ball.x = i as f32;
            push_trail(&mut trail, &ball);
        }
        assert_eq!(trail.len(), TRAIL_LEN, "trail must be capped at TRAIL_LEN");
        assert_eq!(trail.back(), Some(&(ball.x, ball.y)));

        ball.attached = true;
        push_trail(&mut trail, &ball);
        assert!(trail.is_empty(), "re-attaching (respawn) clears the trail");
    }

    #[test]
    fn tumble_axis_is_deterministic_and_unit_length() {
        let a = tumble_axis(120.0, 340.0);
        let b = tumble_axis(120.0, 340.0);
        assert_eq!(a, b, "same spawn position must give the same axis");
        assert!((a.length() - 1.0).abs() < 1e-4);
        assert_ne!(
            a,
            tumble_axis(500.0, 10.0),
            "different bricks should tumble around different axes"
        );
    }

    #[test]
    fn tumble_ghost_falls_and_expires_after_its_duration() {
        let mut ghost = TumbleGhost::spawn(100.0, 80.0, 52.0, 22.0, BrickKind::Normal);
        assert!(ghost.alive());
        let start_y = ghost.y_offset;
        for _ in 0..61 {
            // ~1.02s at 60 fps -- safely past TUMBLE_DURATION_SECS (0.5s).
            ghost.step(1.0 / 60.0);
        }
        assert!(!ghost.alive(), "ghost should have expired by now");
        assert_ne!(ghost.y_offset, start_y, "gravity should have moved it");
        assert_eq!(ghost.alpha_fade(), 0.0, "a dead ghost is fully faded");
    }

    /// This bead's acceptance criteria: "tumble animation has a headless
    /// determinism test given a seeded event stream." Two independent runs
    /// ingesting the same `Vec<GameEvent>` and stepping the same fixed `dt`
    /// sequence must land on bit-for-bit identical ghost state -- no RNG,
    /// no wall-clock reads, anywhere in `ingest_brick_destroyed_events` or
    /// `TumbleGhost::{spawn,step}`.
    #[test]
    fn tumble_ghosts_are_deterministic_given_a_seeded_event_stream() {
        let events = vec![
            GameEvent::BrickDestroyedAt {
                x: 100.0,
                y: 80.0,
                width: 52.0,
                height: 22.0,
                kind: BrickKind::Normal,
            },
            GameEvent::BrickDestroyed, // must be ignored, not just skipped silently
            GameEvent::BrickDestroyedAt {
                x: -260.0,
                y: 300.0,
                width: 52.0,
                height: 22.0,
                kind: BrickKind::Armored,
            },
            GameEvent::LevelCleared,
        ];

        let run = || {
            let mut ghosts = Vec::new();
            ingest_brick_destroyed_events(&mut ghosts, &events);
            for _ in 0..15 {
                // 0.25s -- partway through the 0.5s tumble, both still alive.
                for g in &mut ghosts {
                    g.step(1.0 / 60.0);
                }
            }
            ghosts
        };

        let a = run();
        let b = run();
        assert_eq!(a, b, "same seeded event stream must replay identically");
        assert_eq!(
            a.len(),
            2,
            "only the two BrickDestroyedAt events spawn a ghost"
        );
        assert!(a.iter().all(TumbleGhost::alive), "0.25s < 0.5s duration");
    }

    #[test]
    fn ingest_brick_destroyed_events_caps_at_max_destroy_ghosts() {
        let events: Vec<GameEvent> = (0..(MAX_DESTROY_GHOSTS + 5))
            .map(|i| GameEvent::BrickDestroyedAt {
                x: i as f32 * 10.0,
                y: 0.0,
                width: 52.0,
                height: 22.0,
                kind: BrickKind::Normal,
            })
            .collect();
        let mut ghosts = Vec::new();
        ingest_brick_destroyed_events(&mut ghosts, &events);
        assert_eq!(ghosts.len(), MAX_DESTROY_GHOSTS);
    }

    // -- camera/instance-buffer math properties (arkanoid-v2-c5) ---------
    //
    // This bead's acceptance criteria calls for exactly these two headless
    // (no-GPU) properties in addition to the tumble determinism test above:
    // a view/projection matrix round-trip, and instance buffer layout.

    use glam::Mat4;

    #[test]
    fn camera_view_proj_round_trips_world_points_through_its_inverse() {
        let aspect = 800.0 / 600.0;
        let vp = Mat4::from_cols_array(&camera_view_proj(aspect, CAMERA_EYE, CAMERA_TARGET));
        let inv = vp.inverse();

        // A spread of points across the playfield's ground plane (world-Y
        // "up" stays 0) plus one lifted to wall height -- project to clip
        // space, perspective-divide to NDC, then unproject back through
        // `inv` and confirm the same world point comes out.
        let points = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(-380.0, 0.0, -280.0),
            Vec3::new(380.0, 0.0, 280.0),
            Vec3::new(120.0, WALL_HEIGHT, -50.0),
        ];
        for p in points {
            let clip = vp * p.extend(1.0);
            assert!(clip.w > 0.0, "point must be in front of the camera");
            let ndc = clip.truncate() / clip.w;
            assert!(
                (0.0..=1.0).contains(&ndc.z),
                "directx-convention NDC z must land in [0, 1], got {}",
                ndc.z
            );

            // Unproject: NDC -> clip (undo the perspective divide) -> world.
            let clip_back = ndc.extend(1.0) * clip.w;
            let world_back = inv * clip_back;
            let world_back = world_back.truncate() / world_back.w;
            // Tolerance is sub-pixel (world units == logical pixels here)
            // but well above the f32 round-off this matrix inversion picks
            // up at ~900-2000 unit camera distances -- tight enough to
            // catch a real bug (wrong handedness, missing `w` divide, wrong
            // FOV convention) while not flaking on float noise.
            assert!(
                (world_back - p).length() < 0.1,
                "round trip drifted: {p:?} -> {world_back:?}"
            );
        }
    }

    #[test]
    fn camera_view_proj_moves_only_off_axis_points_when_aspect_changes() {
        // Resizing the window changes `aspect` but never the fixed eye/
        // target/FOV (see `camera_view_proj`'s doc comment) -- a point
        // directly on the camera's view axis must land at the same NDC no
        // matter the aspect ratio; only an off-axis point's NDC x should
        // move, since aspect only rescales the horizontal FOV.
        let on_axis = CAMERA_TARGET;
        let off_axis = CAMERA_TARGET + Vec3::new(200.0, 0.0, 0.0);
        let ndc = |aspect: f32, p: Vec3| {
            let vp = Mat4::from_cols_array(&camera_view_proj(aspect, CAMERA_EYE, CAMERA_TARGET));
            let clip = vp * p.extend(1.0);
            clip.truncate() / clip.w
        };

        let on_axis_narrow = ndc(1.0, on_axis);
        let on_axis_wide = ndc(2.0, on_axis);
        assert!((on_axis_narrow.x - on_axis_wide.x).abs() < 1e-5);
        assert!((on_axis_narrow.y - on_axis_wide.y).abs() < 1e-5);

        let off_axis_narrow = ndc(1.0, off_axis);
        let off_axis_wide = ndc(2.0, off_axis);
        assert!(
            (off_axis_narrow.x - off_axis_wide.x).abs() > 1e-3,
            "a wider aspect ratio must change an off-axis point's NDC x"
        );
    }

    #[test]
    fn instance3d_field_offsets_match_the_wgpu_vertex_attr_array() {
        // `INSTANCE_ATTRS` is built by `wgpu::vertex_attr_array!`, which
        // derives each attribute's byte offset purely from the *order* and
        // *format* of the entries listed -- it has no idea what
        // `Instance3D` actually looks like. If a field were ever added,
        // removed, or reordered without updating that macro call to match,
        // every instance this pipeline draws would silently read the wrong
        // bytes for that field. This pins the two together.
        let expected: &[(u32, usize)] = &[
            (2, std::mem::offset_of!(Instance3D, center)),
            (3, std::mem::offset_of!(Instance3D, scale)),
            (4, std::mem::offset_of!(Instance3D, color)),
            (6, std::mem::offset_of!(Instance3D, uv_rect)),
            (7, std::mem::offset_of!(Instance3D, rotation)),
            (8, std::mem::offset_of!(Instance3D, emissive)),
        ];
        assert_eq!(INSTANCE_ATTRS.len(), expected.len());
        for attr in INSTANCE_ATTRS {
            let (_, offset) = expected
                .iter()
                .find(|(loc, _)| *loc == attr.shader_location)
                .unwrap_or_else(|| {
                    panic!("no expected offset for location {}", attr.shader_location)
                });
            assert_eq!(
                attr.offset as usize, *offset,
                "shader_location {} offset drifted from Instance3D's real field layout",
                attr.shader_location
            );
        }
        // The vertex-buffer array stride the pipeline is configured with
        // (`size_of::<Instance3D>()`, see `Renderer3D::new`) must cover
        // exactly the last field with no trailing padding, or instance N+1
        // would start reading mid-struct.
        let emissive_offset = std::mem::offset_of!(Instance3D, emissive);
        assert_eq!(size_of::<Instance3D>(), emissive_offset + size_of::<f32>());
    }

    #[test]
    fn vertex3d_field_offsets_match_the_wgpu_vertex_attr_array() {
        let expected: &[(u32, usize)] = &[
            (0, std::mem::offset_of!(Vertex3D, position)),
            (1, std::mem::offset_of!(Vertex3D, normal)),
            (5, std::mem::offset_of!(Vertex3D, uv)),
        ];
        assert_eq!(VERTEX_ATTRS.len(), expected.len());
        for attr in VERTEX_ATTRS {
            let (_, offset) = expected
                .iter()
                .find(|(loc, _)| *loc == attr.shader_location)
                .unwrap_or_else(|| {
                    panic!("no expected offset for location {}", attr.shader_location)
                });
            assert_eq!(attr.offset as usize, *offset);
        }
        let uv_offset = std::mem::offset_of!(Vertex3D, uv);
        assert_eq!(size_of::<Vertex3D>(), uv_offset + size_of::<[f32; 2]>());
    }
}
