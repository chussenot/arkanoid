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

use std::path::PathBuf;

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
            // b3 replaces this arm with: try to decode `path` into an
            // `Atlas` via the `image` crate; on any I/O/decode error,
            // `eprintln!` a warning and fall back to exactly this.
            TextureSource::Pack(_path) => Atlas::procedural_placeholder(),
        }
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

    /// Flat-color placeholder atlas: fills each sprite's cell with a
    /// single solid color loosely matching `render.rs`'s existing
    /// flat-color palette, so the type is real, drawable, and
    /// deterministically testable before b2's recipe-driven pass lands.
    /// b2 replaces the *pixels* this produces, not its role as
    /// `Procedural`'s default builder.
    fn procedural_placeholder() -> Atlas {
        let (width, height) = Self::grid_size();
        let mut atlas = Atlas {
            width,
            height,
            pixels: vec![0; (width * height * 4) as usize],
        };
        for &(id, color) in PLACEHOLDER_COLORS.iter() {
            atlas.set_sprite(id, &solid_rgba(color));
        }
        atlas
    }
}

/// One solid RGBA8 color per [`SpriteId`], loosely matching the flat
/// colors `render.rs` already draws (`PADDLE_COLOR`, `BALL_COLOR`,
/// `NORMAL_BRICK_COLOR`, etc.) so the placeholder atlas doesn't look like
/// an unrelated palette swap.
const PLACEHOLDER_COLORS: [(SpriteId, [u8; 4]); 6] = [
    (SpriteId::Paddle, [204, 217, 242, 255]),
    (SpriteId::Ball, [255, 199, 51, 255]),
    (SpriteId::BrickNormal, [217, 64, 64, 255]),
    (SpriteId::BrickArmoredIntact, [77, 115, 173, 255]),
    (SpriteId::BrickArmoredHit, [230, 153, 38, 255]),
    (SpriteId::BrickIndestructible, [64, 64, 71, 255]),
];

/// One `CELL_SIZE x CELL_SIZE` RGBA8 sprite's worth of a single solid
/// color, ready for [`Atlas::set_sprite`].
fn solid_rgba(color: [u8; 4]) -> Vec<u8> {
    color[..].repeat((CELL_SIZE * CELL_SIZE) as usize)
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
}
