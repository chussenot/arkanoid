//! The interface layer both renderers share: the canvas and its letterbox,
//! the HUD band, the state screens (Menu, Paused, GameOver, Victory), and
//! the letters on falling capsules (DESIGN.md, "Layout" and "Components").
//!
//! # Canvas
//! The game draws on an 800×648 canvas: a 48 px HUD band on top of the
//! fixed 800×600 field. The band sits outside the field, so the score
//! never overlaps the ball's flight space. The canvas is scaled uniformly
//! into the window and letterboxed ([`Letterbox`]), never stretched. The
//! simulation never sees any of this: field coordinates are the
//! simulation's own, offset by [`BAND_HEIGHT`] here.
//!
//! # One layer, two lenses
//! [`layout`] is a pure function from the game to quads and text spans, so
//! both renderers show the same words, numbers, sizes and colors, and the
//! tests can check it without a GPU. [`Hud`] owns the GPU side: one
//! flat-color quad pipeline and one glyphon text renderer. A renderer
//! draws [`Hud::draw_backdrop`] first (band and field ground), its own
//! world in [`Letterbox::field_viewport`], then [`Hud::draw_overlay`]
//! last.
//!
//! # Type
//! One family, JetBrains Mono (SIL OFL 1.1, `assets/fonts/`, see
//! ATTRIBUTION), loaded from the binary instead of the system, so the game
//! reads the same on every machine. Monospace because the HUD is numbers
//! that change every frame: fixed advances keep them from jittering, and
//! let the layout measure text without shaping it ([`text_width`]).

use std::mem::size_of;

use glyphon::cosmic_text::Align;
use glyphon::{
    Attrs, Buffer as TextBuffer, Cache as TextCache, Family, FontSystem, Metrics, Resolution,
    Shaping, SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Weight,
};

use crate::events::PowerUpKind;
use crate::game::{Game, GameState, LevelSetKind, PLAYFIELD_HEIGHT, PLAYFIELD_WIDTH};
use crate::palette::{self, Rgb};

/// Height of the HUD band above the field, in canvas pixels.
pub const BAND_HEIGHT: f32 = 48.0;
/// Canvas size: the field plus the band.
pub const CANVAS_WIDTH: f32 = PLAYFIELD_WIDTH;
pub const CANVAS_HEIGHT: f32 = BAND_HEIGHT + PLAYFIELD_HEIGHT;

const FONT_FAMILY: &str = "JetBrains Mono";
const FONT_REGULAR: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf");
const FONT_BOLD: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf");
/// JetBrains Mono's advance width: 600 units per 1000-unit em.
const ADVANCE_EM: f32 = 0.6;

/// `BRYX_REDUCED_MOTION=1` (any value but `0` or empty) turns off screen
/// and camera shake in both renderers. Every other effect is already
/// brief, event-bound, and free of flashing above 3 Hz (DESIGN.md,
/// "Motion"). Read once.
pub fn reduced_motion() -> bool {
    static REDUCED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *REDUCED.get_or_init(|| {
        std::env::var("BRYX_REDUCED_MOTION").is_ok_and(|v| !v.is_empty() && v != "0")
    })
}

// -- letterbox --------------------------------------------------------------

/// Where the canvas sits in the window: a uniform scale and an origin,
/// centered, with bars on whichever axis has room to spare.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Letterbox {
    pub x: f32,
    pub y: f32,
    pub scale: f32,
}

impl Letterbox {
    pub fn fit(width: u32, height: u32) -> Self {
        let (w, h) = (width.max(1) as f32, height.max(1) as f32);
        let scale = (w / CANVAS_WIDTH).min(h / CANVAS_HEIGHT);
        Self {
            x: ((w - CANVAS_WIDTH * scale) / 2.0).floor(),
            y: ((h - CANVAS_HEIGHT * scale) / 2.0).floor(),
            scale,
        }
    }

    /// The canvas as a viewport `(x, y, width, height)` in surface pixels.
    pub fn canvas_viewport(&self) -> (f32, f32, f32, f32) {
        (
            self.x,
            self.y,
            CANVAS_WIDTH * self.scale,
            CANVAS_HEIGHT * self.scale,
        )
    }

    /// The field (the canvas below the band) as a viewport: what both
    /// renderers draw the game world into.
    pub fn field_viewport(&self) -> (f32, f32, f32, f32) {
        (
            self.x,
            self.y + BAND_HEIGHT * self.scale,
            PLAYFIELD_WIDTH * self.scale,
            PLAYFIELD_HEIGHT * self.scale,
        )
    }
}

// -- layout: what the interface shows, as plain data ------------------------

/// A flat-color rectangle in canvas pixels.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Quad {
    pub center: [f32; 2],
    pub half_size: [f32; 2],
    pub color: [f32; 4],
}

impl Quad {
    fn rect(x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) -> Self {
        Self {
            center: [x + w / 2.0, y + h / 2.0],
            half_size: [w / 2.0, h / 2.0],
            color,
        }
    }
}

/// A run of text in one style.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub text: String,
    pub color: Rgb,
    pub bold: bool,
}

fn span(text: impl Into<String>, color: Rgb, bold: bool) -> Span {
    Span {
        text: text.into(),
        color,
        bold,
    }
}

/// One line of text, positioned in canvas pixels: `x` is the left edge for
/// `Align::Left`, the right edge for `Align::Right`, the center for
/// `Align::Center`; `y` is the top of the line box.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub spans: Vec<Span>,
    pub size: f32,
    /// Tracking in em; uppercase captions and titles are spaced out.
    pub tracking: f32,
    pub x: f32,
    pub y: f32,
    pub align: Align,
}

impl Line {
    fn line_height(&self) -> f32 {
        (self.size * 1.2).ceil()
    }

    #[cfg(test)]
    fn text(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }
}

/// Everything the HUD layer draws in one frame.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Layout {
    /// Under the world: band and field ground, the ceiling rule.
    pub backdrop: Vec<Quad>,
    /// Over the world: scrim, keycaps, life icons, the autopilot tag.
    pub overlay: Vec<Quad>,
    /// Over everything.
    pub lines: Vec<Line>,
}

/// Per-frame inputs that are not `Game` state.
#[derive(Debug, Clone, Copy)]
pub struct Frame<'a> {
    pub game: &'a Game,
    /// `--autopilot` is playing (PRODUCT.md: honest about the machine).
    pub autopilot: bool,
    /// Falling capsules at their on-screen canvas position, as the
    /// renderer drew them: field position plus band in 2D, the projected
    /// center in 3D.
    pub capsules: &'a [(f32, f32, PowerUpKind)],
    /// The stored best score for this progression (`record.rs`).
    pub record: Record,
}

/// The best score as the HUD shows it: `best` is `None` where no record is
/// kept (loaded levelsets); `new_best` marks the end screen of a run that
/// just beat it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Record {
    pub best: Option<u32>,
    pub new_best: bool,
}

/// The type scale, in canvas pixels. Steps of roughly 1.25 below the
/// display sizes, so a caption, a value and a title never look alike.
mod size {
    pub const DISPLAY: f32 = 64.0;
    pub const TITLE: f32 = 44.0;
    pub const VALUE: f32 = 20.0;
    pub const BODY: f32 = 15.0;
    pub const SUMMARY: f32 = 16.0;
    pub const NOTE: f32 = 13.0;
    pub const KEY: f32 = 11.0;
    pub const CAPTION: f32 = 10.0;
    pub const CAPSULE: f32 = 11.0;
}

const MARGIN_X: f32 = 20.0;
const CAPTION_Y: f32 = 8.0;
const VALUE_Y: f32 = 20.0;
const CAPTION_TRACKING: f32 = 0.16;
/// The field's center in canvas pixels.
const FIELD_CENTER_Y: f32 = BAND_HEIGHT + PLAYFIELD_HEIGHT / 2.0;

/// Width of `text` set in JetBrains Mono at `size` with `tracking`, in
/// canvas pixels. Exact for a monospace face, which is what lets keycaps
/// and tags size themselves without shaping.
pub fn text_width(text: &str, size: f32, tracking: f32) -> f32 {
    text.chars().count() as f32 * size * (ADVANCE_EM + tracking)
}

/// The score as six (or more) digits, leading zeros split off so they can
/// be set fainter: `(zeros, digits)`.
fn score_digits(score: u32) -> (String, String) {
    let padded = format!("{score:06}");
    let digits = score.to_string();
    let zeros = padded.len() - digits.len();
    (padded[..zeros].to_string(), digits)
}

/// The level as `("07", "/32")`.
fn level_digits(game: &Game) -> (String, String) {
    (
        format!("{:02}", game.level),
        format!("/{:02}", game.level_count()),
    )
}

/// A change in lives the HUD is still marking: the moment a life is lost
/// (or gained from a Player capsule) gets a beat of its own instead of an
/// icon silently disappearing. `age` is seconds since the change.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LifeCue {
    pub lost: bool,
    pub age: f32,
}

/// How long a [`LifeCue`] plays. One beat, no repetition: nothing in the
/// HUD flashes, let alone above 3 Hz (DESIGN.md, "Motion").
pub const LIFE_CUE_SECS: f32 = 0.9;

impl LifeCue {
    /// 1 at the change, easing out to 0 at [`LIFE_CUE_SECS`].
    fn strength(&self) -> f32 {
        let t = (self.age / LIFE_CUE_SECS).clamp(0.0, 1.0);
        (1.0 - t) * (1.0 - t)
    }
}

/// Builds the interface for one frame. Pure: same game and cue, same layout.
pub fn layout(frame: &Frame, cue: Option<LifeCue>) -> Layout {
    let game = frame.game;
    let mut out = Layout::default();
    backdrop(&mut out);
    band(&mut out, frame, cue);
    match game.state {
        GameState::Playing => {
            capsule_letters(&mut out, frame.capsules);
            if game.ball.attached && !frame.autopilot {
                serve_prompt(&mut out, game);
            }
        }
        state => state_screen(&mut out, game, state, frame.record),
    }
    out
}

fn backdrop(out: &mut Layout) {
    let band = palette::BAND.linear();
    let field = palette::FIELD.linear();
    out.backdrop
        .push(Quad::rect(0.0, 0.0, CANVAS_WIDTH, BAND_HEIGHT, band));
    out.backdrop.push(Quad::rect(
        0.0,
        BAND_HEIGHT,
        CANVAS_WIDTH,
        PLAYFIELD_HEIGHT,
        field,
    ));
    // The ceiling: a rule at the field's top edge, graduated every 25 px
    // with a major tick every 100 px -- the field drawn as the measured
    // 800-pixel instrument it is. Ticks rise into the band, never into the
    // field.
    let rule = palette::RULE_STRONG.linear();
    out.backdrop
        .push(Quad::rect(0.0, BAND_HEIGHT - 1.0, CANVAS_WIDTH, 1.0, rule));
    for i in 1..(CANVAS_WIDTH as u32 / 25) {
        let x = i as f32 * 25.0;
        let (h, color) = if i % 4 == 0 {
            (5.0, rule)
        } else {
            (2.0, palette::RULE.linear())
        };
        out.backdrop
            .push(Quad::rect(x - 0.5, BAND_HEIGHT - 1.0 - h, 1.0, h, color));
    }
}

fn caption(text: &str, x: f32, align: Align) -> Line {
    Line {
        spans: vec![span(text, palette::TEXT_DIM, true)],
        size: size::CAPTION,
        tracking: CAPTION_TRACKING,
        x,
        y: CAPTION_Y,
        align,
    }
}

fn band(out: &mut Layout, frame: &Frame, cue: Option<LifeCue>) {
    let game = frame.game;

    // Score, left.
    out.lines.push(caption("SCORE", MARGIN_X, Align::Left));
    let (zeros, digits) = score_digits(game.score);
    out.lines.push(Line {
        spans: vec![
            span(zeros, palette::TEXT_FAINT, true),
            span(digits, palette::TEXT, true),
        ],
        size: size::VALUE,
        tracking: 0.0,
        x: MARGIN_X,
        y: VALUE_Y,
        align: Align::Left,
    });

    // The autopilot tag, beside the score it is earning.
    if frame.autopilot {
        let label = "AUTOPILOT";
        let (pad, dot, gap) = (7.0, 4.0, 6.0);
        let w = pad + dot + gap + text_width(label, size::CAPTION, CAPTION_TRACKING) + pad;
        let (x, y, h) = (
            MARGIN_X + text_width("000000", size::VALUE, 0.0) + 18.0,
            24.0,
            17.0,
        );
        outline(&mut out.overlay, x, y, w, h, palette::RULE_STRONG.linear());
        out.overlay.push(Quad::rect(
            x + pad,
            y + (h - dot) / 2.0,
            dot,
            dot,
            palette::TEXT.linear(),
        ));
        out.lines.push(Line {
            spans: vec![span(label, palette::TEXT, true)],
            size: size::CAPTION,
            tracking: CAPTION_TRACKING,
            x: x + pad + dot + gap,
            y: y + 2.0,
            align: Align::Left,
        });
    }

    // Level, center: number, count, then the level's name if it has one.
    out.lines
        .push(caption("LEVEL", CANVAS_WIDTH / 2.0, Align::Center));
    let (number, count) = level_digits(game);
    let mut spans = vec![
        span(number, palette::TEXT, true),
        span(count, palette::TEXT_DIM, true),
    ];
    if let Some(name) = game.level_name() {
        let name: String = name.chars().take(28).collect();
        spans.push(span(format!("  {name}"), palette::TEXT, false));
    }
    out.lines.push(Line {
        spans,
        size: size::SUMMARY,
        tracking: 0.0,
        x: CANVAS_WIDTH / 2.0,
        y: VALUE_Y + 2.0,
        align: Align::Center,
    });

    // Lives, right: one small paddle per life, the count past six.
    let right = CANVAS_WIDTH - MARGIN_X;
    let cue = cue.filter(|c| c.age < LIFE_CUE_SECS);
    // During a cue the caption takes the cause's hue: the laser red of a
    // lost ball, the Player capsule's gold of a gained life.
    let cue_hue = cue.map(|c| {
        if c.lost {
            palette::CAPSULE_LASER
        } else {
            palette::CAPSULE_PLAYER
        }
    });
    let caption_color = match (cue, cue_hue) {
        (Some(c), Some(hue)) if c.strength() > 0.25 => hue,
        _ => palette::TEXT_DIM,
    };
    out.lines.push(Line {
        spans: vec![span("LIVES", caption_color, true)],
        ..caption("LIVES", right, Align::Right)
    });
    let (icon_w, icon_h, gap) = (16.0, 4.0, 6.0);
    let mut x = right;
    if game.lives > 6 {
        let extra = format!("+{}", game.lives - 5);
        out.lines.push(Line {
            spans: vec![span(extra.clone(), palette::TEXT, true)],
            size: size::NOTE,
            tracking: 0.0,
            x,
            y: 23.0,
            align: Align::Right,
        });
        x -= text_width(&extra, size::NOTE, 0.0) + gap;
    }
    let icons = if game.lives > 6 {
        5
    } else {
        game.lives as usize
    };
    let icon_y = 31.0 - icon_h / 2.0;
    for i in 0..icons {
        x -= icon_w;
        // A gained life's icon (the leftmost) starts gold and settles to
        // metal.
        let color = match (cue, cue_hue) {
            (Some(c), Some(hue)) if !c.lost && i + 1 == icons => {
                mix(palette::PADDLE.linear(), hue.linear(), c.strength())
            }
            _ => palette::PADDLE.linear(),
        };
        out.overlay
            .push(Quad::rect(x, icon_y, icon_w, icon_h, color));
        x -= gap;
    }
    if let (Some(c), Some(hue)) = (cue, cue_hue) {
        if c.lost {
            // The life that went: its icon, red, in the slot it left,
            // fading (and, with motion allowed, dropping a few pixels).
            if game.lives < 6 {
                let drop = if reduced_motion() {
                    0.0
                } else {
                    5.0 * (1.0 - c.strength())
                };
                out.overlay.push(Quad::rect(
                    x - icon_w,
                    icon_y + drop,
                    icon_w,
                    icon_h,
                    hue.linear_alpha(c.strength()),
                ));
            }
            // And where it went: the field's floor lights once, fading.
            out.overlay.push(Quad::rect(
                0.0,
                CANVAS_HEIGHT - 3.0,
                CANVAS_WIDTH,
                3.0,
                hue.linear_alpha(0.9 * c.strength()),
            ));
        }
    }
}

/// Linear mix of two linear RGBA colors, `t` of the way from `a` to `b`.
fn mix(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        a[3] + (b[3] - a[3]) * t,
    ]
}

/// A 1 px rectangle outline.
fn outline(quads: &mut Vec<Quad>, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
    quads.push(Quad::rect(x, y, w, 1.0, color));
    quads.push(Quad::rect(x, y + h - 1.0, w, 1.0, color));
    quads.push(Quad::rect(x, y, 1.0, h, color));
    quads.push(Quad::rect(x + w - 1.0, y, 1.0, h, color));
}

/// Capsule letters: centered on each capsule, in the field's ink.
fn capsule_letters(out: &mut Layout, capsules: &[(f32, f32, PowerUpKind)]) {
    for &(x, y, kind) in capsules {
        out.lines.push(Line {
            spans: vec![span(
                palette::capsule_letter(kind),
                palette::CAPSULE_INK,
                true,
            )],
            size: size::CAPSULE,
            tracking: 0.0,
            x,
            y: y - (size::CAPSULE * 1.2).ceil() / 2.0,
            align: Align::Center,
        });
    }
}

/// A keyboard key, drawn: an outlined cap with its label. Returns the
/// cap's width.
fn keycap(out: &mut Layout, label: &str, x: f32, y: f32) -> f32 {
    let (pad, h) = (7.0, 22.0);
    let w = (text_width(label, size::KEY, 0.04) + 2.0 * pad).max(h);
    out.overlay
        .push(Quad::rect(x, y, w, h, palette::BAND.linear()));
    outline(&mut out.overlay, x, y, w, h, palette::RULE_STRONG.linear());
    // Keycap bottom edge, a pixel heavier: the key reads as pressable.
    out.overlay.push(Quad::rect(
        x,
        y + h - 2.0,
        w,
        2.0,
        palette::RULE_STRONG.linear(),
    ));
    out.lines.push(Line {
        spans: vec![span(label, palette::TEXT, true)],
        size: size::KEY,
        tracking: 0.04,
        x: x + w / 2.0,
        y: y + 4.0,
        align: Align::Center,
    });
    w
}

/// The controls, as a two-column table: drawn keys right-aligned on the
/// field's center line, the action left of it. Keys in a group are
/// alternatives; groups are separated by a wider gap.
fn controls(out: &mut Layout, top: f32, pause_label: &str) {
    key_rows(
        out,
        top,
        &[
            (&[&["←", "→"], &["A", "D"]], "Move".into()),
            (&[&["SPACE"]], "Launch, release, fire".into()),
            (&[&["ESC"], &["P"]], pause_label.into()),
        ],
    );
}

/// Rows of drawn keys and the action they do: keys right-aligned on the
/// field's center line, the action left of it. Keys in a group are
/// alternatives; groups are separated by a wider gap.
fn key_rows(out: &mut Layout, top: f32, rows: &[(&[&[&str]], String)]) {
    let (key_gap, group_gap, pitch) = (4.0, 14.0, 34.0);
    let column = CANVAS_WIDTH / 2.0 - 8.0;
    for (i, (groups, action)) in rows.iter().enumerate() {
        let y = top + i as f32 * pitch;
        let width_of = |key: &str| (text_width(key, size::KEY, 0.04) + 14.0).max(22.0);
        let total: f32 = groups
            .iter()
            .map(|g| g.iter().map(|k| width_of(k)).sum::<f32>() + key_gap * (g.len() - 1) as f32)
            .sum::<f32>()
            + group_gap * (groups.len() - 1) as f32;
        let mut x = column - total;
        for (gi, group) in groups.iter().enumerate() {
            if gi > 0 {
                x += group_gap;
            }
            for (ki, key) in group.iter().enumerate() {
                if ki > 0 {
                    x += key_gap;
                }
                x += keycap(out, key, x, y);
            }
        }
        out.lines.push(Line {
            spans: vec![span(action.clone(), palette::TEXT, false)],
            size: size::BODY,
            tracking: 0.0,
            x: column + 18.0,
            y: y + 2.0,
            align: Align::Left,
        });
    }
}

fn title(text: &str, size: f32, tracking: f32, y: f32) -> Line {
    Line {
        spans: vec![span(text, palette::TEXT, true)],
        size,
        tracking,
        // Tracking also follows the last letter; shift by half of it so
        // the word, not its trailing space, is centered.
        x: CANVAS_WIDTH / 2.0 + size * tracking / 2.0,
        y,
        align: Align::Center,
    }
}

fn centered(spans: Vec<Span>, size: f32, tracking: f32, y: f32) -> Line {
    Line {
        spans,
        size,
        tracking,
        x: CANVAS_WIDTH / 2.0 + size * tracking / 2.0,
        y,
        align: Align::Center,
    }
}

/// The slate: an opaque full-width band of field ink behind a state
/// screen's text, ruled top and bottom like the ceiling. The scrim dims the
/// scene; the slate clears it from behind the words. That matters in 3D,
/// where the perspective puts the bricks and the paddle exactly where the
/// title and the prompt sit. Full-bleed, so it reads as a band across the
/// instrument rather than a card floating on it.
fn slate(out: &mut Layout, top: f32, bottom: f32) {
    out.overlay.push(Quad::rect(
        0.0,
        top,
        CANVAS_WIDTH,
        bottom - top,
        palette::FIELD.linear(),
    ));
    let rule = palette::RULE.linear();
    out.overlay
        .push(Quad::rect(0.0, top, CANVAS_WIDTH, 1.0, rule));
    out.overlay
        .push(Quad::rect(0.0, bottom - 1.0, CANVAS_WIDTH, 1.0, rule));
}

/// While the ball waits on the paddle (each level's start, after each lost
/// life): which level this is, by number and name, and the key that
/// starts it. It sits in the empty middle of the field, between the bricks
/// and the paddle, and goes the moment the ball does. Hidden under the autopilot,
/// which launches on its own.
fn serve_prompt(out: &mut Layout, game: &Game) {
    let (number, count) = level_digits(game);
    let mut spans = vec![
        span(number, palette::TEXT, true),
        span(count, palette::TEXT_DIM, true),
    ];
    if let Some(name) = game.level_name() {
        let name: String = name.chars().take(28).collect();
        spans.push(span(format!("  {name}"), palette::TEXT, false));
    }
    // The field's middle band, clear in both lenses: below the deepest
    // brick rows and above the paddle, which the 3D camera draws about a
    // fifth of the field higher than 2D does.
    let y = FIELD_CENTER_Y + 10.0;
    out.lines.push(centered(spans, size::SUMMARY, 0.0, y));
    key_rows(out, y + 30.0, &[(&[&["SPACE"]], "Launch".into())]);
}

fn state_screen(out: &mut Layout, game: &Game, state: GameState, record: Record) {
    out.overlay.push(Quad::rect(
        0.0,
        BAND_HEIGHT,
        CANVAS_WIDTH,
        PLAYFIELD_HEIGHT,
        palette::SCRIM.linear_alpha(palette::SCRIM_ALPHA),
    ));
    let dim = palette::TEXT_DIM;
    match state {
        GameState::Menu => {
            slate(out, FIELD_CENTER_Y - 134.0, FIELD_CENTER_Y + 182.0);
            out.lines
                .push(title("BRYX", size::DISPLAY, 0.28, FIELD_CENTER_Y - 118.0));
            let set = match game.level_set() {
                LevelSetKind::Campaign => "CAMPAIGN",
                LevelSetKind::Classic => "CLASSIC",
                LevelSetKind::Loaded => "LEVELSET",
            };
            let mut subtitle = vec![span(
                format!("{set} · {} LEVELS", game.level_count()),
                dim,
                true,
            )];
            if let Some(best) = record.best.filter(|&b| b > 0) {
                let (zeros, digits) = score_digits(best);
                subtitle.push(span(" · BEST ", dim, true));
                subtitle.push(span(zeros, palette::TEXT_FAINT, true));
                subtitle.push(span(digits, palette::TEXT, true));
            }
            out.lines.push(centered(
                subtitle,
                size::NOTE,
                CAPTION_TRACKING,
                FIELD_CENTER_Y - 30.0,
            ));
            controls(out, FIELD_CENTER_Y + 22.0, "Pause");
            out.lines.push(centered(
                vec![span("PRESS SPACE TO START", palette::TEXT, true)],
                size::NOTE,
                CAPTION_TRACKING,
                FIELD_CENTER_Y + 150.0,
            ));
        }
        GameState::Paused => {
            slate(out, FIELD_CENTER_Y - 112.0, FIELD_CENTER_Y + 96.0);
            out.lines
                .push(title("PAUSED", size::TITLE, 0.2, FIELD_CENTER_Y - 96.0));
            controls(out, FIELD_CENTER_Y - 10.0, "Resume");
        }
        GameState::GameOver | GameState::Victory => {
            let victory = state == GameState::Victory;
            let (heading, reached) = if victory {
                let all = format!("ALL {} LEVELS", game.level_count());
                ("VICTORY", vec![span(all, palette::TEXT, true)])
            } else {
                let (number, count) = level_digits(game);
                let reached = vec![
                    span("LEVEL ", dim, true),
                    span(number, palette::TEXT, true),
                    span(count, dim, true),
                ];
                ("GAME OVER", reached)
            };
            slate(out, FIELD_CENTER_Y - 136.0, FIELD_CENTER_Y + 140.0);
            out.lines
                .push(title(heading, size::TITLE, 0.2, FIELD_CENTER_Y - 120.0));
            let (zeros, digits) = score_digits(game.score);
            let mut summary = vec![
                span("SCORE ", dim, true),
                span(zeros, palette::TEXT_FAINT, true),
                span(digits, palette::TEXT, true),
                span("    ", dim, true),
            ];
            summary.extend(reached);
            out.lines.push(centered(
                summary,
                size::SUMMARY,
                0.04,
                FIELD_CENTER_Y - 48.0,
            ));
            // The record: a new best in the Player capsule's gold (the one
            // reward hue), otherwise the best to beat.
            if record.new_best {
                out.lines.push(centered(
                    vec![span("NEW BEST", palette::CAPSULE_PLAYER, true)],
                    size::NOTE,
                    CAPTION_TRACKING,
                    FIELD_CENTER_Y - 20.0,
                ));
            } else if let Some(best) = record.best.filter(|&b| b > 0) {
                let (zeros, digits) = score_digits(best);
                out.lines.push(centered(
                    vec![
                        span("BEST ", dim, true),
                        span(zeros, palette::TEXT_FAINT, true),
                        span(digits, dim, true),
                    ],
                    size::NOTE,
                    CAPTION_TRACKING,
                    FIELD_CENTER_Y - 20.0,
                ));
            }
            // What the keys do now (`Game::tick`'s terminal states).
            let checkpoint = game.checkpoint();
            let rows: Vec<(&[&[&str]], String)> = if victory || checkpoint == 1 {
                vec![(&[&["SPACE"]], "Play again".into())]
            } else {
                vec![
                    (
                        &[&["SPACE"]],
                        format!("Continue from level {checkpoint:02}"),
                    ),
                    (&[&["ESC"], &["P"]], "New game".into()),
                ]
            };
            key_rows(out, FIELD_CENTER_Y + 20.0, &rows);
            out.lines.push(centered(
                vec![span("Close the window to quit", dim, false)],
                size::NOTE,
                0.0,
                FIELD_CENTER_Y + 106.0,
            ));
        }
        GameState::Playing => {}
    }
}

/// Turns the lives count, sampled once per rendered frame, into a
/// [`LifeCue`]. Render-side like the renderers' other juice: a diff
/// against the last frame, never a simulation hook (`events.rs`'s
/// `LifeLost` is drained before the frame renders).
#[derive(Debug, Default)]
struct LivesWatch {
    last: Option<u32>,
    change: Option<(bool, std::time::Instant)>,
}

impl LivesWatch {
    fn observe(&mut self, lives: u32, now: std::time::Instant) -> Option<LifeCue> {
        if let Some(last) = self.last {
            if lives != last {
                self.change = Some((lives < last, now));
            }
        }
        self.last = Some(lives);
        let (lost, at) = self.change?;
        let age = now.duration_since(at).as_secs_f32();
        if age >= LIFE_CUE_SECS {
            self.change = None;
            return None;
        }
        Some(LifeCue { lost, age })
    }
}

// -- GPU side ---------------------------------------------------------------

const MAX_QUADS: usize = 256;

fn shader_source() -> String {
    format!(
        r#"
struct Instance {{
    @location(0) center: vec2<f32>,
    @location(1) half_size: vec2<f32>,
    @location(2) color: vec4<f32>,
}};

struct VertexOutput {{
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
}};

const CANVAS: vec2<f32> = vec2<f32>({w:.1}, {h:.1});

@vertex
fn vs_main(@builtin(vertex_index) index: u32, inst: Instance) -> VertexOutput {{
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let p = inst.center + corners[index] * inst.half_size;
    var out: VertexOutput;
    out.position = vec4<f32>(p.x / CANVAS.x * 2.0 - 1.0, 1.0 - p.y / CANVAS.y * 2.0, 0.0, 1.0);
    out.color = inst.color;
    return out;
}}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {{
    return in.color;
}}
"#,
        w = CANVAS_WIDTH,
        h = CANVAS_HEIGHT
    )
}

const INSTANCE_ATTRS: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4];

/// The GPU half of the HUD layer: see the module doc for the draw order.
pub struct Hud {
    pipeline: wgpu::RenderPipeline,
    instances: wgpu::Buffer,
    backdrop_count: u32,
    overlay_count: u32,
    letterbox: Letterbox,
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    text_renderer: TextRenderer,
    buffers: Vec<(TextBuffer, f32, f32)>,
    lives: LivesWatch,
}

impl Hud {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hud shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source().into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hud pipeline layout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hud pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Quad>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &INSTANCE_ATTRS,
                })],
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
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hud quads"),
            size: (size_of::<Quad>() * MAX_QUADS) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Only the bundled faces: no system font scan at startup, and the
        // same glyphs on every machine.
        let mut db = glyphon::fontdb::Database::new();
        db.load_font_data(FONT_REGULAR.to_vec());
        db.load_font_data(FONT_BOLD.to_vec());
        let font_system = FontSystem::new_with_locale_and_db("en-US".to_string(), db);
        let text_cache = TextCache::new(device);
        let viewport = Viewport::new(device, &text_cache);
        let mut atlas = TextAtlas::new(device, queue, &text_cache, format);
        let text_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);

        Self {
            pipeline,
            instances,
            backdrop_count: 0,
            overlay_count: 0,
            letterbox: Letterbox::fit(1, 1),
            font_system,
            swash_cache: SwashCache::new(),
            viewport,
            atlas,
            text_renderer,
            buffers: Vec::new(),
            lives: LivesWatch::default(),
        }
    }

    /// Lays out `frame`, uploads its quads and shapes its text for a
    /// surface of `width`×`height`. Call once per frame, before either
    /// draw.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        frame: &Frame,
    ) {
        self.letterbox = Letterbox::fit(width, height);
        let cue = self
            .lives
            .observe(frame.game.lives, std::time::Instant::now());
        let layout = layout(frame, cue);
        let mut quads = layout.backdrop;
        quads.truncate(MAX_QUADS);
        self.backdrop_count = quads.len() as u32;
        quads.extend(layout.overlay);
        quads.truncate(MAX_QUADS);
        self.overlay_count = quads.len() as u32 - self.backdrop_count;
        queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&quads));

        let lb = self.letterbox;
        self.buffers.clear();
        for line in &layout.lines {
            let metrics = Metrics::new(line.size, line.line_height());
            let mut buffer = TextBuffer::new(&mut self.font_system, metrics);
            let box_width = CANVAS_WIDTH;
            buffer.set_size(Some(box_width), None);
            let spans = line.spans.iter().map(|s| {
                (
                    s.text.as_str(),
                    Attrs::new()
                        .family(Family::Name(FONT_FAMILY))
                        .weight(if s.bold { Weight::BOLD } else { Weight::NORMAL })
                        .letter_spacing(line.tracking)
                        .color(s.color.text()),
                )
            });
            buffer.set_rich_text(
                spans,
                &Attrs::new().family(Family::Name(FONT_FAMILY)),
                Shaping::Advanced,
                Some(line.align),
            );
            buffer.shape_until_scroll(&mut self.font_system, false);
            let left = match line.align {
                Align::Center => line.x - box_width / 2.0,
                Align::Right | Align::End => line.x - box_width,
                _ => line.x,
            };
            self.buffers
                .push((buffer, lb.x + left * lb.scale, lb.y + line.y * lb.scale));
        }
        let areas = self
            .buffers
            .iter()
            .map(|(buffer, left, top)| TextArea {
                buffer,
                left: *left,
                top: *top,
                scale: lb.scale,
                bounds: TextBounds::default(),
                default_color: palette::TEXT.text(),
                custom_glyphs: &[],
            })
            .collect::<Vec<_>>();
        self.viewport.update(queue, Resolution { width, height });
        self.text_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                areas,
                &mut self.swash_cache,
            )
            .expect("glyphon text preparation failed");
    }

    /// The letterbox computed by the last [`Hud::prepare`].
    pub fn letterbox(&self) -> Letterbox {
        self.letterbox
    }

    fn set_canvas_viewport(&self, pass: &mut wgpu::RenderPass<'_>) {
        let (x, y, w, h) = self.letterbox.canvas_viewport();
        pass.set_viewport(x, y, w, h, 0.0, 1.0);
    }

    /// Band and field ground: draw first, before the world.
    pub fn draw_backdrop(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.set_canvas_viewport(pass);
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        pass.draw(0..6, 0..self.backdrop_count);
    }

    /// Scrim, HUD chrome and all text: draw last, over the world.
    pub fn draw_overlay(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.set_canvas_viewport(pass);
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        let start = self.backdrop_count;
        pass.draw(0..6, start..start + self.overlay_count);
        // glyphon positions in surface pixels; give it the whole surface.
        let (w, h) = (
            self.viewport.resolution().width as f32,
            self.viewport.resolution().height as f32,
        );
        pass.set_viewport(0.0, 0.0, w, h, 0.0, 1.0);
        self.text_renderer
            .render(&self.atlas, &self.viewport, pass)
            .expect("glyphon text render failed");
    }

    /// Frees glyphs no longer drawn; call after submitting the frame.
    pub fn trim(&mut self) {
        self.atlas.trim();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::Game;

    fn frame(game: &Game) -> Frame<'_> {
        Frame {
            game,
            autopilot: false,
            capsules: &[],
            record: Record::default(),
        }
    }

    fn all_text(layout: &Layout) -> String {
        layout
            .lines
            .iter()
            .map(Line::text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn letterbox_keeps_the_canvas_aspect_and_centers_it() {
        let wide = Letterbox::fit(1600, 648);
        assert_eq!(wide.scale, 1.0);
        assert_eq!((wide.x, wide.y), (400.0, 0.0));
        let tall = Letterbox::fit(800, 1000);
        assert_eq!(tall.scale, 1.0);
        assert_eq!(tall.x, 0.0);
        assert!(tall.y > 0.0);
        let hidpi = Letterbox::fit(1600, 1296);
        assert_eq!((hidpi.scale, hidpi.x, hidpi.y), (2.0, 0.0, 0.0));
        let (_, fy, fw, fh) = hidpi.field_viewport();
        assert_eq!((fy, fw, fh), (96.0, 1600.0, 1200.0));
    }

    #[test]
    fn score_pads_to_six_digits_and_splits_the_zeros() {
        assert_eq!(score_digits(0), ("00000".into(), "0".into()));
        assert_eq!(score_digits(12_340), ("0".into(), "12340".into()));
        assert_eq!(score_digits(1_234_567), ("".into(), "1234567".into()));
    }

    #[test]
    fn every_state_names_itself_and_playing_shows_no_screen() {
        let mut game = Game::new();
        for (state, word) in [
            (GameState::Menu, "BRYX"),
            (GameState::Paused, "PAUSED"),
            (GameState::GameOver, "GAME OVER"),
            (GameState::Victory, "VICTORY"),
        ] {
            game.state = state;
            let layout = layout(&frame(&game), None);
            assert!(all_text(&layout).contains(word), "{state:?}");
        }
        game.state = GameState::Playing;
        let text = all_text(&layout(&frame(&game), None));
        assert!(!text.contains("BRYX") && !text.contains("PAUSED"));
        assert!(text.contains("SCORE") && text.contains("LIVES"));
    }

    #[test]
    fn game_over_and_victory_show_the_final_score() {
        let mut game = Game::new();
        game.score = 1234;
        for state in [GameState::GameOver, GameState::Victory] {
            game.state = state;
            let text = all_text(&layout(&frame(&game), None));
            assert!(text.contains("SCORE 001234"), "{state:?}: {text}");
        }
    }

    #[test]
    fn a_lost_life_lights_the_floor_once_then_clears() {
        let start = std::time::Instant::now();
        let at = |secs: f32| start + std::time::Duration::from_secs_f32(secs);
        let mut watch = LivesWatch::default();
        assert_eq!(watch.observe(3, at(0.0)), None, "first sample sets no cue");
        assert_eq!(watch.observe(3, at(0.1)), None);
        let cue = watch.observe(2, at(0.2)).expect("a loss starts a cue");
        assert!(cue.lost && cue.age == 0.0);
        assert!(watch.observe(2, at(0.6)).is_some_and(|c| c.lost));
        assert_eq!(watch.observe(2, at(0.2 + LIFE_CUE_SECS)), None);
        let gained = watch.observe(3, at(2.0)).expect("a gain starts a cue");
        assert!(!gained.lost);

        let mut game = Game::new();
        game.state = GameState::Playing;
        let floor = |cue| {
            layout(&frame(&game), cue).overlay.iter().any(|q| {
                q.center[1] + q.half_size[1] == CANVAS_HEIGHT
                    && q.half_size[0] == CANVAS_WIDTH / 2.0
            })
        };
        assert!(floor(Some(LifeCue {
            lost: true,
            age: 0.0
        })));
        assert!(!floor(Some(LifeCue {
            lost: false,
            age: 0.0
        })));
        assert!(!floor(None));
    }

    #[test]
    fn the_serve_prompt_names_the_level_and_the_key_until_launch() {
        let mut game = Game::new().with_campaign();
        game.state = GameState::Playing;
        let text = all_text(&layout(&frame(&game), None));
        assert!(
            text.contains("Warm-up") && text.contains("Launch"),
            "{text}"
        );
        let auto = all_text(&layout(
            &Frame {
                autopilot: true,
                ..frame(&game)
            },
            None,
        ));
        assert!(
            !auto.contains("Launch"),
            "the autopilot launches on its own"
        );
        game.ball.attached = false;
        assert!(!all_text(&layout(&frame(&game), None)).contains("Launch"));
    }

    #[test]
    fn game_over_offers_the_checkpoint_and_a_new_game() {
        let mut game = Game::new().with_campaign();
        game.state = GameState::GameOver;
        game.level = 12;
        let text = all_text(&layout(&frame(&game), None));
        assert!(
            text.contains("Continue from level 09") && text.contains("New game"),
            "{text}"
        );
        game.level = 5;
        let text = all_text(&layout(&frame(&game), None));
        assert!(
            text.contains("Play again") && !text.contains("Continue"),
            "{text}"
        );
    }

    #[test]
    fn the_record_shows_on_the_menu_and_a_new_best_on_the_end_screen() {
        let mut game = Game::new().with_campaign();
        let with = |game: &Game, record| {
            all_text(&layout(
                &Frame {
                    record,
                    ..frame(game)
                },
                None,
            ))
        };
        let best = Record {
            best: Some(20_000),
            new_best: false,
        };
        assert!(with(&game, best).contains("BEST 020000"));
        assert!(!with(&game, Record::default()).contains("BEST"));
        game.state = GameState::GameOver;
        let new_best = Record {
            best: Some(25_000),
            new_best: true,
        };
        assert!(with(&game, new_best).contains("NEW BEST"));
    }

    #[test]
    fn menu_and_pause_show_the_controls() {
        let mut game = Game::new();
        for state in [GameState::Menu, GameState::Paused] {
            game.state = state;
            let text = all_text(&layout(&frame(&game), None));
            for key in ["SPACE", "ESC", "A", "D", "←", "→"] {
                assert!(text.lines().any(|l| l == key), "{state:?} lacks {key}");
            }
        }
    }

    #[test]
    fn the_autopilot_tag_appears_only_with_the_autopilot() {
        let game = Game::new();
        let without = all_text(&layout(&frame(&game), None));
        let with = all_text(&layout(
            &Frame {
                autopilot: true,
                ..frame(&game)
            },
            None,
        ));
        assert!(!without.contains("AUTOPILOT"));
        assert!(with.contains("AUTOPILOT"));
    }

    #[test]
    fn capsule_letters_show_only_in_play() {
        let mut game = Game::new();
        let capsules = [(100.0, 200.0, PowerUpKind::Laser)];
        let with = |game: &Game| {
            layout(
                &Frame {
                    capsules: &capsules,
                    ..frame(game)
                },
                None,
            )
        };
        game.state = GameState::Playing;
        assert!(all_text(&with(&game)).lines().any(|l| l == "L"));
        game.state = GameState::Paused;
        assert!(!all_text(&with(&game)).lines().any(|l| l == "L"));
    }

    #[test]
    fn nothing_is_laid_out_off_canvas_and_band_text_stays_in_the_band() {
        let mut game = Game::new().with_campaign();
        game.lives = 9;
        game.score = 987_654;
        game.level = 30;
        for state in [
            GameState::Menu,
            GameState::Playing,
            GameState::Paused,
            GameState::GameOver,
            GameState::Victory,
        ] {
            game.state = state;
            let layout = layout(
                &Frame {
                    autopilot: true,
                    record: Record {
                        best: Some(999_999),
                        new_best: true,
                    },
                    ..frame(&game)
                },
                Some(LifeCue {
                    lost: true,
                    age: 0.1,
                }),
            );
            for q in layout.backdrop.iter().chain(&layout.overlay) {
                assert!(q.center[0] - q.half_size[0] >= 0.0);
                assert!(q.center[0] + q.half_size[0] <= CANVAS_WIDTH);
                assert!(q.center[1] - q.half_size[1] >= 0.0);
                assert!(q.center[1] + q.half_size[1] <= CANVAS_HEIGHT);
            }
            for line in &layout.lines {
                let w = text_width(&line.text(), line.size, line.tracking);
                let left = match line.align {
                    Align::Center => line.x - w / 2.0,
                    Align::Right => line.x - w,
                    _ => line.x,
                };
                assert!(left >= 0.0 && left + w <= CANVAS_WIDTH, "{:?}", line.text());
                if line.y < BAND_HEIGHT {
                    assert!(
                        line.y + line.line_height() <= BAND_HEIGHT,
                        "{:?}",
                        line.text()
                    );
                }
            }
        }
    }
}
