//! Events emitted by the simulation each tick, describing what happened
//! during that tick so other layers (render now, audio later) can react
//! without the simulation depending on them.
//!
//! Emission pattern (for `game.rs`, wired up in a later milestone): `Game`
//! owns a `Vec<GameEvent>` field that `tick()` pushes onto as things happen
//! during that tick. The caller (`main.rs`'s loop) drains it after every
//! tick — `std::mem::take(&mut game.events)` or `game.events.drain(..)` —
//! before the next tick runs, so events never accumulate across ticks and
//! never leak into the interpolated render frames between ticks.
//!
//! Events are notifications, not a state snapshot: a consumer reacting to
//! `LevelCleared` reads the new level number off `Game` itself rather than
//! finding it on the event. The only payload carried here is what a reactor
//! cannot otherwise recover — which power-up kind spawned or was caught.

/// The power-ups a brick can drop: v1's three, plus Glue, Laser, Super laser,
/// Tiny, Player, Break, Explosive ball and Metal ball (docs/spec.md,
/// "Additions after v1"; off under `game::Features::V1`).
///
/// ponytail: no code constructs these yet -- brick/power-up gameplay lands
/// in M3/M4. `#[allow(dead_code)]` until then; drop it once `game.rs` emits
/// real `PowerUpSpawned`/`PowerUpCaught` events.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PowerUpKind {
    Widen,
    Slow,
    Multiball,
    Glue,
    Laser,
    SuperLaser,
    Tiny,
    Player,
    Break,
    ExplosiveBall,
    MetalBall,
}

/// Something the simulation did this tick.
///
/// ponytail: `Game::tick` is still the M1 no-op skeleton, so nothing
/// constructs these variants yet -- `#[allow(dead_code)]` until paddle/
/// ball/brick gameplay (M2+) starts pushing onto `Game::events`.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GameEvent {
    /// A brick was destroyed (last hit on a normal or armored brick).
    BrickDestroyed,
    /// A ball bounced off the paddle. The autopilot (`src/autopilot/`)
    /// counts these as rallies: each one is a point where a new shot is
    /// planned.
    PaddleHit,
    /// The paddle slid out through the open Break gate: the level is left
    /// (`LevelCleared` follows, or `Victory` on the last one).
    Warped,
    /// The paddle's gun fired (one Laser pair, or one Super laser bolt).
    GunFired(crate::game::GunKind),
    /// An asteroid drifted in from the top.
    AsteroidSpawned,
    /// An asteroid was blown up by a ball or the paddle, at its last
    /// position: it is gone from `Game::asteroids` by the time this fires,
    /// so the renderers' explosion effect reads its spot from here.
    AsteroidExploded { x: f32, y: f32 },
    /// An explosive ball blew up against a brick, centered on that brick:
    /// the renderers draw it like an asteroid's explosion, the director
    /// plays it a little quieter (there can be many in a row).
    BallExploded { x: f32, y: f32 },
    /// A ball fell below the bottom edge. Fires once per ball, including
    /// during multiball — see `LifeLost` for the life-loss rule.
    BallLost,
    /// A life was actually spent: the *last* ball on the field was lost.
    LifeLost,
    /// All destructible bricks on the current level are gone.
    LevelCleared,
    /// A power-up began falling from a destroyed brick.
    PowerUpSpawned(PowerUpKind),
    /// The paddle caught a falling power-up.
    PowerUpCaught(PowerUpKind),
    /// The last level was cleared: the player won.
    Victory,
    /// Lives reached zero.
    GameOver,
    /// Same moment as `BrickDestroyed`, carrying the destroyed brick's
    /// world geometry and kind -- data `BrickDestroyed` itself doesn't
    /// carry. Added for the 3D renderer's destroy-tumble effect
    /// (arkanoid-v2-c3): by the time the event fires the brick is already
    /// gone from `Game::bricks` (removed just before this pushes), so a
    /// render-side consumer has nowhere else to read its position/size/
    /// kind from in order to spawn a tumbling cube in the right place.
    /// Pushed alongside `BrickDestroyed` at the same call site, same
    /// tick -- not a replacement for it.
    BrickDestroyedAt {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        kind: crate::levels::BrickKind,
    },
}
