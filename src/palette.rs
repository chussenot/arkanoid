//! The one palette both renderers draw with (DESIGN.md, "Colors").
//!
//! Every color is authored here once, as the sRGB hex a designer would
//! write. Each renderer converts it at the point of use:
//! - [`Rgb::linear`] for vertex colors, clear colors and uniforms, because
//!   the shaders write to an sRGB surface and the hardware encodes on
//!   output;
//! - [`Rgb::srgb`] for the procedural sprite recipes in `assets.rs`, whose
//!   bytes go into an `Rgba8UnormSrgb` texture;
//! - [`Rgb::text`] for glyphon, which takes sRGB bytes.
//!
//! Before this module, the same triples were copied into three files and
//! written to the sRGB surface as if they were linear, so `#D94040`
//! rendered as `#ED8989`: a pastel wash over the whole game.
//! `linear_round_trips_through_srgb` pins the conversion.
//!
//! Hue carries meaning in both renderers, and never alone: every capsule
//! also carries its letter ([`capsule_letter`]), and every brick kind its
//! pattern (`assets.rs`). See PRODUCT.md, "Accessibility & Inclusion".

use crate::events::PowerUpKind;
use crate::levels::BrickKind;

/// An sRGB color as authored: 8 bits per channel, no alpha.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

const fn hex(v: u32) -> Rgb {
    Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// The sRGB transfer function's decode half, one channel in `0..=1`.
fn decode(c: f32) -> f32 {
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

impl Rgb {
    /// Linear-light RGBA, for anything a shader writes to the sRGB surface.
    pub fn linear(self) -> [f32; 4] {
        self.linear_alpha(1.0)
    }

    /// [`Rgb::linear`] with an explicit alpha (alpha is never encoded).
    pub fn linear_alpha(self, alpha: f32) -> [f32; 4] {
        [
            decode(f32::from(self.0) / 255.0),
            decode(f32::from(self.1) / 255.0),
            decode(f32::from(self.2) / 255.0),
            alpha,
        ]
    }

    /// The authored channels as `0..=1` floats, still sRGB-encoded: the
    /// working space of `assets.rs`'s recipes.
    pub fn srgb(self) -> [f32; 3] {
        [
            f32::from(self.0) / 255.0,
            f32::from(self.1) / 255.0,
            f32::from(self.2) / 255.0,
        ]
    }

    /// A clear color for a render pass (linear, opaque).
    pub fn clear(self) -> wgpu::Color {
        let [r, g, b, _] = self.linear();
        wgpu::Color {
            r: f64::from(r),
            g: f64::from(g),
            b: f64::from(b),
            a: 1.0,
        }
    }

    /// The color as glyphon text takes it.
    pub fn text(self) -> glyphon::Color {
        glyphon::Color::rgb(self.0, self.1, self.2)
    }
}

// -- surfaces -------------------------------------------------------------
// Dark because the game is a field of small, fast, bright objects: the
// ground recedes so they read. Blue-black rather than neutral, to sit under
// warm and cool entity hues alike.

/// Outside the canvas, when the window's aspect differs from the canvas's.
pub const LETTERBOX: Rgb = hex(0x040507);
/// The HUD band above the field.
pub const BAND: Rgb = hex(0x080A0F);
/// The playfield's ground, in both renderers.
pub const FIELD: Rgb = hex(0x10141C);
/// Hairlines: the ceiling rule's minor ticks, keycap outlines.
pub const RULE: Rgb = hex(0x2C3445);
/// The ceiling rule itself and its major ticks.
pub const RULE_STRONG: Rgb = hex(0x56617A);
/// The 3D renderer's side and top walls.
pub const WALL: Rgb = hex(0x2A3140);

// -- text -----------------------------------------------------------------

/// Values, titles, anything the player reads first. 16:1 on `BAND`.
pub const TEXT: Rgb = hex(0xE9ECF3);
/// Captions and secondary lines. 6.7:1 on `BAND`, 5.9:1 on the scrim.
pub const TEXT_DIM: Rgb = hex(0x8E97AB);
/// Leading zeros of the score: present, but not read.
pub const TEXT_FAINT: Rgb = hex(0x485166);

/// Over the field in Menu/Paused/GameOver/Victory: the field's own ink at
/// this alpha, so the dimmed scene keeps its hue instead of going gray.
pub const SCRIM: Rgb = FIELD;
pub const SCRIM_ALPHA: f32 = 0.95;

// -- the player's objects -------------------------------------------------

/// The ball: the brightest object on screen, warm white so it is not
/// mistaken for any capsule or for the paddle's cool metal.
pub const BALL: Rgb = hex(0xFFF2CF);
/// Brushed-metal paddle base (the sprite recipe adds grain and gloss).
pub const PADDLE: Rgb = hex(0xC6CEDC);
/// One-frame flash on a brick that was hit.
pub const HIT_FLASH: Rgb = hex(0xFFFFFF);

// -- bricks: hue plus pattern (assets.rs draws the pattern) ---------------

/// Normal: plain beveled panel.
pub const BRICK_NORMAL: Rgb = hex(0xE0533F);
/// Armored, intact: steel blue, an inset plate with four rivets.
pub const BRICK_ARMORED: Rgb = hex(0x4D7FD6);
/// Armored after its first hit: amber, the same plate split by a crack.
pub const BRICK_ARMORED_HIT: Rgb = hex(0xDE9A2C);
/// Indestructible: graphite with diagonal hatching.
pub const BRICK_INDESTRUCTIBLE: Rgb = hex(0x4A5160);

/// A brick's color for its kind and remaining hits.
pub fn brick(kind: BrickKind, hits_remaining: u8) -> Rgb {
    match kind {
        BrickKind::Normal => BRICK_NORMAL,
        BrickKind::Armored if hits_remaining >= 2 => BRICK_ARMORED,
        BrickKind::Armored => BRICK_ARMORED_HIT,
        BrickKind::Indestructible => BRICK_INDESTRUCTIBLE,
    }
}

// -- capsules: hue plus letter ---------------------------------------------

pub const CAPSULE_WIDEN: Rgb = hex(0x3DC46B);
pub const CAPSULE_SLOW: Rgb = hex(0x3F8EF5);
pub const CAPSULE_MULTIBALL: Rgb = hex(0x9C66F2);
pub const CAPSULE_GLUE: Rgb = hex(0xEE62A0);
pub const CAPSULE_LASER: Rgb = hex(0xEE463B);
pub const CAPSULE_SUPER_LASER: Rgb = hex(0x2BCFD8);
pub const CAPSULE_TINY: Rgb = hex(0x9BA5B9);
pub const CAPSULE_PLAYER: Rgb = hex(0xF0C232);
pub const CAPSULE_BREAK: Rgb = hex(0xFF8A2B);

/// A capsule's hue. The open Break gate and the laser bolts reuse their
/// capsule's hue, so the cause and the effect read as one thing.
pub fn capsule(kind: PowerUpKind) -> Rgb {
    match kind {
        PowerUpKind::Widen => CAPSULE_WIDEN,
        PowerUpKind::Slow => CAPSULE_SLOW,
        PowerUpKind::Multiball => CAPSULE_MULTIBALL,
        PowerUpKind::Glue => CAPSULE_GLUE,
        PowerUpKind::Laser => CAPSULE_LASER,
        PowerUpKind::SuperLaser => CAPSULE_SUPER_LASER,
        PowerUpKind::Tiny => CAPSULE_TINY,
        PowerUpKind::Player => CAPSULE_PLAYER,
        PowerUpKind::Break => CAPSULE_BREAK,
    }
}

/// The letter on a capsule: the color-blind-safe half of its identity.
/// Unique across the nine; Super laser takes X because S is Slow's.
pub fn capsule_letter(kind: PowerUpKind) -> &'static str {
    match kind {
        PowerUpKind::Widen => "W",
        PowerUpKind::Slow => "S",
        PowerUpKind::Multiball => "M",
        PowerUpKind::Glue => "G",
        PowerUpKind::Laser => "L",
        PowerUpKind::SuperLaser => "X",
        PowerUpKind::Tiny => "T",
        PowerUpKind::Player => "P",
        PowerUpKind::Break => "B",
    }
}

/// The ink a capsule's letter is set in: the field's own dark, which
/// clears 4.5:1 on all nine hues (`capsule_letters_clear_contrast`).
pub const CAPSULE_INK: Rgb = FIELD;

// -- hazards and effects ---------------------------------------------------

/// Asteroids: cratered stone, cool and desaturated, so the one warm thing
/// about them is the explosion they become.
pub const ASTEROID: Rgb = hex(0x8B7F9A);
/// An asteroid's explosion.
pub const EXPLOSION: Rgb = hex(0xFFB55E);

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_CAPSULES: [PowerUpKind; 9] = [
        PowerUpKind::Widen,
        PowerUpKind::Slow,
        PowerUpKind::Multiball,
        PowerUpKind::Glue,
        PowerUpKind::Laser,
        PowerUpKind::SuperLaser,
        PowerUpKind::Tiny,
        PowerUpKind::Player,
        PowerUpKind::Break,
    ];

    /// WCAG relative luminance of an sRGB color.
    fn luminance(c: Rgb) -> f32 {
        let [r, g, b, _] = c.linear();
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    fn contrast(a: Rgb, b: Rgb) -> f32 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    #[test]
    fn linear_round_trips_through_srgb() {
        // The two ends and the classic mid-gray: 0.5 linear is about 188.
        assert_eq!(hex(0x000000).linear(), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(hex(0xFFFFFF).linear(), [1.0, 1.0, 1.0, 1.0]);
        let mid = hex(0xBCBCBC).linear()[0];
        assert!((mid - 0.5).abs() < 0.01, "#BC decodes to {mid}");
    }

    #[test]
    fn capsule_letters_are_unique() {
        let mut letters: Vec<_> = ALL_CAPSULES.iter().map(|&k| capsule_letter(k)).collect();
        letters.sort_unstable();
        letters.dedup();
        assert_eq!(letters.len(), ALL_CAPSULES.len());
    }

    #[test]
    fn capsule_hues_are_distinct() {
        for (i, &a) in ALL_CAPSULES.iter().enumerate() {
            for &b in &ALL_CAPSULES[i + 1..] {
                assert_ne!(capsule(a), capsule(b), "{a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn capsule_letters_clear_contrast() {
        for kind in ALL_CAPSULES {
            let ratio = contrast(capsule(kind), CAPSULE_INK);
            assert!(ratio >= 4.5, "{kind:?}: {ratio:.2}:1");
        }
    }

    #[test]
    fn text_clears_contrast_on_its_grounds() {
        for ground in [BAND, FIELD] {
            assert!(contrast(TEXT, ground) >= 7.0);
            assert!(contrast(TEXT_DIM, ground) >= 4.5);
        }
    }

    #[test]
    fn the_ball_is_the_brightest_moving_object() {
        let ball = luminance(BALL);
        for kind in ALL_CAPSULES {
            assert!(luminance(capsule(kind)) < ball, "{kind:?}");
        }
        assert!(luminance(ASTEROID) < ball && luminance(PADDLE) < ball);
    }
}
