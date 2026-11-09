//! Window/event loop wiring only (see docs/spec.md, Architecture section).

mod assets;
mod audio;
mod autopilot;
mod cli;
mod events;
mod fx;
mod game;
mod hud;
mod levels;
mod levelset;
mod menu;
mod palette;
mod rally;
mod record;
mod render;
mod render3d;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use assets::Atlas;
use autopilot::Autopilot;
use events::GameEvent;
use game::{Game, Input};
use render::{RenderState, Renderer};

/// Dispatches between the classic 2D quad renderer and the sibling 3D
/// perspective renderer per `--renderer` (see `cli.rs`). A plain enum
/// rather than a trait object: exactly two implementations exist, and this
/// is the one place that ever needs to pick between them.
enum AnyRenderer {
    // Boxed: both renderers hold sizeable wgpu/glyphon state inline, and an
    // enum's size is its largest variant's -- boxing avoids every `App`
    // (there's only ever one) paying for whichever variant it didn't pick.
    Classic(Box<Renderer>),
    ThreeD(Box<render3d::Renderer3D>),
}

impl AnyRenderer {
    /// Both renderers draw their sprites from the same `atlas` (procedural
    /// or `--assets`), so a brick kind looks the same through either lens.
    fn new(
        kind: cli::RendererKind,
        window: Arc<Window>,
        atlas: Atlas,
        autopilot: bool,
        quality: fx::Quality,
    ) -> Self {
        match kind {
            cli::RendererKind::Classic => {
                Self::Classic(Box::new(Renderer::new(window, atlas, autopilot, quality)))
            }
            cli::RendererKind::ThreeD => Self::ThreeD(Box::new(render3d::Renderer3D::new(
                window, atlas, autopilot, quality,
            ))),
        }
    }

    fn set_record(&mut self, record: hud::Record) {
        match self {
            Self::Classic(r) => r.set_record(record),
            Self::ThreeD(r) => r.set_record(record),
        }
    }

    fn set_sound(&mut self, label: Option<&'static str>) {
        match self {
            Self::Classic(r) => r.set_sound(label),
            Self::ThreeD(r) => r.set_sound(label),
        }
    }

    fn set_menu(&mut self, view: &hud::MenuView) {
        match self {
            Self::Classic(r) => r.set_menu(view),
            Self::ThreeD(r) => r.set_menu(view),
        }
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        match self {
            Self::Classic(r) => r.resize(size),
            Self::ThreeD(r) => r.resize(size),
        }
    }

    fn render(
        &mut self,
        clear_color: wgpu::Color,
        prev: &RenderState,
        curr: &Game,
        alpha: f32,
        fx: &fx::FxFrame,
    ) {
        match self {
            Self::Classic(r) => r.render(clear_color, prev, curr, alpha, fx),
            Self::ThreeD(r) => r.render(clear_color, prev, curr, alpha, fx),
        }
    }

    /// Forwards one tick's `GameEvent`s to the renderer before
    /// `about_to_wait` drains `Game::events` for that tick (see events.rs's
    /// drain contract). The 3D renderer's destroyed bricks tumble from
    /// them; every other effect is the shared `fx::FxSystem`'s.
    fn ingest_tick_events(&mut self, events: &[GameEvent]) {
        if let Self::ThreeD(r) = self {
            r.ingest_tick_events(events);
        }
    }
}

/// The window opens at the canvas size: the fixed 800x600 playfield plus
/// the HUD band above it (`hud::CANVAS_*`). A resized window letterboxes
/// the canvas (`hud::Letterbox`); the simulation never changes.
const LOGICAL_WIDTH: u32 = hud::CANVAS_WIDTH as u32;
const LOGICAL_HEIGHT: u32 = hud::CANVAS_HEIGHT as u32;

/// Simulation tick rate: 120 Hz, independent of the display's refresh rate.
const FIXED_DT: f32 = 1.0 / 120.0;

/// Cap on ticks run per frame. Without this, a long stall (window drag,
/// debugger pause) would make the loop try to "catch up" by ticking
/// indefinitely on the next update -- the spiral of death. Past the cap the
/// simulation just loses the extra time instead.
const MAX_TICKS_PER_FRAME: u32 = 10;

/// Winit application state: the window/renderer (created on `resumed`) plus
/// the fixed-timestep accumulator driving `Game::tick`.
struct App {
    window: Option<Arc<Window>>,
    renderer: Option<AnyRenderer>,
    /// Which renderer `resumed()` constructs, set once from `--renderer`
    /// before the event loop starts running (see `main()`).
    renderer_kind: cli::RendererKind,
    /// Sprite pixels this session draws with (see `assets::TextureSource`),
    /// resolved once in `main` -- held here only until `resumed` builds the
    /// renderer that uploads it, since window/renderer creation has to wait
    /// for `ApplicationHandler::resumed`.
    atlas: Atlas,
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
    /// `--autopilot`: when set, it produces each tick's `Input` in place of
    /// the keyboard (Pause stays the player's).
    autopilot: Option<Autopilot>,
    /// The stored best score (`record.rs`), `None` where no record is kept;
    /// `new_best` while the end screen of a record-breaking run is up.
    record: hud::Record,
    /// Last frame's state, to catch the moment a run ends.
    last_state: game::GameState,
    /// The kinetic effects (`fx/`): fed each tick's events, built each
    /// frame, drawn by whichever renderer runs.
    fx: fx::FxSystem,
    /// When `fx` last advanced, for its wall-clock ages.
    fx_clock: Instant,
    /// The menu's mode picker (`menu.rs`): what the next run plays.
    menu: menu::Menu,
    /// `menu` as the HUD draws it, rebuilt whenever it or a record changes.
    menu_view: hud::MenuView,
    /// The stored bests of the two built-in progressions, read once at
    /// start and kept current here, so the menu never reads a file per
    /// frame.
    best_campaign: Option<u32>,
    best_classic: Option<u32>,
    /// Sound (`audio/`), opened in `main`; `None` only before that.
    audio: Option<audio::Audio>,
}

impl App {
    fn new(atlas: Atlas) -> Self {
        let game = Game::new();
        Self {
            window: None,
            renderer: None,
            renderer_kind: cli::RendererKind::default(),
            atlas,
            render_prev: RenderState::from(&game),
            game,
            input: Input::default(),
            accumulator: 0.0,
            last_update: Instant::now(),
            render_alpha: 0.0,
            autopilot: None,
            record: hud::Record::default(),
            last_state: game::GameState::Menu,
            audio: None,
            fx: fx::FxSystem::new(fx::Quality::from_env()),
            fx_clock: Instant::now(),
            menu: menu::Menu::new(Vec::new(), false),
            menu_view: hud::MenuView::default(),
            best_campaign: None,
            best_classic: None,
        }
    }

    /// The stored best for `set`, `None` where no record is kept.
    fn best(&self, set: game::LevelSetKind) -> Option<u32> {
        match set {
            game::LevelSetKind::Campaign => self.best_campaign,
            game::LevelSetKind::Classic => self.best_classic,
            game::LevelSetKind::Loaded => None,
        }
    }

    /// Back to the menu with a fresh game of the highlighted mode: at
    /// start, after every change in the picker, and on Q. Space then starts
    /// it through `Game::tick`'s own `Menu -> Playing`.
    fn show_menu(&mut self) {
        self.game = self.menu.build(&mut rand::rng());
        self.render_prev = RenderState::from(&self.game);
        self.record = hud::Record {
            best: self.best(self.game.level_set()),
            new_best: false,
        };
        self.last_state = self.game.state;
        self.refresh_menu_view();
    }

    fn refresh_menu_view(&mut self) {
        self.menu_view = self.menu.view(|set| self.best(set));
    }

    /// Interface keys: moving through the picker on the menu, and Q back
    /// to it from the pause and end screens. Never simulation input. Held
    /// arrows repeat (the OS's key repeat), so a long list of sets scrolls.
    fn interface_key(&mut self, code: KeyCode, repeat: bool) {
        use game::GameState::{GameOver, Menu, Paused, Victory};
        match self.game.state {
            Menu => {
                let nav = match code {
                    KeyCode::ArrowUp | KeyCode::KeyW => menu::Nav::Up,
                    KeyCode::ArrowDown | KeyCode::KeyS => menu::Nav::Down,
                    KeyCode::ArrowLeft | KeyCode::KeyA => menu::Nav::Left,
                    KeyCode::ArrowRight | KeyCode::KeyD => menu::Nav::Right,
                    _ => return,
                };
                if self.menu.nav(nav) {
                    self.show_menu();
                }
            }
            Paused | GameOver | Victory if code == KeyCode::KeyQ && !repeat => self.show_menu(),
            _ => {}
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
            .with_title("BRYX")
            .with_inner_size(LogicalSize::new(LOGICAL_WIDTH, LOGICAL_HEIGHT))
            .with_resizable(true);
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .expect("failed to create window"),
        );

        self.renderer = Some(AnyRenderer::new(
            self.renderer_kind,
            Arc::clone(&window),
            self.atlas.clone(),
            self.autopilot.is_some(),
            self.fx.quality(),
        ));
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
            WindowEvent::CloseRequested => {
                if let Some(autopilot) = &self.autopilot {
                    println!("autopilot: {}", autopilot.stats());
                }
                event_loop.exit();
            }
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
                        repeat,
                        ..
                    },
                is_synthetic: false,
                ..
            } => {
                // M cycles the sound (all, effects only, off). An interface
                // key, like closing the window: never a simulation input.
                if physical_key == PhysicalKey::Code(KeyCode::KeyM)
                    && state == ElementState::Pressed
                    && !repeat
                {
                    if let Some(audio) = &mut self.audio {
                        audio.cycle_mode();
                    }
                }
                if let (PhysicalKey::Code(code), ElementState::Pressed) = (physical_key, state) {
                    self.interface_key(code, repeat);
                }
                self.set_input_key(physical_key, state)
            }
            WindowEvent::RedrawRequested => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.set_record(self.record);
                    renderer.set_sound(self.audio.as_ref().and_then(audio::Audio::label));
                    renderer.set_menu(&self.menu_view);
                    let now = Instant::now();
                    let dt = now.duration_since(self.fx_clock).as_secs_f32();
                    self.fx_clock = now;
                    // The streaks follow the ball where it is drawn, between
                    // ticks, not where the last tick left it.
                    let drawn = RenderState::lerp(
                        &self.render_prev,
                        &RenderState::from(&self.game),
                        self.render_alpha,
                    );
                    let frame = self.fx.update(dt, &self.game, &drawn.ball);
                    renderer.render(
                        palette::LETTERBOX.clear(),
                        &self.render_prev,
                        &self.game,
                        self.render_alpha,
                        frame,
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
            let input = match &mut self.autopilot {
                Some(autopilot) => Input {
                    pause: self.input.pause,
                    ..autopilot.input(&self.game, FIXED_DT)
                },
                None => self.input,
            };
            self.game.tick(&input, FIXED_DT);
            if let Some(autopilot) = &mut self.autopilot {
                autopilot.observe(&self.game);
            }
            // render3d's destroy-tumble effect (arkanoid-v2-c3) needs this
            // tick's events before they're gone -- see
            // `AnyRenderer::ingest_tick_events`'s doc comment. No audio
            // consumer yet (later milestone); events are still drained
            // right after so they never pile up across ticks, per the
            // contract documented in events.rs.
            // The 3D renderer's tumbling bricks are the fallback when the
            // effects are off: with them on, `fx` breaks the brick (and
            // the asteroid) in both lenses, and a brick does not both
            // tumble away whole and shatter.
            if self.fx.quality() == fx::Quality::Off {
                if let Some(renderer) = &mut self.renderer {
                    renderer.ingest_tick_events(&self.game.events);
                }
            }
            // So do the effects, and the sound, before they are drained:
            // the same events, the same tick, so a brick's note and its
            // light start together.
            self.fx.observe(&self.game, &self.game.events);
            if let Some(audio) = &mut self.audio {
                audio.observe(&self.game, &self.game.events);
            }
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
        self.update_record();
    }
}

impl App {
    /// Keeps the best score: when a run ends (Game Over or Victory) with a
    /// score above the stored best, store it and flag the end screen; clear
    /// the flag when the next run starts. Autopilot runs show the record
    /// but never set it (PRODUCT.md: honest about the machine).
    fn update_record(&mut self) {
        use game::GameState::{GameOver, Victory};
        let state = self.game.state;
        let ended = matches!(state, GameOver | Victory);
        if ended && !matches!(self.last_state, GameOver | Victory) {
            if let Some(best) = self.record.best {
                if self.autopilot.is_none() && self.game.score > best {
                    let set = self.game.level_set();
                    record::save(set, self.game.score);
                    self.record.best = Some(self.game.score);
                    self.record.new_best = true;
                    match set {
                        game::LevelSetKind::Campaign => self.best_campaign = Some(self.game.score),
                        game::LevelSetKind::Classic => self.best_classic = Some(self.game.score),
                        game::LevelSetKind::Loaded => {}
                    }
                    self.refresh_menu_view();
                }
            }
        } else if !ended {
            self.record.new_best = false;
        }
        self.last_state = state;
    }

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

/// Where `scripts/fetch-levelsets.sh` leaves the LBreakoutHD sets, and so
/// where the menu looks for them unless `--levelset` names a directory.
const LEVELSET_DIR: &str = "levels";

/// The levelsets the menu offers, and the one `--levelset` asked for.
///
/// - No flag: every set in `default_dir`.
/// - `--levelset <dir>`: every set in that directory instead, the first
///   highlighted (as the flag has always started the directory's first).
/// - `--levelset <file>`: every set in `default_dir`, plus that file if it
///   is not one of them, highlighted.
/// - A path that holds no set: a warning, then as with no flag.
///
/// Each set is named for its file. Load warnings are printed only for a
/// set the player named; a whole directory's would bury the terminal.
fn discover_levelsets(
    arg: Option<&str>,
    default_dir: &Path,
) -> (Vec<menu::NamedSet>, Option<String>) {
    // Named for the file: `NewBricks`, `P.I.T.A.`; a `.lbl` file without
    // its extension.
    let named = |path: &Path, set: levelset::LevelSet| {
        let lbl = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("lbl"));
        let name = if lbl {
            path.file_stem()
        } else {
            path.file_name()
        };
        menu::NamedSet {
            name: name.map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            ),
            levels: set.levels,
        }
    };
    let scan = |dir: &Path| -> Vec<(PathBuf, menu::NamedSet)> {
        levelset::load_dir(dir)
            .unwrap_or_default()
            .into_iter()
            .map(|(path, set)| {
                let set = named(&path, set);
                (path, set)
            })
            .collect()
    };
    let sets_only = |found: Vec<(PathBuf, menu::NamedSet)>| -> Vec<menu::NamedSet> {
        found.into_iter().map(|(_, set)| set).collect()
    };
    let Some(arg) = arg else {
        return (sets_only(scan(default_dir)), None);
    };
    let path = Path::new(arg);
    if path.is_dir() {
        let sets = sets_only(scan(path));
        if sets.is_empty() {
            eprintln!("--levelset {arg}: no levelset found there; showing the built-in levels");
            return (sets_only(scan(default_dir)), None);
        }
        println!("--levelset {arg}: {} levelset(s) in the menu", sets.len());
        let first = sets.first().map(|s| s.name.clone());
        return (sets, first);
    }
    let set = match levelset::load_file(path) {
        Ok(set) if !set.levels.is_empty() => set,
        _ => {
            eprintln!("--levelset {arg}: no levelset found there; showing the built-in levels");
            return (sets_only(scan(default_dir)), None);
        }
    };
    for warning in &set.warnings {
        eprintln!("levelset warning: {warning}");
    }
    println!(
        "--levelset {arg}: loaded {} level(s){}",
        set.levels.len(),
        if set.warnings.is_empty() {
            ""
        } else {
            " (see warnings above)"
        }
    );
    let wanted = named(path, set);
    let found = scan(default_dir);
    let canonical = |p: &Path| std::fs::canonicalize(p).ok();
    // The flag's set, found in the directory too: highlight that copy (its
    // name), rather than list it twice.
    let in_dir = found
        .iter()
        .find(|(p, _)| canonical(p).is_some() && canonical(p) == canonical(path))
        .map(|(_, set)| set.name.clone());
    let mut sets = sets_only(found);
    let name = match in_dir {
        Some(name) => name,
        None => {
            let name = wanted.name.clone();
            sets.insert(0, wanted);
            name
        }
    };
    (sets, Some(name))
}

/// The autopilot `--autopilot` asked for: TypeSafe-advised when built with
/// the `typesafe` feature and a client can be configured from the
/// environment, the deterministic heuristic otherwise (said on stderr, so a
/// missing key never looks like a model playing).
fn build_autopilot(consult_every: u32) -> Autopilot {
    #[cfg(feature = "typesafe")]
    let strategist: Option<Box<dyn autopilot::Strategist>> = {
        use autopilot::advisor;
        match advisor::client_from_env() {
            Ok(client) => match advisor::Background::spawn(client, advisor::model_from_env()) {
                Ok(background) => Some(Box::new(background)),
                Err(error) => {
                    eprintln!("autopilot: no advisor thread ({error}); heuristic only");
                    None
                }
            },
            Err(error) => {
                eprintln!("autopilot: no TypeSafe client ({error}); heuristic only");
                None
            }
        }
    };
    #[cfg(not(feature = "typesafe"))]
    let strategist: Option<Box<dyn autopilot::Strategist>> = {
        eprintln!("autopilot: built without the `typesafe` feature; heuristic only");
        None
    };
    Autopilot::new(strategist, consult_every).with_log(true)
}

fn main() {
    let args = cli::parse();

    // Loading here is what actually exercises `--assets <dir>`: a missing
    // or malformed pack directory warns on stderr and falls back to
    // procedural pixels instead of panicking
    // (assets::TextureSource::load's contract). The resulting `Atlas` rides
    // along on `App` until `resumed` builds the classic `Renderer` that
    // uploads it (see render.rs's bead b5 for the texture/UV wiring); the 3D
    // renderer textures its bricks from the same atlas.
    let texture_source = match args.assets {
        Some(dir) => assets::TextureSource::Pack(std::path::PathBuf::from(dir)),
        None => assets::TextureSource::default(),
    };
    let atlas = texture_source.load();

    // The menu's picker (`menu.rs`): every fetched levelset, and the mode
    // the flags ask for highlighted, so Space at once plays what the flags
    // always played.
    let (sets, wanted) = discover_levelsets(args.levelset.as_deref(), Path::new(LEVELSET_DIR));
    if args.levelset.is_none() && !sets.is_empty() {
        println!("levelsets: {} in {LEVELSET_DIR}/, in the menu", sets.len());
    }
    let mut picker = menu::Menu::new(sets, args.classic);
    match wanted {
        Some(name) => picker.select_set(&name),
        None if args.classic => picker.select(menu::Mode::Classic),
        None => {}
    }

    let event_loop = EventLoop::new().expect("failed to create event loop");
    let mut app = App::new(atlas);
    app.renderer_kind = args.renderer;
    app.best_campaign = record::load(game::LevelSetKind::Campaign);
    app.best_classic = record::load(game::LevelSetKind::Classic);
    app.menu = picker;
    app.show_menu();
    app.audio = Some(audio::Audio::new(audio::SoundMode::from_env()));
    if args.autopilot {
        let autopilot = build_autopilot(args.autopilot_every.unwrap_or(1));
        println!("autopilot: {}", autopilot.describe());
        app.autopilot = Some(autopilot);
    }
    event_loop
        .run_app(&mut app)
        .expect("event loop terminated with an error");
}

#[cfg(test)]
mod discover_levelsets_tests {
    use super::*;

    /// A default directory that holds no set, as on a checkout where the
    /// fetch script never ran.
    const NO_SETS: &str = "does/not/exist";

    fn names(sets: &[menu::NamedSet]) -> Vec<&str> {
        sets.iter().map(|s| s.name.as_str()).collect()
    }

    #[test]
    fn without_the_flag_the_menu_offers_the_default_directory() {
        let (sets, wanted) = discover_levelsets(None, Path::new("tests/fixtures"));
        assert_eq!(names(&sets), ["homemade"]);
        assert_eq!(wanted, None);
        let (sets, _) = discover_levelsets(None, Path::new(NO_SETS));
        assert!(sets.is_empty());
    }

    #[test]
    fn a_missing_path_falls_back_to_the_default_directory() {
        let (sets, wanted) =
            discover_levelsets(Some("does/not/exist.lbl"), Path::new("tests/fixtures"));
        assert_eq!(names(&sets), ["homemade"]);
        assert_eq!(
            wanted, None,
            "nothing highlighted: the menu opens on Campaign"
        );
    }

    #[test]
    fn a_file_is_added_and_highlighted() {
        let (sets, wanted) =
            discover_levelsets(Some("tests/fixtures/homemade.lbl"), Path::new(NO_SETS));
        assert_eq!(names(&sets), ["homemade"]);
        assert_eq!(sets[0].levels.len(), 3);
        assert_eq!(wanted.as_deref(), Some("homemade"));
    }

    #[test]
    fn a_file_already_in_the_directory_is_listed_once() {
        let (sets, wanted) = discover_levelsets(
            Some("tests/fixtures/homemade.lbl"),
            Path::new("tests/fixtures"),
        );
        assert_eq!(names(&sets), ["homemade"]);
        assert_eq!(wanted.as_deref(), Some("homemade"));
    }

    #[test]
    fn a_directory_replaces_the_default_one_and_starts_on_its_first_set() {
        let (sets, wanted) = discover_levelsets(Some("tests/fixtures"), Path::new(NO_SETS));
        assert_eq!(names(&sets), ["homemade"]);
        assert_eq!(wanted.as_deref(), Some("homemade"));
    }

    /// The picker's choice reaches `Game` (what `main` wires): Space on
    /// the highlighted set plays that set's levels, not the built-in ones.
    #[test]
    fn the_highlighted_set_is_the_game_space_starts() {
        let (sets, wanted) =
            discover_levelsets(Some("tests/fixtures/homemade.lbl"), Path::new(NO_SETS));
        let expected_first_level_bricks = sets[0].levels[0].bricks.len();
        let mut picker = menu::Menu::new(sets, false);
        picker.select_set(&wanted.unwrap());
        let game = picker.build(&mut rand::rng());
        assert_eq!(game.level_set(), game::LevelSetKind::Loaded);
        assert_eq!(game.level_count(), 3);
        assert_eq!(game.bricks.len(), expected_first_level_bricks);
    }
}
