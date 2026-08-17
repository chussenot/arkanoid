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
//! so it has no access to the `arkanoid` binary's modules otherwise --
//! there is no `src/lib.rs` to depend on). `#![allow(dead_code)]`: most of
//! `render.rs`'s classic 2D renderer and HUD text plumbing comes along for
//! the ride (only `RenderState` is actually needed here) since it's all one
//! file: real dead code in the `arkanoid` binary would be a bug, but here
//! it's an unavoidable side effect of reusing the module wholesale.
#![allow(dead_code)]

#[path = "../events.rs"]
mod events;
#[path = "../game.rs"]
mod game;
#[path = "../levels.rs"]
mod levels;
#[path = "../render.rs"]
mod render;
#[path = "../render3d/mod.rs"]
mod render3d;

use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use events::GameEvent;
use game::{Ball, Brick, Game, GameState, PowerUp};
use levels::BrickKind;
use render::RenderState;
use render3d::Renderer3D;

/// Frames discarded before measuring starts (pipeline warm-up: shader
/// compile is already done by `Renderer3D::new`, but the first few
/// presents on a fresh swapchain are routinely slower).
const WARMUP_FRAMES: usize = 60;
/// Frames actually measured and averaged.
const MEASURED_FRAMES: usize = 600;

const CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.05,
    g: 0.06,
    b: 0.09,
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
    renderer: Option<Renderer3D>,
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
            },
            Ball {
                x: 500.0,
                y: 320.0,
                vx: 200.0,
                vy: -240.0,
                radius: game.ball.radius,
                attached: false,
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
            .with_inner_size(LogicalSize::new(800u32, 600u32));
        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .expect("failed to create window"),
        );
        self.renderer = Some(Renderer3D::new(Arc::clone(&window)));
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
        renderer.ingest_tick_events(&destroy_events(20));

        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f64();
        self.last = now;

        // Offscreen (no swapchain/compositor involved -- see
        // `Renderer3D::render_offscreen_for_bench`'s doc comment for why):
        // `dt` here is real CPU-enqueue + GPU-execute time, not vsync or
        // compositor pacing.
        renderer.render_offscreen_for_bench(CLEAR_COLOR, &self.prev, &self.game, 1.0);

        self.frame += 1;
        if self.frame.is_multiple_of(100) {
            eprintln!("progress: frame {} dt={:.4}s", self.frame, dt);
        }
        if self.frame > WARMUP_FRAMES {
            self.times.push(dt);
        }
        if self.frame >= WARMUP_FRAMES + MEASURED_FRAMES {
            report(&self.times);
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

fn main() {
    let event_loop = EventLoop::new().expect("failed to create event loop");
    let mut bench = Bench::new();
    event_loop
        .run_app(&mut bench)
        .expect("event loop terminated with an error");
}
