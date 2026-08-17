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

use glyphon::cosmic_text::Align;
use glyphon::{
    Attrs, Buffer as TextBuffer, Cache as TextCache, Family, FontSystem, Metrics, Resolution,
    Shaping, SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
};
use wgpu::util::DeviceExt;
use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::assets::{Atlas, SpriteId, UvRect};
use crate::events::PowerUpKind;
use crate::game::{Ball, Brick, Game, GameState, Paddle, PLAYFIELD_HEIGHT, PLAYFIELD_WIDTH};
use crate::levels::BrickKind;

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
    /// before this bead, still used by juice/HUD quads with no sprite of
    /// their own (trail, extra-ball dots kept simple, powerups, the scrim,
    /// life icons).
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

/// Defensive cap on how many life icons ever get drawn. `STARTING_LIVES`
/// (private to `game.rs`) is 3 and lives never increase above their
/// starting count in this game, but nothing here depends on that -- this
/// just bounds the instance buffer.
const MAX_LIFE_ICONS: usize = 10;

/// Defensive caps on `Game::extra_balls`/`Game::powerups` for sizing the
/// instance buffer -- `game.rs` doesn't hard-cap either (Multiball can be
/// caught repeatedly, several bricks can drop a powerup in the same tick),
/// so these just bound a pathological frame rather than reflect a real
/// gameplay limit; `.take(N)` at the draw site is the enforcement.
const MAX_EXTRA_BALLS: usize = 16;
const MAX_POWERUPS: usize = 16;

/// Falling power-up capsules are square, smaller than a brick but bigger
/// than the ball -- big enough to read as "catch me" at a glance.
const POWERUP_HALF_SIZE: f32 = 9.0;

/// Quads in the ball's fading trail (spec: "a 4-quad fading trail").
const TRAIL_LEN: usize = 4;

/// Upper bound on how many "just destroyed" ghost quads (see
/// `bricks_just_destroyed`) get drawn in a single frame. Normally 0 or 1 --
/// at most one brick is destroyed per ball per substep -- generous enough
/// to cover a worst-case multiball/catch-up-ticks frame without sizing the
/// instance buffer for a full second copy of `MAX_BRICKS`.
const MAX_DESTROY_GHOSTS: usize = 8;

/// Fixed number of quad instances the instance buffer has room for: the
/// paddle, the ball trail, the ball, the largest possible brick grid (plus
/// its worst-case destroy-ghosts), the dark scrim quad drawn behind
/// Menu/Paused/GameOver/Victory overlays, and the lives-icon row. Sized
/// once at the ceiling rather than resized per frame.
const MAX_QUADS: usize = 2
    + TRAIL_LEN
    + MAX_BRICKS
    + MAX_DESTROY_GHOSTS
    + 1
    + MAX_LIFE_ICONS
    + MAX_EXTRA_BALLS
    + MAX_POWERUPS;

/// Flat fallback color kept only for quads that don't carry a sprite of
/// their own (ball trail, life icons) -- the paddle and ball's own quads
/// are textured (see `render()`), not flat-filled with this.
const BALL_COLOR: [f32; 4] = [1.0, 0.78, 0.2, 1.0];

const POWERUP_WIDEN_COLOR: [f32; 4] = [0.35, 0.85, 0.40, 1.0];
const POWERUP_SLOW_COLOR: [f32; 4] = [0.35, 0.55, 0.95, 1.0];
const POWERUP_MULTIBALL_COLOR: [f32; 4] = [0.80, 0.35, 0.90, 1.0];

/// One distinct color per `PowerUpKind` so a falling capsule reads as
/// "which power-up" at a glance, no icon/text needed.
fn powerup_color(kind: PowerUpKind) -> [f32; 4] {
    match kind {
        PowerUpKind::Widen => POWERUP_WIDEN_COLOR,
        PowerUpKind::Slow => POWERUP_SLOW_COLOR,
        PowerUpKind::Multiball => POWERUP_MULTIBALL_COLOR,
    }
}

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
// without a GPU `Renderer` to construct, same spirit as `overlay_text`
// above.

/// White flash color for a brick that was just hit (surviving or not) --
/// drawn flat (bypassing the atlas sample entirely, see `render()`), so it
/// reads as a distinct one-frame pop rather than just a lighter version of
/// the brick's normal sprite.
const HIT_FLASH_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

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

// -- HUD / menu / pause / game-over / victory screens ----------------------
//
// Score/level/overlay text is drawn with glyphon in *physical* surface
// pixels (via `Renderer::to_physical`, since that's the space `Viewport`
// operates in), while the scrim and lives-icon quads below stay in the same
// *logical* 800x600 playfield space every other quad in this file uses (the
// shared vertex shader maps that space to NDC on its own). Both line up at
// the window's default 800x600 size; on a resized window they stretch
// together the same non-uniform way the brick/paddle/ball quads already do
// -- true aspect-preserving letterboxing is still a later milestone, same
// limitation `Renderer::resize` already documents.

const HUD_MARGIN: f32 = 18.0;
const HUD_FONT_SIZE: f32 = 22.0;
const HUD_LINE_HEIGHT: f32 = 26.0;
const TITLE_FONT_SIZE: f32 = 52.0;
const TITLE_LINE_HEIGHT: f32 = 58.0;
const DETAIL_FONT_SIZE: f32 = 20.0;
const DETAIL_LINE_HEIGHT: f32 = 26.0;
/// Vertical offset (logical px) of the overlay title/detail lines from the
/// playfield's vertical center.
const TITLE_OFFSET_Y: f32 = -40.0;
const DETAIL_OFFSET_Y: f32 = 30.0;

const LIFE_ICON_SIZE: f32 = 16.0;
const LIFE_ICON_GAP: f32 = 8.0;

const HUD_TEXT_COLOR: glyphon::Color = glyphon::Color::rgb(230, 232, 240);
const TITLE_TEXT_COLOR: glyphon::Color = glyphon::Color::rgb(255, 210, 90);
/// Dark, partly-transparent quad drawn over the whole playfield behind any
/// non-`Playing` overlay -- needs the pipeline's blend mode to actually
/// blend (see `Renderer::new`'s pipeline setup) rather than replace.
const SCRIM_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 0.62];

/// Title + detail line for the screen drawn on top of (Menu, Paused) or
/// instead of (GameOver, Victory) the live game whenever `state` isn't
/// `Playing`. Pure function of `Game`, so it's unit-testable without a GPU
/// -- the one branchy (5-way) piece of logic this module adds for wiring
/// the state machine to "which screen renders".
fn overlay_text(game: &Game) -> Option<(&'static str, String)> {
    match game.state {
        GameState::Menu => Some((
            "ARKANOID",
            "SPACE: Start   ARROWS / A D: Move   ESC or P: Pause".to_string(),
        )),
        GameState::Paused => Some(("PAUSED", "ESC or P: Resume".to_string())),
        GameState::GameOver => Some(("GAME OVER", format!("Final Score: {}", game.score))),
        GameState::Victory => Some(("VICTORY!", format!("Final Score: {}", game.score))),
        GameState::Playing => None,
    }
}

/// Builds a single-line (or wrapped, if it's long enough) glyphon text
/// buffer, shaped and ready to hand to `TextRenderer::prepare` via a
/// `TextArea`. `wrap_width` also doubles as the box `align` centers/aligns
/// within -- callers pass the full surface width for anything that should
/// center across the screen.
fn make_line_buffer(
    font_system: &mut FontSystem,
    text: &str,
    font_size: f32,
    line_height: f32,
    wrap_width: f32,
    align: Align,
) -> TextBuffer {
    let mut buffer = TextBuffer::new(font_system, Metrics::new(font_size, line_height));
    buffer.set_size(Some(wrap_width), None);
    buffer.set_text(
        text,
        &Attrs::new().family(Family::SansSerif),
        Shaping::Basic,
        Some(align),
    );
    buffer.shape_until_scroll(font_system, false);
    buffer
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
// so a bind group would only add ceremony. Note this stretches the 800x600
// field to fill whatever the surface size is -- proper aspect-preserving
// letterboxing on resize is deferred, same as `Renderer::resize` already
// documents; nothing new is being deferred here.
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
    if in.textured > 0.5 {
        return textureSample(atlas_tex, atlas_sampler, in.uv) * in.color;
    }
    return in.color;
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
    fn lerp(prev: &Self, curr: &Self, alpha: f32) -> Self {
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
                }
            },
        }
    }
}

/// Owns the wgpu instance-derived state for one window: the adapter-negotiated
/// device/queue, and the surface configured to present to that window.
///
/// Draws the paddle, ball, and every standing brick as instances of the one
/// shared quad pipeline; HUD lands as more instances in a later milestone.
pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
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
    // -- text: HUD score/level, menu/pause/game-over/victory overlays --
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    text_renderer: TextRenderer,
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
    pub fn new(window: Arc<Window>, sprite_atlas: Atlas) -> Self {
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
            label: Some("arkanoid device"),
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
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
                    format: config.format,
                    // Alpha blending, not `REPLACE`: every opaque quad
                    // (paddle/ball/bricks/life-icons) has alpha 1.0, so
                    // blending produces the exact same pixels `REPLACE`
                    // did -- but it's also what lets the semi-transparent
                    // menu/pause/game-over/victory scrim quad (see
                    // `SCRIM_COLOR`) actually blend instead of just
                    // painting over the scene as a solid rectangle.
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

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

        // `text_cache` isn't kept on `Renderer`: `TextAtlas::new` clones it
        // internally (it's a cheap `Arc` underneath, shared pipeline/layout
        // state), and `Viewport` doesn't need it past construction.
        let font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let text_cache = TextCache::new(&device);
        let viewport = Viewport::new(&device, &text_cache);
        let mut atlas = TextAtlas::new(&device, &queue, &text_cache, config.format);
        let text_renderer =
            TextRenderer::new(&mut atlas, &device, wgpu::MultisampleState::default(), None);

        Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            quad_vertex_buffer,
            instance_buffer,
            sprite_atlas,
            sprite_atlas_bind_group,
            font_system,
            swash_cache,
            viewport,
            atlas,
            text_renderer,
            ball_trail: VecDeque::new(),
            prev_bricks: Vec::new(),
            prev_level: 0,
            prev_score: 0,
            shake_remaining: 0.0,
            last_frame_instant: Instant::now(),
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
            // Not a `*Srgb` format: `assets.rs`'s recipes (and every flat
            // color constant in this file) already work in the same plain
            // 0..1 space `fs_main` writes straight to an sRGB *surface* --
            // sampling this texture must hand those bytes back unchanged,
            // not sRGB-decode them a second time.
            format: wgpu::TextureFormat::Rgba8Unorm,
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
    /// Scaling/letterboxing the fixed 800x600 playfield into the new size is
    /// a later milestone -- this just keeps the surface valid so resizing
    /// doesn't crash.
    pub fn resize(&mut self, new_size: PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }
        self.config.width = new_size.width;
        self.config.height = new_size.height;
        self.surface.configure(&self.device, &self.config);
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
    pub fn render(
        &mut self,
        clear_color: wgpu::Color,
        prev: &RenderState,
        curr: &Game,
        alpha: f32,
    ) {
        let drawn = RenderState::lerp(prev, &RenderState::from(curr), alpha);
        let overlay = overlay_text(curr);

        // -- juice bookkeeping: see the "-- juice --" comment above
        // `bricks_just_hit` for why this is a diff against last frame
        // rather than a read of `Game::events`.
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame_instant).as_secs_f32();
        self.last_frame_instant = now;

        let score_increased = curr.score > self.prev_score;
        self.prev_score = curr.score;
        self.shake_remaining = next_shake_remaining(self.shake_remaining, dt, score_increased);
        let shake = if self.shake_remaining > 0.0 {
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
        let level_changed = curr.level != self.prev_level;
        self.prev_level = curr.level;
        let (hit_flash, destroyed_ghosts) = if level_changed {
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
                + 1
                + MAX_LIFE_ICONS
                + curr.extra_balls.len().min(MAX_EXTRA_BALLS)
                + curr.powerups.len().min(MAX_POWERUPS),
        );
        instances.push(QuadInstance::new_textured(
            shaken(drawn.paddle.x, drawn.paddle.y),
            [drawn.paddle.width / 2.0, drawn.paddle.height / 2.0],
            self.sprite_atlas.uv_rect(SpriteId::Paddle),
            WHITE_TINT,
        ));

        // Ball's fading trail: up to `TRAIL_LEN` quads at its last few
        // interpolated positions (oldest/faintest first), drawn *before*
        // the opaque ball quad below so the ball itself always ends up on
        // top of its own trail. Uses the trail as it stood at the *start*
        // of this frame, then updates it (`push_trail`) for next frame.
        let trail_len = self.ball_trail.len().max(1) as f32;
        instances.extend(self.ball_trail.iter().enumerate().map(|(i, &(x, y))| {
            let fade = (i + 1) as f32 / trail_len;
            let radius = drawn.ball.radius * (0.5 + 0.5 * fade);
            QuadInstance::new(
                shaken(x, y),
                [radius, radius],
                [
                    BALL_COLOR[0],
                    BALL_COLOR[1],
                    BALL_COLOR[2],
                    TRAIL_BASE_ALPHA * fade,
                ],
            )
        }));
        push_trail(&mut self.ball_trail, &drawn.ball);

        let ball_uv = self.sprite_atlas.uv_rect(SpriteId::Ball);
        instances.push(QuadInstance::new_textured(
            shaken(drawn.ball.x, drawn.ball.y),
            [drawn.ball.radius, drawn.ball.radius],
            ball_uv,
            WHITE_TINT,
        ));
        // `.take(MAX_BRICKS)`: defensive only -- `levels.rs`'s own tests
        // already enforce every built-in level fits this cap. A brick that
        // was just hit and survived (e.g. armored's first hit) flashes flat
        // white for this one frame instead of its normal sprite -- a tinted
        // *sample* wouldn't pop the same way (see `HIT_FLASH_COLOR`'s doc).
        instances.extend(curr.bricks.iter().take(MAX_BRICKS).map(|brick| {
            let half_size = [brick.width / 2.0, brick.height / 2.0];
            let center = shaken(brick.x, brick.y);
            if hit_flash.contains(&(brick.x, brick.y)) {
                QuadInstance::new(center, half_size, HIT_FLASH_COLOR)
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
                HIT_FLASH_COLOR,
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
                WHITE_TINT,
            )
        }));

        // Falling power-up capsules, colored by kind (see `powerup_color`).
        instances.extend(curr.powerups.iter().take(MAX_POWERUPS).map(|powerup| {
            QuadInstance::new(
                shaken(powerup.x, powerup.y),
                [POWERUP_HALF_SIZE, POWERUP_HALF_SIZE],
                powerup_color(powerup.kind),
            )
        }));

        // Menu/Paused/GameOver/Victory: dim the scene before the icons and
        // overlay text are drawn on top of it, so HUD chrome stays legible.
        // Not shaken: HUD chrome stays put even while the world does.
        if overlay.is_some() {
            instances.push(QuadInstance::new(
                [PLAYFIELD_WIDTH / 2.0, PLAYFIELD_HEIGHT / 2.0],
                [PLAYFIELD_WIDTH / 2.0, PLAYFIELD_HEIGHT / 2.0],
                SCRIM_COLOR,
            ));
        }

        // Lives, top-right, nearest-to-the-edge icon first. Not shaken,
        // same reasoning as the scrim above.
        let life_icon_count = (curr.lives as usize).min(MAX_LIFE_ICONS);
        for i in 0..life_icon_count {
            let step = LIFE_ICON_SIZE + LIFE_ICON_GAP;
            let x = PLAYFIELD_WIDTH - HUD_MARGIN - LIFE_ICON_SIZE / 2.0 - i as f32 * step;
            let y = HUD_MARGIN + LIFE_ICON_SIZE / 2.0;
            instances.push(QuadInstance::new(
                [x, y],
                [LIFE_ICON_SIZE / 2.0, LIFE_ICON_SIZE / 2.0],
                BALL_COLOR,
            ));
        }

        self.queue
            .write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&instances));

        // -- text: score (top-left), level (top-center), and the overlay
        // title/detail (Menu/Paused/GameOver/Victory) if any -- built fresh
        // every frame. These are a handful of short strings redrawn at
        // display refresh rate, not a hot simulation path, so reshaping
        // them each call is the simplest correct thing to do.
        //
        // ponytail: no persistent-buffer/dirty-tracking cache here; add one
        // if profiling ever shows text shaping actually costing a frame.
        let surface_width = self.config.width as f32;
        let score_text = format!("Score: {}", curr.score);
        let level_text = format!("Level {}", curr.level);
        let score_buffer = make_line_buffer(
            &mut self.font_system,
            &score_text,
            HUD_FONT_SIZE,
            HUD_LINE_HEIGHT,
            surface_width,
            Align::Left,
        );
        let level_buffer = make_line_buffer(
            &mut self.font_system,
            &level_text,
            HUD_FONT_SIZE,
            HUD_LINE_HEIGHT,
            surface_width,
            Align::Center,
        );
        let overlay_buffers = overlay.map(|(title, detail)| {
            let title_buffer = make_line_buffer(
                &mut self.font_system,
                title,
                TITLE_FONT_SIZE,
                TITLE_LINE_HEIGHT,
                surface_width,
                Align::Center,
            );
            let detail_buffer = make_line_buffer(
                &mut self.font_system,
                &detail,
                DETAIL_FONT_SIZE,
                DETAIL_LINE_HEIGHT,
                surface_width,
                Align::Center,
            );
            (title_buffer, detail_buffer)
        });

        let (hud_margin_x, hud_margin_y) = self.to_physical(HUD_MARGIN, HUD_MARGIN);
        let mut text_areas = vec![
            Self::text_area(&score_buffer, hud_margin_x, hud_margin_y, HUD_TEXT_COLOR),
            Self::text_area(&level_buffer, 0.0, hud_margin_y, HUD_TEXT_COLOR),
        ];
        if let Some((title_buffer, detail_buffer)) = &overlay_buffers {
            let (_, title_y) = self.to_physical(0.0, PLAYFIELD_HEIGHT / 2.0 + TITLE_OFFSET_Y);
            let (_, detail_y) = self.to_physical(0.0, PLAYFIELD_HEIGHT / 2.0 + DETAIL_OFFSET_Y);
            text_areas.push(Self::text_area(
                title_buffer,
                0.0,
                title_y,
                TITLE_TEXT_COLOR,
            ));
            text_areas.push(Self::text_area(
                detail_buffer,
                0.0,
                detail_y,
                HUD_TEXT_COLOR,
            ));
        }

        self.viewport.update(
            &self.queue,
            Resolution {
                width: self.config.width,
                height: self.config.height,
            },
        );
        self.text_renderer
            .prepare(
                &self.device,
                &self.queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                text_areas,
                &mut self.swash_cache,
            )
            .expect("glyphon text preparation failed");

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
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame encoder"),
            });
        {
            // Scoped: the render pass borrows `encoder` and must be dropped
            // (ending the pass) before `encoder.finish()` below.
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("quad pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear_color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.sprite_atlas_bind_group, &[]);
            pass.set_vertex_buffer(0, self.quad_vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
            pass.draw(0..QUAD_VERTICES.len() as u32, 0..instances.len() as u32);

            self.text_renderer
                .render(&self.atlas, &self.viewport, &mut pass)
                .expect("glyphon text render failed");
        }
        self.queue.submit(std::iter::once(encoder.finish()));
        self.queue.present(surface_texture);
        // Reclaims atlas space for glyphs that stopped being used this
        // frame (e.g. the previous overlay's text) -- cheap no-op most
        // frames, matters once several different screens have all drawn
        // text at some point in the session.
        self.atlas.trim();
    }

    /// Maps a point in the logical 800x600 playfield to the physical
    /// surface pixel space glyphon's `Viewport`/`TextArea` operate in,
    /// using the same non-uniform stretch the quad vertex shader already
    /// applies to every other quad (see `SHADER_SRC`) -- keeps HUD text
    /// aligned with the quads at whatever size the window's been resized
    /// to, even though true aspect-preserving letterboxing is still a
    /// later milestone (same limitation `resize` documents).
    fn to_physical(&self, x: f32, y: f32) -> (f32, f32) {
        (
            x / PLAYFIELD_WIDTH * self.config.width as f32,
            y / PLAYFIELD_HEIGHT * self.config.height as f32,
        )
    }

    /// Builds one `TextArea` covering the whole surface horizontally at
    /// `(left, top)`, uniformly colored `color` -- every text element this
    /// module draws is a single independently-positioned line/block, so
    /// they all share this same shape.
    fn text_area(buffer: &TextBuffer, left: f32, top: f32, color: glyphon::Color) -> TextArea<'_> {
        TextArea {
            buffer,
            left,
            top,
            scale: 1.0,
            bounds: TextBounds::default(),
            default_color: color,
            custom_glyphs: &[],
        }
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
    fn overlay_text_is_none_only_while_playing() {
        let mut game = Game::new();
        game.state = GameState::Playing;
        assert!(
            overlay_text(&game).is_none(),
            "no overlay should draw during Playing"
        );

        for state in [
            GameState::Menu,
            GameState::Paused,
            GameState::GameOver,
            GameState::Victory,
        ] {
            game.state = state;
            assert!(
                overlay_text(&game).is_some(),
                "{state:?} must show an overlay"
            );
        }
    }

    #[test]
    fn game_over_and_victory_overlays_show_the_final_score() {
        let mut game = Game::new();
        game.score = 1234;

        game.state = GameState::GameOver;
        let (_, detail) = overlay_text(&game).expect("GameOver must have an overlay");
        assert!(
            detail.contains("1234"),
            "game over overlay must show the final score"
        );

        game.state = GameState::Victory;
        let (_, detail) = overlay_text(&game).expect("Victory must have an overlay");
        assert!(
            detail.contains("1234"),
            "victory overlay must show the final score"
        );
    }

    #[test]
    fn menu_overlay_has_a_title_and_mentions_the_start_key() {
        let mut game = Game::new(); // Game::new() already starts in Menu
        game.state = GameState::Menu;
        let (title, detail) = overlay_text(&game).expect("Menu must have an overlay");
        assert_eq!(title, "ARKANOID");
        assert!(
            detail.contains("SPACE"),
            "must tell the player how to start"
        );
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
    fn each_powerup_kind_gets_its_own_distinct_color() {
        let widen = powerup_color(PowerUpKind::Widen);
        let slow = powerup_color(PowerUpKind::Slow);
        let multiball = powerup_color(PowerUpKind::Multiball);
        assert_ne!(widen, slow);
        assert_ne!(widen, multiball);
        assert_ne!(slow, multiball);
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
}
