//! Command-line flags, additive only: each v2 workstream adds its own
//! field to `Args` and its own `match` arm in `parse()` -- never
//! restructures what's already there. See docs/fleet-patterns.md for why
//! this file exists as its own foundation piece rather than three
//! separate parsers colliding inside `main.rs`.
//!
//! Hand-rolled rather than a `clap` dependency: three optional flags
//! don't justify a new crate in a project with a fixed dependency budget.

/// Parsed CLI flags. Workstream B adds `assets: Option<String>`, C adds
/// `renderer` -- each its own field here, never restructuring this one.
#[derive(Debug, Default)]
pub struct Args {
    /// `--levelset <path>`: a single `.lbl` file, or a directory of them
    /// (see `levelset::load_dir`), to load instead of relying solely on
    /// the built-in 3 levels (`levels::LEVELS`). `None` (no flag) keeps
    /// today's built-in-only behavior.
    pub levelset: Option<String>,
}

/// Parses `std::env::args()` into [`Args`]. An unrecognized flag is
/// ignored rather than treated as an error -- there is no shared owner
/// of "the whole command line" to validate against once three workstreams
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
        // Single arm today only because sibling epics' flags live on their
        // own branches, not this one -- multiple arms is the point of this
        // `match` (see the module doc), so silence the lint rather than
        // collapse to `if` and lose that shape for the next arm to land.
        #[allow(clippy::single_match)]
        match flag.as_str() {
            "--levelset" => args.levelset = it.next(),
            // Workstream flags get their own match arm here, e.g.:
            //   "--assets" => args.assets = it.next(),
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
        assert_eq!(parse_of(&[]).levelset, None);
    }

    #[test]
    fn parse_recognizes_levelset_flag_with_its_value() {
        let args = parse_of(&["--levelset", "levels/custom.lbl"]);
        assert_eq!(args.levelset.as_deref(), Some("levels/custom.lbl"));
    }

    #[test]
    fn parse_ignores_unknown_flags() {
        assert_eq!(parse_of(&["--bogus", "x"]).levelset, None);
    }

    #[test]
    fn parse_of_levelset_with_no_following_value_leaves_it_none() {
        assert_eq!(parse_of(&["--levelset"]).levelset, None);
    }
}
