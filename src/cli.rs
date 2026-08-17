//! Command-line flags, additive only: each v2 workstream adds its own
//! field to `Args` and its own `match` arm in `parse()` -- never
//! restructures what's already there. See docs/fleet-patterns.md for why
//! this file exists as its own foundation piece rather than three
//! separate parsers colliding inside `main.rs`.
//!
//! Hand-rolled rather than a `clap` dependency: three optional flags
//! don't justify a new crate in a project with a fixed dependency budget.

/// Parsed CLI flags. Empty for now -- Workstream A adds e.g. `levelset:
/// Option<String>`, B adds `assets: Option<String>`, C adds `renderer`.
#[derive(Debug, Default)]
pub struct Args {}

/// Parses `std::env::args()` into [`Args`]. An unrecognized flag is
/// ignored rather than treated as an error -- there is no shared owner
/// of "the whole command line" to validate against once three workstreams
/// each add their own flag independently.
pub fn parse() -> Args {
    let args = Args::default();
    for _flag in std::env::args().skip(1) {
        // Workstream flags get their own match arm here, e.g.:
        //   "--levelset" => args.levelset = it.next(),
        // No flags recognized yet, so every argument is currently ignored.
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_of_empty_args_does_not_panic() {
        let _ = Args::default();
        // `parse()` itself reads real process args in a test binary
        // (e.g. the test harness's own flags), so this only checks the
        // zero-field struct is constructible -- the real parse loop is
        // exercised by whichever workstream first gives it a flag to
        // recognize.
    }
}
