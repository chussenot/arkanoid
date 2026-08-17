//! Texture atlas abstraction (Workstream B, bead b1): the seam between
//! *where* texture pixels come from and *how* a renderer draws with them.
//! [`TextureSource`] says which; [`Atlas`] is the only type on the other
//! side of that seam. render.rs (bead b5) and c4 (`epic/presentation-3d`,
//! cross-epic reference only -- see docs/fleet-patterns.md) both consume
//! an `Atlas` and never match on `TextureSource` themselves. That's the
//! whole point: game/render code picks a source once, then only ever
//! holds an `Atlas`.
//!
//! ponytail: nothing calls into this module yet -- b5 is what wires an
//! `Atlas` into the actual draw path. `#[allow(dead_code)]` until then;
//! delete it once b5 lands.
//!
//! # Who does what (so b2/b3/b5/c4 don't have to re-read this file)
//! - **This bead**: the [`TextureSource`] enum, the [`SpriteId`] set, and
//!   [`Atlas`]'s layout/lookup/blit API, plus a flat-color placeholder
//!   builder so the type is real and testable before b2 lands.
//! - **b2** (procedural pass): replaces [`Atlas::procedural_placeholder`]'s
//!   flat fills with the real beveled/gloss/noise recipe. Same function,
//!   same call site (`TextureSource::Procedural`'s arm of
//!   [`TextureSource::load`]) -- b2 doesn't need a new entry point.
//! - **b3** (pack support): gives the `Pack(path)` arm of
//!   [`TextureSource::load`] a real body -- decode images with the
//!   `image` crate, [`Atlas::set_sprite`] each one into its
//!   [`Atlas::cell_rect`], and fall back to
//!   [`Atlas::procedural_placeholder`] with a logged warning on any
//!   I/O/decode error. Never panic: [`TextureSource::load`]'s contract is
//!   that it always returns a drawable `Atlas`.
//! - **b5** (classic renderer wiring): adds UV coordinates to
//!   `render.rs`'s `QuadInstance` and looks them up via
//!   [`Atlas::uv_rect`]; owns turning `Atlas::pixels` into a wgpu texture.
//! - **c4** (`epic/presentation-3d`, cross-epic reference only): reads
//!   this file with `git show epic/textures:src/assets.rs` and does not
//!   merge `epic/textures` -- see docs/fleet-patterns.md's "Cross-epic
//!   dependencies" section. Consumes [`Atlas`] for brick front-face
//!   texturing the same way b5 does for the classic renderer.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// Where texture pixels come from. `Procedural` is the default and can
/// never fail; `Pack` names a directory a loader (b3) reads sprite images
/// from. Turning either variant into an [`Atlas`] via
/// [`TextureSource::load`] is infallible from the caller's point of
/// view -- a missing or malformed pack degrades to `Procedural`'s pixels
/// plus a logged warning, not an `Err`. Game/render code only ever sees
/// `TextureSource -> Atlas`; it never learns which arm actually ran.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TextureSource {
    #[default]
    Procedural,
    /// Directory a pack loader (b3) reads sprite images from. Stored
    /// here (not duplicated in `cli.rs`) so [`TextureSource::load`] is
    /// the one place that turns a source into pixels; `cli.rs`'s
    /// `--assets` flag just constructs this variant from the path it
    /// parsed.
    Pack(PathBuf),
}

impl TextureSource {
    /// Builds the [`Atlas`] this source describes. Always succeeds: a bad
    /// `Pack` path falls back to the same pixels `Procedural` would have
    /// produced (see the module doc's "who does what" for which bead
    /// implements which arm's real pixels).
    pub fn load(&self) -> Atlas {
        match self {
            TextureSource::Procedural => Atlas::procedural_placeholder(),
            TextureSource::Pack(path) => Atlas::load_pack(path).unwrap_or_else(|err| {
                eprintln!(
                    "warning: failed to load asset pack from {} ({err}) -- falling back to procedural textures",
                    path.display()
                );
                Atlas::procedural_placeholder()
            }),
        }
    }
}

/// Filename a pack directory must provide for `id`, exhaustively matched
/// so adding a `SpriteId` without adding its filename here is a compile
/// error rather than a silent missing-sprite fallback at runtime.
fn pack_filename(id: SpriteId) -> &'static str {
    match id {
        SpriteId::Paddle => "paddle.png",
        SpriteId::Ball => "ball.png",
        SpriteId::BrickNormal => "brick_normal.png",
        SpriteId::BrickArmoredIntact => "brick_armored_intact.png",
        SpriteId::BrickArmoredHit => "brick_armored_hit.png",
        SpriteId::BrickIndestructible => "brick_indestructible.png",
    }
}

/// One entry per drawable thing that can carry a texture. Bricks are
/// split by *visual* state, not 1:1 with `levels::BrickKind`:
/// `BrickKind::Armored` gets two sprites (`BrickArmoredIntact` /
/// `BrickArmoredHit`, matching the two colors `render.rs::brick_color`
/// already draws) because that hit transition is meant to read as a
/// distinct texture, not a runtime tint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpriteId {
    Paddle,
    Ball,
    BrickNormal,
    BrickArmoredIntact,
    BrickArmoredHit,
    BrickIndestructible,
}

impl SpriteId {
    /// Fixes both the sprite set and its packing order in the atlas grid
    /// ([`Atlas::cell_rect`] indexes into this by position). Add new
    /// sprites at the end so existing UV rects don't shift under
    /// whoever's already reading them.
    pub const ALL: [SpriteId; 6] = [
        SpriteId::Paddle,
        SpriteId::Ball,
        SpriteId::BrickNormal,
        SpriteId::BrickArmoredIntact,
        SpriteId::BrickArmoredHit,
        SpriteId::BrickIndestructible,
    ];
}

/// Normalized UV rectangle: `(u0, v0)` top-left to `(u1, v1)`
/// bottom-right, in `[0, 1]` texture space with y-down (matching
/// wgpu/wgsl sampling convention). A renderer multiplies an instance's
/// local quad UV by this rect to sample the right sprite out of
/// [`Atlas::pixels`]. Plain `f32` fields, not tied to any GPU vertex
/// layout -- b5 is free to copy these into whatever `QuadInstance` field
/// shape it adds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UvRect {
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
}

/// Side length in pixels of the square cell every sprite is packed into.
/// One size for every sprite keeps grid packing (this bead's job) a fixed
/// lookup instead of a packing algorithm; a sprite that wants finer
/// detail is free to use more of its cell, it just can't have a *bigger*
/// cell without a layout change here.
pub const CELL_SIZE: u32 = 64;

/// Sprites per atlas row. `SpriteId::ALL`'s length divides evenly by this
/// today (6 / 3 = 2 rows); if a future sprite makes it uneven,
/// `Atlas::grid_size` still packs correctly, just with a partial last
/// row.
const GRID_COLS: u32 = 3;

/// An RGBA8 pixel buffer plus a fixed grid of sprite regions. This is the
/// only type render.rs/c4 need to hold onto -- neither ever matches on
/// [`TextureSource`].
#[derive(Debug, Clone)]
pub struct Atlas {
    pub width: u32,
    pub height: u32,
    /// Tightly packed RGBA8 (unorm), row-major, top row first --
    /// uploadable directly as a wgpu texture with `bytes_per_row = width
    /// * 4`.
    pub pixels: Vec<u8>,
}

impl Atlas {
    /// Total pixel size of an atlas packing `SpriteId::ALL.len()` sprites
    /// `GRID_COLS` wide into `CELL_SIZE` cells.
    fn grid_size() -> (u32, u32) {
        let cols = GRID_COLS.min(SpriteId::ALL.len() as u32).max(1);
        let rows = (SpriteId::ALL.len() as u32).div_ceil(cols);
        (cols * CELL_SIZE, rows * CELL_SIZE)
    }

    /// Pixel-space `(x, y, w, h)` of `id`'s cell -- e.g. for a pack
    /// loader to know where to blit a decoded sprite image.
    pub fn cell_rect(id: SpriteId) -> (u32, u32, u32, u32) {
        let index = SpriteId::ALL
            .iter()
            .position(|&s| s == id)
            .expect("SpriteId::ALL is exhaustive over SpriteId by construction");
        let cols = GRID_COLS.min(SpriteId::ALL.len() as u32).max(1);
        let col = index as u32 % cols;
        let row = index as u32 / cols;
        (col * CELL_SIZE, row * CELL_SIZE, CELL_SIZE, CELL_SIZE)
    }

    /// Normalized UV rect for `id` within this atlas's actual
    /// `width`/`height` (not just `CELL_SIZE`, so this stays correct even
    /// if a future atlas is padded/mipmapped to a larger backing size).
    pub fn uv_rect(&self, id: SpriteId) -> UvRect {
        let (x, y, w, h) = Self::cell_rect(id);
        UvRect {
            u0: x as f32 / self.width as f32,
            v0: y as f32 / self.height as f32,
            u1: (x + w) as f32 / self.width as f32,
            v1: (y + h) as f32 / self.height as f32,
        }
    }

    /// Writes one sprite's pixels into its cell. `rgba` must be exactly
    /// `CELL_SIZE * CELL_SIZE * 4` bytes (one RGBA8 pixel per texel,
    /// row-major) -- both the procedural pass (b2) and the pack loader
    /// (b3) use this, so neither needs to know how cells are laid out in
    /// the backing buffer.
    ///
    /// # Panics
    /// If `rgba.len()` isn't exactly one cell's worth of RGBA8 pixels.
    pub fn set_sprite(&mut self, id: SpriteId, rgba: &[u8]) {
        let cell_bytes = (CELL_SIZE * CELL_SIZE * 4) as usize;
        assert_eq!(
            rgba.len(),
            cell_bytes,
            "set_sprite: rgba must be exactly one {CELL_SIZE}x{CELL_SIZE} RGBA8 sprite ({cell_bytes} bytes)"
        );
        let (x, y, w, h) = Self::cell_rect(id);
        let atlas_stride = self.width as usize * 4;
        let cell_stride = w as usize * 4;
        for row in 0..h as usize {
            let src = &rgba[row * cell_stride..(row + 1) * cell_stride];
            let dst_start = (y as usize + row) * atlas_stride + x as usize * 4;
            self.pixels[dst_start..dst_start + cell_stride].copy_from_slice(src);
        }
    }

    /// Procedural texture atlas (bead b2): every sprite is generated at
    /// startup from the const-driven recipes below this `impl` block
    /// instead of loaded from disk. This is `TextureSource::Procedural`'s
    /// builder and b3's fallback when a pack fails to load.
    ///
    /// Deterministic by construction, which is what this bead's
    /// acceptance criterion ("same seed -> byte-identical atlas") reduces
    /// to: every recipe below derives its pixels -- including noise --
    /// purely from a pixel's fixed `(x, y)` position in the atlas grid via
    /// [`hash_signed`], never from wall-clock time or a stateful RNG
    /// stream. There is exactly one seed, [`NOISE_SEED`], and it's a
    /// `const`, so two calls (same process or two separate processes)
    /// always produce byte-identical pixels.
    fn procedural_placeholder() -> Atlas {
        let (width, height) = Self::grid_size();
        let mut atlas = Atlas {
            width,
            height,
            pixels: vec![0; (width * height * 4) as usize],
        };
        atlas.set_sprite(SpriteId::Paddle, &brushed_metal_paddle());
        atlas.set_sprite(SpriteId::Ball, &specular_ball());
        for &(id, base) in BRICK_PALETTE.iter() {
            atlas.set_sprite(id, &beveled_brick_panel(id, base));
        }
        atlas
    }

    /// Decodes one PNG per [`SpriteId`] out of `dir` (see [`pack_filename`]
    /// for the expected name) into a fresh atlas with the same grid layout
    /// [`procedural_placeholder`](Self::procedural_placeholder) uses.
    /// A source image that isn't exactly [`CELL_SIZE`] square is resized
    /// to fit -- `scripts/fetch-assets.sh`'s converted pack already is, but
    /// a hand-edited pack directory shouldn't have to be pixel-perfect.
    ///
    /// All-or-nothing: the first missing file or decode error aborts the
    /// whole load ([`TextureSource::load`] catches it and falls back to
    /// the full procedural atlas) rather than mixing some real sprites
    /// with some placeholders, which would be a more confusing failure
    /// mode than either atlas on its own.
    fn load_pack(dir: &Path) -> image::ImageResult<Atlas> {
        let (width, height) = Self::grid_size();
        let mut atlas = Atlas {
            width,
            height,
            pixels: vec![0; (width * height * 4) as usize],
        };
        for id in SpriteId::ALL {
            let path = dir.join(pack_filename(id));
            let sprite = image::open(&path)?
                .resize_exact(CELL_SIZE, CELL_SIZE, image::imageops::FilterType::Triangle)
                .to_rgba8();
            atlas.set_sprite(id, sprite.as_raw());
        }
        Ok(atlas)
    }
}

/// Fixed seed for every procedural noise/hash lookup in this module. The
/// *only* knob the "seed" in this bead's acceptance criterion refers to --
/// it never changes at runtime, so determinism is just "this is a plain
/// `const`, not read from the clock or an RNG."
const NOISE_SEED: u64 = 0x9E37_79B9_7F4A_7C15;

/// Color as three `0.0..=1.0` linear-ish channels, the working space for
/// every recipe below. Converted to `u8` only at the very end
/// ([`to_rgba8`]), so bevel/gloss/noise adjustments can add and multiply
/// freely without repeated round-trip rounding error.
type Rgb = [f32; 3];

fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

/// Multiplies every channel by `factor` (e.g. a gloss-ramp or noise
/// brightness multiplier), clamped back into range.
fn scale(c: Rgb, factor: f32) -> Rgb {
    [
        clamp01(c[0] * factor),
        clamp01(c[1] * factor),
        clamp01(c[2] * factor),
    ]
}

/// Blends `c` toward white by `amount` (`0` = unchanged, `1` = white) --
/// used for bevel highlight edges and the ball's specular hotspot.
fn lighten(c: Rgb, amount: f32) -> Rgb {
    [
        clamp01(c[0] + (1.0 - c[0]) * amount),
        clamp01(c[1] + (1.0 - c[1]) * amount),
        clamp01(c[2] + (1.0 - c[2]) * amount),
    ]
}

/// Blends `c` toward black by `amount` -- used for bevel shadow edges and
/// the ball's rim shading.
fn darken(c: Rgb, amount: f32) -> Rgb {
    [
        clamp01(c[0] * (1.0 - amount)),
        clamp01(c[1] * (1.0 - amount)),
        clamp01(c[2] * (1.0 - amount)),
    ]
}

fn to_rgba8(c: Rgb, alpha: u8) -> [u8; 4] {
    [
        (c[0] * 255.0).round() as u8,
        (c[1] * 255.0).round() as u8,
        (c[2] * 255.0).round() as u8,
        alpha,
    ]
}

/// Writes one `[u8; 4]` pixel into a `CELL_SIZE`-square RGBA8 buffer at
/// local `(x, y)`. Every recipe below builds its sprite into a fresh
/// buffer this way before handing it to [`Atlas::set_sprite`].
fn put_pixel(buf: &mut [u8], x: u32, y: u32, rgba: [u8; 4]) {
    let idx = ((y * CELL_SIZE + x) * 4) as usize;
    buf[idx..idx + 4].copy_from_slice(&rgba);
}

/// Cheap deterministic position hash (the MurmurHash3 finalizer mix,
/// values folded into [`NOISE_SEED`] up front). A pure function of
/// `(x, y)` -- no RNG stream to advance, no iteration-order dependence --
/// which is *why* noise "seeded by grid position" is automatically stable
/// across calls/frames/processes: the same coordinate always mixes down
/// to the same bits.
fn hash_u32(x: u32, y: u32) -> u32 {
    let mut h = NOISE_SEED
        ^ (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    h = h.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    h ^= h >> 33;
    h as u32
}

/// [`hash_u32`] remapped to a signed unit range, ready to use as additive
/// noise/brightness jitter.
fn hash_signed(x: u32, y: u32) -> f32 {
    (hash_u32(x, y) as f32 / u32::MAX as f32) * 2.0 - 1.0
}

/// Bevel rim thickness, in pixels, on every brick sprite's top/left
/// (highlight) and bottom/right (shadow) edges.
const BEVEL_WIDTH: u32 = 5;
/// How far the top/left rim is blended toward white.
const BEVEL_HIGHLIGHT: f32 = 0.35;
/// How far the bottom/right rim is blended toward black.
const BEVEL_SHADOW: f32 = 0.35;
/// Vertical gloss-ramp magnitude on a brick's interior: the top of the
/// panel is brighter, the bottom darker, simulating a light from above
/// (the same corner the bevel highlight implies).
const GLOSS_STRENGTH: f32 = 0.22;
/// Amplitude of the subtle per-pixel brightness noise on brick interiors.
/// Small on purpose -- "subtle," per this bead's brief -- it should read
/// as material grain, not visible static.
const BRICK_NOISE_AMPLITUDE: f32 = 0.05;

/// Per-type brick palette: base color only, loosely matching the flat
/// colors `render.rs` already draws (`NORMAL_BRICK_COLOR`,
/// `ARMORED_BRICK_COLOR`, `ARMORED_BRICK_COLOR_HIT`,
/// `INDESTRUCTIBLE_BRICK_COLOR`) so the new beveled look doesn't read as
/// an unrelated palette swap. Everything else -- bevel, gloss, noise -- is
/// the one shared [`beveled_brick_panel`] recipe; only the base color
/// changes per type.
const BRICK_PALETTE: [(SpriteId, Rgb); 4] = [
    (SpriteId::BrickNormal, [0.85, 0.25, 0.25]),
    (SpriteId::BrickArmoredIntact, [0.30, 0.45, 0.68]),
    (SpriteId::BrickArmoredHit, [0.90, 0.60, 0.15]),
    (SpriteId::BrickIndestructible, [0.25, 0.25, 0.28]),
];

/// Shared brick recipe: a directional gloss ramp (bright top, dark
/// bottom) plus subtle position-seeded noise on the interior, framed by a
/// beveled rim (light top/left, shadow bottom/right) so every brick type
/// reads as a raised panel rather than a flat swatch.
///
/// Noise is seeded by this sprite's *absolute* pixel position in the
/// atlas grid (via [`Atlas::cell_rect`]), not just its local `(x, y)`
/// within the cell -- so distinct brick types never accidentally share
/// the same noise pattern, without needing a separate per-type salt
/// constant.
fn beveled_brick_panel(id: SpriteId, base: Rgb) -> Vec<u8> {
    let (cell_x, cell_y, _, _) = Atlas::cell_rect(id);
    let mut out = vec![0u8; (CELL_SIZE * CELL_SIZE * 4) as usize];
    for y in 0..CELL_SIZE {
        let v = y as f32 / (CELL_SIZE - 1) as f32;
        let gloss = 1.0 + GLOSS_STRENGTH * (0.5 - v);
        for x in 0..CELL_SIZE {
            let noise = hash_signed(cell_x + x, cell_y + y) * BRICK_NOISE_AMPLITUDE;
            let mut color = scale(base, gloss + noise);
            if x < BEVEL_WIDTH || y < BEVEL_WIDTH {
                color = lighten(color, BEVEL_HIGHLIGHT);
            } else if x >= CELL_SIZE - BEVEL_WIDTH || y >= CELL_SIZE - BEVEL_WIDTH {
                color = darken(color, BEVEL_SHADOW);
            }
            put_pixel(&mut out, x, y, to_rgba8(color, 255));
        }
    }
    out
}

/// `render.rs`'s flat `PADDLE_COLOR` -- the brushed-metal recipe's base.
const PADDLE_BASE: Rgb = [0.80, 0.85, 0.95];
/// Row-to-row brightness jitter amplitude: brushed metal's horizontal
/// grain lines. Deliberately depends only on `y` (see
/// [`brushed_metal_paddle`]), never `x`, so the streaks run horizontally
/// the full width of the paddle.
const PADDLE_STREAK_AMPLITUDE: f32 = 0.06;
/// Fine per-pixel brightness jitter on top of the row streaks, for grain
/// within a streak rather than perfectly flat bands.
const PADDLE_GRAIN_AMPLITUDE: f32 = 0.025;
/// Where the specular gloss band sits, as a fraction down the cell (near
/// the top, like an overhead light reflecting off brushed aluminum).
const PADDLE_GLOSS_BAND_CENTER: f32 = 0.30;
/// Gaussian falloff width of the gloss band.
const PADDLE_GLOSS_BAND_WIDTH: f32 = 0.18;
/// Peak brightness boost at the gloss band's center.
const PADDLE_GLOSS_STRENGTH: f32 = 0.30;

/// Brushed-metal paddle: horizontal brightness streaks (one jittered
/// value per row, so grain reads as anisotropic/directional the way a
/// brushed finish does) plus fine per-pixel grain and a soft horizontal
/// specular gloss band near the top.
fn brushed_metal_paddle() -> Vec<u8> {
    let (cell_x, cell_y, _, _) = Atlas::cell_rect(SpriteId::Paddle);
    let mut out = vec![0u8; (CELL_SIZE * CELL_SIZE * 4) as usize];
    for y in 0..CELL_SIZE {
        let streak = hash_signed(0, cell_y + y) * PADDLE_STREAK_AMPLITUDE;
        let v = y as f32 / (CELL_SIZE - 1) as f32;
        let band_dist = v - PADDLE_GLOSS_BAND_CENTER;
        let band = (-(band_dist * band_dist)
            / (2.0 * PADDLE_GLOSS_BAND_WIDTH * PADDLE_GLOSS_BAND_WIDTH))
            .exp();
        for x in 0..CELL_SIZE {
            let grain = hash_signed(cell_x + x, cell_y + y) * PADDLE_GRAIN_AMPLITUDE;
            let brightness = 1.0 + streak + grain + band * PADDLE_GLOSS_STRENGTH;
            let color = scale(PADDLE_BASE, brightness);
            put_pixel(&mut out, x, y, to_rgba8(color, 255));
        }
    }
    out
}

/// `render.rs`'s flat `BALL_COLOR` -- the specular ball recipe's base.
const BALL_BASE: Rgb = [1.0, 0.78, 0.2];
/// Ball radius as a fraction of the cell's half-size, leaving a thin
/// transparent margin so the round sprite doesn't touch the cell edge.
const BALL_RADIUS_FRAC: f32 = 0.94;
/// Width, in pixels, of the antialiased silhouette edge.
const BALL_EDGE_SOFTEN_PX: f32 = 1.5;
/// Specular hotspot center, in unit-circle space, offset toward the same
/// top-left light direction the brick bevels imply.
const BALL_SPECULAR_OFFSET: (f32, f32) = (-0.30, -0.30);
/// Specular hotspot falloff radius, in unit-circle space.
const BALL_SPECULAR_RADIUS: f32 = 0.35;
/// Peak brightness boost at the specular hotspot's center.
const BALL_SPECULAR_STRENGTH: f32 = 0.9;
/// How far the ball darkens from center to silhouette rim.
const BALL_RIM_DARKEN: f32 = 0.55;

/// Specular ball: a round (alpha-masked) sprite shaded darker toward its
/// rim, with a bright specular highlight offset toward the top-left, the
/// way a glossy sphere reflects an overhead light.
fn specular_ball() -> Vec<u8> {
    let mut out = vec![0u8; (CELL_SIZE * CELL_SIZE * 4) as usize];
    let center = (CELL_SIZE - 1) as f32 / 2.0;
    let radius_px = center * BALL_RADIUS_FRAC;
    for y in 0..CELL_SIZE {
        for x in 0..CELL_SIZE {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let dist = (dx * dx + dy * dy).sqrt();
            let alpha = if dist <= radius_px - BALL_EDGE_SOFTEN_PX {
                1.0
            } else if dist >= radius_px {
                0.0
            } else {
                (radius_px - dist) / BALL_EDGE_SOFTEN_PX
            };
            if alpha <= 0.0 {
                put_pixel(&mut out, x, y, [0, 0, 0, 0]);
                continue;
            }
            let nx = dx / radius_px;
            let ny = dy / radius_px;
            let rim = (nx * nx + ny * ny).sqrt().min(1.0);
            let mut color = darken(BALL_BASE, rim * BALL_RIM_DARKEN);
            let sx = nx - BALL_SPECULAR_OFFSET.0;
            let sy = ny - BALL_SPECULAR_OFFSET.1;
            let spec_dist = (sx * sx + sy * sy).sqrt();
            let spec = (1.0 - spec_dist / BALL_SPECULAR_RADIUS).max(0.0);
            color = lighten(color, spec * BALL_SPECULAR_STRENGTH);
            put_pixel(
                &mut out,
                x,
                y,
                to_rgba8(color, (alpha * 255.0).round() as u8),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test for the seam this bead owns: both `TextureSource`
    /// variants load without panicking and produce a non-empty,
    /// correctly-sized buffer. Atlas layout invariants and procedural
    /// determinism get their full property-style coverage in b4; this is
    /// just "the abstraction is real," not a substitute for that.
    #[test]
    fn load_produces_a_correctly_sized_atlas_for_every_source() {
        for source in [
            TextureSource::Procedural,
            TextureSource::Pack(PathBuf::from("/nonexistent/pack")),
        ] {
            let atlas = source.load();
            assert_eq!(
                atlas.pixels.len(),
                (atlas.width * atlas.height * 4) as usize
            );
            assert!(atlas.width > 0 && atlas.height > 0);
        }
    }

    #[test]
    fn every_sprite_id_has_a_distinct_in_bounds_cell() {
        let atlas = TextureSource::Procedural.load();
        let mut seen = Vec::new();
        for id in SpriteId::ALL {
            let (x, y, w, h) = Atlas::cell_rect(id);
            assert!(x + w <= atlas.width && y + h <= atlas.height);
            assert!(!seen.contains(&(x, y)), "duplicate cell origin for {id:?}");
            seen.push((x, y));

            let uv = atlas.uv_rect(id);
            assert!((0.0..=1.0).contains(&uv.u0) && (0.0..=1.0).contains(&uv.v0));
            assert!((0.0..=1.0).contains(&uv.u1) && (0.0..=1.0).contains(&uv.v1));
        }
    }

    #[test]
    fn default_texture_source_is_procedural() {
        assert_eq!(TextureSource::default(), TextureSource::Procedural);
    }

    #[test]
    #[should_panic(expected = "set_sprite")]
    fn set_sprite_panics_on_wrong_size_buffer() {
        let mut atlas = TextureSource::Procedural.load();
        atlas.set_sprite(SpriteId::Ball, &[0u8; 4]);
    }

    /// This bead's acceptance criterion, stated directly: generating the
    /// procedural atlas twice (same fixed `NOISE_SEED`, no RNG stream,
    /// see `procedural_placeholder`'s doc comment) must yield the exact
    /// same bytes, not just "close" or "same dimensions."
    #[test]
    fn procedural_atlas_is_byte_identical_across_generations() {
        let a = TextureSource::Procedural.load();
        let b = TextureSource::Procedural.load();
        assert_eq!(a.width, b.width);
        assert_eq!(a.height, b.height);
        assert_eq!(a.pixels, b.pixels);
    }

    /// Exercises the beveled-panel recipe's directional lighting: with a
    /// light implied from the top-left, that corner should end up
    /// brighter than the bottom-right corner. A regression that dropped
    /// the bevel (or reversed its direction) would flatten or invert this.
    #[test]
    fn brick_sprite_has_a_directional_bevel() {
        let atlas = TextureSource::Procedural.load();
        let (cx, cy, w, h) = Atlas::cell_rect(SpriteId::BrickNormal);
        let pixel = |x: u32, y: u32| -> u32 {
            let idx = (((cy + y) * atlas.width + (cx + x)) * 4) as usize;
            atlas.pixels[idx..idx + 3].iter().map(|&c| c as u32).sum()
        };
        let top_left = pixel(0, 0);
        let bottom_right = pixel(w - 1, h - 1);
        assert!(
            top_left > bottom_right,
            "expected a brighter top-left highlight than bottom-right shadow, got {top_left} vs {bottom_right}"
        );
    }

    /// Exercises the specular-ball recipe's round alpha mask and shading:
    /// the cell's corner sits outside the ball's radius (transparent) and
    /// its center sits deep inside it (fully opaque).
    #[test]
    fn ball_sprite_is_round_with_transparent_corners() {
        let atlas = TextureSource::Procedural.load();
        let (cx, cy, w, h) = Atlas::cell_rect(SpriteId::Ball);
        let alpha = |x: u32, y: u32| -> u8 {
            let idx = (((cy + y) * atlas.width + (cx + x)) * 4) as usize;
            atlas.pixels[idx + 3]
        };
        assert_eq!(alpha(0, 0), 0, "cell corner should be outside the ball");
        assert_eq!(
            alpha(w / 2, h / 2),
            255,
            "cell center should be fully inside the ball"
        );
    }

    /// A fresh, unique scratch directory under the OS temp dir -- these
    /// pack-loader tests are the one place in this module that touches
    /// the filesystem, so each test gets its own directory (PID + an
    /// atomic counter) rather than risking two tests, or two parallel
    /// test runs, colliding on one path.
    fn make_temp_dir(label: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "arkanoid-assets-test-{}-{label}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("failed to create test scratch dir");
        dir
    }

    /// A pack directory with a correctly named PNG (deliberately *not*
    /// `CELL_SIZE` square, to exercise `load_pack`'s resize path) for
    /// every `SpriteId` should decode into an atlas carrying that exact
    /// pixel, proving the pack path is actually wired up end to end
    /// rather than silently falling back to procedural pixels.
    #[test]
    fn pack_loads_a_distinct_pixel_per_sprite_from_disk() {
        let dir = make_temp_dir("valid");
        for id in SpriteId::ALL {
            let pixel = image::Rgba([id as u8 * 10, 20, 30, 255]);
            image::RgbaImage::from_pixel(10, 10, pixel)
                .save(dir.join(pack_filename(id)))
                .expect("failed to write test sprite PNG");
        }

        let atlas = TextureSource::Pack(dir.clone()).load();
        for id in SpriteId::ALL {
            let (x, y, _, _) = Atlas::cell_rect(id);
            let idx = ((y * atlas.width + x) * 4) as usize;
            assert_eq!(
                atlas.pixels[idx..idx + 4],
                [id as u8 * 10, 20, 30, 255],
                "sprite {id:?} did not load from its pack file"
            );
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    /// This bead's headline acceptance criterion: a directory that simply
    /// doesn't exist must warn (not tested here -- `eprintln!` isn't
    /// observable from a unit test) and fall back to byte-identical
    /// procedural pixels, never panic.
    #[test]
    fn pack_falls_back_to_procedural_on_missing_directory() {
        let atlas =
            TextureSource::Pack(PathBuf::from("/nonexistent/definitely-not-a-real-pack-dir"))
                .load();
        let procedural = TextureSource::Procedural.load();
        assert_eq!(atlas.pixels, procedural.pixels);
    }

    /// One-shot fixture generator, not part of the suite (`#[ignore]`):
    /// run manually (`cargo test --ignored generate_fixture_pack_pngs --
    /// --exact`) whenever `tests/fixtures/pack/`'s committed PNGs need
    /// regenerating. Writes the same flat-color-per-sprite pixels
    /// `pack_loader_reads_our_committed_fixture_atlas` asserts against,
    /// at a size that isn't `CELL_SIZE` square (so the committed fixture
    /// also exercises `load_pack`'s resize path, same as the temp-dir
    /// test above).
    #[test]
    #[ignore = "generates the committed tests/fixtures/pack PNGs; run manually to regenerate"]
    fn generate_fixture_pack_pngs() {
        let dir = fixture_pack_dir();
        std::fs::create_dir_all(&dir).expect("failed to create tests/fixtures/pack");
        for id in SpriteId::ALL {
            let [r, g, b, a] = fixture_pixel(id);
            image::RgbaImage::from_pixel(6, 6, image::Rgba([r, g, b, a]))
                .save(dir.join(pack_filename(id)))
                .expect("failed to write fixture sprite PNG");
        }
    }

    /// Absolute path to the committed fixture pack directory this bead
    /// owns, resolved from the crate root so the test works regardless of
    /// the process's current directory.
    fn fixture_pack_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pack")
    }

    /// Our own flat RGBA pixel for `id`'s fixture sprite -- plain solid
    /// colors we picked, not Kenney art, distinct per sprite so a mix-up
    /// between sprites would fail loudly.
    fn fixture_pixel(id: SpriteId) -> [u8; 4] {
        match id {
            SpriteId::Paddle => [10, 20, 30, 255],
            SpriteId::Ball => [40, 50, 60, 255],
            SpriteId::BrickNormal => [70, 80, 90, 255],
            SpriteId::BrickArmoredIntact => [100, 110, 120, 255],
            SpriteId::BrickArmoredHit => [130, 140, 150, 255],
            SpriteId::BrickIndestructible => [160, 170, 180, 255],
        }
    }

    /// This bead's headline deliverable: a pack-loader test against a
    /// fixture atlas *committed to the repo* under `tests/fixtures/`
    /// (unlike the temp-dir tests above, which write and delete their
    /// pack directory per run) so the pixels a reviewer sees in `git show`
    /// are exactly the pixels this test loads and checks -- our own flat
    /// colors, never Kenney's.
    ///
    /// The fixture directory holds all six `SpriteId::ALL` files, not
    /// just two, because `load_pack` is all-or-nothing (see its doc
    /// comment): a directory missing any one sprite fails the whole load
    /// and falls back to procedural pixels, which this test's exact-pixel
    /// assertions would then correctly fail rather than silently pass.
    /// The "2-sprite" fixture this bead's brief asks for is expressed in
    /// what the test *checks*: two representative sprites (Paddle, Ball)
    /// spot-checked by exact pixel, since exhaustively re-checking all six
    /// is already covered by `pack_loads_a_distinct_pixel_per_sprite_from_disk`
    /// above.
    #[test]
    fn pack_loader_reads_our_committed_fixture_atlas() {
        let atlas = TextureSource::Pack(fixture_pack_dir()).load();

        let pixel_at = |id: SpriteId| -> [u8; 4] {
            let (x, y, _, _) = Atlas::cell_rect(id);
            let idx = ((y * atlas.width + x) * 4) as usize;
            atlas.pixels[idx..idx + 4].try_into().unwrap()
        };

        assert_eq!(
            pixel_at(SpriteId::Paddle),
            fixture_pixel(SpriteId::Paddle),
            "Paddle did not load its committed fixture pixel"
        );
        assert_eq!(
            pixel_at(SpriteId::Ball),
            fixture_pixel(SpriteId::Ball),
            "Ball did not load its committed fixture pixel"
        );
    }

    /// Same fallback contract, but for a pack directory that exists and
    /// has the right filenames, just not valid image data in them --
    /// the "malformed pack" half of this bead's acceptance criterion.
    #[test]
    fn pack_falls_back_to_procedural_on_malformed_sprite() {
        let dir = make_temp_dir("malformed");
        for id in SpriteId::ALL {
            std::fs::write(dir.join(pack_filename(id)), b"not a png file")
                .expect("failed to write malformed test file");
        }

        let atlas = TextureSource::Pack(dir.clone()).load();
        let procedural = TextureSource::Procedural.load();
        assert_eq!(atlas.pixels, procedural.pixels);

        std::fs::remove_dir_all(&dir).ok();
    }
}
