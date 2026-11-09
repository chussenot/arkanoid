use super::*;

fn playing() -> Game {
    let mut game = Game::new();
    game.state = GameState::Playing;
    game.ball.attached = false;
    game
}

const RECT: [f32; 4] = [400.0, 200.0, 52.0, 22.0];

fn struck(kind: BrickKind, hits_left: u8, by: Striker) -> GameEvent {
    GameEvent::BrickStruck {
        rect: RECT,
        at: [410.0, 217.0],
        normal: [0.0, 1.0],
        kind,
        hits_left,
        by,
    }
}

fn destroyed(kind: BrickKind) -> GameEvent {
    GameEvent::BrickDestroyedAt {
        x: RECT[0],
        y: RECT[1],
        width: RECT[2],
        height: RECT[3],
        kind,
    }
}

fn kinds(fx: &FxSystem) -> Vec<&'static str> {
    fx.effects
        .iter()
        .map(|e| match e.kind {
            Kind::Flash { .. } => "flash",
            Kind::Ring { .. } => "ring",
            Kind::Dust { .. } => "dust",
            Kind::Arc { .. } => "arc",
            Kind::Core { .. } => "core",
            Kind::Glint { .. } => "glint",
            Kind::Burst { rocks: false, .. } => "splinters",
            Kind::Burst { rocks: true, .. } => "rocks",
            Kind::Sparks { .. } => "sparks",
            Kind::CrackLight { .. } => "crack",
            Kind::Fracture { .. } => "fracture",
            Kind::Shock { .. } => "shock",
            Kind::Sweep { .. } => "sweep",
        })
        .collect()
}

#[test]
fn an_armored_brick_cracks_where_it_was_hit_then_breaks_along_the_crack() {
    let game = playing();
    let mut fx = FxSystem::new(Quality::Full);
    fx.observe(&game, &[struck(BrickKind::Armored, 1, Striker::Ball)]);
    assert_eq!(kinds(&fx), ["crack", "flash"]);
    assert_eq!(fx.cracks.len(), 1, "the brick remembers its crack");
    let origin = fx.cracks[0].origin;
    assert!((origin[0] - 410.0).abs() < 1.0 && (origin[1] - 210.0).abs() < 1.5);

    fx.observe(
        &game,
        &[
            struck(BrickKind::Armored, 0, Striker::Ball),
            destroyed(BrickKind::Armored),
        ],
    );
    assert!(fx.cracks.is_empty(), "spent by the fracture");
    let fracture = fx.effects.iter().find_map(|e| match e.kind {
        Kind::Fracture {
            origin: o, mesh, ..
        } => Some((o, mesh)),
        _ => None,
    });
    let (fracture_origin, mesh) = fracture.expect("a fracture");
    assert_eq!(fracture_origin, origin, "along the first hit's crack");
    assert!(fx.meshes[mesh].parts.len() >= 3);
    assert!(kinds(&fx).contains(&"shock"), "a small shockwave");
}

#[test]
fn a_normal_brick_splinters_off_the_struck_face_with_light_along_it() {
    let game = playing();
    let mut fx = FxSystem::new(Quality::Full);
    fx.observe(
        &game,
        &[
            struck(BrickKind::Normal, 0, Striker::Ball),
            destroyed(BrickKind::Normal),
        ],
    );
    assert_eq!(kinds(&fx), ["flash", "splinters", "glint"]);
    let dir = fx.effects.iter().find_map(|e| match e.kind {
        Kind::Burst { dir, .. } => Some(dir),
        _ => None,
    });
    assert_eq!(dir, Some([0.0, 1.0]), "thrown back out of the bottom face");
}

#[test]
fn an_indestructible_brick_throws_back_an_arc_and_loses_nothing() {
    let game = playing();
    let mut fx = FxSystem::new(Quality::Full);
    fx.observe(
        &game,
        &[struck(BrickKind::Indestructible, 1, Striker::Ball)],
    );
    assert_eq!(kinds(&fx), ["arc", "glint"]);
    fx.observe(
        &game,
        &[struck(BrickKind::Indestructible, 1, Striker::MetalBall)],
    );
    assert!(kinds(&fx).contains(&"sparks"), "metal strikes sparks");
}

#[test]
fn a_blast_breaks_its_neighbors_as_its_front_reaches_them() {
    let game = playing();
    let mut fx = FxSystem::new(Quality::Full);
    let neighbor = |x: f32| GameEvent::BrickDestroyedAt {
        x,
        y: 200.0,
        width: 52.0,
        height: 22.0,
        kind: BrickKind::Normal,
    };
    fx.observe(
        &game,
        &[
            GameEvent::BallExploded { x: 400.0, y: 200.0 },
            neighbor(452.0),
            neighbor(504.0),
        ],
    );
    let delays: Vec<f32> = fx
        .effects
        .iter()
        .filter(|e| matches!(e.kind, Kind::Flash { .. }))
        .map(|e| e.delay)
        .collect();
    assert_eq!(delays.len(), 2);
    assert!(delays[0] > 0.0 && delays[1] > delays[0], "{delays:?}");
    let blast: Vec<_> = kinds(&fx).into_iter().take(3).collect();
    assert_eq!(blast, ["core", "ring", "shock"], "the blast itself first");
}

#[test]
fn an_asteroid_falls_apart_into_rock_not_splinters() {
    let game = playing();
    let mut fx = FxSystem::new(Quality::Full);
    fx.observe(&game, &[GameEvent::AsteroidExploded { x: 300.0, y: 300.0 }]);
    let k = kinds(&fx);
    assert!(k.contains(&"rocks") && k.contains(&"dust") && !k.contains(&"splinters"));
    assert!(k.contains(&"core"), "fire inside the rock, round");
}

#[test]
fn the_light_takes_the_rallys_step_the_note_the_director_plays() {
    let game = playing();
    let mut fx = FxSystem::new(Quality::Full);
    let mut breaks = Vec::new();
    for _ in 0..6 {
        breaks.push(destroyed(BrickKind::Normal));
    }
    fx.observe(&game, &breaks);
    let colors: Vec<Rgb> = fx
        .effects
        .iter()
        .filter_map(|e| match e.kind {
            Kind::Flash { color, .. } => Some(color),
            _ => None,
        })
        .collect();
    let expected: Vec<Rgb> = (0..6).map(palette::rally).collect();
    assert_eq!(colors, expected);
    // The sixth break is the upper octave: it adds a ring.
    assert!(kinds(&fx).contains(&"ring"));
}

#[test]
fn effects_end_and_give_back_their_meshes() {
    let game = playing();
    let mut fx = FxSystem::new(Quality::Full);
    fx.observe(&game, &[struck(BrickKind::Armored, 1, Striker::Ball)]);
    assert!(fx.mesh_busy.iter().any(|&b| b));
    let ball = game.ball;
    for _ in 0..60 {
        fx.update(1.0 / 60.0, &game, &ball);
    }
    assert!(fx.effects.is_empty(), "all over within a second");
    assert!(fx.mesh_busy.iter().all(|&b| !b));
    // The fissure stays, drawn every frame, until the brick breaks.
    let frame = fx.update(1.0 / 60.0, &game, &ball);
    assert!(!frame.canvas.vertices.is_empty());
}

#[test]
fn the_same_events_and_frame_times_draw_the_same_frame() {
    let run = || {
        let game = playing();
        let mut fx = FxSystem::new(Quality::Full);
        fx.observe(
            &game,
            &[
                struck(BrickKind::Armored, 0, Striker::Ball),
                destroyed(BrickKind::Armored),
                GameEvent::BallExploded { x: 300.0, y: 120.0 },
            ],
        );
        let ball = game.ball;
        fx.update(0.05, &game, &ball);
        let frame = fx.update(0.03, &game, &ball);
        (frame.canvas.vertices.clone(), frame.pulses.clone())
    };
    assert_eq!(run(), run());
}

#[test]
fn a_flood_of_events_stays_inside_every_bound_without_allocating() {
    let game = playing();
    let mut fx = FxSystem::new(Quality::Full);
    let ball = game.ball;
    let mut events = Vec::new();
    for i in 0..40 {
        let rect = [
            30.0 + 52.0 * (i % 14) as f32,
            80.0 + 22.0 * (i / 14) as f32,
            52.0,
            22.0,
        ];
        let at = [rect[0], rect[1] + 17.0];
        events.push(GameEvent::BrickStruck {
            rect,
            at,
            normal: [0.0, 1.0],
            kind: BrickKind::Armored,
            hits_left: 0,
            by: Striker::MetalBall,
        });
        events.push(GameEvent::BrickDestroyedAt {
            x: rect[0],
            y: rect[1],
            width: 52.0,
            height: 22.0,
            kind: BrickKind::Armored,
        });
        events.push(GameEvent::BallExploded {
            x: rect[0],
            y: rect[1],
        });
    }
    // Warm up: the pool's meshes and the frame reach their working sizes.
    for _ in 0..3 {
        fx.observe(&game, &events);
        fx.update(0.016, &game, &ball);
    }
    let capacities = |fx: &FxSystem| {
        (
            fx.effects.capacity(),
            fx.cracks.capacity(),
            fx.frame.canvas.vertices.capacity(),
            fx.frame.canvas.indices.capacity(),
            fx.frame.pulses.capacity(),
        )
    };
    let before = capacities(&fx);
    for _ in 0..30 {
        fx.observe(&game, &events);
        let frame = fx.update(0.016, &game, &ball);
        assert!(frame.canvas.vertices.len() <= geometry::MAX_VERTICES);
        assert!(frame.pulses.len() <= MAX_PULSES);
    }
    assert!(fx.effects.len() <= MAX_EFFECTS);
    assert_eq!(capacities(&fx), before);
}

#[test]
fn off_draws_nothing_and_the_menu_draws_nothing() {
    let game = playing();
    let mut fx = FxSystem::new(Quality::Off);
    fx.observe(&game, &[struck(BrickKind::Armored, 1, Striker::Ball)]);
    let ball = game.ball;
    let frame = fx.update(0.016, &game, &ball);
    assert!(
        frame.canvas.vertices.is_empty() && !frame.glows,
        "nothing to bloom"
    );

    let mut fx = FxSystem::new(Quality::Full);
    fx.observe(&game, &[struck(BrickKind::Armored, 1, Striker::Ball)]);
    let mut menu = game;
    menu.state = GameState::Menu;
    assert!(fx.update(0.016, &menu, &ball).canvas.vertices.is_empty());
}

#[test]
fn only_a_powered_or_very_fast_ball_leaves_a_streak() {
    let mut canvas = Canvas::default();
    let mut ball = playing().ball;
    (ball.vx, ball.vy) = (0.0, -400.0);
    assert!(!streak(&mut canvas, &ball, None));
    assert!(
        canvas.vertices.is_empty(),
        "an ordinary rally draws nothing"
    );
    assert!(streak(&mut canvas, &ball, Some(BallMode::Metal)));
    assert_eq!(canvas.vertices.len(), 8, "steel streak and its core");
    // Behind the ball, never ahead of it.
    assert!(canvas
        .vertices
        .iter()
        .all(|v| v.position[1] >= ball.y - 0.01));
}
