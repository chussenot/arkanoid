//! Command-line flags, additive only: each v2 workstream adds its own
//! field to `Args` and its own `match` arm in `parse()` -- never
//! restructures what's already there. See docs/fleet-patterns.md for why
//! this file exists as its own foundation piece rather than three
//! separate parsers colliding inside `main.rs`.
//!
//! Hand-rolled rather than a `clap` dependency: three optional flags
//! don't justify a new crate in a project with a fixed dependency budget.

/// Which renderer implementation draws the game world: `render.rs`'s
/// classic instanced-quad 2D pipeline, or `render3d/`'s perspective-camera
/// cube+sphere pipeline (arkanoid-v2-c1). Classic is the default and stays
/// the default until the whole presentation-3d epic is DONE-done -- see
/// docs/fleet-patterns.md.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RendererKind {
    #[default]
    Classic,
    ThreeD,
}

/// Parsed CLI flags. Workstream A adds e.g. `levelset: Option<String>`, B
/// adds `assets: Option<String>` -- each workstream's own field, never a
/// restructuring of what's already here (see the module doc comment).
#[derive(Debug, Default)]
pub struct Args {
    pub renderer: RendererKind,
}

/// Parses `std::env::args()` into [`Args`]. An unrecognized flag (or an
/// unrecognized value for a recognized one) is ignored rather than treated
/// as an error -- there is no shared owner of "the whole command line" to
/// validate against once three workstreams each add their own flag
/// independently.
pub fn parse() -> Args {
    parse_from(std::env::args().skip(1))
}

/// The actual parse loop, factored out of [`parse`] so it's testable
/// without depending on the test binary's own real `std::env::args()`.
fn parse_from(args: impl Iterator<Item = String>) -> Args {
    let mut result = Args::default();
    let mut it = args;
    while let Some(flag) = it.next() {
        if flag == "--renderer" {
            match it.next().as_deref() {
                Some("classic") => result.renderer = RendererKind::Classic,
                Some("3d") => result.renderer = RendererKind::ThreeD,
                // Missing or unrecognized value: keep the default rather
                // than panic (see this function's doc comment).
                _ => {}
            }
        }
        // Workstream flags get their own `if`/match arm here, e.g.:
        //   "--levelset" => result.levelset = it.next(),
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(flags: &[&str]) -> Args {
        parse_from(flags.iter().map(|s| s.to_string()))
    }

    #[test]
    fn parse_of_empty_args_does_not_panic() {
        let _ = Args::default();
        // `parse()` itself reads real process args in a test binary
        // (e.g. the test harness's own flags), so this only checks the
        // zero-field struct is constructible -- the real parse loop is
        // exercised by `parse_from`'s own tests below.
    }

    #[test]
    fn renderer_flag_defaults_to_classic() {
        assert_eq!(args(&[]).renderer, RendererKind::Classic);
    }

    #[test]
    fn renderer_flag_selects_3d() {
        assert_eq!(args(&["--renderer", "3d"]).renderer, RendererKind::ThreeD);
    }

    #[test]
    fn renderer_flag_selects_classic_explicitly() {
        assert_eq!(
            args(&["--renderer", "3d", "--renderer", "classic"]).renderer,
            RendererKind::Classic
        );
    }

    #[test]
    fn unrecognized_renderer_value_keeps_the_default() {
        assert_eq!(
            args(&["--renderer", "bogus"]).renderer,
            RendererKind::Classic
        );
    }

    #[test]
    fn missing_renderer_value_keeps_the_default() {
        assert_eq!(args(&["--renderer"]).renderer, RendererKind::Classic);
    }
}
