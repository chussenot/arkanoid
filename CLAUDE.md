# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:6cd5cc61 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md for details and anti-patterns.

## Agent Context Profiles

The managed Beads block is task-tracking guidance, not permission to override repository, user, or orchestrator instructions.

- **Conservative (default)**: Use `bd` for task tracking. Do not run git commits, git pushes, or Dolt remote sync unless explicitly asked. At handoff, report changed files, validation, and suggested next commands.
- **Minimal**: Keep tool instruction files as pointers to `bd prime`; use the same conservative git policy unless active instructions say otherwise.
- **Team-maintainer**: Only when the repository explicitly opts in, agents may close beads, run quality gates, commit, and push as part of session close. A current "do not commit" or "do not push" instruction still wins.

## Session Completion

This protocol applies when ending a Beads implementation workflow. It is subordinate to explicit user, repository, and orchestrator instructions.

1. **File issues for remaining work** - Create beads for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **Handle git/sync by active profile**:
   ```bash
   # Conservative/minimal/default: report status and proposed commands; wait for approval.
   git status

   # Team-maintainer opt-in only, unless current instructions forbid it:
   git pull --rebase
   git push
   git status
   ```
5. **Hand off** - Summarize changes, validation, issue status, and any blocked sync/commit/push step

**Critical rules:**
- Explicit user or orchestrator instructions override this Beads block.
- Do not commit or push without clear authority from the active profile or the current user request.
- If a required sync or push is blocked, stop and report the exact command and error.
<!-- END BEADS INTEGRATION -->


## Project status

Pre-implementation. **`docs/spec.md` is the full contract for this project — read it before writing any code.** There is no `Cargo.toml`/`src/` yet; the next work is Milestone 1 (window + wgpu clear color + fixed-timestep loop skeleton — see spec's Milestones section).

## Build & Test

Rust 2021, stable toolchain (managed by `mise.toml`). `mise.toml` also defines tasks wrapping these:

```bash
mise run run                                 # == cargo run --release (launch the game)
mise run build                               # == cargo build
mise run test                                # == cargo test (headless, src/game.rs — no GPU needed)
mise run clippy                              # == cargo clippy --all-targets -- -D warnings — must be clean
mise run fmt                                 # == cargo fmt
mise run fmt-check                           # == cargo fmt --check
mise run ci                                  # fmt-check + clippy + test — the full pre-commit/CI gate
```

`prek` (config in `.pre-commit-config.yaml`) runs trailing-whitespace/EOF/YAML/large-file checks; `prek install` to activate the git hook. CI is fmt + clippy + test on ubuntu-latest only — no windowed/GPU run is needed or expected.

## Architecture (per docs/spec.md — technology choices are decided, not up for relitigation)

- **Rendering**: `wgpu`, one instanced-quad pipeline, every entity (paddle, ball, bricks, walls, HUD) is a quad instance, target one-or-two draw calls per frame. No engine (no bevy/ggez/macroquad) — the point is a small, legible codebase.
- **Windowing/input**: `winit`. **Timing**: fixed-timestep simulation at 120 Hz, decoupled from vsync, with render-side interpolation between ticks.
- **Text**: `glyphon` for the score/lives/level HUD.
- **Audio**: none in v1 — `events.rs` exists specifically so a future audio layer can subscribe to `GameEvent`s without touching simulation code.
- Dependency budget is fixed: wgpu, winit, glyphon, bytemuck, rand. Anything beyond that needs a code comment justifying it.

Module boundaries (the strict separation is the architectural point — don't blur it):
- `src/main.rs` — window/event loop wiring only.
- `src/game.rs` — pure simulation (`struct Game`, `fn tick(&mut self, input: &Input, dt_fixed)`). No wgpu/winit/I/O types here; this is what keeps gameplay logic unit-testable headless.
- `src/render.rs` — wgpu setup + the instanced-quad pipeline.
- `src/levels.rs` — level grids as const `&[&str]` data (`.` empty, `1` normal, `2` armored, `X` indestructible). Adding a level must require touching only this file.
- `src/events.rs` — `enum GameEvent { BrickDestroyed, BallLost, ... }` emitted by the simulation each tick; render (and later audio) consume it.
- Game states are an explicit enum-driven machine: `Menu → Playing ⇄ Paused → GameOver/Victory`. No ad-hoc booleans for state.

## Gameplay contract highlights (full detail in docs/spec.md)

- Fixed logical playfield 800×600; window resize scales/letterboxes, never changes physics.
- Ball exit angle is a linear map from paddle-hit-offset to 30°–150°, clamped away from 90°±5° (no perfectly vertical trajectories); speed +4% per paddle hit, capped at 700 px/s.
- Collision is AABB-vs-circle resolved against the nearest face; tunneling at max ball speed is prevented by substepping the tick — this needs a property-style test (random angles at 700 px/s never cross a 22 px brick uncaught), not just an implementation.
- Exactly three power-ups: Widen (paddle ×1.5, 15s, timers refresh not stack-add), Slow (ball ×0.7 once, floored at launch speed), Multiball (2 extra balls; life lost only when the *last* ball drops). Do not add a fourth without updating the spec first.

## Conventions

- Conventional Commits (`cog.toml` / Cocogitto tracks these for changelog generation).
- Commit after each milestone in docs/spec.md, in order — don't jump ahead or batch milestones into one commit.
- The spec explicitly says: ask before deviating from it.

<!-- pact:begin -->
## pact coordination protocol

Claude Code loads this file, not `AGENTS.md`, so the protocol is imported
here instead of copied — one source of truth, in the file the other agents
already read. Run `pact init` to refresh it.

@AGENTS.md
<!-- pact:end -->
