//! Kinetic effects: one visual signature per gameplay event, drawn the same
//! way by both renderers (docs/design/kinetic-fx.md).
//!
//! The flow, one direction only:
//!
//! ```text
//! Game tick ─ GameEvent ─┬─ audio::Director ─ note   (each keeps its own
//!                        └─ FxSystem::observe        rally.rs Rally: same
//!                              │                     step, same moment)
//!   every frame: FxSystem::update ─ FxFrame { triangles, pulses }
//!                              │
//!   renderer: scene + FxFrame triangles ─ fx::gpu bloom/shockwave ─ HUD
//! ```
//!
//! - **Gameplay is the source of truth.** Effects start from this tick's
//!   events (`observe`, called once per tick), never from diffing state,
//!   and never feed anything back: no rule, score or timer reads them.
//! - **Each effect is a pure function of its age.** It is spawned with all
//!   its parameters fixed (variety from a seed hashed from the event, not
//!   a global RNG), then `update` only advances ages by wall-clock time and
//!   rebuilds the frame's triangles. The same events and frame times give
//!   the same pictures.
//! - **Everything is bounded.** At most [`MAX_EFFECTS`] effects, a fixed
//!   pool of meshes, a fixed number of remembered cracks, and a frame of
//!   at most `geometry::MAX_VERTICES`. When full, the newest yields: an
//!   effect is dropped, never a buffer grown. After the first frames, the
//!   hot path allocates nothing.
//!
//! The grammar, one signature per event: a normal brick breaks with a
//! flash, splinters thrown off the struck face and light running along
//! it; an armored brick cracks from the real impact point and, on its
//! last hit, comes apart along that crack; an indestructible brick throws
//! back a cold arc, no debris; an explosive ball's blast is a white core,
//! a pressure wave and then, one by one as the wave reaches them, its
//! neighbors breaking; an asteroid falls apart into dull rock; a metal ball
//! strikes sparks and leaves a steel streak. The light's hue is the rally's
//! step, the note the director plays for the same brick.

pub mod geometry;
pub mod gpu;

use crate::events::{GameEvent, Striker};
use crate::game::{
    Ball, BallMode, Game, GameState, ASTEROID_BLAST_RADIUS, ASTEROID_RADIUS, BALL_BLAST_RADIUS,
    PLAYFIELD_HEIGHT, PLAYFIELD_WIDTH,
};
use crate::levels::BrickKind;
use crate::palette::{self, Rgb};
use crate::rally::{Rally, STEPS};
use geometry::{Canvas, Crack, Dice, Mesh, Templates, Tessellator, ROCKS, SHARDS};

/// How much of the system runs: `BRYX_FX=off|geometry|full` (default
/// full), for comparison and for slow machines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    /// Geometry, bloom and shockwaves.
    Full,
    /// Geometry only: no bloom, no distortion.
    Geometry,
    /// No effects at all; the scene still goes through the composite.
    Off,
}

impl Quality {
    pub fn from_env() -> Self {
        match std::env::var("BRYX_FX").as_deref() {
            Ok("off" | "0") => Self::Off,
            Ok("geometry") => Self::Geometry,
            _ => Self::Full,
        }
    }

    pub fn post(self) -> bool {
        self == Self::Full
    }
}

/// Effects alive at once, at most.
pub const MAX_EFFECTS: usize = 192;
/// Crack and fracture meshes in the pool.
const MESHES: usize = 48;
/// Cracked armored bricks remembered, so their fracture follows the crack.
const MAX_CRACKS: usize = 128;
/// Shockwaves the composite can distort with at once.
pub const MAX_PULSES: usize = 8;

/// Where an effect sits above the board in 3D: on a brick's top, on the
/// floor, at a ball's or an asteroid's center. The 2D renderer ignores it.
const BRICK_TOP: f32 = 22.5;
const FLOOR: f32 = 0.5;
const BALL_HEIGHT: f32 = 6.0;

/// A blast's wave front, in field pixels per second: the bricks it breaks
/// break as it reaches them, after the blast's own flash.
const BLAST_FRONT_SPEED: f32 = 420.0;

/// A shockwave for the composite pass, in field pixels: `fx::gpu` maps it
/// to the screen for whichever renderer draws it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pulse {
    pub center: [f32; 2],
    pub height: f32,
    pub radius: f32,
    pub width: f32,
    /// Displacement at the band's steepest, in field pixels.
    pub strength: f32,
    /// Light on the wavefront, added to the scene.
    pub light: f32,
}

/// What a renderer draws for the effects this frame.
#[derive(Debug, Default)]
pub struct FxFrame {
    pub canvas: Canvas,
    pub pulses: Vec<Pulse>,
    /// Whether anything in it emits light: without, the bloom has nothing
    /// to pick up and is skipped.
    pub glows: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Effect {
    age: f32,
    /// Seconds before it starts: a blast's neighbors wait for its front.
    delay: f32,
    life: f32,
    height: f32,
    kind: Kind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    /// Light over a brick, spreading a little as it fades.
    Flash {
        rect: [f32; 4],
        color: Rgb,
        peak: f32,
    },
    /// A ring of light expanding, its stroke thinning.
    Ring {
        center: [f32; 2],
        radius: (f32, f32),
        width: (f32, f32),
        color: Rgb,
        peak: f32,
    },
    /// Dull, opaque ring: dust, not light.
    Dust {
        center: [f32; 2],
        radius: (f32, f32),
        width: (f32, f32),
        color: Rgb,
    },
    /// An indestructible brick's rebound arc, facing the way the ball
    /// left.
    Arc { at: [f32; 2], angle: f32 },
    /// The hot core of a blast: a disc of light, shrinking into nothing.
    Core {
        center: [f32; 2],
        radius: (f32, f32),
        color: Rgb,
        peak: f32,
    },
    /// Light running along the struck face of a brick, from the impact.
    Glint {
        from: [f32; 2],
        a: [f32; 2],
        b: [f32; 2],
        color: Rgb,
        peak: f32,
    },
    /// Splinters (or rock chips) thrown from `origin` around `dir`.
    Burst {
        origin: [f32; 2],
        dir: [f32; 2],
        spread: f32,
        count: u8,
        speed: (f32, f32),
        size: (f32, f32),
        color: Rgb,
        rocks: bool,
        seed: u32,
    },
    /// Straight sparks: a metal ball striking.
    Sparks {
        origin: [f32; 2],
        dir: [f32; 2],
        spread: f32,
        count: u8,
        seed: u32,
    },
    /// A crack's light running out from the impact, then fading.
    CrackLight { mesh: usize },
    /// An armored brick coming apart along its crack.
    Fracture {
        mesh: usize,
        origin: [f32; 2],
        dir: [f32; 2],
        color: Rgb,
        seed: u32,
    },
    /// A shockwave for the composite pass: no geometry.
    Shock {
        center: [f32; 2],
        radius: f32,
        strength: f32,
        light: f32,
    },
    /// A level ending: a line of light crossing the field, down for a
    /// level cleared, right (out through the gate) for a warp.
    Sweep { color: Rgb, across: bool },
}

impl Kind {
    fn mesh(&self) -> Option<usize> {
        match *self {
            Kind::CrackLight { mesh } | Kind::Fracture { mesh, .. } => Some(mesh),
            _ => None,
        }
    }
}

/// The contact a brick break is drawn from.
#[derive(Debug, Clone, Copy)]
struct Contact {
    rect: [f32; 4],
    at: [f32; 2],
    normal: [f32; 2],
}

/// The effects: what is playing, and what the next frame draws.
pub struct FxSystem {
    quality: Quality,
    rally: Rally,
    /// Ticks observed: with an event's position, the seed of its variety.
    tick: u32,
    level: usize,
    tess: Tessellator,
    templates: Templates,
    effects: Vec<Effect>,
    meshes: Vec<Mesh>,
    mesh_busy: [bool; MESHES],
    cracks: Vec<Crack>,
    /// This tick's rally steps by event index, reused tick to tick.
    steps: Vec<Option<usize>>,
    frame: FxFrame,
}

fn normalize(v: [f32; 2]) -> [f32; 2] {
    let len = v[0].hypot(v[1]);
    if len < 1e-6 {
        [0.0, -1.0]
    } else {
        [v[0] / len, v[1] / len]
    }
}

fn seed(tick: u32, at: [f32; 2]) -> u32 {
    tick.wrapping_mul(0x85EB_CA6B)
        ^ (at[0].to_bits().rotate_left(7))
        ^ (at[1].to_bits().rotate_left(19))
}

/// Premultiplied light: additive, `intensity` times the color.
fn light(c: Rgb, intensity: f32) -> [f32; 4] {
    let [r, g, b, _] = c.linear();
    let k = intensity.max(0.0);
    [r * k, g * k, b * k, 0.0]
}

/// Premultiplied paint: covers what is under it, at `alpha`.
fn paint(c: Rgb, alpha: f32) -> [f32; 4] {
    let [r, g, b, _] = c.linear();
    let a = alpha.clamp(0.0, 1.0);
    [r * a, g * a, b * a, a]
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t) * (1.0 - t)
}

/// 1 until `from` of the way through, then down to 0 at the end.
fn fade_after(t: f32, from: f32) -> f32 {
    (1.0 - ((t - from) / (1.0 - from)).clamp(0.0, 1.0)).powi(2)
}

impl FxSystem {
    pub fn new(quality: Quality) -> Self {
        let mut tess = Tessellator::default();
        let templates = Templates::build(&mut tess);
        let mut frame = FxFrame::default();
        frame.pulses.reserve(MAX_PULSES);
        Self {
            quality,
            rally: Rally::default(),
            tick: 0,
            level: 0,
            tess,
            templates,
            effects: Vec::with_capacity(MAX_EFFECTS),
            meshes: vec![Mesh::default(); MESHES],
            mesh_busy: [false; MESHES],
            cracks: Vec::with_capacity(MAX_CRACKS),
            steps: Vec::with_capacity(64),
            frame,
        }
    }

    pub fn quality(&self) -> Quality {
        self.quality
    }

    fn spawn(&mut self, life: f32, delay: f32, height: f32, kind: Kind) {
        if self.effects.len() < MAX_EFFECTS {
            self.effects.push(Effect {
                age: 0.0,
                delay,
                life,
                height,
                kind,
            });
        } else if let Some(mesh) = kind.mesh() {
            self.mesh_busy[mesh] = false;
        }
    }

    fn take_mesh(&mut self) -> Option<usize> {
        let free = self.mesh_busy.iter().position(|&busy| !busy)?;
        self.mesh_busy[free] = true;
        Some(free)
    }

    /// One tick's events, in order. Call once per simulation tick, before
    /// the events are drained.
    pub fn observe(&mut self, game: &Game, events: &[GameEvent]) {
        self.tick = self.tick.wrapping_add(1);
        let mut steps = std::mem::take(&mut self.steps);
        steps.clear();
        steps.resize(events.len(), None);
        self.rally
            .observe(game, events, |i, step| steps[i] = Some(step));
        if game.state == GameState::Menu || game.level != self.level {
            self.cracks.clear();
            self.level = game.level;
        }
        if self.quality != Quality::Off {
            self.translate(events, &steps);
        }
        self.steps = steps;
    }

    fn translate(&mut self, events: &[GameEvent], steps: &[Option<usize>]) {
        let has = |want: &GameEvent| events.iter().any(|e| e == want);
        let won = has(&GameEvent::Victory);
        // The last contact that broke a brick: its BrickDestroyedAt follows.
        let mut contact: Option<Contact> = None;
        // A blast this tick: the bricks it breaks break as its front
        // arrives, thrown away from its center.
        let mut blast: Option<[f32; 2]> = None;
        for (i, event) in events.iter().enumerate() {
            match *event {
                GameEvent::BrickStruck {
                    rect,
                    at,
                    normal,
                    kind,
                    hits_left,
                    by,
                } => {
                    if by == Striker::MetalBall {
                        let count = if kind == BrickKind::Indestructible {
                            9
                        } else {
                            5
                        };
                        self.sparks(at, normal, count);
                    }
                    match (kind, hits_left) {
                        (BrickKind::Indestructible, _) => self.deflect(rect, at, normal),
                        (_, 0) => contact = Some(Contact { rect, at, normal }),
                        (BrickKind::Armored, _) => self.crack(rect, at, normal),
                        _ => {}
                    }
                }
                GameEvent::BrickDestroyedAt {
                    x,
                    y,
                    width,
                    height,
                    kind,
                } => {
                    let rect = [x, y, width, height];
                    let step = steps[i].unwrap_or(0);
                    let matched = contact.filter(|c| c.rect[0] == x && c.rect[1] == y);
                    let delay = match (matched, blast) {
                        (None, Some(center)) => {
                            let d = (x - center[0]).hypot(y - center[1]);
                            d / BLAST_FRONT_SPEED
                        }
                        _ => 0.0,
                    };
                    let hit = matched.unwrap_or_else(|| {
                        let away = blast.map_or([0.0, 1.0], |c| normalize([x - c[0], y - c[1]]));
                        Contact {
                            rect,
                            at: [x - away[0] * 8.0, y - away[1] * 8.0],
                            normal: [-away[0], -away[1]],
                        }
                    });
                    if matched.is_some() {
                        contact = None;
                    }
                    self.brick_break(hit, kind, step, delay, matched.is_some());
                }
                GameEvent::BallExploded { x, y } => {
                    self.blast([x, y]);
                    blast = Some([x, y]);
                }
                GameEvent::AsteroidExploded { x, y } => {
                    self.asteroid([x, y]);
                    blast = Some([x, y]);
                }
                GameEvent::LevelCleared if !won && !has(&GameEvent::Warped) => {
                    let kind = Kind::Sweep {
                        color: palette::rally(0),
                        across: false,
                    };
                    self.spawn(0.9, 0.0, FLOOR, kind);
                }
                GameEvent::Warped => {
                    let kind = Kind::Sweep {
                        color: palette::CAPSULE_BREAK,
                        across: true,
                    };
                    self.spawn(0.6, 0.0, FLOOR, kind);
                }
                _ => {}
            }
        }
    }

    /// A brick broken: by `hit` (the struck face, or the blast that broke
    /// it), at `step` of the rally, after `delay`. `struck` when a striker
    /// touched it (the light runs along that face).
    fn brick_break(
        &mut self,
        hit: Contact,
        kind: BrickKind,
        step: usize,
        delay: f32,
        struck: bool,
    ) {
        let color = palette::rally(step);
        let rect = hit.rect;
        let center = [rect[0], rect[1]];
        let seed = seed(self.tick, center);
        let upper_octave = step >= FX_SCALE_LEN;
        match kind {
            BrickKind::Armored => {
                let record = self
                    .cracks
                    .iter()
                    .position(|c| c.rect[0] == rect[0] && c.rect[1] == rect[1]);
                let crack = match record {
                    Some(i) => self.cracks.swap_remove(i),
                    None => Crack::new(rect, hit.at, hit.normal, seed),
                };
                if let Some(mesh) = self.take_mesh() {
                    crack.fracture_into(&mut self.tess, &mut self.meshes[mesh]);
                    let kind = Kind::Fracture {
                        mesh,
                        origin: crack.origin,
                        dir: hit.normal,
                        color: palette::BRICK_ARMORED_HIT,
                        seed,
                    };
                    self.spawn(0.6, delay, BRICK_TOP, kind);
                }
                self.spawn(
                    0.16,
                    delay,
                    BRICK_TOP,
                    Kind::Flash {
                        rect,
                        color,
                        peak: 2.6,
                    },
                );
                let ring = Kind::Ring {
                    center: crack.origin,
                    radius: (6.0, 34.0),
                    width: (2.4, 0.6),
                    color,
                    peak: 2.2,
                };
                self.spawn(0.3, delay, BRICK_TOP, ring);
                let shock = Kind::Shock {
                    center,
                    radius: 42.0,
                    strength: 2.0,
                    light: 0.01,
                };
                self.spawn(0.3, delay, BRICK_TOP, shock);
            }
            _ => {
                self.spawn(
                    0.12,
                    delay,
                    BRICK_TOP,
                    Kind::Flash {
                        rect,
                        color,
                        peak: 2.0,
                    },
                );
                let burst = Kind::Burst {
                    origin: hit.at,
                    dir: hit.normal,
                    spread: 0.8,
                    count: 4,
                    speed: (120.0, 240.0),
                    size: (2.5, 4.5),
                    color: palette::brick(kind, 1),
                    rocks: false,
                    seed,
                };
                self.spawn(0.36, delay, BRICK_TOP, burst);
                if struck {
                    self.glint(rect, hit.at, hit.normal, color, delay);
                }
            }
        }
        // The rally's upper octave: a ring, then two at the top of the
        // scale. Momentum shown by shape, not by more brightness.
        if upper_octave {
            let rings = if step == STEPS - 1 { 2 } else { 1 };
            for k in 0..rings {
                let ring = Kind::Ring {
                    center,
                    radius: (8.0, 30.0 + 12.0 * k as f32),
                    width: (1.6, 0.4),
                    color,
                    peak: 1.6,
                };
                self.spawn(0.28, delay + 0.05 * k as f32, BRICK_TOP, ring);
            }
        }
    }

    /// Light along the struck face, spreading both ways from the impact.
    fn glint(&mut self, rect: [f32; 4], at: [f32; 2], normal: [f32; 2], color: Rgb, delay: f32) {
        let [cx, cy, w, h] = rect;
        let (a, b, from) = if normal[1].abs() >= normal[0].abs() {
            let y = cy + normal[1].signum() * h / 2.0;
            let x = at[0].clamp(cx - w / 2.0, cx + w / 2.0);
            ([cx - w / 2.0, y], [cx + w / 2.0, y], [x, y])
        } else {
            let x = cx + normal[0].signum() * w / 2.0;
            let y = at[1].clamp(cy - h / 2.0, cy + h / 2.0);
            ([x, cy - h / 2.0], [x, cy + h / 2.0], [x, y])
        };
        let kind = Kind::Glint {
            from,
            a,
            b,
            color,
            peak: 2.4,
        };
        self.spawn(0.2, delay, BRICK_TOP, kind);
    }

    /// An armored brick's first hit: a crack from the impact point, lit
    /// as it runs and then left as a fissure until the brick breaks.
    fn crack(&mut self, rect: [f32; 4], at: [f32; 2], normal: [f32; 2]) {
        let crack = Crack::new(rect, at, normal, seed(self.tick, at));
        if let Some(i) = self
            .cracks
            .iter()
            .position(|c| c.rect[0] == rect[0] && c.rect[1] == rect[1])
        {
            self.cracks[i] = crack;
        } else if self.cracks.len() < MAX_CRACKS {
            self.cracks.push(crack);
        }
        if let Some(mesh) = self.take_mesh() {
            crack.stroke_into(&mut self.tess, 3.2, &mut self.meshes[mesh]);
            self.spawn(0.5, 0.0, BRICK_TOP, Kind::CrackLight { mesh });
        }
        let flash = Kind::Flash {
            rect,
            color: palette::FX_CRACK,
            peak: 1.4,
        };
        self.spawn(0.1, 0.0, BRICK_TOP, flash);
    }

    /// An indestructible brick turning a striker back: a cold arc on the
    /// face and light along it. Resistance, not damage: nothing breaks off.
    fn deflect(&mut self, rect: [f32; 4], at: [f32; 2], normal: [f32; 2]) {
        let [cx, cy, w, h] = rect;
        let on_face = [
            at[0].clamp(cx - w / 2.0, cx + w / 2.0),
            at[1].clamp(cy - h / 2.0, cy + h / 2.0),
        ];
        let angle = normal[1].atan2(normal[0]);
        self.spawn(0.18, 0.0, BRICK_TOP, Kind::Arc { at: on_face, angle });
        self.glint(rect, at, normal, palette::FX_DEFLECT, 0.0);
    }

    fn sparks(&mut self, at: [f32; 2], normal: [f32; 2], count: u8) {
        let kind = Kind::Sparks {
            origin: at,
            dir: normal,
            spread: 0.9,
            count,
            seed: seed(self.tick, at) ^ 0xA5A5,
        };
        self.spawn(0.16, 0.0, BALL_HEIGHT, kind);
    }

    /// An explosive ball's blast: the white core and the wave first; the
    /// bricks it breaks follow as the front reaches them (`translate`).
    fn blast(&mut self, center: [f32; 2]) {
        let core = Kind::Core {
            center,
            radius: (8.0, 24.0),
            color: palette::FX_BLAST_CORE,
            peak: 3.0,
        };
        self.spawn(0.14, 0.0, BRICK_TOP, core);
        let ring = Kind::Ring {
            center,
            radius: (10.0, BALL_BLAST_RADIUS * 1.7),
            width: (5.0, 1.0),
            color: palette::EXPLOSION,
            peak: 2.4,
        };
        self.spawn(0.34, 0.0, BRICK_TOP, ring);
        let shock = Kind::Shock {
            center,
            radius: 64.0,
            strength: 4.5,
            light: 0.015,
        };
        self.spawn(0.4, 0.0, BRICK_TOP, shock);
    }

    /// An asteroid breaking up: dull rock and dust, a little fire inside.
    fn asteroid(&mut self, center: [f32; 2]) {
        let seed = seed(self.tick, center);
        let burst = Kind::Burst {
            origin: center,
            dir: [0.0, -1.0],
            spread: std::f32::consts::PI,
            count: 7,
            speed: (50.0, 150.0),
            size: (2.5, 5.5),
            color: palette::ASTEROID,
            rocks: true,
            seed,
        };
        self.spawn(0.7, 0.0, ASTEROID_RADIUS, burst);
        let dust = Kind::Dust {
            center,
            radius: (ASTEROID_RADIUS, ASTEROID_BLAST_RADIUS),
            width: (8.0, 2.0),
            color: palette::ASTEROID,
        };
        self.spawn(0.42, 0.0, FLOOR, dust);
        let core = Kind::Core {
            center,
            radius: (ASTEROID_RADIUS * 0.6, ASTEROID_RADIUS * 1.2),
            color: palette::EXPLOSION,
            peak: 1.6,
        };
        self.spawn(0.14, 0.0, ASTEROID_RADIUS, core);
        let shock = Kind::Shock {
            center,
            radius: ASTEROID_BLAST_RADIUS * 1.3,
            strength: 3.0,
            light: 0.01,
        };
        self.spawn(0.35, 0.0, ASTEROID_RADIUS, shock);
    }

    /// Advances every effect by `dt` seconds of wall-clock time and builds
    /// the frame: their triangles and shockwaves, plus the streaks behind
    /// fast and powered balls. `ball` is the main ball as drawn this frame
    /// (interpolated); the others are read from `game`.
    pub fn update(&mut self, dt: f32, game: &Game, ball: &Ball) -> &FxFrame {
        let dt = dt.clamp(0.0, 0.1);
        let mut i = 0;
        while i < self.effects.len() {
            let e = &mut self.effects[i];
            e.age += dt;
            if e.age >= e.delay + e.life {
                if let Some(mesh) = e.kind.mesh() {
                    self.mesh_busy[mesh] = false;
                }
                self.effects.swap_remove(i);
            } else {
                i += 1;
            }
        }

        let mut frame = std::mem::take(&mut self.frame);
        frame.canvas.clear();
        frame.pulses.clear();
        frame.glows = false;
        if self.quality != Quality::Off && game.state != GameState::Menu {
            self.fissures(&mut frame.canvas);
            for e in &self.effects {
                let t = e.age - e.delay;
                if t >= 0.0 {
                    self.draw(e, t / e.life, &mut frame);
                    frame.glows = true;
                }
            }
            let power = game.ball_power.map(|p| p.mode);
            for b in std::iter::once(ball).chain(&game.extra_balls) {
                frame.glows |= streak(&mut frame.canvas, b, power);
            }
        }
        self.frame = frame;
        &self.frame
    }

    /// Every cracked armored brick's crack, as thin dark lines: the brick
    /// remembers where it was hit until it breaks.
    fn fissures(&self, canvas: &mut Canvas) {
        let ink = paint(palette::FX_FISSURE, 0.8);
        for crack in &self.cracks {
            for arm in crack.arms() {
                for pair in arm.windows(2) {
                    canvas.segment(pair[0], pair[1], 1.2, 1.2, BRICK_TOP, ink, ink);
                }
            }
        }
    }

    fn draw(&self, e: &Effect, t: f32, frame: &mut FxFrame) {
        let canvas = &mut frame.canvas;
        let h = e.height;
        let t = t.clamp(0.0, 1.0);
        match e.kind {
            Kind::Flash { rect, color, peak } => {
                // The brick lit from within, its outline burning hot and
                // spilling outward: bright at the edge, never a white box.
                let k = (1.0 - t).powi(2);
                let spread = 3.0 + 7.0 * ease_out(t);
                let inner = light(color, 0.3 * peak * k);
                let edge = light(color, peak * k);
                canvas.glow_rect(rect, spread, h, inner, edge);
            }
            Kind::Ring {
                center,
                radius,
                width,
                color,
                peak,
            } => {
                let r = radius.0 + (radius.1 - radius.0) * ease_out(t);
                let w = width.0 + (width.1 - width.0) * t;
                let c = light(color, peak * (1.0 - t).powf(1.5));
                canvas.stroke(&self.templates.ring, center, 0.0, r, w, h, c);
            }
            Kind::Dust {
                center,
                radius,
                width,
                color,
            } => {
                let r = radius.0 + (radius.1 - radius.0) * ease_out(t);
                let w = width.0 + (width.1 - width.0) * t;
                let c = paint(color, 0.45 * (1.0 - t));
                canvas.stroke(&self.templates.ring, center, 0.0, r, w, h, c);
            }
            Kind::Arc { at, angle } => {
                let r = 7.0 + 8.0 * ease_out(t);
                let w = 2.4 - 1.8 * t;
                let c = light(palette::FX_DEFLECT, 2.2 * (1.0 - t).powi(2));
                canvas.stroke(&self.templates.arc, at, angle, r, w, h, c);
            }
            Kind::Core {
                center,
                radius,
                color,
                peak,
            } => {
                let r = radius.0 + (radius.1 - radius.0) * ease_out(t);
                let c = light(color, peak * (1.0 - t).powi(3));
                canvas.fill(&self.templates.disc, center, 0.0, r, h, c);
            }
            Kind::Glint {
                from,
                a,
                b,
                color,
                peak,
            } => {
                // Runs out from the impact both ways in the first third.
                let reach = ease_out((t * 3.0).min(1.0));
                let toward = |end: [f32; 2]| {
                    [
                        from[0] + (end[0] - from[0]) * reach,
                        from[1] + (end[1] - from[1]) * reach,
                    ]
                };
                let hot = light(color, peak * (1.0 - t).powi(2));
                let cool = light(color, 0.0);
                let w = 2.2 - 1.4 * t;
                canvas.segment(from, toward(a), w, 0.4, h, hot, cool);
                canvas.segment(from, toward(b), w, 0.4, h, hot, cool);
            }
            Kind::Burst {
                origin,
                dir,
                spread,
                count,
                speed,
                size,
                color,
                rocks,
                seed,
            } => {
                let mut dice = Dice::new(seed);
                let base = dir[1].atan2(dir[0]);
                let alpha = fade_after(t, 0.55);
                // Splinters leave hot and cool to the brick's own color.
                let glow = if rocks {
                    0.0
                } else {
                    1.5 * (1.0 - t * 5.0).max(0.0)
                };
                for _ in 0..count {
                    let angle = base + dice.range(-spread, spread);
                    let v = dice.range(speed.0, speed.1);
                    let scale = dice.range(size.0, size.1);
                    let spin = dice.range(-14.0, 14.0);
                    let hop = dice.range(6.0, 18.0);
                    let shape = (dice.next() * if rocks { ROCKS } else { SHARDS } as f32) as usize;
                    let age = t * e.life;
                    // Decelerating to rest at the end of its life.
                    let d = v * (age - age * age / (2.0 * e.life));
                    let at = [origin[0] + angle.cos() * d, origin[1] + angle.sin() * d];
                    let mesh = if rocks {
                        &self.templates.rocks[shape.min(ROCKS - 1)]
                    } else {
                        &self.templates.shards[shape.min(SHARDS - 1)]
                    };
                    let [r, g, b, _] = paint(color, alpha);
                    let k = 1.0 + glow;
                    let c = [r * k, g * k, b * k, alpha];
                    let lift = h + hop * 4.0 * t * (1.0 - t);
                    canvas.fill(mesh, at, spin * age, scale * (1.0 - 0.3 * t), lift, c);
                }
            }
            Kind::Sparks {
                origin,
                dir,
                spread,
                count,
                seed,
            } => {
                let mut dice = Dice::new(seed);
                let base = dir[1].atan2(dir[0]);
                let age = t * e.life;
                for _ in 0..count {
                    let angle = base + dice.range(-spread, spread);
                    let v = dice.range(260.0, 520.0);
                    let len = dice.range(6.0, 12.0) * (1.0 - t);
                    let d = v * (age - age * age / (2.0 * e.life));
                    let u = [angle.cos(), angle.sin()];
                    let head = [origin[0] + u[0] * d, origin[1] + u[1] * d];
                    let tail = [head[0] - u[0] * len, head[1] - u[1] * len];
                    let hot = light(palette::FX_SPARK, 2.6 * (1.0 - t));
                    canvas.segment(head, tail, 1.6, 0.3, h, hot, light(palette::FX_SPARK, 0.0));
                }
            }
            Kind::CrackLight { mesh } => {
                let mesh = &self.meshes[mesh];
                // Runs out at 900 px/s, then its light dies away.
                let front = t * e.life * 900.0;
                let fade = (1.0 - t).powi(2);
                canvas.part(mesh, 0, h, |v| {
                    let lit = ((front - v.t) / 6.0).clamp(0.0, 1.0);
                    (
                        [v.p[0] + v.o[0], v.p[1] + v.o[1]],
                        light(palette::FX_CRACK, 2.4 * lit * fade),
                    )
                });
            }
            Kind::Fracture {
                mesh,
                origin,
                dir,
                color,
                seed,
            } => {
                let mesh = &self.meshes[mesh];
                let mut dice = Dice::new(seed);
                let age = t * e.life;
                let alpha = fade_after(t, 0.5);
                for (k, part) in mesh.parts.iter().enumerate() {
                    // Each piece leaves the impact point, pushed out of the
                    // struck face, turning as it goes.
                    let out = normalize([
                        part.center[0] - origin[0] + dir[0] * 6.0,
                        part.center[1] - origin[1] + dir[1] * 6.0,
                    ]);
                    let v = dice.range(60.0, 130.0);
                    let spin = dice.range(-6.0, 6.0);
                    let hop = dice.range(10.0, 24.0);
                    let d = v * (age - age * age / (2.0 * e.life));
                    let at = [part.center[0] + out[0] * d, part.center[1] + out[1] * d];
                    let (s, c) = (spin * age).sin_cos();
                    let shrink = 1.0 - 0.25 * t;
                    let glow = 1.0 + 1.2 * (1.0 - t * 6.0).max(0.0);
                    let [r, g, b, _] = paint(color, alpha);
                    let paint = [r * glow, g * glow, b * glow, alpha];
                    let lift = h + hop * 4.0 * t * (1.0 - t);
                    canvas.part(mesh, k, lift, |v| {
                        let (x, y) = (v.p[0] * shrink, v.p[1] * shrink);
                        ([at[0] + x * c - y * s, at[1] + x * s + y * c], paint)
                    });
                }
            }
            Kind::Shock {
                center,
                radius,
                strength,
                light,
            } => {
                if frame.pulses.len() < MAX_PULSES {
                    frame.pulses.push(Pulse {
                        center,
                        height: h,
                        radius: radius * ease_out(t),
                        width: 9.0 + 6.0 * t,
                        strength: strength * (1.0 - t).powi(2),
                        light: light * (1.0 - t),
                    });
                }
            }
            Kind::Sweep { color, across } => {
                let s = ease_out(t);
                let fade = (std::f32::consts::PI * t).sin();
                let hot = light(color, 1.8 * fade);
                let trail = light(color, 0.25 * fade);
                let none = light(color, 0.0);
                if across {
                    let x = PLAYFIELD_WIDTH * s;
                    canvas.segment([x, 0.0], [x, PLAYFIELD_HEIGHT], 3.0, 3.0, h, hot, hot);
                    canvas.quad(
                        [
                            [x - 60.0, 0.0],
                            [x, 0.0],
                            [x, PLAYFIELD_HEIGHT],
                            [x - 60.0, PLAYFIELD_HEIGHT],
                        ],
                        h,
                        [none, trail, trail, none],
                    );
                } else {
                    let y = PLAYFIELD_HEIGHT * s;
                    canvas.segment([0.0, y], [PLAYFIELD_WIDTH, y], 3.0, 3.0, h, hot, hot);
                    canvas.quad(
                        [
                            [0.0, y - 60.0],
                            [PLAYFIELD_WIDTH, y - 60.0],
                            [PLAYFIELD_WIDTH, y],
                            [0.0, y],
                        ],
                        h,
                        [none, none, trail, trail],
                    );
                }
            }
        }
    }
}

const FX_SCALE_LEN: usize = palette::FX_SCALE.len();

/// The streak behind a ball: a metal ball's steel, an explosive ball's
/// charge, and for an unpowered ball, only past 520 px/s, a faint trace
/// of its speed. A tapered quad behind it, never ahead, so the ball's own
/// sprite and trail stay what the eye follows.
///
/// Returns whether it drew one.
fn streak(canvas: &mut Canvas, ball: &Ball, power: Option<BallMode>) -> bool {
    if ball.attached || ball.glued.is_some() {
        return false;
    }
    let speed = ball.vx.hypot(ball.vy);
    let (color, seconds, intensity, core) = match power {
        Some(BallMode::Metal) => (palette::CAPSULE_METAL_BALL, 0.07, 1.5, true),
        Some(BallMode::Explosive) => (palette::CAPSULE_EXPLOSIVE_BALL, 0.05, 1.2, false),
        None if speed > 520.0 => (palette::BALL, 0.03 * (speed - 520.0) / 180.0, 0.6, false),
        None => return false,
    };
    if speed < 1.0 {
        return false;
    }
    let u = [ball.vx / speed, ball.vy / speed];
    let len = speed * seconds;
    let head = [
        ball.x - u[0] * ball.radius * 0.5,
        ball.y - u[1] * ball.radius * 0.5,
    ];
    let tail = [head[0] - u[0] * len, head[1] - u[1] * len];
    let w = ball.radius * 1.6;
    canvas.segment(
        head,
        tail,
        w,
        0.0,
        BALL_HEIGHT,
        light(color, intensity),
        light(color, 0.0),
    );
    if core {
        // Metal's high-energy line: a white-hot core down the streak.
        let hot = light(palette::FX_SPARK, 2.2);
        let short = [head[0] - u[0] * len * 0.6, head[1] - u[1] * len * 0.6];
        canvas.segment(
            head,
            short,
            1.4,
            0.0,
            BALL_HEIGHT,
            hot,
            light(palette::FX_SPARK, 0.0),
        );
    }
    true
}

#[cfg(test)]
mod tests;
