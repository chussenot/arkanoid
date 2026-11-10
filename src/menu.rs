//! The menu's mode picker: which levels the next run plays, chosen in the
//! game instead of on the command line (docs/design/level-select.md).
//!
//! Four modes, one row each:
//! - **Campaign**: the 32 named levels, every addition on.
//! - **Classic**: v1's three levels under v1's rules (`Features::V1`).
//! - **Levelset**: one fetched LBreakoutHD set, picked with ←/→.
//! - **Shuffle**: ten levels drawn from every fetched set, a new draw each
//!   time the row is picked.
//!
//! The picker never touches a running game. While the menu is up, `main`
//! rebuilds the game from the highlighted row on every change ([`Menu::build`]),
//! so what Space starts, the record the menu shows and the field behind the
//! scrim are always the highlighted mode's, and `Game::tick`'s own
//! `Menu -> Playing` transition starts it unchanged. The simulation stays
//! pure: reading sets from disk happens once, in `main`, before this.

use rand::seq::IndexedRandom;
use rand::Rng;

use crate::game::{Features, Game};
use crate::hud::{Aside, MenuRow, MenuView};
use crate::levelset::ExternalLevel;
use crate::record::{Best, Progression, RecordKey};

/// Levels a Shuffle run plays.
const SHUFFLE_LEVELS: usize = 10;

/// One fetched levelset, as the picker offers it.
#[derive(Debug, Clone, PartialEq)]
pub struct NamedSet {
    /// The file's name: `levels/NewBricks` is "NewBricks".
    pub name: String,
    pub levels: Vec<ExternalLevel>,
}

/// A mode the picker offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Campaign,
    Classic,
    Levelset,
    Shuffle,
}

/// A move through the picker, from a key press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Up,
    Down,
    Left,
    Right,
}

/// The picker's state: the modes on offer, which is highlighted, and which
/// levelset the Levelset row shows.
#[derive(Debug, Clone)]
pub struct Menu {
    modes: Vec<Mode>,
    selected: usize,
    sets: Vec<NamedSet>,
    set: usize,
    /// `--classic` with `--levelset`: loaded sets play v1's rules too.
    classic_rules: bool,
    /// `--rally-score`: every mode but Classic scores by the rally.
    rally_score: bool,
}

impl Menu {
    /// A picker over the built-in modes and `sets`. Sets without a level
    /// are dropped; Shuffle is offered only when a set remains. The
    /// Levelset row stays either way, disabled when there is nothing to
    /// pick, so a player learns the mode exists and where sets come from.
    pub fn new(sets: Vec<NamedSet>, classic_rules: bool) -> Self {
        let sets: Vec<NamedSet> = sets.into_iter().filter(|s| !s.levels.is_empty()).collect();
        let mut modes = vec![Mode::Campaign, Mode::Classic, Mode::Levelset];
        if !sets.is_empty() {
            modes.push(Mode::Shuffle);
        }
        Self {
            modes,
            selected: 0,
            sets,
            set: 0,
            classic_rules,
            rally_score: false,
        }
    }

    /// Turns on rally scoring (`Features::rally_score`) for every mode
    /// that plays BRYX's own rules: never Classic or v1-rules sets, whose
    /// scores stay v1's.
    pub fn with_rally_score(mut self, on: bool) -> Self {
        self.rally_score = on;
        self
    }

    /// Highlights `mode`, if it can be played.
    pub fn select(&mut self, mode: Mode) {
        if let Some(i) = self.modes.iter().position(|&m| m == mode) {
            if self.enabled(mode) {
                self.selected = i;
            }
        }
    }

    /// Shows the set named `name` on the Levelset row and highlights it.
    pub fn select_set(&mut self, name: &str) {
        if let Some(i) = self.sets.iter().position(|s| s.name == name) {
            self.set = i;
            self.select(Mode::Levelset);
        }
    }

    /// The highlighted mode.
    pub fn mode(&self) -> Mode {
        self.modes[self.selected]
    }

    fn enabled(&self, mode: Mode) -> bool {
        match mode {
            Mode::Campaign | Mode::Classic => true,
            Mode::Levelset | Mode::Shuffle => !self.sets.is_empty(),
        }
    }

    /// Applies one key press. ↑/↓ move the highlight, wrapping, over the
    /// rows that can be played; ←/→ change the set when the Levelset row is
    /// highlighted, wrapping too. Returns whether the choice changed, so
    /// the caller rebuilds the game only then.
    pub fn nav(&mut self, nav: Nav) -> bool {
        match nav {
            Nav::Up | Nav::Down => {
                let n = self.modes.len();
                let step = if nav == Nav::Down { 1 } else { n - 1 };
                let mut i = self.selected;
                for _ in 0..n {
                    i = (i + step) % n;
                    if self.enabled(self.modes[i]) {
                        break;
                    }
                }
                let changed = i != self.selected;
                self.selected = i;
                changed
            }
            Nav::Left | Nav::Right if self.mode() == Mode::Levelset && self.sets.len() > 1 => {
                let n = self.sets.len();
                self.set = if nav == Nav::Right {
                    (self.set + 1) % n
                } else {
                    (self.set + n - 1) % n
                };
                true
            }
            Nav::Left | Nav::Right => false,
        }
    }

    /// A fresh game of the highlighted mode, on its menu screen. `rng`
    /// draws Shuffle's levels.
    pub fn build<R: Rng + ?Sized>(&self, rng: &mut R) -> Game {
        let rules = Features {
            rally_score: self.rally_score,
            ..Features::ALL
        };
        let loaded_rules = if self.classic_rules {
            Features::V1
        } else {
            rules
        };
        match self.mode() {
            Mode::Campaign => Game::new().with_campaign().with_features(rules),
            Mode::Classic => Game::new().with_features(Features::V1),
            Mode::Levelset => {
                let mut game = Game::new().with_features(loaded_rules);
                game.load_external_levels(self.sets[self.set].levels.clone());
                game
            }
            Mode::Shuffle => {
                let pool: Vec<&ExternalLevel> = self.sets.iter().flat_map(|s| &s.levels).collect();
                let levels = pool
                    .sample(rng, SHUFFLE_LEVELS)
                    .map(|&level| level.clone())
                    .collect();
                let mut game = Game::new().with_features(loaded_rules);
                game.load_external_levels(levels);
                game
            }
        }
    }

    /// The records a run of `mode` counts toward: its levels (the
    /// highlighted set for Levelset), and the rules that change a score's
    /// meaning.
    fn key(&self, mode: Mode, set: usize) -> RecordKey {
        let (progression, loaded) = match mode {
            Mode::Campaign => (Progression::Campaign, false),
            Mode::Classic => (Progression::Classic, false),
            Mode::Shuffle => (Progression::Shuffle, true),
            Mode::Levelset => (
                Progression::Set(
                    self.sets
                        .get(set)
                        .map_or(0, |s| crate::record::set_hash(&s.levels)),
                ),
                true,
            ),
        };
        RecordKey {
            progression,
            classic_rules: loaded && self.classic_rules,
            // Classic is v1, whose score rally scoring never touches.
            rally_score: self.rally_score
                && mode != Mode::Classic
                && !(loaded && self.classic_rules),
        }
    }

    /// The records a run of the highlighted mode counts toward.
    pub fn record_key(&self) -> RecordKey {
        self.key(self.mode(), self.set)
    }

    /// Every record the picker can show: each mode's, each set's.
    pub fn record_keys(&self) -> Vec<RecordKey> {
        let mut keys: Vec<RecordKey> = [Mode::Campaign, Mode::Classic, Mode::Shuffle]
            .into_iter()
            .map(|mode| self.key(mode, 0))
            .collect();
        keys.extend((0..self.sets.len()).map(|i| self.key(Mode::Levelset, i)));
        keys
    }

    /// The picker as the HUD draws it. `best` gives the stored record of a
    /// progression that keeps one (`record.rs`).
    pub fn view(&self, best: impl Fn(&RecordKey) -> Best) -> MenuView {
        let rows = self
            .modes
            .iter()
            .map(|&mode| self.row(mode, &best))
            .collect();
        MenuView {
            rows,
            selected: self.selected,
        }
    }

    fn row(&self, mode: Mode, best: &impl Fn(&RecordKey) -> Best) -> MenuRow {
        let aside_best = |mode| Aside::Best(best(&self.key(mode, self.set)).score);
        let levels = |n: usize| format!("{n} level{}", if n == 1 { "" } else { "s" });
        match mode {
            Mode::Campaign => MenuRow {
                name: "CAMPAIGN",
                value: None,
                detail: format!(
                    "{} named levels in four acts",
                    crate::levels::CAMPAIGN.len()
                ),
                aside: aside_best(Mode::Campaign),
                enabled: true,
            },
            Mode::Classic => MenuRow {
                name: "CLASSIC",
                value: None,
                detail: format!(
                    "{}, the original rules",
                    levels(crate::levels::LEVELS.len())
                ),
                aside: aside_best(Mode::Classic),
                enabled: true,
            },
            Mode::Levelset => match self.sets.get(self.set) {
                // The aside counts the sets, so the set's best rides on its
                // detail line.
                Some(set) => MenuRow {
                    name: "LEVELSET",
                    value: Some(set.name.clone()),
                    detail: match best(&self.key(Mode::Levelset, self.set)).score {
                        0 => levels(set.levels.len()),
                        score => format!("{}, best {score}", levels(set.levels.len())),
                    },
                    aside: Aside::Position {
                        index: self.set + 1,
                        count: self.sets.len(),
                    },
                    enabled: true,
                },
                None => MenuRow {
                    name: "LEVELSET",
                    value: None,
                    detail: "none found: run scripts/fetch-levelsets.sh".into(),
                    aside: Aside::Nothing,
                    enabled: false,
                },
            },
            Mode::Shuffle => MenuRow {
                name: "SHUFFLE",
                value: None,
                detail: format!(
                    "{} drawn from all {} sets",
                    levels(SHUFFLE_LEVELS),
                    self.sets.len()
                ),
                aside: aside_best(Mode::Shuffle),
                enabled: true,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{GameState, LevelSetKind};
    use crate::levelset::ExternalBrickSpawn;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    /// A set of `count` levels, each one brick in its own column (its
    /// index), set A's on row 0 and set B's on row 1, so a game's first
    /// brick tells which set and which level it came from.
    fn set(name: &str, count: usize) -> NamedSet {
        let row = if name == "B" { 1.0 } else { 0.0 };
        NamedSet {
            name: name.into(),
            levels: (0..count)
                .map(|i| ExternalLevel {
                    author: "test".into(),
                    name: format!("{name} {i}"),
                    bricks: vec![ExternalBrickSpawn {
                        x: i as f32 * crate::levels::BRICK_WIDTH,
                        y: row * crate::levels::BRICK_HEIGHT,
                        kind: crate::levels::BrickKind::Normal,
                        powerup: None,
                    }],
                })
                .collect(),
        }
    }

    fn rng() -> StdRng {
        StdRng::seed_from_u64(7)
    }

    #[test]
    fn without_sets_the_levelset_row_shows_but_is_stepped_over() {
        let mut menu = Menu::new(Vec::new(), false);
        let view = menu.view(|_| Best::default());
        let names: Vec<_> = view.rows.iter().map(|r| r.name).collect();
        assert_eq!(names, ["CAMPAIGN", "CLASSIC", "LEVELSET"], "no Shuffle");
        assert!(!view.rows[2].enabled);
        assert!(view.rows[2].detail.contains("fetch-levelsets"));

        assert!(menu.nav(Nav::Down));
        assert_eq!(menu.mode(), Mode::Classic);
        assert!(menu.nav(Nav::Down), "wraps past the disabled row");
        assert_eq!(menu.mode(), Mode::Campaign);
        assert!(menu.nav(Nav::Up));
        assert_eq!(menu.mode(), Mode::Classic);
        menu.select(Mode::Levelset);
        assert_eq!(
            menu.mode(),
            Mode::Classic,
            "a disabled mode can't be picked"
        );
    }

    #[test]
    fn up_and_down_walk_every_mode_and_wrap() {
        let mut menu = Menu::new(vec![set("A", 3)], false);
        let mut seen = vec![menu.mode()];
        for _ in 0..4 {
            menu.nav(Nav::Down);
            seen.push(menu.mode());
        }
        use Mode::*;
        assert_eq!(seen, [Campaign, Classic, Levelset, Shuffle, Campaign]);
        menu.nav(Nav::Up);
        assert_eq!(menu.mode(), Shuffle);
    }

    #[test]
    fn left_and_right_cycle_sets_only_on_the_levelset_row() {
        let mut menu = Menu::new(vec![set("A", 3), set("Empty", 0), set("B", 5)], false);
        assert!(!menu.nav(Nav::Right), "Campaign has no value to change");
        menu.select(Mode::Levelset);
        let shown = |menu: &Menu| {
            menu.view(|_| Best::default()).rows[2]
                .value
                .clone()
                .unwrap()
        };
        assert_eq!(shown(&menu), "A");
        assert!(menu.nav(Nav::Right));
        assert_eq!(shown(&menu), "B", "an empty set is never offered");
        assert!(menu.nav(Nav::Right));
        assert_eq!(shown(&menu), "A", "wraps");
        assert!(menu.nav(Nav::Left));
        assert_eq!(shown(&menu), "B");
        assert_eq!(
            menu.view(|_| Best::default()).rows[2].aside,
            Aside::Position { index: 2, count: 2 }
        );
    }

    #[test]
    fn each_mode_builds_its_own_game_waiting_on_the_menu() {
        let mut menu = Menu::new(vec![set("A", 3), set("B", 25)], false);
        let game = menu.build(&mut rng());
        assert_eq!(
            (game.state, game.level_set(), game.level_count()),
            (GameState::Menu, LevelSetKind::Campaign, 32)
        );
        assert_eq!(game.features, Features::ALL);

        menu.select(Mode::Classic);
        let game = menu.build(&mut rng());
        assert_eq!(
            (game.level_set(), game.level_count()),
            (LevelSetKind::Classic, 3)
        );
        assert_eq!(game.features, Features::V1);

        menu.select_set("B");
        let game = menu.build(&mut rng());
        assert_eq!(
            (game.level_set(), game.level_count()),
            (LevelSetKind::Loaded, 25)
        );
        assert_eq!(game.state, GameState::Menu);

        menu.select(Mode::Shuffle);
        let game = menu.build(&mut rng());
        assert_eq!(
            (game.level_set(), game.level_count()),
            (LevelSetKind::Loaded, 10)
        );
    }

    #[test]
    fn shuffle_draws_ten_levels_from_every_set_anew_each_time() {
        let mut menu = Menu::new(vec![set("A", 6), set("B", 6)], false);
        menu.select(Mode::Shuffle);
        let mut firsts = std::collections::HashSet::new();
        for seed in 0..40 {
            let game = menu.build(&mut StdRng::seed_from_u64(seed));
            assert_eq!(game.level_count(), SHUFFLE_LEVELS);
            let brick = game.bricks[0];
            firsts.insert(((brick.x as i32), (brick.y as i32)));
        }
        let rows: std::collections::HashSet<i32> = firsts.iter().map(|&(_, y)| y).collect();
        assert_eq!(rows.len(), 2, "first levels came from both sets");
        assert!(firsts.len() > 6, "and vary from draw to draw: {firsts:?}");
    }

    #[test]
    fn classic_rules_carry_to_loaded_sets() {
        let mut menu = Menu::new(vec![set("A", 3)], true);
        menu.select(Mode::Levelset);
        assert_eq!(menu.build(&mut rng()).features, Features::V1);
    }

    #[test]
    fn the_view_shows_each_record_and_marks_the_highlight() {
        let mut menu = Menu::new(vec![set("A", 3)], false);
        menu.nav(Nav::Down);
        let a = crate::record::set_hash(&set("A", 3).levels);
        let view = menu.view(|key| {
            let score = match key.progression {
                Progression::Campaign => 12_000,
                Progression::Classic => 3_400,
                Progression::Shuffle => 900,
                Progression::Set(hash) if hash == a => 450,
                Progression::Set(_) => 0,
            };
            Best { score, rally: 0 }
        });
        assert_eq!(view.selected, 1);
        assert_eq!(view.rows[0].aside, Aside::Best(12_000));
        assert_eq!(view.rows[1].aside, Aside::Best(3_400));
        assert_eq!(view.rows[2].detail, "3 levels, best 450");
        assert_eq!(view.rows[3].aside, Aside::Best(900));
        assert_eq!(view.rows[3].detail, "10 levels drawn from all 1 sets");
    }

    /// Each set, Shuffle, v1 rules and rally scoring keep records apart;
    /// Classic stays v1's whatever the flags.
    #[test]
    fn every_mode_and_set_keeps_its_own_record() {
        let mut menu = Menu::new(vec![set("A", 3), set("B", 2)], false);
        let keys = menu.record_keys();
        let unique: std::collections::HashSet<_> = keys.iter().collect();
        assert_eq!(unique.len(), keys.len(), "{keys:?}");
        assert_eq!(keys.len(), 5);

        menu.select(Mode::Levelset);
        let first = menu.record_key();
        menu.nav(Nav::Right);
        assert_ne!(menu.record_key(), first, "the next set");

        let rally = Menu::new(vec![set("A", 3)], false).with_rally_score(true);
        assert!(rally.record_key().rally_score, "campaign, rally scored");
        let mut rally_classic = rally.clone();
        rally_classic.select(Mode::Classic);
        assert!(!rally_classic.record_key().rally_score, "Classic is v1's");

        let mut v1 = Menu::new(vec![set("A", 3)], true).with_rally_score(true);
        v1.select(Mode::Levelset);
        let key = v1.record_key();
        assert!(key.classic_rules && !key.rally_score, "{key:?}");
    }
}
