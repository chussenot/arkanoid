//! Command-line flags, additive only: each v2 workstream adds its own
//! field to `Args` and its own `match` arm in `parse()` -- never
//! restructures what's already there. See docs/fleet-patterns.md for why
//! this file exists as its own foundation piece rather than three
//! separate parsers colliding inside `main.rs`.
//!
//! Hand-rolled rather than a `clap` dependency: three optional flags
//! don't justify a new crate in a project with a fixed dependency budget.

/// Parsed CLI flags. Workstream A adds e.g. `levelset: Option<String>`, C
/// adds `renderer`. `assets` is Workstream B's (bead arkanoid-v2-b3):
/// a pack directory for `assets::TextureSource::Pack`, defaulting to
/// `TextureSource::Procedural` when unset (see `main.rs`).
#[derive(Debug, Default)]
pub struct Args {
    pub assets: Option<String>,
}

/// Parses `std::env::args()` into [`Args`]. Thin wrapper around
/// [`parse_from`] so the real parse loop is unit-testable against a
/// literal argument list instead of only the test binary's own argv.
pub fn parse() -> Args {
    parse_from(std::env::args().skip(1))
}

/// An unrecognized flag is ignored rather than treated as an error --
/// there is no shared owner of "the whole command line" to validate
/// against once three workstreams each add their own flag independently.
fn parse_from(args: impl Iterator<Item = String>) -> Args {
    let mut parsed = Args::default();
    let mut it = args;
    while let Some(flag) = it.next() {
        // Only one arm today, so clippy reads this as a match-as-if-let --
        // it's deliberately a `match`, not an `if`, because this file's
        // whole contract is other workstreams adding sibling arms here.
        #[allow(clippy::single_match)]
        match flag.as_str() {
            "--assets" => parsed.assets = it.next(),
            // Other workstream flags get their own match arm here, e.g.:
            //   "--levelset" => parsed.levelset = it.next(),
            _ => {}
        }
    }
    parsed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_of_empty_args_does_not_panic() {
        let args = parse_from(std::iter::empty());
        assert_eq!(args.assets, None);
    }

    #[test]
    fn parse_reads_the_assets_flag_value() {
        let args = parse_from(["--assets", "some/pack/dir"].into_iter().map(String::from));
        assert_eq!(args.assets.as_deref(), Some("some/pack/dir"));
    }

    #[test]
    fn parse_ignores_unrecognized_flags() {
        let args = parse_from(
            ["--bogus", "x", "--assets", "p"]
                .into_iter()
                .map(String::from),
        );
        assert_eq!(args.assets.as_deref(), Some("p"));
    }
}
