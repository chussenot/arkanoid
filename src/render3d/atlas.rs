//! Local, hand-synced copy of Workstream B's real texture-atlas recipe
//! (epic/textures, beads b1/b2's `src/assets.rs` -- `Atlas`, `SpriteId`,
//! `UvRect`, and the beveled-brick-panel procedural generator), scoped
//! down to just the four brick sprites this bead (arkanoid-v2-c4) needs:
//! bricks get textured front faces, paddle/ball/powerups stay flat-colored
//! per `render3d/mod.rs`'s module doc comment.
//!
//! # Why a copy, not an import
//! `src/assets.rs` lives on `epic/textures`, a sibling epic branch this
//! worktree (`epic/presentation-3d`) must not merge -- see
//! `docs/fleet-patterns.md`'s "Cross-epic dependencies" section and this
//! bead's own description. b1/b2 have already landed there (verified via
//! `git show epic/textures:src/assets.rs`), so this is a real recipe, not
//! an invented placeholder: the beveled-panel algorithm, base colors, and
//! `CELL_SIZE` below are ported straight from that file. `GRID_COLS` is
//! the one deliberate deviation (2, not 3) -- this copy only carries 4
//! sprites (not B's full 6, since paddle/ball stay flat here), and 2
//! packs those 4 into a grid with no empty cells.
//!
//! # Reconciliation
//! At the final human-reviewed merge of the three v2 epics into `master`,
//! this module should be deleted and `render3d/mod.rs` should import
//! `crate::assets::{Atlas, SpriteId}` directly instead -- the type/field/
//! method shapes here were kept identical to B's real interface
//! specifically to make that swap mechanical rather than a rewrite.

/// Side length in pixels of the square cell every sprite is packed into.
/// Matches B's real `assets::CELL_SIZE` exactly.
pub(super) const CELL_SIZE: u32 = 64;

/// Sprites per atlas row. B's real atlas uses 3 (6 sprites, 2 full rows);
/// this local copy only carries brick sprites, so 2 packs its 4 sprites
/// into a 2x2 grid with no empty cells.
const GRID_COLS: u32 = 2;

/// One entry per brick *visual* state -- matches B's real `SpriteId`
/// variant names/order for the brick subset (armored bricks get two
/// sprites, matching `render3d::brick_color`'s own intact/hit split). All
/// four sharing a `Brick` prefix only looks redundant in isolation --
/// B's real enum also has `Paddle`/`Ball` variants alongside these, where
/// the prefix earns its keep; dropping it here would break the name-for-
/// name match this module's doc comment promises for reconciliation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(clippy::enum_variant_names)]
pub(super) enum SpriteId {
    BrickNormal,
    BrickArmoredIntact,
    BrickArmoredHit,
    BrickIndestructible,
}

impl SpriteId {
    pub(super) const ALL: [SpriteId; 4] = [
        SpriteId::BrickNormal,
        SpriteId::BrickArmoredIntact,
        SpriteId::BrickArmoredHit,
        SpriteId::BrickIndestructible,
    ];
}

/// Normalized UV rectangle, y-down -- identical shape to B's real
/// `UvRect`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct UvRect {
    pub(super) u0: f32,
    pub(super) v0: f32,
    pub(super) u1: f32,
    pub(super) v1: f32,
}

/// RGBA8 pixel buffer plus a fixed sprite grid -- identical shape to B's
/// real `Atlas`.
#[derive(Debug, Clone)]
pub(super) struct Atlas {
    pub(super) width: u32,
    pub(super) height: u32,
    /// Row-major RGBA8 (unorm), top row first -- `bytes_per_row = width *
    /// 4`, uploadable directly as a wgpu texture.
    pub(super) pixels: Vec<u8>,
}

impl Atlas {
    fn grid_size() -> (u32, u32) {
        let cols = GRID_COLS.min(SpriteId::ALL.len() as u32).max(1);
        let rows = (SpriteId::ALL.len() as u32).div_ceil(cols);
        (cols * CELL_SIZE, rows * CELL_SIZE)
    }

    /// Pixel-space `(x, y, w, h)` of `id`'s cell.
    fn cell_rect(id: SpriteId) -> (u32, u32, u32, u32) {
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
    /// `width`/`height`.
    pub(super) fn uv_rect(&self, id: SpriteId) -> UvRect {
        let (x, y, w, h) = Self::cell_rect(id);
        UvRect {
            u0: x as f32 / self.width as f32,
            v0: y as f32 / self.height as f32,
            u1: (x + w) as f32 / self.width as f32,
            v1: (y + h) as f32 / self.height as f32,
        }
    }

    /// Writes one sprite's pixels into its cell.
    ///
    /// # Panics
    /// If `rgba.len()` isn't exactly one cell's worth of RGBA8 pixels.
    fn set_sprite(&mut self, id: SpriteId, rgba: &[u8]) {
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

    /// Builds the atlas from the beveled-brick-panel recipe below --
    /// ported from B's real `Atlas::procedural_placeholder`/b2's recipe
    /// (see this module's doc comment).
    pub(super) fn procedural_placeholder() -> Atlas {
        let (width, height) = Self::grid_size();
        let mut atlas = Atlas {
            width,
            height,
            pixels: vec![0; (width * height * 4) as usize],
        };
        for &(id, base) in BRICK_PALETTE.iter() {
            atlas.set_sprite(id, &beveled_brick_panel(id, base));
        }
        atlas
    }
}

// -- beveled-brick-panel recipe (ported from B's real assets.rs) --------
//
// Directional gloss ramp (bright top, dark bottom) plus subtle
// position-seeded noise on the interior, framed by a beveled rim (light
// top/left, shadow bottom/right) so every brick type reads as a raised
// panel rather than a flat swatch. Deterministic by construction: every
// pixel derives purely from its fixed `(x, y)` position via `hash_signed`,
// never from wall-clock time or a stateful RNG stream.

/// Fixed seed for every procedural noise/hash lookup -- a plain `const`,
/// so two calls always produce byte-identical pixels.
const NOISE_SEED: u64 = 0x9E37_79B9_7F4A_7C15;

/// Color as three `0.0..=1.0` linear-ish channels, the working space for
/// the recipe below. Converted to `u8` only at the very end
/// ([`to_rgba8`]).
type Rgb = [f32; 3];

fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

fn scale(c: Rgb, factor: f32) -> Rgb {
    [
        clamp01(c[0] * factor),
        clamp01(c[1] * factor),
        clamp01(c[2] * factor),
    ]
}

fn lighten(c: Rgb, amount: f32) -> Rgb {
    [
        clamp01(c[0] + (1.0 - c[0]) * amount),
        clamp01(c[1] + (1.0 - c[1]) * amount),
        clamp01(c[2] + (1.0 - c[2]) * amount),
    ]
}

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

fn put_pixel(buf: &mut [u8], x: u32, y: u32, rgba: [u8; 4]) {
    let idx = ((y * CELL_SIZE + x) * 4) as usize;
    buf[idx..idx + 4].copy_from_slice(&rgba);
}

/// Cheap deterministic position hash (the MurmurHash3 finalizer mix).
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

fn hash_signed(x: u32, y: u32) -> f32 {
    (hash_u32(x, y) as f32 / u32::MAX as f32) * 2.0 - 1.0
}

/// Bevel rim thickness, in pixels, on a brick sprite's top/left
/// (highlight) and bottom/right (shadow) edges.
const BEVEL_WIDTH: u32 = 5;
const BEVEL_HIGHLIGHT: f32 = 0.35;
const BEVEL_SHADOW: f32 = 0.35;
/// Vertical gloss-ramp magnitude on a brick's interior.
const GLOSS_STRENGTH: f32 = 0.22;
/// Amplitude of the subtle per-pixel brightness noise on brick interiors.
const BRICK_NOISE_AMPLITUDE: f32 = 0.05;

/// Per-type base color, matching `render3d::mod`'s own flat brick colors
/// (`NORMAL_BRICK_COLOR` etc.) so the beveled look doesn't read as an
/// unrelated palette swap.
const BRICK_PALETTE: [(SpriteId, Rgb); 4] = [
    (SpriteId::BrickNormal, [0.85, 0.25, 0.25]),
    (SpriteId::BrickArmoredIntact, [0.30, 0.45, 0.68]),
    (SpriteId::BrickArmoredHit, [0.90, 0.60, 0.15]),
    (SpriteId::BrickIndestructible, [0.25, 0.25, 0.28]),
];

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn procedural_atlas_has_a_correctly_sized_pixel_buffer() {
        let atlas = Atlas::procedural_placeholder();
        assert_eq!(
            atlas.pixels.len(),
            (atlas.width * atlas.height * 4) as usize
        );
        assert!(atlas.width > 0 && atlas.height > 0);
    }

    #[test]
    fn every_sprite_id_has_a_distinct_in_bounds_cell() {
        let atlas = Atlas::procedural_placeholder();
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

    /// This bead's atlas must be deterministic, same as B's real one:
    /// generating it twice yields byte-identical pixels.
    #[test]
    fn procedural_atlas_is_byte_identical_across_generations() {
        let a = Atlas::procedural_placeholder();
        let b = Atlas::procedural_placeholder();
        assert_eq!(a.width, b.width);
        assert_eq!(a.height, b.height);
        assert_eq!(a.pixels, b.pixels);
    }

    /// Exercises the beveled-panel recipe's directional lighting: with a
    /// light implied from the top-left, that corner should end up
    /// brighter than the bottom-right corner.
    #[test]
    fn brick_sprite_has_a_directional_bevel() {
        let atlas = Atlas::procedural_placeholder();
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

    #[test]
    #[should_panic(expected = "set_sprite")]
    fn set_sprite_panics_on_wrong_size_buffer() {
        let mut atlas = Atlas::procedural_placeholder();
        atlas.set_sprite(SpriteId::BrickNormal, &[0u8; 4]);
    }
}
