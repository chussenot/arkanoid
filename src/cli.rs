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

/// Parsed CLI flags. Workstream A adds `levelset`, B adds `assets`, C adds
/// `renderer` -- each its own field here, never restructuring this one.
#[derive(Debug, Default)]
pub struct Args {
    /// `--levelset <path>`: a single `.lbl` file, or a directory of them
    /// (see `levelset::load_dir`), to load instead of relying solely on
    /// the built-in 3 levels (`levels::LEVELS`). `None` (no flag) keeps
    /// today's built-in-only behavior.
    pub levelset: Option<String>,
    /// `--assets <dir>`: a pack directory for `assets::TextureSource::Pack`,
    /// defaulting to `TextureSource::Procedural` when unset (see `main.rs`).
    pub assets: Option<String>,
    /// `--renderer {classic|3d}`: which of `render.rs`/`render3d/` draws
    /// the game. Defaults to classic; an unrecognized or missing value
    /// keeps the default rather than panicking.
    pub renderer: RendererKind,
}

/// Parses `std::env::args()` into [`Args`]. Thin wrapper around
/// [`parse_args`] so the real parse loop is unit-testable against a
/// literal argument list instead of only the test binary's own argv. An
/// unrecognized flag (or an unrecognized value for a recognized one) is
/// ignored rather than treated as an error -- there is no shared owner of
/// "the whole command line" to validate against once three workstreams
/// each add their own flag independently.
pub fn parse() -> Args {
    parse_args(std::env::args().skip(1))
}

/// The actual parse loop, factored out from [`parse`] so it can be driven
/// by an explicit list of strings in tests instead of the real process
/// argv -- `parse()` itself stays a one-line wrapper over this.
fn parse_args<I: Iterator<Item = String>>(mut it: I) -> Args {
    let mut args = Args::default();
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--levelset" => args.levelset = it.next(),
            "--assets" => args.assets = it.next(),
            "--renderer" => match it.next().as_deref() {
                Some("classic") => args.renderer = RendererKind::Classic,
                Some("3d") => args.renderer = RendererKind::ThreeD,
                // Missing or unrecognized value: keep the default rather
                // than panic (see this function's doc comment).
                _ => {}
            },
            // Other workstream flags get their own match arm here.
            _ => {}
        }
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_of(argv: &[&str]) -> Args {
        parse_args(argv.iter().map(|s| s.to_string()))
    }

    #[test]
    fn parse_of_empty_args_does_not_panic() {
        let args = parse_of(&[]);
        assert_eq!(args.levelset, None);
        assert_eq!(args.assets, None);
        assert_eq!(args.renderer, RendererKind::Classic);
    }

    #[test]
    fn parse_recognizes_levelset_flag_with_its_value() {
        let args = parse_of(&["--levelset", "levels/custom.lbl"]);
        assert_eq!(args.levelset.as_deref(), Some("levels/custom.lbl"));
    }

    #[test]
    fn parse_ignores_unknown_flags() {
        let args = parse_of(&["--bogus", "x"]);
        assert_eq!(args.levelset, None);
        assert_eq!(args.assets, None);
    }

    #[test]
    fn parse_of_levelset_with_no_following_value_leaves_it_none() {
        assert_eq!(parse_of(&["--levelset"]).levelset, None);
    }

    #[test]
    fn parse_reads_the_assets_flag_value() {
        let args = parse_of(&["--assets", "some/pack/dir"]);
        assert_eq!(args.assets.as_deref(), Some("some/pack/dir"));
    }

    #[test]
    fn parse_ignores_unrecognized_flags_but_keeps_reading() {
        let args = parse_of(&["--bogus", "x", "--assets", "p"]);
        assert_eq!(args.assets.as_deref(), Some("p"));
    }

    #[test]
    fn renderer_flag_defaults_to_classic() {
        assert_eq!(parse_of(&[]).renderer, RendererKind::Classic);
    }

    #[test]
    fn renderer_flag_selects_3d() {
        assert_eq!(
            parse_of(&["--renderer", "3d"]).renderer,
            RendererKind::ThreeD
        );
    }

    #[test]
    fn renderer_flag_selects_classic_explicitly() {
        assert_eq!(
            parse_of(&["--renderer", "3d", "--renderer", "classic"]).renderer,
            RendererKind::Classic
        );
    }

    #[test]
    fn unrecognized_renderer_value_keeps_the_default() {
        assert_eq!(
            parse_of(&["--renderer", "bogus"]).renderer,
            RendererKind::Classic
        );
    }

    #[test]
    fn missing_renderer_value_keeps_the_default() {
        assert_eq!(parse_of(&["--renderer"]).renderer, RendererKind::Classic);
    }

    #[test]
    fn flags_combine_independently() {
        let args = parse_of(&[
            "--levelset",
            "levels/custom.lbl",
            "--assets",
            "some/pack",
            "--renderer",
            "3d",
        ]);
        assert_eq!(args.levelset.as_deref(), Some("levels/custom.lbl"));
        assert_eq!(args.assets.as_deref(), Some("some/pack"));
        assert_eq!(args.renderer, RendererKind::ThreeD);
    }
}
