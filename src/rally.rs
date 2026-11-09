//! The rally: a run of bricks broken since the ball last left the paddle,
//! counted the one way both presentation layers read it.
//!
//! The sound director (`audio/director.rs`) plays each broken brick one
//! step higher on a pentatonic scale, and the effects layer (`fx/`) gives
//! each one the matching step's color. Both keep their own `Rally` and
//! feed it the same ticks, so they agree by construction, with no shared
//! state and no dependency between them: the note you hear and the pulse
//! you see are the same step.
//!
//! Pure, like `game.rs`: it reads game state and events, never changes
//! them.

use crate::events::GameEvent;
use crate::game::{Game, GameState};

/// Steps of the scale a rally climbs before it stays on the top one.
pub const STEPS: usize = 10;

/// Bricks broken in a row, with the rules for when a run starts over.
#[derive(Debug, Default, Clone)]
pub struct Rally {
    step: usize,
    last_state: Option<GameState>,
    ball_attached: bool,
}

impl Rally {
    /// One tick: its state change, then its events in order, then the
    /// serve. `on_break` gets each `BrickDestroyedAt`'s index in `events`
    /// and its step (0 is the root, capped at `STEPS - 1`).
    ///
    /// A run starts over when play starts or ends (into Playing from the
    /// menu or an end screen, or back to the menu), on a paddle hit, a
    /// lost life, a level cleared brick by brick, and the serve.
    pub fn observe(
        &mut self,
        game: &Game,
        events: &[GameEvent],
        mut on_break: impl FnMut(usize, usize),
    ) {
        let state = game.state;
        if let Some(last) = self.last_state.filter(|&s| s != state) {
            let restart = matches!(
                (last, state),
                (
                    GameState::Menu | GameState::GameOver | GameState::Victory,
                    GameState::Playing
                ) | (_, GameState::Menu)
            );
            if restart {
                self.step = 0;
            }
        }
        self.last_state = Some(state);

        let has = |want: &GameEvent| events.iter().any(|e| e == want);
        let run_over = has(&GameEvent::GameOver);
        let won = has(&GameEvent::Victory);
        let warped = has(&GameEvent::Warped);
        for (i, event) in events.iter().enumerate() {
            match event {
                GameEvent::BrickDestroyedAt { .. } => {
                    on_break(i, self.step.min(STEPS - 1));
                    self.step += 1;
                }
                GameEvent::PaddleHit => self.step = 0,
                GameEvent::LifeLost if !run_over => self.step = 0,
                GameEvent::LevelCleared if !won && !warped => self.step = 0,
                _ => {}
            }
        }

        let attached = game.ball.attached;
        if self.ball_attached && !attached && state == GameState::Playing {
            self.step = 0;
        }
        self.ball_attached = attached;
    }

    /// Bricks broken so far in this run (uncapped).
    #[cfg(test)]
    pub fn length(&self) -> usize {
        self.step
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::levels::BrickKind;

    fn brick() -> GameEvent {
        GameEvent::BrickDestroyedAt {
            x: 0.0,
            y: 0.0,
            width: 52.0,
            height: 22.0,
            kind: BrickKind::Normal,
        }
    }

    fn steps(rally: &mut Rally, game: &Game, events: &[GameEvent]) -> Vec<usize> {
        let mut out = Vec::new();
        rally.observe(game, events, |_, step| out.push(step));
        out
    }

    #[test]
    fn breaks_climb_and_a_paddle_hit_starts_over() {
        let mut game = Game::new();
        game.state = GameState::Playing;
        game.ball.attached = false;
        let mut rally = Rally::default();
        assert_eq!(steps(&mut rally, &game, &[brick(), brick()]), [0, 1]);
        assert_eq!(steps(&mut rally, &game, &[brick()]), [2]);
        let mid = [brick(), GameEvent::PaddleHit, brick()];
        assert_eq!(steps(&mut rally, &game, &mid), [3, 0]);
        assert_eq!(rally.length(), 1);
    }

    #[test]
    fn the_step_caps_at_the_top_of_the_scale() {
        let mut game = Game::new();
        game.state = GameState::Playing;
        game.ball.attached = false;
        let mut rally = Rally::default();
        let burst = vec![brick(); STEPS + 3];
        let seen = steps(&mut rally, &game, &burst);
        assert_eq!(seen[STEPS - 1..], [STEPS - 1; 4]);
    }

    #[test]
    fn the_serve_and_the_menu_start_a_run_over() {
        let mut game = Game::new();
        game.state = GameState::Playing;
        game.ball.attached = true;
        let mut rally = Rally::default();
        steps(&mut rally, &game, &[brick(), brick()]);
        game.ball.attached = false; // served
        rally.observe(&game, &[], |_, _| {});
        assert_eq!(rally.length(), 0);
        steps(&mut rally, &game, &[brick()]);
        game.state = GameState::Menu;
        rally.observe(&game, &[], |_, _| {});
        assert_eq!(rally.length(), 0);
    }
}
