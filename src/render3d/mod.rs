//! Sibling 3D renderer (arkanoid-v2-c1): a perspective-camera, depth-tested
//! pipeline that draws the same `Game` state as `render.rs`'s classic
//! instanced-quad 2D renderer, but as instanced cubes (bricks/paddle/walls)
//! and a sphere (ball(s)), lit by one Blinn-Phong shader (key light + fill
//! ambient). Selected via `--renderer 3d` (see `cli.rs`); `render.rs` is
//! untouched and stays the default renderer for the rest of this epic.
//!
//! Scope note: this bead is the pipeline foundation only. HUD text, the
//! menu/pause/game-over/victory overlay, and the trail/flash/shake "juice"
//! `render.rs` layers on top of its own quads are explicitly out of scope
//! here -- see arkanoid-v2-c2 (event data for juice), arkanoid-v2-c3
//! (juice itself), and arkanoid-v2-c4 (brick texturing), which build on
//! this module. What *is* drawn -- paddle, ball(s), bricks, falling
//! power-ups, and three static boundary walls -- is interpolated between
//! ticks exactly like the classic renderer, per this bead's acceptance
//! criteria.
//!
//! World space: playfield x/y (pixels, y-down, per `game.rs`) map onto this
//! renderer's ground plane as world X/Z, centered so world (0, 0, 0) is the
//! playfield's center; world Y is "up", used only to extrude otherwise-flat
//! 2D shapes into slabs the perspective camera and Blinn-Phong shader below
//! have something to shade. Nothing about *where* an entity is on the board
//! changes -- it's `render.rs`'s own layout, just given a third dimension.

use std::f32::consts::PI;
use std::mem::size_of;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use glam::camera::rh::{proj::directx, view};
use glam::Vec3;
use wgpu::util::DeviceExt;
use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::events::PowerUpKind;
use crate::game::{Ball, Brick, Game, Paddle, PLAYFIELD_HEIGHT, PLAYFIELD_WIDTH};
use crate::levels::BrickKind;
use crate::render::RenderState;

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
const MAX_CUBE_INSTANCES: usize = 1 /* paddle */ + MAX_BRICKS + MAX_POWERUPS + WALL_COUNT;
const MAX_SPHERE_INSTANCES: usize = 1 /* main ball */ + MAX_EXTRA_BALLS;

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

fn brick_color(brick: &Brick) -> [f32; 4] {
    match brick.kind {
        BrickKind::Normal => NORMAL_BRICK_COLOR,
        BrickKind::Armored if brick.hits_remaining >= 2 => ARMORED_BRICK_COLOR,
        BrickKind::Armored => ARMORED_BRICK_COLOR_HIT,
        BrickKind::Indestructible => INDESTRUCTIBLE_BRICK_COLOR,
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
fn camera_view_proj(aspect: f32) -> [f32; 16] {
    let view_mat = view::look_at_mat4(CAMERA_EYE, CAMERA_TARGET, Vec3::Y);
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
}

/// Per-instance transform + color for one cube or sphere. Deliberately just
/// a translation (`center`) and an axis-aligned `scale` -- no rotation --
/// which keeps the vertex shader's normal transform exact (inverse-
/// transpose of a diagonal scale matrix is just its reciprocal) without a
/// full per-instance normal matrix. A future juice bead adding rotation
/// (tumble) needs to extend this format; see the module doc comment.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Instance3D {
    center: [f32; 3],
    scale: [f32; 3],
    color: [f32; 4],
}

impl Instance3D {
    fn new(center: [f32; 3], scale: [f32; 3], color: [f32; 4]) -> Self {
        Self {
            center,
            scale,
            color,
        }
    }
}

/// Unit cube (-1..1 each axis), 24 vertices (4 per face) so each face keeps
/// its own flat normal -- the whole point of drawing boxes instead of
/// smooth-shaded shapes here. Hardcoded like `render.rs`'s `QUAD_VERTICES`:
/// small, fixed data with no reason to generate it at runtime.
const CUBE_VERTICES: [Vertex3D; 24] = [
    // +X
    Vertex3D {
        position: [1.0, -1.0, -1.0],
        normal: [1.0, 0.0, 0.0],
    },
    Vertex3D {
        position: [1.0, 1.0, -1.0],
        normal: [1.0, 0.0, 0.0],
    },
    Vertex3D {
        position: [1.0, 1.0, 1.0],
        normal: [1.0, 0.0, 0.0],
    },
    Vertex3D {
        position: [1.0, -1.0, 1.0],
        normal: [1.0, 0.0, 0.0],
    },
    // -X
    Vertex3D {
        position: [-1.0, -1.0, 1.0],
        normal: [-1.0, 0.0, 0.0],
    },
    Vertex3D {
        position: [-1.0, 1.0, 1.0],
        normal: [-1.0, 0.0, 0.0],
    },
    Vertex3D {
        position: [-1.0, 1.0, -1.0],
        normal: [-1.0, 0.0, 0.0],
    },
    Vertex3D {
        position: [-1.0, -1.0, -1.0],
        normal: [-1.0, 0.0, 0.0],
    },
    // +Y
    Vertex3D {
        position: [-1.0, 1.0, -1.0],
        normal: [0.0, 1.0, 0.0],
    },
    Vertex3D {
        position: [-1.0, 1.0, 1.0],
        normal: [0.0, 1.0, 0.0],
    },
    Vertex3D {
        position: [1.0, 1.0, 1.0],
        normal: [0.0, 1.0, 0.0],
    },
    Vertex3D {
        position: [1.0, 1.0, -1.0],
        normal: [0.0, 1.0, 0.0],
    },
    // -Y
    Vertex3D {
        position: [-1.0, -1.0, 1.0],
        normal: [0.0, -1.0, 0.0],
    },
    Vertex3D {
        position: [-1.0, -1.0, -1.0],
        normal: [0.0, -1.0, 0.0],
    },
    Vertex3D {
        position: [1.0, -1.0, -1.0],
        normal: [0.0, -1.0, 0.0],
    },
    Vertex3D {
        position: [1.0, -1.0, 1.0],
        normal: [0.0, -1.0, 0.0],
    },
    // +Z (toward the camera/paddle side)
    Vertex3D {
        position: [-1.0, -1.0, 1.0],
        normal: [0.0, 0.0, 1.0],
    },
    Vertex3D {
        position: [1.0, -1.0, 1.0],
        normal: [0.0, 0.0, 1.0],
    },
    Vertex3D {
        position: [1.0, 1.0, 1.0],
        normal: [0.0, 0.0, 1.0],
    },
    Vertex3D {
        position: [-1.0, 1.0, 1.0],
        normal: [0.0, 0.0, 1.0],
    },
    // -Z (far side, away from the camera)
    Vertex3D {
        position: [1.0, -1.0, -1.0],
        normal: [0.0, 0.0, -1.0],
    },
    Vertex3D {
        position: [-1.0, -1.0, -1.0],
        normal: [0.0, 0.0, -1.0],
    },
    Vertex3D {
        position: [-1.0, 1.0, -1.0],
        normal: [0.0, 0.0, -1.0],
    },
    Vertex3D {
        position: [1.0, 1.0, -1.0],
        normal: [0.0, 0.0, -1.0],
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
            vertices.push(Vertex3D {
                position: [x, y, z],
                normal: [x, y, z],
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

const VERTEX_ATTRS: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
const INSTANCE_ATTRS: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![2 => Float32x3, 3 => Float32x3, 4 => Float32x4];

const SHADER_SRC: &str = r#"
struct Camera {
    view_proj: mat4x4<f32>,
    eye_pos: vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
};
struct InstanceInput {
    @location(2) center: vec3<f32>,
    @location(3) scale: vec3<f32>,
    @location(4) color: vec4<f32>,
};
struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) color: vec4<f32>,
};

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
    let world_pos = inst.center + vert.position * inst.scale;
    // Axis-aligned, non-rotated scaling only (see `Instance3D`'s doc
    // comment): the inverse-transpose of a diagonal scale matrix is just
    // its reciprocal, so this rescale-then-normalize is the exact correct
    // normal transform for every shape this pipeline currently draws.
    let world_normal = normalize(vert.normal / inst.scale);

    var out: VertexOutput;
    out.clip_position = camera.view_proj * vec4<f32>(world_pos, 1.0);
    out.world_pos = world_pos;
    out.world_normal = world_normal;
    out.color = inst.color;
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

    let lit = in.color.rgb * (AMBIENT + diffuse) + vec3<f32>(spec, spec, spec);
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
    cube_vertex_buffer: wgpu::Buffer,
    cube_index_buffer: wgpu::Buffer,
    cube_instance_buffer: wgpu::Buffer,
    sphere_vertex_buffer: wgpu::Buffer,
    sphere_index_buffer: wgpu::Buffer,
    sphere_index_count: u32,
    sphere_instance_buffer: wgpu::Buffer,
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

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("render3d blinn-phong shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SRC.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("render3d pipeline layout"),
            bind_group_layouts: &[Some(&camera_bind_group_layout)],
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
                    // No blending needed: every instance this pipeline
                    // draws is fully opaque (no scrim/trail here, unlike
                    // `render.rs` -- see the module doc comment).
                    blend: None,
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
            cube_vertex_buffer,
            cube_index_buffer,
            cube_instance_buffer,
            sphere_vertex_buffer,
            sphere_index_buffer,
            sphere_index_count: sphere_indices.len() as u32,
            sphere_instance_buffer,
        }
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

    /// Clears the surface and depth buffer, then draws the paddle, ball(s),
    /// bricks, falling power-ups, and the three boundary walls as instances
    /// of one shared pipeline in two draw calls (cubes, then the sphere
    /// mesh). `prev`/`curr`/`alpha` are the same interpolation inputs
    /// `render::Renderer::render` takes -- see `interpolate`.
    pub fn render(
        &mut self,
        clear_color: wgpu::Color,
        prev: &RenderState,
        curr: &Game,
        alpha: f32,
    ) {
        let drawn = interpolate(prev, &RenderState::from(curr), alpha);

        let aspect = self.config.width as f32 / self.config.height as f32;
        let camera_uniform = CameraUniform {
            view_proj: camera_view_proj(aspect),
            eye_pos: [CAMERA_EYE.x, CAMERA_EYE.y, CAMERA_EYE.z, 0.0],
        };
        self.queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&camera_uniform));

        let mut cube_instances =
            Vec::with_capacity(1 + curr.bricks.len() + curr.powerups.len() + WALL_COUNT);
        let mut sphere_instances = Vec::with_capacity(1 + curr.extra_balls.len());

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

        // Bricks. `.take(MAX_BRICKS)` is defensive only -- `levels.rs`'s
        // own tests already enforce every built-in level fits this cap.
        cube_instances.extend(curr.bricks.iter().take(MAX_BRICKS).map(|brick| {
            let (x, z) = world_xz(brick.x, brick.y);
            Instance3D::new(
                [x, BRICK_THICKNESS / 2.0, z],
                [brick.width / 2.0, BRICK_THICKNESS / 2.0, brick.height / 2.0],
                brick_color(brick),
            )
        }));

        // Falling power-up capsules, colored by kind.
        cube_instances.extend(curr.powerups.iter().take(MAX_POWERUPS).map(|powerup| {
            let (x, z) = world_xz(powerup.x, powerup.y);
            Instance3D::new(
                [x, POWERUP_HALF, z],
                [POWERUP_HALF; 3],
                powerup_color(powerup.kind),
            )
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
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("render3d frame encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("render3d pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
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
        self.queue.present(surface_texture);
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
}
