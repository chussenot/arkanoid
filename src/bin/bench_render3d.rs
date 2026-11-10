//! One-off perf harness for arkanoid-v2-c5 ("measure before optimizing"):
//! drives the *real* drawing code (`Renderer3D::render_offscreen_for_bench`,
//! which shares its guts with the production `render()` via `render_into`)
//! at this bead's named stress scenario -- a full 14x8 board (112 bricks),
//! Multiball's 2 extra balls, and 20 concurrently tumbling destroy-ghosts --
//! and reports measured frame times/fps. Offscreen rather than through the
//! real swapchain deliberately: a desktop compositor's present-mode pacing
//! (vsync, or throttling an unfocused/off-screen window to as little as 1
//! fps to save power -- both observed while building this) has nothing to
//! do with the renderer's own cost, so this measures CPU-enqueue + GPU-
//! execute time only, forced synchronous via `device.poll`.
//!
//! Not part of `cargo test`/CI: this is a manual `cargo run --release
//! --bin bench_render3d` tool, kept around so the next perf pass on this
//! renderer has a repeatable way to re-measure rather than re-deriving one.
//! Reuses the crate's real modules via `#[path]` (this file is `main.rs`
//! for its own separate binary crate, per Cargo's `src/bin/` convention,
//! so it has no access to the `bryx` binary's modules otherwise --
//! there is no `src/lib.rs` to depend on). `#![allow(dead_code)]`: most of
//! `render.rs`'s classic 2D renderer and HUD text plumbing comes along for
//! the ride (only `RenderState` is actually needed here) since it's all one
//! file: real dead code in the `bryx` binary would be a bug, but here
//! it's an unavoidable side effect of reusing the module wholesale.
#![allow(dead_code)]

#[path = "../assets.rs"]
mod assets;
#[path = "../events.rs"]
mod events;
#[path = "../fx/mod.rs"]
mod fx;
#[path = "../game.rs"]
mod game;
#[path = "../hud.rs"]
mod hud;
#[path = "../levels.rs"]
mod levels;
#[path = "../levelset.rs"]
mod levelset;
#[path = "../palette.rs"]
mod palette;
#[path = "../rally.rs"]
mod rally;
#[path = "../render.rs"]
mod render;
#[path = "../render3d/mod.rs"]
mod render3d;

use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use events::{GameEvent, Striker};
use game::{Ball, Brick, Game, GameState, PowerUp};
use levels::BrickKind;
use render::{RenderState, Renderer};
use render3d::Renderer3D;

/// The kinetic-effects load (docs/design/kinetic-fx.md, "Performance"):
/// one tick's worth of the heaviest events, every `FX_EVERY` frames on a
/// fixed 1/60 s clock, so the effect population is the same on any
/// machine: brick breaks with contacts, an armored fracture, an explosive
/// blast breaking its ring, an asteroid, metal strikes.
const FX_EVERY: usize = 6;
const FX_DT: f32 = 1.0 / 60.0;

fn fx_events(frame: usize) -> Vec<GameEvent> {
    let mut events = Vec::new();
    let base = (frame / FX_EVERY) as f32;
    for i in 0..6 {
        let x = 60.0 + ((base * 97.0 + i as f32 * 113.0) % 680.0);
        let y = 80.0 + ((base * 41.0 + i as f32 * 23.0) % 160.0);
        let kind = if i == 0 {
            BrickKind::Armored
        } else {
            BrickKind::Normal
        };
        let by = if i == 1 {
            Striker::MetalBall
        } else {
            Striker::Ball
        };
        events.push(GameEvent::BrickStruck {
            rect: [x, y, levels::BRICK_WIDTH, levels::BRICK_HEIGHT],
            at: [x + 6.0, y + 17.0],
            normal: [0.0, 1.0],
            kind,
            hits_left: 0,
            by,
        });
        events.push(GameEvent::BrickDestroyedAt {
            x,
            y,
            width: levels::BRICK_WIDTH,
            height: levels::BRICK_HEIGHT,
            kind,
        });
    }
    events.push(GameEvent::BallExploded { x: 400.0, y: 150.0 });
    for dx in [-52.0, 52.0] {
        events.push(GameEvent::BrickDestroyedAt {
            x: 400.0 + dx,
            y: 150.0,
            width: levels::BRICK_WIDTH,
            height: levels::BRICK_HEIGHT,
            kind: BrickKind::Normal,
        });
    }
    events.push(GameEvent::AsteroidExploded { x: 250.0, y: 300.0 });
    events.push(GameEvent::BrickStruck {
        rect: [600.0, 120.0, levels::BRICK_WIDTH, levels::BRICK_HEIGHT],
        at: [600.0, 137.0],
        normal: [0.0, 1.0],
        kind: BrickKind::Indestructible,
        hits_left: 1,
        by: Striker::MetalBall,
    });
    events
}

/// `BENCH_RENDERER=2d` benches the classic renderer; anything else, 3D.
enum AnyRenderer {
    Classic(Box<Renderer>),
    ThreeD(Box<Renderer3D>),
}

/// `BENCH_SIZE=WxH` in physical pixels (default 800x648, the canvas at
/// 1x): the bloom chain and the composite scale with the pixel count. The
/// frame is drawn offscreen at exactly this size: the renderer is resized
/// to it after creation, because a Wayland compositor sizes the window as
/// it likes (945x1023 here, whatever was requested) and `Renderer::new`
/// reads that. The report's header prints the sizes actually drawn.
fn bench_size() -> PhysicalSize<u32> {
    let parse = |s: &str| {
        let (w, h) = s.split_once('x')?;
        Some(PhysicalSize::new(w.parse().ok()?, h.parse().ok()?))
    };
    std::env::var("BENCH_SIZE")
        .ok()
        .and_then(|s| parse(&s))
        .unwrap_or(PhysicalSize::new(800, 648))
}

/// Frames discarded before measuring starts (pipeline warm-up: shader
/// compile is already done by `Renderer3D::new`, but the first few
/// presents on a fresh swapchain are routinely slower).
const WARMUP_FRAMES: usize = 60;
/// Frames actually measured and averaged.
const MEASURED_FRAMES: usize = 600;

const CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};

/// Full 14x8 board (112 normal bricks) -- this bead's named worst case,
/// independent of `levels::LEVELS`'s three built-in (smaller) levels.
fn worst_case_bricks() -> Vec<Brick> {
    const COLS: usize = 14;
    const ROWS: usize = 8;
    let offset_x = (game::PLAYFIELD_WIDTH - COLS as f32 * levels::BRICK_WIDTH) / 2.0;
    let offset_y = 60.0;
    (0..ROWS)
        .flat_map(|row| {
            (0..COLS).map(move |col| Brick {
                x: offset_x + col as f32 * levels::BRICK_WIDTH + levels::BRICK_WIDTH / 2.0,
                y: offset_y + row as f32 * levels::BRICK_HEIGHT + levels::BRICK_HEIGHT / 2.0,
                width: levels::BRICK_WIDTH,
                height: levels::BRICK_HEIGHT,
                kind: BrickKind::Normal,
                hits_remaining: 1,
                score: 10,
            })
        })
        .collect()
}

/// `n` fresh `BrickDestroyedAt` events, spread across the board, alternating
/// kind (armored bricks tumble too, not just normal ones).
fn destroy_events(n: usize) -> Vec<GameEvent> {
    (0..n)
        .map(|i| GameEvent::BrickDestroyedAt {
            x: 40.0 + ((i as f32 * 53.0) % 720.0),
            y: 80.0 + ((i as f32 * 31.0) % 400.0),
            width: levels::BRICK_WIDTH,
            height: levels::BRICK_HEIGHT,
            kind: if i % 3 == 0 {
                BrickKind::Armored
            } else {
                BrickKind::Normal
            },
        })
        .collect()
}

struct Bench {
    window: Option<Arc<Window>>,
    renderer: Option<AnyRenderer>,
    fx: fx::FxSystem,
    /// CPU seconds in `FxSystem::update` per measured frame.
    fx_times: Vec<f64>,
    /// The largest effect frame seen: vertices, indices, pulses.
    fx_peak: (usize, usize, usize),
    /// GPU milliseconds per stage (`fx::gpu::STAGE_NAMES`), one entry per
    /// measured frame the stage ran in; all empty without `BRYX_GPU_TIMING=1`.
    gpu_times: [Vec<f64>; 4],
    size: PhysicalSize<u32>,
    game: Game,
    prev: RenderState,
    frame: usize,
    times: Vec<f64>,
    last: Instant,
}

impl Bench {
    fn new() -> Self {
        let mut game = Game::new();
        game.state = GameState::Playing;
        game.bricks = worst_case_bricks();
        game.ball.attached = false;
        game.ball.vx = 220.0;
        game.ball.vy = -300.0;
        // Multiball: the main ball plus 2 extra balls in flight (spec:
        // "2 extra balls").
        game.extra_balls = vec![
            Ball {
                x: 300.0,
                y: 250.0,
                vx: -180.0,
                vy: 260.0,
                radius: game.ball.radius,
                attached: false,
                glued: None,
            },
            Ball {
                x: 500.0,
                y: 320.0,
                vx: 200.0,
                vy: -240.0,
                radius: game.ball.radius,
                attached: false,
                glued: None,
            },
        ];
        game.powerups = vec![
            PowerUp {
                x: 200.0,
                y: 400.0,
                kind: events::PowerUpKind::Widen,
            },
            PowerUp {
                x: 420.0,
                y: 350.0,
                kind: events::PowerUpKind::Slow,
            },
            PowerUp {
                x: 600.0,
                y: 300.0,
                kind: events::PowerUpKind::Multiball,
            },
        ];
        let prev = RenderState::from(&game);
        Self {
            window: None,
            renderer: None,
            fx: fx::FxSystem::new(fx::Quality::from_env()),
            fx_times: Vec::new(),
            fx_peak: (0, 0, 0),
            gpu_times: Default::default(),
            size: bench_size(),
            game,
            prev,
            frame: 0,
            times: Vec::with_capacity(MEASURED_FRAMES),
            last: Instant::now(),
        }
    }
}

impl ApplicationHandler for Bench {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("bench_render3d")
            .with_inner_size(self.size);
        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .expect("failed to create window"),
        );
        let atlas = assets::TextureSource::Procedural.load();
        let quality = self.fx.quality();
        self.renderer = Some(match std::env::var("BENCH_RENDERER").as_deref() {
            Ok("2d") => AnyRenderer::Classic(Box::new(Renderer::new(
                Arc::clone(&window),
                atlas,
                false,
                quality,
            ))),
            _ => AnyRenderer::ThreeD(Box::new(Renderer3D::new(
                Arc::clone(&window),
                atlas,
                false,
                quality,
            ))),
        });
        if let Some(renderer) = &mut self.renderer {
            match renderer {
                AnyRenderer::Classic(r) => r.resize(self.size),
                AnyRenderer::ThreeD(r) => r.resize(self.size),
            }
        }
        self.window = Some(window);
        self.last = Instant::now();
        event_loop.set_control_flow(ControlFlow::Poll);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        if event == WindowEvent::CloseRequested {
            event_loop.exit();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(renderer) = &mut self.renderer else {
            return;
        };

        // Top off the tumble-ghost population every frame so the scene
        // sustains 20 concurrently tumbling corpses for the whole
        // benchmark, not just for their individual 0.5s lifetime --
        // `ingest_brick_destroyed_events` is a no-op past the cap, so this
        // only ever backfills slots that expired since the last frame.
        if let AnyRenderer::ThreeD(renderer) = renderer {
            renderer.ingest_tick_events(&destroy_events(20));
        }
        // `BENCH_FX_LOAD=0`: no effect events, the cost of the pipeline
        // itself on an ordinary frame between impacts.
        let load = std::env::var("BENCH_FX_LOAD").as_deref() != Ok("0");
        if load && self.frame.is_multiple_of(FX_EVERY) {
            self.fx.observe(&self.game, &fx_events(self.frame));
        }
        let fx_start = Instant::now();
        let fx_frame = self.fx.update(FX_DT, &self.game, &self.game.ball);
        let fx_time = fx_start.elapsed().as_secs_f64();
        self.fx_peak = (
            self.fx_peak.0.max(fx_frame.canvas.vertices.len()),
            self.fx_peak.1.max(fx_frame.canvas.indices.len()),
            self.fx_peak.2.max(fx_frame.pulses.len()),
        );

        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f64();
        self.last = now;

        // Offscreen (no swapchain/compositor involved -- see
        // `Renderer3D::render_offscreen_for_bench`'s doc comment for why):
        // `dt` here is real CPU-enqueue + GPU-execute time, not vsync or
        // compositor pacing.
        let gpu = match renderer {
            AnyRenderer::Classic(r) => {
                r.render_offscreen_for_bench(CLEAR_COLOR, &self.prev, &self.game, 1.0, fx_frame)
            }
            AnyRenderer::ThreeD(r) => {
                r.render_offscreen_for_bench(CLEAR_COLOR, &self.prev, &self.game, 1.0, fx_frame)
            }
        };

        self.frame += 1;
        if self.frame.is_multiple_of(100) {
            eprintln!("progress: frame {} dt={:.4}s", self.frame, dt);
        }
        if self.frame > WARMUP_FRAMES {
            self.times.push(dt);
            self.fx_times.push(fx_time);
            if let Some(stages) = gpu {
                for (bucket, time) in self.gpu_times.iter_mut().zip(stages) {
                    bucket.extend(time.map(f64::from));
                }
            }
        }
        if self.frame >= WARMUP_FRAMES + MEASURED_FRAMES {
            // The sizes the frame was really drawn at: the surface config
            // (the offscreen target follows it) and the HDR scene target.
            let (name, (surface, hdr)) = match renderer {
                AnyRenderer::Classic(r) => ("2d", r.bench_sizes()),
                AnyRenderer::ThreeD(r) => ("3d", r.bench_sizes()),
            };
            println!(
                "--- bench: {name} requested {}x{} surface {}x{} hdr {}x{} fx={:?} load={} ---",
                self.size.width,
                self.size.height,
                surface.0,
                surface.1,
                hdr.0,
                hdr.1,
                self.fx.quality(),
                u8::from(load),
            );
            report(&self.times);
            report_gpu(&self.gpu_times);
            let mean = self.fx_times.iter().sum::<f64>() / self.fx_times.len() as f64;
            let mut sorted = self.fx_times.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
            println!(
                "fx ({:?}): update {:.3} ms mean, {:.3} ms p95; peak {} vertices, {} indices, {} pulses",
                self.fx.quality(),
                mean * 1000.0,
                sorted[(sorted.len() as f64 * 0.95) as usize] * 1000.0,
                self.fx_peak.0,
                self.fx_peak.1,
                self.fx_peak.2,
            );
            event_loop.exit();
        }
    }
}

fn report(times: &[f64]) {
    let n = times.len() as f64;
    let total: f64 = times.iter().sum();
    let mean = total / n;
    let mut sorted = times.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p95 = sorted[((sorted.len() as f64) * 0.95) as usize];
    let worst = *sorted.last().unwrap();

    println!("--- bench_render3d: full 14x8 board + multiball + 20 tumbling corpses ---");
    println!("frames measured: {}", times.len());
    println!(
        "mean frame time: {:.3} ms ({:.1} fps)",
        mean * 1000.0,
        1.0 / mean
    );
    println!(
        "p95 frame time:  {:.3} ms ({:.1} fps)",
        p95 * 1000.0,
        1.0 / p95
    );
    println!(
        "worst frame:     {:.3} ms ({:.1} fps)",
        worst * 1000.0,
        1.0 / worst
    );
}

/// Per-stage GPU time (mean / p95 ms, and how many measured frames ran the
/// stage), when `BRYX_GPU_TIMING=1` had timestamps to read.
fn report_gpu(stages: &[Vec<f64>; 4]) {
    if stages.iter().all(Vec::is_empty) {
        println!(
            "gpu stages: unavailable (BRYX_GPU_TIMING=1 on an adapter with timestamp queries)"
        );
        return;
    }
    println!("gpu stages (ms mean / p95, frames):");
    for (name, times) in fx::gpu::STAGE_NAMES.iter().zip(stages) {
        if times.is_empty() {
            println!("  {name:<10} n/a");
            continue;
        }
        let mean = times.iter().sum::<f64>() / times.len() as f64;
        let mut sorted = times.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p95 = sorted[(sorted.len() as f64 * 0.95) as usize];
        println!("  {name:<10} {mean:.3} / {p95:.3}  ({})", times.len());
    }
}

fn main() {
    let event_loop = EventLoop::new().expect("failed to create event loop");
    let mut bench = Bench::new();
    event_loop
        .run_app(&mut bench)
        .expect("event loop terminated with an error");
}
