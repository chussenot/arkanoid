//! wgpu setup and the single instanced-quad pipeline.
//!
//! Every entity (paddle, ball, bricks, walls, HUD) is a quad instance,
//! rendered in one or two draw calls. Populated starting at Milestone 1.

use std::collections::VecDeque;
use std::future::Future;
use std::mem::size_of;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::time::Instant;

use wgpu::util::DeviceExt;
use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::assets::{Atlas, SpriteId, UvRect, CAPSULE_SIZE};
use crate::fx::gpu::{
    adapter_line, field_view_proj, FxGpu, GpuTimer, Projection, ScreenMap, Span, Stage, StageTimes,
    SCENE_FORMAT,
};
use crate::fx::{FxFrame, Quality};
use crate::game::{
    Ball, BallMode, Brick, Game, GameState, GunKind, Paddle, GATE_REACH_ABOVE_PADDLE,
    PLAYFIELD_HEIGHT, PLAYFIELD_WIDTH,
};
use crate::hud::{self, Hud, BAND_HEIGHT};
use crate::levels::BrickKind;
use crate::palette;

/// One corner of the shared unit quad every entity is drawn from. Kept in
/// `[-1, 1]` on both axes so a vertex shader can place it with a single
/// multiply-add against an instance's `half_size`/`center`.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    corner: [f32; 2],
}

/// Two triangles covering the unit quad. No index buffer: six vertices is
/// little enough data that an index buffer would only add a second buffer
/// to manage for no real savings.
const QUAD_VERTICES: [Vertex; 6] = [
    Vertex {
        corner: [-1.0, -1.0],
    },
    Vertex {
        corner: [1.0, -1.0],
    },
    Vertex { corner: [1.0, 1.0] },
    Vertex {
        corner: [-1.0, -1.0],
    },
    Vertex { corner: [1.0, 1.0] },
    Vertex {
        corner: [-1.0, 1.0],
    },
];

/// Per-instance data for one quad: where it is, how big it is, its color
/// (a flat fill, or -- when `textured` is set -- a tint multiplied onto an
/// atlas sample), and which atlas rect (if any) to sample. Every entity
/// (paddle, ball, bricks, walls, HUD) becomes one of these; the pipeline
/// never changes.
///
/// `center`/`half_size` are in logical playfield pixels (800x600, y-down),
/// matching `Paddle`/`Ball`'s own coordinate convention in `game.rs`.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct QuadInstance {
    center: [f32; 2],
    half_size: [f32; 2],
    color: [f32; 4],
    /// Atlas UV rect this quad samples, `(u0, v0, u1, v1)` -- see
    /// `Atlas::uv_rect`. Meaningless (any value is fine) when `textured` is
    /// `0.0`.
    uv: [f32; 4],
    /// `1.0`: sample the atlas texture at `uv` and multiply by `color`.
    /// `0.0`: draw `color` flat, ignoring `uv` -- every quad's behavior
    /// before this bead, still used by quads with no sprite of their own
    /// (hit flashes, the Break gate, laser bolts).
    textured: f32,
}

impl QuadInstance {
    /// Flat-colored quad -- no atlas sample. The pre-existing entry point;
    /// every call site that isn't paddle/ball/a brick keeps using this
    /// unchanged.
    fn new(center: [f32; 2], half_size: [f32; 2], color: [f32; 4]) -> Self {
        Self {
            center,
            half_size,
            color,
            uv: [0.0; 4],
            textured: 0.0,
        }
    }

    /// Textured quad: samples `uv` out of the atlas and multiplies by
    /// `tint` (pass opaque white to draw the sprite unmodified; a flash
    /// effect wants flat `QuadInstance::new` instead, not a tinted sample --
    /// see `render()`'s hit-flash handling).
    fn new_textured(center: [f32; 2], half_size: [f32; 2], uv: UvRect, tint: [f32; 4]) -> Self {
        Self {
            center,
            half_size,
            color: tint,
            uv: [uv.u0, uv.v0, uv.u1, uv.v1],
            textured: 1.0,
        }
    }
}

/// Opaque white -- the tint that leaves a textured quad's sampled color
/// unmodified.
const WHITE_TINT: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// Largest brick grid a level can declare (spec, checked by
/// `levels.rs`'s own tests: up to 14 columns x 8 rows).
const MAX_BRICKS: usize = 14 * 8;

/// Defensive caps on `Game::extra_balls`/`Game::powerups` for sizing the
/// instance buffer -- `game.rs` doesn't hard-cap either (Multiball can be
/// caught repeatedly, several bricks can drop a powerup in the same tick),
/// so these just bound a pathological frame rather than reflect a real
/// gameplay limit; `.take(N)` at the draw site is the enforcement.
const MAX_EXTRA_BALLS: usize = 16;
const MAX_POWERUPS: usize = 16;
/// `game.rs` keeps at most 3 asteroids; this bounds the instance buffer
/// the same defensive way.
const MAX_ASTEROIDS: usize = 8;
/// `game.rs` caps bolts in flight at 16.
const MAX_BOLTS: usize = 16;

/// Falling capsules are pills (`assets::CAPSULE_SIZE`) wide enough to
/// carry their letter, which the HUD layer sets on top (`hud.rs`).
const POWERUP_HALF_SIZE: [f32; 2] = [CAPSULE_SIZE.0 / 2.0, CAPSULE_SIZE.1 / 2.0];

/// Quads in the ball's fading trail (spec: "a 4-quad fading trail").
const TRAIL_LEN: usize = 4;

/// Upper bound on how many "just destroyed" ghost quads (see
/// `bricks_just_destroyed`) get drawn in a single frame. Normally 0 or 1 --
/// one brick per ball per substep -- and up to 9 for an explosive ball's
/// blast (the struck brick and its ring); generous enough to cover a blast
/// plus a multiball/catch-up-ticks frame without sizing the instance
/// buffer for a full second copy of `MAX_BRICKS`.
const MAX_DESTROY_GHOSTS: usize = 16;

/// Fixed number of quad instances the instance buffer has room for: the
/// paddle, the ball trail, the ball, the largest possible brick grid (plus
/// its worst-case destroy-ghosts), and every capped entity list. The HUD
/// (band, scrim, lives, text) is `hud.rs`'s. Sized once at the ceiling
/// rather than resized per frame.
const MAX_QUADS: usize = 2
    + TRAIL_LEN
    + MAX_BRICKS
    + MAX_DESTROY_GHOSTS
    + MAX_EXTRA_BALLS
    + MAX_POWERUPS
    + MAX_ASTEROIDS
    + MAX_BOLTS
    + 1 /* the Break gate */;

/// The open Break gate: a bar this half-width down the right edge.
const GATE_HALF_WIDTH: f32 = 3.0;

/// Linear RGBA of `c` mixed toward white by `1 - k`: the paddle's tint while
/// a capsule's effect is on it. A full-strength tint would darken the
/// brushed metal into the capsule's own hue; this keeps the paddle metal
/// and lets the hue show.
fn paddle_tint(c: palette::Rgb, k: f32) -> [f32; 4] {
    let [r, g, b, _] = c.linear();
    [1.0 - k + k * r, 1.0 - k + k * g, 1.0 - k + k * b, 1.0]
}

/// The tint that turns the ball sprite (drawn in `palette::BALL`) into
/// `palette::ball(mode)`: the target over the base, channel by channel, so
/// the sprite's base tone lands on the target hue and its shading keeps
/// its shape. A channel above 1 is fine: the shader multiplies in linear
/// light and only the surface write clamps.
fn ball_tint(mode: Option<BallMode>) -> [f32; 4] {
    let [r, g, b, _] = palette::ball(mode).linear();
    let [br, bg, bb, _] = palette::BALL.linear();
    [r / br, g / bg, b / bb, 1.0]
}

/// How strongly a capsule's hue tints the paddle while its effect lasts.
const PADDLE_TINT_STRENGTH: f32 = 0.55;

/// Picks a brick's sprite from its kind and remaining hits. Armored bricks
/// are the one kind with two sprites: `hits_remaining` alone (no separate
/// flag) tells us whether the first hit has already landed -- mirrors
/// `assets::pack_filename`'s doc on why armored gets an intact/hit pair.
fn brick_sprite(brick: &Brick) -> SpriteId {
    match brick.kind {
        BrickKind::Normal => SpriteId::BrickNormal,
        BrickKind::Armored if brick.hits_remaining >= 2 => SpriteId::BrickArmoredIntact,
        BrickKind::Armored => SpriteId::BrickArmoredHit,
        BrickKind::Indestructible => SpriteId::BrickIndestructible,
    }
}

// -- juice: ball trail, brick hit-flash, destruction shake -----------------
//
// All three are cheap, frame-to-frame *diffs* rather than a subscription to
// `Game::events`: by the time `Renderer::render` runs, `main.rs`'s
// fixed-timestep loop has already drained whatever events this frame's
// tick(s) pushed (see events.rs's drain contract), so there is nothing left
// on `curr` to read. Comparing this frame's `Game` state against a snapshot
// of the last-rendered frame (kept on `Renderer`) sidesteps that -- and the
// diffing itself is plain data in, plain data out, so it's unit-testable
// without a GPU `Renderer` to construct, same spirit as `hud::layout`.

/// White flash color for a brick that was just hit (surviving or not) --
/// drawn flat (bypassing the atlas sample entirely, see `render()`), so it
/// reads as a distinct one-frame pop rather than just a lighter version of
/// the brick's normal sprite.
fn hit_flash_color() -> [f32; 4] {
    palette::HIT_FLASH.linear()
}

/// Spec: "screen-space shake 3 px / 80 ms on brick destruction".
const SHAKE_MAX_OFFSET: f32 = 3.0;
const SHAKE_DURATION_SECS: f32 = 0.08;

/// Alpha of the newest (closest to the ball) trail quad; older ones fade
/// linearly toward 0 from there.
const TRAIL_BASE_ALPHA: f32 = 0.5;

/// Which currently-standing bricks just took a hit and survived it (e.g. an
/// armored brick's first hit): present in both snapshots at the same
/// position, with fewer hits left now than before. Position is a brick's
/// stable identity across frames -- bricks never move, so matching by
/// `(x, y)` is matching the same brick.
fn bricks_just_hit(prev: &[Brick], curr: &[Brick]) -> Vec<(f32, f32)> {
    curr.iter()
        .filter(|b| {
            prev.iter()
                .any(|p| p.x == b.x && p.y == b.y && p.hits_remaining > b.hits_remaining)
        })
        .map(|b| (b.x, b.y))
        .collect()
}

/// Bricks present in `prev` but missing from `curr` at the same position --
/// destroyed since the last frame, and in need of one final white "ghost"
/// quad this frame so a one-hit brick actually visibly flashes instead of
/// just vanishing (it's already gone from `curr.bricks` by the time this
/// runs). Indestructible bricks are excluded: they never get removed by
/// combat, only by `advance_level`'s wholesale grid reload, which would
/// otherwise register as every leftover indestructible brick being
/// "destroyed" on a level-transition frame.
fn bricks_just_destroyed(prev: &[Brick], curr: &[Brick]) -> Vec<Brick> {
    prev.iter()
        .filter(|p| {
            p.kind != BrickKind::Indestructible && !curr.iter().any(|b| b.x == p.x && b.y == p.y)
        })
        .copied()
        .collect()
}

/// Shake time remaining after `dt` seconds elapse, refreshed back to the
/// full `SHAKE_DURATION_SECS` when `score_increased` (this frame's diff saw
/// score go up, i.e. a brick was destroyed -- see `render()`'s call site
/// for why score, not a brick-position diff, is what drives this one).
fn next_shake_remaining(current: f32, dt: f32, score_increased: bool) -> f32 {
    let refreshed = if score_increased {
        SHAKE_DURATION_SECS
    } else {
        current
    };
    (refreshed - dt).max(0.0)
}

/// Ball trail history update for one frame: cleared while the ball is
/// parked on the paddle (nothing moving to leave a trail behind), otherwise
/// the new position is appended and the oldest one dropped once there are
/// more than `TRAIL_LEN`.
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

// The shader below hardcodes the playfield size as WGSL consts (see its own
// comment for why). This guard catches the two numbers drifting apart at
// compile time instead of as a silently squished/stretched playfield if
// `game.rs`'s constants ever change.
const _: () = assert!(PLAYFIELD_WIDTH as u32 == 800 && PLAYFIELD_HEIGHT as u32 == 600);

const SHADER_SRC: &str = r#"
struct VertexInput {
    @location(0) corner: vec2<f32>,
};

struct InstanceInput {
    @location(1) center: vec2<f32>,
    @location(2) half_size: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) uv_rect: vec4<f32>,
    @location(5) textured: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) textured: f32,
};

// Fixed logical playfield (spec: 800x600, physics never changes on resize).
// Hardcoded rather than passed as a uniform: the value truly never changes,
// so a bind group would only add ceremony. The render pass's viewport is
// the letterboxed field rect (`hud::Letterbox::field_viewport`), so these
// coordinates fill the field at any window size without stretching.
const PLAYFIELD_WIDTH: f32 = 800.0;
const PLAYFIELD_HEIGHT: f32 = 600.0;

@group(0) @binding(0) var atlas_tex: texture_2d<f32>;
@group(0) @binding(1) var atlas_sampler: sampler;

@vertex
fn vs_main(vert: VertexInput, inst: InstanceInput) -> VertexOutput {
    let pixel_pos = inst.center + vert.corner * inst.half_size;
    let ndc_x = (pixel_pos.x / PLAYFIELD_WIDTH) * 2.0 - 1.0;
    let ndc_y = 1.0 - (pixel_pos.y / PLAYFIELD_HEIGHT) * 2.0;

    // Local UV of this corner within the quad: corner is [-1, 1] on both
    // axes (see `QUAD_VERTICES`), corner.y == -1 is the *top* of the quad
    // (less pixel_pos.y, since this playfield space is y-down) -- matches
    // `UvRect`/`Atlas::pixels`'s "top row first" convention, so no flip is
    // needed here.
    let local_uv = (vert.corner + vec2<f32>(1.0, 1.0)) * 0.5;

    var out: VertexOutput;
    out.clip_position = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.color = inst.color;
    out.uv = mix(inst.uv_rect.xy, inst.uv_rect.zw, local_uv);
    out.textured = inst.textured;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var c = in.color;
    if in.textured > 0.5 {
        c = textureSample(atlas_tex, atlas_sampler, in.uv) * in.color;
    }
    // The scene target is HDR (`fx::gpu`); the world stays within the
    // palette, so only effect light passes the bloom's 1.0 threshold.
    return vec4<f32>(min(c.rgb, vec3<f32>(1.0)), c.a);
}
"#;

const VERTEX_ATTRS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x2];
const INSTANCE_ATTRS: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    1 => Float32x2,
    2 => Float32x2,
    3 => Float32x4,
    4 => Float32x4,
    5 => Float32,
];

/// A snapshot of just the fields `Renderer::render` draws, read off `Game`.
/// Two of these one fixed tick apart (see `main.rs`'s loop) are what let
/// rendering interpolate positions between simulation ticks -- spec:
/// "no visible stutter when monitor Hz != 120".
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderState {
    pub paddle: Paddle,
    pub ball: Ball,
}

impl From<&Game> for RenderState {
    fn from(game: &Game) -> Self {
        Self {
            paddle: game.paddle,
            ball: game.ball,
        }
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

impl RenderState {
    /// Blend `prev` toward `curr` by `alpha` (0 = `prev`, 1 = `curr`),
    /// clamped so a caller passing a slightly-over-budget accumulator can't
    /// extrapolate past `curr`.
    ///
    /// The ball is the one exception: when it just reattached to the
    /// paddle (life lost -> respawn), `prev`'s position is wherever it fell
    /// off the bottom of the playfield, and blending toward that would
    /// draw a one-frame streak from off-screen back up to the paddle.
    /// Snapping to `curr` for that single frame instead is the simplest
    /// fix for what is otherwise a real (if small) visible glitch.
    pub fn lerp(prev: &Self, curr: &Self, alpha: f32) -> Self {
        let alpha = alpha.clamp(0.0, 1.0);
        let just_respawned = !prev.ball.attached && curr.ball.attached;
        Self {
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
                    glued: curr.ball.glued,
                }
            },
        }
    }
}

/// Owns the wgpu instance-derived state for one window: the adapter-negotiated
/// device/queue, and the surface configured to present to that window.
///
/// Draws the game world as instances of the one shared quad pipeline, inside
/// the letterboxed field; `hud.rs` draws the band, screens and text.
pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    /// The same, drawing straight to the surface (`FxGpu::direct`).
    direct_pipeline: wgpu::RenderPipeline,
    quad_vertex_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    /// Sprite pixels this session is drawing with -- kept around so
    /// `sprite_uv` calls in `render()` have somewhere to read
    /// `Atlas::uv_rect` from. The GPU-side copy lives in
    /// `sprite_atlas_bind_group`; this is the CPU-side source of truth.
    sprite_atlas: Atlas,
    /// Binds `sprite_atlas`'s uploaded texture + sampler to `fs_main`'s
    /// `@group(0)` -- see `create_atlas_bind_group`.
    sprite_atlas_bind_group: wgpu::BindGroup,
    /// The band, state screens and capsule letters (`hud.rs`), shared
    /// with the 3D renderer.
    hud: Hud,
    /// `--autopilot` is playing; the HUD says so.
    autopilot: bool,
    /// The best score the HUD shows, set by `main` (`set_record`).
    record: hud::Record,
    /// The sound setting the HUD shows by the M key (`set_sound`).
    sound: Option<&'static str>,
    /// The menu's mode picker, as `main.rs` last set it.
    menu: hud::MenuView,
    // -- juice: ball trail, brick hit-flash/destroy-ghosts, shake -- see
    // the module-level "-- juice --" comment for why this is diffed here
    // rather than read off `Game::events`.
    /// Ball's last few interpolated positions, oldest-first; see
    /// `push_trail`.
    ball_trail: VecDeque<(f32, f32)>,
    /// Snapshot of `curr.bricks` as of the last `render()` call, diffed
    /// against this frame's by `bricks_just_hit`/`bricks_just_destroyed`.
    prev_bricks: Vec<Brick>,
    /// `curr.level` as of the last `render()` call -- a change means the
    /// brick diff above would be comparing two different levels' grids, so
    /// it's skipped for that one frame (see `bricks_just_destroyed`'s doc).
    prev_level: usize,
    /// `curr.score` as of the last `render()` call -- see
    /// `next_shake_remaining`.
    prev_score: u32,
    /// Seconds of screen shake left to play; see `next_shake_remaining`.
    shake_remaining: f32,
    /// Wall-clock time of the last `render()` call, used to compute the
    /// real elapsed `dt` the shake timer decays by (frames don't map 1:1 to
    /// fixed simulation ticks, so a tick-count-based timer would drift).
    last_frame_instant: Instant,
    /// The kinetic effects' GPU side: scene target, effect geometry,
    /// bloom and composite (`fx::gpu`).
    fx: FxGpu,
}

impl Renderer {
    /// Negotiates an adapter/device for `window` and configures its surface
    /// (sRGB format, vsync-on present mode).
    ///
    /// Blocks on adapter/device acquisition -- this only runs once at
    /// startup, so synchronous is simplest.
    ///
    /// `sprite_atlas` is whatever `assets::TextureSource::load` produced
    /// (procedural or an on-disk pack, `TextureSource::load`'s contract
    /// guarantees it's always a drawable atlas either way) -- uploaded once
    /// here as a texture the whole session's paddle/ball/brick quads sample
    /// from.
    pub fn new(
        window: Arc<Window>,
        sprite_atlas: Atlas,
        autopilot: bool,
        quality: Quality,
    ) -> Self {
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
        eprintln!("{}", adapter_line(&adapter.get_info()));

        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("bryx device"),
            required_features: GpuTimer::features(&adapter),
            ..Default::default()
        }))
        .expect("failed to request wgpu device");

        let caps = surface.get_capabilities(&adapter);
        // sRGB per spec; fall back to the adapter's top preference if it
        // somehow offers no sRGB format at all.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);
        // Vsync on: FifoRelaxed where supported (avoids a stutter when the
        // frame misses vsync by a hair), Fifo otherwise -- both are
        // guaranteed-supported-or-better vsync modes.
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

        let (sprite_atlas_bind_group_layout, sprite_atlas_bind_group) =
            Self::create_atlas_bind_group(&device, &queue, &sprite_atlas);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("quad shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SRC.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("quad pipeline layout"),
            bind_group_layouts: &[Some(&sprite_atlas_bind_group_layout)],
            immediate_size: 0,
        });
        // One pipeline per target: the HDR scene for frames with effect
        // light or a shockwave, the surface itself for the rest (`fx::gpu`).
        let pipeline_for = |format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("quad pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[
                        Some(wgpu::VertexBufferLayout {
                            array_stride: size_of::<Vertex>() as wgpu::BufferAddress,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &VERTEX_ATTRS,
                        }),
                        Some(wgpu::VertexBufferLayout {
                            array_stride: size_of::<QuadInstance>() as wgpu::BufferAddress,
                            step_mode: wgpu::VertexStepMode::Instance,
                            attributes: &INSTANCE_ATTRS,
                        }),
                    ],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        // Alpha blending, not `REPLACE`: every opaque quad
                        // (paddle/ball/bricks) has alpha 1.0, so
                        // blending produces the exact same pixels `REPLACE`
                        // did -- but it's also what lets the semi-transparent
                        // ball trail and explosions actually blend instead of just
                        // painting over the scene as a solid rectangle.
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipeline = pipeline_for(SCENE_FORMAT);
        let direct_pipeline = pipeline_for(config.format);

        let quad_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("quad vertices"),
            contents: bytemuck::cast_slice(&QUAD_VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });
        // Written fresh every frame via `queue.write_buffer` in `render()`,
        // so no initial contents -- just reserve room for MAX_QUADS.
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quad instances"),
            size: (size_of::<QuadInstance>() * MAX_QUADS) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let hud = Hud::new(&device, &queue, config.format, SCENE_FORMAT);
        let fx = FxGpu::new(
            &device,
            &queue,
            config.format,
            (config.width, config.height),
            Projection::Field,
            quality,
        );

        Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            direct_pipeline,
            quad_vertex_buffer,
            instance_buffer,
            sprite_atlas,
            sprite_atlas_bind_group,
            hud,
            autopilot,
            record: hud::Record::default(),
            sound: None,
            menu: hud::MenuView::default(),
            ball_trail: VecDeque::new(),
            prev_bricks: Vec::new(),
            prev_level: 0,
            prev_score: 0,
            shake_remaining: 0.0,
            last_frame_instant: Instant::now(),
            fx,
        }
    }

    /// Uploads `atlas`'s pixels as a wgpu texture and builds the bind
    /// group (plus its layout, needed once more to build the pipeline
    /// layout) that `fs_main`'s `@group(0)` samples through -- the one seam
    /// between this bead's atlas plumbing and the rest of the pipeline.
    fn create_atlas_bind_group(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        atlas: &Atlas,
    ) -> (wgpu::BindGroupLayout, wgpu::BindGroup) {
        let size = wgpu::Extent3d {
            width: atlas.width,
            height: atlas.height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sprite atlas texture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // sRGB: the atlas holds sRGB-encoded bytes (pack PNGs and the
            // recipes' `palette` colors alike), so sampling decodes them to
            // linear, matching every flat `palette::Rgb::linear` color, and
            // the sRGB surface encodes the result once on output.
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
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
            size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        // Linear, not nearest: sprites are drawn at whatever on-screen size
        // the paddle/ball/brick quads happen to be, essentially never a 1:1
        // pixel match with a `CELL_SIZE`-square cell, so smoothing the
        // scale reads better than a blocky nearest-neighbor blowup. No
        // mipmap chain (`mip_level_count: 1` above) -- this atlas is small
        // and never drawn shrunk enough for aliasing to matter.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sprite atlas sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sprite atlas bind group layout"),
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
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sprite atlas bind group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        (layout, bind_group)
    }

    /// Reconfigures the surface after a window resize.
    ///
    /// A zero-area size (window minimized) is skipped: wgpu forbids
    /// configuring to zero, and there is nothing to render to anyway.
    /// The letterbox is recomputed from the surface size every frame
    /// (`hud::Letterbox::fit`), so nothing else changes here.
    pub fn resize(&mut self, new_size: PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }
        self.config.width = new_size.width;
        self.config.height = new_size.height;
        self.surface.configure(&self.device, &self.config);
        self.fx
            .resize(&self.device, (new_size.width, new_size.height));
    }

    /// Clears the surface to `clear_color`, then draws the paddle, ball, and
    /// every brick still standing on `curr`'s current level as instances of
    /// the shared quad pipeline in a single draw call.
    ///
    /// `prev`/`curr` are the render-relevant `Game` state one fixed tick
    /// apart and `alpha` is how far into that tick the current wall-clock
    /// frame falls (`accumulator / dt_fixed`, 0..=1) -- see `RenderState`
    /// for why interpolating between them is what keeps motion smooth
    /// when the display's refresh rate isn't a multiple of 120 Hz. Bricks
    /// don't move, so they're read straight off `curr.bricks` with no
    /// interpolation -- a level transition (bricks cleared, the next
    /// level's grid loaded) is just `curr.bricks` already being a different
    /// `Vec` by the time this runs; there's no separate state to reconcile.
    /// The best score to show from the next frame on (see `record.rs`).
    pub fn set_record(&mut self, record: hud::Record) {
        self.record = record;
    }

    /// The sound setting to show by the M key (`None`: no sound to set).
    /// The menu's mode picker, drawn on the Menu screen.
    pub fn set_menu(&mut self, view: &hud::MenuView) {
        if self.menu != *view {
            self.menu = view.clone();
        }
    }

    pub fn set_sound(&mut self, label: Option<&'static str>) {
        self.sound = label;
    }

    /// Draws one frame to the window's surface and presents it.
    pub fn render(
        &mut self,
        clear_color: wgpu::Color,
        prev: &RenderState,
        curr: &Game,
        alpha: f32,
        fx: &FxFrame,
    ) {
        let surface_texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            // Surface config no longer matches the window; reconfigure and
            // pick it up next frame instead of presenting a stale frame.
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            // Transient: nothing to draw to right now, try again next frame.
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => return,
        };

        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.render_into(clear_color, prev, curr, alpha, fx, &view);
        self.queue.present(surface_texture);
    }

    /// The configured surface size and the HDR scene target's, for the
    /// bench's header.
    #[allow(dead_code)]
    pub fn bench_sizes(&self) -> ((u32, u32), (u32, u32)) {
        (
            (self.config.width, self.config.height),
            self.fx.scene_size(),
        )
    }

    /// The same frame into an offscreen texture, waiting for the GPU to
    /// finish it: for `src/bin/bench_render3d.rs`, which times real work
    /// with no swapchain pacing in the loop (see `Renderer3D`'s twin).
    /// Returns the frame's GPU stage times when `BRYX_GPU_TIMING=1`.
    #[allow(dead_code)]
    pub fn render_offscreen_for_bench(
        &mut self,
        clear_color: wgpu::Color,
        prev: &RenderState,
        curr: &Game,
        alpha: f32,
        fx: &FxFrame,
    ) -> Option<StageTimes> {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("render offscreen bench target"),
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
        self.render_into(clear_color, prev, curr, alpha, fx, &view);
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll failed");
        self.fx.read_timestamps(&self.device)
    }

    #[allow(clippy::too_many_arguments)]
    fn render_into(
        &mut self,
        clear_color: wgpu::Color,
        prev: &RenderState,
        curr: &Game,
        alpha: f32,
        fx: &FxFrame,
        view: &wgpu::TextureView,
    ) {
        let drawn = RenderState::lerp(prev, &RenderState::from(curr), alpha);

        // -- juice bookkeeping: see the "-- juice --" comment above
        // `bricks_just_hit` for why this is a diff against last frame
        // rather than a read of `Game::events`.
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame_instant).as_secs_f32();
        self.last_frame_instant = now;

        let score_increased = curr.score > self.prev_score;
        self.prev_score = curr.score;
        self.shake_remaining = next_shake_remaining(self.shake_remaining, dt, score_increased);
        let shake = if self.shake_remaining > 0.0 && !hud::reduced_motion() {
            let amplitude = SHAKE_MAX_OFFSET * (self.shake_remaining / SHAKE_DURATION_SECS);
            [
                rand::random_range(-amplitude..=amplitude),
                rand::random_range(-amplitude..=amplitude),
            ]
        } else {
            [0.0, 0.0]
        };
        let shaken = |x: f32, y: f32| [x + shake[0], y + shake[1]];

        // Level change: `advance_level` reloads `bricks` wholesale rather
        // than only removing what combat actually destroyed, so the diff
        // below would misread every leftover brick as "destroyed" -- skip
        // it for this one frame instead (see `bricks_just_destroyed`'s doc).
        // On the menu the whole game is swapped for the picked mode's
        // (`menu.rs`), with no brick broken: no diff either.
        let level_changed = curr.level != self.prev_level || curr.state == GameState::Menu;
        self.prev_level = curr.level;
        // With the kinetic effects on, impact light is theirs (`fx/`): these
        // one-frame white flashes are the fallback for `BRYX_FX=off`, not
        // a second flash stacked on the first.
        let legacy = self.fx.quality() == Quality::Off;
        let (hit_flash, destroyed_ghosts) = if level_changed || !legacy {
            (Vec::new(), Vec::new())
        } else {
            (
                bricks_just_hit(&self.prev_bricks, &curr.bricks),
                bricks_just_destroyed(&self.prev_bricks, &curr.bricks)
                    .into_iter()
                    .take(MAX_DESTROY_GHOSTS)
                    .collect::<Vec<_>>(),
            )
        };
        self.prev_bricks.clear();
        self.prev_bricks.extend_from_slice(&curr.bricks);

        let mut instances = Vec::with_capacity(
            2 + TRAIL_LEN
                + curr.bricks.len()
                + destroyed_ghosts.len()
                + curr.extra_balls.len().min(MAX_EXTRA_BALLS)
                + curr.powerups.len().min(MAX_POWERUPS)
                + curr.asteroids.len().min(MAX_ASTEROIDS)
                + curr.bolts.len().min(MAX_BOLTS)
                + 1,
        );
        // A power about to run out pulses its hue; otherwise a gun shows
        // over Glue: it changes what Space does.
        let reduced_motion = hud::reduced_motion();
        let paddle_tint = if let Some((hue, k)) = palette::paddle_expiry(curr, reduced_motion) {
            paddle_tint(hue, PADDLE_TINT_STRENGTH * k)
        } else {
            match curr.gun.map(|g| g.kind) {
                Some(GunKind::Laser) => paddle_tint(palette::CAPSULE_LASER, PADDLE_TINT_STRENGTH),
                Some(GunKind::SuperLaser) => {
                    paddle_tint(palette::CAPSULE_SUPER_LASER, PADDLE_TINT_STRENGTH)
                }
                None if curr.glue_timer > 0.0 => {
                    paddle_tint(palette::CAPSULE_GLUE, PADDLE_TINT_STRENGTH)
                }
                None => WHITE_TINT,
            }
        };
        instances.push(QuadInstance::new_textured(
            shaken(drawn.paddle.x, drawn.paddle.y),
            [drawn.paddle.width / 2.0, drawn.paddle.height / 2.0],
            self.sprite_atlas.uv_rect(SpriteId::Paddle),
            paddle_tint,
        ));

        // Ball's fading trail: up to `TRAIL_LEN` quads at its last few
        // interpolated positions (oldest/faintest first), drawn *before*
        // the opaque ball quad below so the ball itself always ends up on
        // top of its own trail. Uses the trail as it stood at the *start*
        // of this frame, then updates it (`push_trail`) for next frame.
        let ball_uv = self.sprite_atlas.uv_rect(SpriteId::Ball);
        let ball_tint = {
            let full = ball_tint(curr.ball_power.map(|p| p.mode));
            let k = palette::ball_power_strength(curr, reduced_motion);
            [
                1.0 + k * (full[0] - 1.0),
                1.0 + k * (full[1] - 1.0),
                1.0 + k * (full[2] - 1.0),
                1.0,
            ]
        };
        let trail_len = self.ball_trail.len().max(1) as f32;
        instances.extend(self.ball_trail.iter().enumerate().map(|(i, &(x, y))| {
            let fade = (i + 1) as f32 / trail_len;
            let radius = drawn.ball.radius * (0.5 + 0.5 * fade);
            QuadInstance::new_textured(
                shaken(x, y),
                [radius, radius],
                ball_uv,
                [
                    ball_tint[0],
                    ball_tint[1],
                    ball_tint[2],
                    TRAIL_BASE_ALPHA * fade,
                ],
            )
        }));
        push_trail(&mut self.ball_trail, &drawn.ball);

        instances.push(QuadInstance::new_textured(
            shaken(drawn.ball.x, drawn.ball.y),
            [drawn.ball.radius, drawn.ball.radius],
            ball_uv,
            ball_tint,
        ));
        // `.take(MAX_BRICKS)`: defensive only -- `levels.rs`'s own tests
        // already enforce every built-in level fits this cap. A brick that
        // was just hit and survived (e.g. armored's first hit) flashes flat
        // white for this one frame instead of its normal sprite -- a tinted
        // *sample* wouldn't pop the same way (see `hit_flash_color`'s doc).
        instances.extend(curr.bricks.iter().take(MAX_BRICKS).map(|brick| {
            let half_size = [brick.width / 2.0, brick.height / 2.0];
            let center = shaken(brick.x, brick.y);
            if hit_flash.contains(&(brick.x, brick.y)) {
                QuadInstance::new(center, half_size, hit_flash_color())
            } else {
                let uv = self.sprite_atlas.uv_rect(brick_sprite(brick));
                QuadInstance::new_textured(center, half_size, uv, WHITE_TINT)
            }
        }));
        // Bricks destroyed this frame get one last white "ghost" quad at
        // their former spot -- otherwise a one-hit brick would vanish
        // without ever visibly flashing (see `bricks_just_destroyed`).
        instances.extend(destroyed_ghosts.iter().map(|ghost| {
            QuadInstance::new(
                shaken(ghost.x, ghost.y),
                [ghost.width / 2.0, ghost.height / 2.0],
                hit_flash_color(),
            )
        }));

        // Multiball's additional balls: same look as the main ball, no
        // trail/interpolation (they're read straight off `curr`, same
        // simplification as bricks -- the main ball already carries the
        // interpolation/trail juice, and these are secondary balls that
        // only exist for a fraction of a run).
        instances.extend(curr.extra_balls.iter().take(MAX_EXTRA_BALLS).map(|ball| {
            QuadInstance::new_textured(
                shaken(ball.x, ball.y),
                [ball.radius, ball.radius],
                ball_uv,
                ball_tint,
            )
        }));

        // Falling capsules: the pill sprite in the kind's hue. Their letters
        // are text, set by the HUD layer at the same spot (`capsules`).
        let capsule_uv = self.sprite_atlas.uv_rect(SpriteId::Capsule);
        let mut capsules = Vec::with_capacity(curr.powerups.len().min(MAX_POWERUPS));
        for powerup in curr.powerups.iter().take(MAX_POWERUPS) {
            let center = shaken(powerup.x, powerup.y);
            instances.push(QuadInstance::new_textured(
                center,
                POWERUP_HALF_SIZE,
                capsule_uv,
                palette::capsule(powerup.kind).linear(),
            ));
            capsules.push((center[0], center[1] + BAND_HEIGHT, powerup.kind));
        }

        // The open Break gate: a bar in the capsule's color down the right
        // edge, from just above the paddle to the bottom. Not shaken: it is
        // part of the frame, like the walls.
        if curr.gate_open {
            let top = curr.paddle.y - GATE_REACH_ABOVE_PADDLE;
            instances.push(QuadInstance::new(
                [
                    PLAYFIELD_WIDTH - GATE_HALF_WIDTH,
                    (top + PLAYFIELD_HEIGHT) / 2.0,
                ],
                [GATE_HALF_WIDTH, (PLAYFIELD_HEIGHT - top) / 2.0],
                palette::CAPSULE_BREAK.linear(),
            ));
        }

        // Bolts in flight, flat quads in their gun's capsule color.
        instances.extend(curr.bolts.iter().take(MAX_BOLTS).map(|bolt| {
            QuadInstance::new(
                shaken(bolt.x, bolt.y),
                bolt.half_size,
                match bolt.kind {
                    GunKind::Laser => palette::CAPSULE_LASER.linear(),
                    GunKind::SuperLaser => palette::CAPSULE_SUPER_LASER.linear(),
                },
            )
        }));

        // Asteroids: the cratered rock sprite, never the ball's.
        let rock_uv = self.sprite_atlas.uv_rect(SpriteId::Asteroid);
        instances.extend(curr.asteroids.iter().take(MAX_ASTEROIDS).map(|asteroid| {
            QuadInstance::new_textured(
                shaken(asteroid.x, asteroid.y),
                [asteroid.radius, asteroid.radius],
                rock_uv,
                palette::ASTEROID.linear(),
            )
        }));

        self.queue
            .write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&instances));

        self.hud.prepare(
            &self.device,
            &self.queue,
            self.config.width,
            self.config.height,
            &hud::Frame {
                game: curr,
                autopilot: self.autopilot,
                capsules: &capsules,
                record: self.record,
                sound: self.sound,
                // Unset (no rows) draws the game's own mode alone.
                menu: (!self.menu.rows.is_empty()).then_some(&self.menu),
            },
        );

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame encoder"),
            });
        // The effects follow the field's rect on the surface, like the
        // world: the same letterbox, so a resize or a HiDPI scale moves
        // them with everything else.
        let (fx_x, fx_y, fx_w, fx_h) = self.hud.letterbox().field_viewport();
        let project = |p: [f32; 2], _h: f32| {
            Some([
                fx_x + p[0] / PLAYFIELD_WIDTH * fx_w,
                fx_y + p[1] / PLAYFIELD_HEIGHT * fx_h,
            ])
        };
        let scale = |_p: [f32; 2], _h: f32| fx_w / PLAYFIELD_WIDTH;
        let map = ScreenMap {
            surface: (self.config.width, self.config.height),
            field: (fx_x, fx_y, fx_w, fx_h),
            project: &project,
            scale: &scale,
        };
        self.fx.prepare(&self.queue, fx, field_view_proj(), &map);
        let direct = self.fx.direct();
        {
            // The scene: band and field ground, then the world inside the
            // field's letterboxed rect, then the effects' geometry over it,
            // into the HDR target, or straight onto the surface on a frame
            // with nothing to post-process (`fx::gpu`).
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("quad pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: self.fx.scene_view(view),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear_color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: self.fx.timestamps(Stage::Scene, Span::Whole),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.hud.draw_backdrop(&mut pass, direct);
            let (x, y, w, h) = self.hud.letterbox().field_viewport();
            pass.set_viewport(x, y, w, h, 0.0, 1.0);
            pass.set_pipeline(if direct {
                &self.direct_pipeline
            } else {
                &self.pipeline
            });
            pass.set_bind_group(0, &self.sprite_atlas_bind_group, &[]);
            pass.set_vertex_buffer(0, self.quad_vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
            pass.draw(0..QUAD_VERTICES.len() as u32, 0..instances.len() as u32);
            self.fx.draw_geometry(&mut pass);
            if direct {
                // Nothing to post-process: the HUD goes on in the same
                // pass, one pass for the whole frame, as before effects.
                self.hud.draw_overlay(&mut pass);
            }
        }
        if !direct {
            // Bloom and shockwaves, scene to surface; then the HUD on top,
            // never post-processed.
            self.fx.finish(&mut encoder, view);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hud pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: self.fx.timestamps(Stage::Hud, Span::Whole),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.hud.draw_overlay(&mut pass);
        }
        self.fx.resolve_timestamps(&mut encoder);
        self.queue.submit(std::iter::once(encoder.finish()));
        // Reclaims glyph atlas space for text that stopped being drawn.
        self.hud.trim();
    }
}

/// Minimal single-purpose executor for the one-shot startup futures wgpu
/// hands back (`request_adapter`/`request_device`). Native backends resolve
/// these without ever really suspending; pulling in a full async runtime
/// crate just to drive two startup calls isn't in this project's dependency
/// budget.
///
/// ponytail: parks/wakes the calling thread rather than running a real
/// reactor. Fine for a couple of startup awaits; revisit with a real
/// executor (or add one to the dependency budget) if async work lands on a
/// hot path later.
fn block_on<F: Future>(future: F) -> F::Output {
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

    fn brick(kind: BrickKind, hits_remaining: u8) -> Brick {
        brick_at(0.0, 0.0, kind, hits_remaining)
    }

    fn brick_at(x: f32, y: f32, kind: BrickKind, hits_remaining: u8) -> Brick {
        Brick {
            x,
            y,
            width: 52.0,
            height: 22.0,
            kind,
            hits_remaining,
            score: 10,
        }
    }

    #[test]
    fn armored_brick_sprite_changes_after_its_first_hit() {
        let fresh = brick_sprite(&brick(BrickKind::Armored, 2));
        let hit = brick_sprite(&brick(BrickKind::Armored, 1));
        assert_ne!(fresh, hit, "the first hit must visibly change its sprite");
        assert_eq!(fresh, SpriteId::BrickArmoredIntact);
        assert_eq!(hit, SpriteId::BrickArmoredHit);
    }

    #[test]
    fn each_brick_kind_gets_its_own_distinct_sprite() {
        let normal = brick_sprite(&brick(BrickKind::Normal, 1));
        let armored = brick_sprite(&brick(BrickKind::Armored, 2));
        let indestructible = brick_sprite(&brick(BrickKind::Indestructible, 0));
        assert_ne!(normal, armored);
        assert_ne!(normal, indestructible);
        assert_ne!(armored, indestructible);
    }

    #[test]
    fn bricks_just_hit_flags_a_surviving_armored_brick_after_its_first_hit() {
        let prev = vec![brick_at(100.0, 50.0, BrickKind::Armored, 2)];
        let curr = vec![brick_at(100.0, 50.0, BrickKind::Armored, 1)];

        let hit = bricks_just_hit(&prev, &curr);

        assert_eq!(hit, vec![(100.0, 50.0)]);
    }

    #[test]
    fn bricks_just_hit_ignores_bricks_whose_hit_count_is_unchanged() {
        let prev = vec![brick_at(100.0, 50.0, BrickKind::Armored, 2)];
        let curr = vec![brick_at(100.0, 50.0, BrickKind::Armored, 2)];

        assert!(bricks_just_hit(&prev, &curr).is_empty());
    }

    #[test]
    fn bricks_just_destroyed_flags_a_removed_destructible_brick() {
        let prev = vec![
            brick_at(100.0, 50.0, BrickKind::Normal, 1),
            brick_at(200.0, 50.0, BrickKind::Normal, 1),
        ];
        // Only the first brick survives into this frame.
        let curr = vec![brick_at(200.0, 50.0, BrickKind::Normal, 1)];

        let ghosts = bricks_just_destroyed(&prev, &curr);

        assert_eq!(ghosts.len(), 1);
        assert_eq!(ghosts[0].x, 100.0);
    }

    #[test]
    fn bricks_just_destroyed_ignores_an_indestructible_brick_dropped_by_a_level_reload() {
        // Simulates `advance_level`'s wholesale grid reload: an
        // indestructible brick vanishes between frames without ever having
        // been destroyed by combat.
        let prev = vec![brick_at(100.0, 50.0, BrickKind::Indestructible, 0)];
        let curr: Vec<Brick> = Vec::new();

        assert!(
            bricks_just_destroyed(&prev, &curr).is_empty(),
            "an indestructible brick disappearing must not register as destroyed"
        );
    }

    #[test]
    fn next_shake_remaining_refreshes_on_a_score_increase_and_decays_otherwise() {
        let refreshed = next_shake_remaining(0.0, 0.01, true);
        assert_eq!(refreshed, SHAKE_DURATION_SECS - 0.01);

        let decayed = next_shake_remaining(0.05, 0.02, false);
        assert!((decayed - 0.03).abs() < 1e-6);

        let floored = next_shake_remaining(0.01, 0.5, false);
        assert_eq!(floored, 0.0, "must not go negative");
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
            glued: None,
        };

        push_trail(&mut trail, &ball);
        assert!(trail.is_empty(), "an attached ball leaves no trail");

        ball.attached = false;
        for i in 0..(TRAIL_LEN as i32 + 3) {
            ball.x = i as f32;
            push_trail(&mut trail, &ball);
        }
        assert_eq!(trail.len(), TRAIL_LEN, "trail must be capped at TRAIL_LEN");
        assert_eq!(
            trail.back(),
            Some(&(ball.x, ball.y)),
            "the most recent position must be the newest entry"
        );

        ball.attached = true;
        push_trail(&mut trail, &ball);
        assert!(trail.is_empty(), "re-attaching (respawn) clears the trail");
    }

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
                glued: None,
            },
        }
    }

    #[test]
    fn lerp_at_zero_and_one_returns_the_endpoints_unchanged() {
        let prev = state(100.0, 100.0, 200.0, false);
        let curr = state(140.0, 150.0, 180.0, false);

        assert_eq!(RenderState::lerp(&prev, &curr, 0.0), prev);
        assert_eq!(RenderState::lerp(&prev, &curr, 1.0), curr);
    }

    #[test]
    fn lerp_at_half_is_the_midpoint_of_both_positions() {
        let prev = state(100.0, 100.0, 200.0, false);
        let curr = state(140.0, 150.0, 180.0, false);

        let mid = RenderState::lerp(&prev, &curr, 0.5);

        assert!((mid.paddle.x - 120.0).abs() < 1e-4);
        assert!((mid.ball.x - 125.0).abs() < 1e-4);
        assert!((mid.ball.y - 190.0).abs() < 1e-4);
    }

    #[test]
    fn lerp_clamps_an_out_of_range_alpha() {
        let prev = state(100.0, 100.0, 200.0, false);
        let curr = state(140.0, 150.0, 180.0, false);

        assert_eq!(RenderState::lerp(&prev, &curr, -1.0), prev);
        assert_eq!(RenderState::lerp(&prev, &curr, 2.0), curr);
    }

    #[test]
    fn lerp_snaps_the_ball_to_curr_on_respawn_instead_of_streaking_in() {
        // prev: ball fell off the bottom of the playfield, still detached.
        let prev = state(400.0, 400.0, 650.0, false);
        // curr: `check_ball_lost` has already re-attached it to the paddle.
        let curr = state(400.0, 400.0, 100.0, true);

        let drawn = RenderState::lerp(&prev, &curr, 0.5);

        assert_eq!(drawn.ball, curr.ball, "no blending through the gap");
        // The paddle (unaffected by the respawn) still interpolates
        // normally -- the snap is ball-specific, not a whole-frame freeze.
        assert!((drawn.paddle.x - 400.0).abs() < 1e-4);
    }

    #[test]
    fn the_ball_tint_turns_the_ball_sprite_into_its_powers_hue() {
        assert_eq!(ball_tint(None), WHITE_TINT);
        let base = palette::BALL.linear();
        for mode in [BallMode::Explosive, BallMode::Metal] {
            let tint = ball_tint(Some(mode));
            let target = palette::ball(Some(mode)).linear();
            for c in 0..3 {
                assert!((base[c] * tint[c] - target[c]).abs() < 1e-5, "{mode:?}");
            }
        }
    }
}
