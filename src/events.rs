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

/// The three power-ups a brick can drop (spec: exactly these three, no more).
///
/// ponytail: no code constructs these yet -- brick/power-up gameplay lands
/// in M3/M4. `#[allow(dead_code)]` until then; drop it once `game.rs` emits
/// real `PowerUpSpawned`/`PowerUpCaught` events.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerUpKind {
    Widen,
    Slow,
    Multiball,
}

/// Something the simulation did this tick.
///
/// ponytail: `Game::tick` is still the M1 no-op skeleton, so nothing
/// constructs these variants yet -- `#[allow(dead_code)]` until paddle/
/// ball/brick gameplay (M2+) starts pushing onto `Game::events`.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameEvent {
    /// A brick was destroyed (last hit on a normal or armored brick).
    BrickDestroyed,
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
}
