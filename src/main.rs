//! Window/event loop wiring only (see docs/spec.md, Architecture section).

mod cli;
mod events;
mod game;
mod levels;
mod levelset;
mod render;

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use game::{Game, Input};
use render::{RenderState, Renderer};

/// Fixed logical playfield (spec: 800x600). Scaling/letterboxing a resized
/// window onto this is a later milestone -- for now the window just opens at
/// this size and resizing merely keeps the surface valid.
const LOGICAL_WIDTH: u32 = 800;
const LOGICAL_HEIGHT: u32 = 600;

/// Simulation tick rate: 120 Hz, independent of the display's refresh rate.
const FIXED_DT: f32 = 1.0 / 120.0;

/// Cap on ticks run per frame. Without this, a long stall (window drag,
/// debugger pause) would make the loop try to "catch up" by ticking
/// indefinitely on the next update -- the spiral of death. Past the cap the
/// simulation just loses the extra time instead.
const MAX_TICKS_PER_FRAME: u32 = 10;

/// Placeholder clear color for Milestone 1 -- a dark blue-gray so it's
/// obviously not "no draw happened at all" (black) once bricks/paddle land.
const CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.05,
    g: 0.06,
    b: 0.09,
    a: 1.0,
};

/// Winit application state: the window/renderer (created on `resumed`) plus
/// the fixed-timestep accumulator driving `Game::tick`.
struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    game: Game,
    /// Held-key state, updated by `WindowEvent::KeyboardInput` and read once
    /// per tick -- persists across frames since a key stays down across many
    /// `about_to_wait` calls between its press and release events.
    input: Input,
    /// Seconds of sim time not yet consumed by a tick.
    accumulator: f32,
    last_update: Instant,
    /// `Game` render state as of the start of this frame's tick loop, i.e.
    /// before any of this frame's ticks ran. Paired with the current
    /// (post-tick) state and `render_alpha`, this is what lets `Renderer`
    /// interpolate between two ticks instead of snapping -- see
    /// `render::RenderState`.
    render_prev: RenderState,
    /// How far into the tick *after* `render_prev` the current wall-clock
    /// frame falls, in `[0, 1]`.
    render_alpha: f32,
}

impl Default for App {
    fn default() -> Self {
        let game = Game::new();
        Self {
            window: None,
            renderer: None,
            render_prev: RenderState::from(&game),
            game,
            input: Input::default(),
            accumulator: 0.0,
            last_update: Instant::now(),
            render_alpha: 0.0,
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Portability note (see ApplicationHandler::resumed docs): resumed
        // can fire more than once (e.g. after a suspend/resume cycle on some
        // platforms); only create the window/renderer the first time.
        if self.window.is_some() {
            return;
        }

        let attributes = Window::default_attributes()
            .with_title("Arkanoid")
            .with_inner_size(LogicalSize::new(LOGICAL_WIDTH, LOGICAL_HEIGHT))
            .with_resizable(true);
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .expect("failed to create window"),
        );

        self.renderer = Some(Renderer::new(Arc::clone(&window)));
        self.window = Some(window);
        self.last_update = Instant::now();

        // Poll (not Wait): the loop must keep ticking/rendering on its own
        // schedule rather than only reacting to OS input events.
        event_loop.set_control_flow(ControlFlow::Poll);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = &self.window else {
            return;
        };
        if window.id() != window_id {
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size);
                }
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key,
                        state,
                        ..
                    },
                is_synthetic: false,
                ..
            } => self.set_input_key(physical_key, state),
            WindowEvent::RedrawRequested => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.render(
                        CLEAR_COLOR,
                        &self.render_prev,
                        &self.game,
                        self.render_alpha,
                    );
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        let Some(window) = &self.window else {
            return;
        };

        let now = Instant::now();
        self.accumulator += now.duration_since(self.last_update).as_secs_f32();
        self.last_update = now;

        // Anchor for this frame's interpolation: state as of before any of
        // this frame's ticks run (see `render_prev`'s doc comment).
        self.render_prev = RenderState::from(&self.game);

        let mut ticks = 0;
        while self.accumulator >= FIXED_DT && ticks < MAX_TICKS_PER_FRAME {
            self.game.tick(&self.input, FIXED_DT);
            // No consumer yet (audio lands in a later milestone) -- drain
            // so events never pile up across ticks, per the contract
            // documented in events.rs.
            self.game.events.clear();
            self.accumulator -= FIXED_DT;
            ticks += 1;
        }
        if ticks == MAX_TICKS_PER_FRAME {
            self.accumulator = 0.0;
        }
        self.render_alpha = (self.accumulator / FIXED_DT).clamp(0.0, 1.0);

        // One render per redraw regardless of how many ticks ran above --
        // this is what decouples the 120 Hz sim from the display's vsync.
        window.request_redraw();
    }
}

impl App {
    /// Updates the held-key `Input` state from one physical key transition.
    /// Bound per spec: paddle on Left/Right *and* A/D, launch on Space.
    /// Matched on `PhysicalKey`/`KeyCode` (key position) rather than the
    /// logical `Key` (character produced) so A/D keep working on non-QWERTY
    /// layouts where a different character sits in that position -- winit's
    /// own `KeyEvent::physical_key` docs recommend this for games.
    fn set_input_key(&mut self, physical_key: PhysicalKey, state: ElementState) {
        let pressed = state == ElementState::Pressed;
        let PhysicalKey::Code(code) = physical_key else {
            return;
        };
        match code {
            KeyCode::ArrowLeft | KeyCode::KeyA => self.input.left = pressed,
            KeyCode::ArrowRight | KeyCode::KeyD => self.input.right = pressed,
            KeyCode::Space => self.input.space = pressed,
            KeyCode::KeyP | KeyCode::Escape => self.input.pause = pressed,
            _ => {}
        }
    }
}

/// Resolves `--levelset <path>` into a loaded [`levelset::LevelSet`].
/// `path_arg` naming a file loads it directly; naming a directory loads
/// every `.lbl` file in it (`levelset::load_dir`) and logs each one found
/// -- that printed list is the "menu" of what was available -- picking
/// the first (sorted-by-path) as active. `None` here (bad path, empty
/// directory, I/O error) means the caller falls back to the built-in 3
/// levels; `main` never panics on a bad `--levelset` value.
///
/// ponytail: picking the directory's first entry rather than presenting
/// an in-game selection screen is deliberately minimal -- an interactive
/// menu needs `game.rs`/`render.rs` (both out of this bead's file scope,
/// and `game.rs` doesn't yet accept an external level source at all).
/// Real progression through a loaded set is arkanoid-v2-a5's job.
fn resolve_levelset(path_arg: &str) -> Option<levelset::LevelSet> {
    let path = Path::new(path_arg);
    if path.is_dir() {
        let mut found = levelset::load_dir(path).ok()?;
        if found.is_empty() {
            return None;
        }
        for (p, set) in &found {
            println!(
                "levelset menu: {} ({} level(s))",
                p.display(),
                set.levels.len()
            );
        }
        Some(found.remove(0).1)
    } else {
        levelset::load_file(path).ok()
    }
}

fn main() {
    let args = cli::parse();
    if let Some(path) = &args.levelset {
        match resolve_levelset(path) {
            Some(set) => {
                for warning in &set.warnings {
                    eprintln!("levelset warning: {warning}");
                }
                println!(
                    "--levelset {path}: loaded {} level(s){}",
                    set.levels.len(),
                    if set.warnings.is_empty() {
                        ""
                    } else {
                        " (see warnings above)"
                    }
                );
                // TODO(arkanoid-v2-a5): feed `set` into Game's level
                // progression -- for now the built-in 3 levels still play.
            }
            None => eprintln!(
                "--levelset {path}: no levelset found there; falling back to the built-in 3 levels"
            ),
        }
    }

    let event_loop = EventLoop::new().expect("failed to create event loop");
    let mut app = App::default();
    event_loop
        .run_app(&mut app)
        .expect("event loop terminated with an error");
}

#[cfg(test)]
mod resolve_levelset_tests {
    use super::*;

    #[test]
    fn missing_path_falls_back_to_none() {
        assert!(resolve_levelset("does/not/exist.lbl").is_none());
    }

    #[test]
    fn file_path_loads_directly() {
        let set = resolve_levelset("tests/fixtures/homemade.lbl").expect("fixture should load");
        assert_eq!(set.levels.len(), 3);
    }

    #[test]
    fn directory_path_picks_the_first_sorted_entry() {
        let set = resolve_levelset("tests/fixtures").expect("fixture dir should load");
        // `tests/fixtures/homemade.lbl` is the only `.lbl` file there today.
        assert_eq!(set.levels.len(), 3);
    }
}
