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

## Screenshots

Every entity draws from the same `Atlas` layout (`src/assets.rs`); the only
difference between these two runs is where its pixels came from:

```bash
cargo run --release                            # procedural placeholder (default)
scripts/fetch-assets.sh                        # one-time: pulls a CC0 sprite pack
cargo run --release -- --assets assets/pack    # on-disk sprite pack
```

**Procedural vs. pack side-by-side: not captured this session.** The build
machine's display was locked (`swaylock`) for the whole session, so `grim`
only ever captured the lock screen, not the game window — this is a real
placeholder, not a skipped step. Once a session has an unlocked display,
run both commands above, screenshot each window (`grim` on Wayland), and
drop them in as `docs/screenshot-procedural.png` / `docs/screenshot-pack.png`
side by side here.

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

