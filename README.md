# arkanoid

A single-player Arkanoid/Breakout clone in Rust, rendered natively with
`wgpu`. See [`docs/spec.md`](docs/spec.md) for the full design contract.

![alt text](image.png)

## Running

```bash
cargo run --release
```

No assets or setup required — it builds and launches straight from a fresh
clone.

## Controls

| Key                | Action                  |
|---------------------|--------------------------|
| Left / Right, A / D | Move the paddle          |
| Space               | Launch the ball          |
| Escape (or P)       | Pause / resume           |

Keyboard only; menu and pause screens show the same key hints on-screen.

## Thousands of levels

Three built-in levels ship in the binary and need no setup. For
thousands more, fetch the community [LBreakoutHD](https://github.com/midzer/lbreakouthd)
levelset archive with one command:

```bash
scripts/fetch-levelsets.sh
```

This downloads a single pinned upstream commit (checksummed before
anything is extracted), and unpacks it into a gitignored `levels/`
directory as 136 plain-text levelset files, one per named set (e.g.
`levels/Classique`, `levels/Bombs`). It also (re)writes a root-level
`ATTRIBUTION` file crediting every levelset author, as the GPL requires.
Upstream's own "Arkanoid" levelset (a fan recreation of Taito's original
brick layouts) is deliberately never fetched. No new dependency: the
script is plain bash + curl/tar/awk/sha256sum (falling back to `shasum`
where `sha256sum` isn't installed).

**Why isn't this data just in the repo?** LBreakoutHD's levelsets are
GPLv3; this project is MIT. Committing GPL-licensed data alongside MIT
code would saddle the whole repo with the stricter license, so `levels/`
stays out of version control (see `.gitignore` and `ATTRIBUTION`) and the
fetch script is the one-command way to get it onto your own disk instead.

Point the game at a fetched set with `--levelset`:

```bash
cargo run --release -- --levelset levels/Classique
```

A path to one file loads that set; a path to a directory (e.g.
`--levelset levels/` for everything you fetched) lists every set found.
Once a set is loaded, the in-game menu also offers **RANDOM10**: instead
of playing a set's full level list top to bottom, it samples 10 random
levels from it for a quicker session. Either way, completion is tracked
per set, keyed by the same path you passed to `--levelset`.

## Fixed-timestep design

The simulation runs on its own fixed 120 Hz clock (`src/game.rs`'s
`Game::tick`), completely decoupled from the display's refresh rate and from
`wgpu`'s vsync-driven present loop: each real frame, the main loop advances
the simulation by as many 120 Hz ticks as have elapsed (substepping further
within a tick when needed to prevent the ball tunneling through bricks at top
speed), then hands the renderer the previous and current simulation states
along with a leftover fractional-tick `alpha` so `src/render.rs` can linearly
interpolate positions between them — this keeps motion smooth at any monitor
Hz without ever changing gameplay physics, which only ever see fixed
120 Hz steps.

