# Fleet patterns: worktrees + `pact merge` + epic branches

How a multi-agent fleet coordinates in this repo when the work needs
independent commit history, not just shared-file safety. Written for the
v2 spec's three-workstream fleet; the pattern generalizes to any future
one. Complements `AGENTS.md`'s pact protocol block (file leases, messages,
`bd` task tracking) rather than replacing it — read that first.

## Why this exists (and why the v1 build didn't need it)

The v1 build (15 agents, one shared checkout, bd + pact file leases) was
the right tool for that job: every agent edited disjoint or
sequentially-handed-off files, and nobody needed their own commit
history — one human-reviewed integration commit at the end was fine.
`docs/pact-audit.md` (Finding 2) flagged the cost of that shape once work
*does* need independent provenance: git's index is the one shared
resource file-leases don't cover, so a shared checkout forces agents to
defer every commit to a single integrator, and per-agent attribution
evaporates.

v2 is exactly the case that finding describes: three genuinely
independent epics, each wanting its own commit history, each landing
(or not) on its own schedule. So this time: **one git worktree per
agent, `pact merge` to land work, epic integration branches instead of a
shared floor.**

## Topology

```
master (protected — nothing lands here unattended)
  ├─ epic/levels           (workstream A)
  ├─ epic/textures          (workstream B)
  └─ epic/presentation-3d   (workstream C)
```

- Each epic branch is created once, from the same `master` commit, before
  that epic's fleet starts. `docs/pact-audit.md`'s point about a shared
  hash function stands here too: the deterministic-replay baseline
  (`game::tests::deterministic_replay_matches_the_pinned_v1_baseline`)
  must exist on `master` *before* any epic branch forks, or the three
  branches have nothing common to compare against.
- Every bead's agent runs in its own `git worktree` off its epic branch
  (Workflow's `isolation: 'worktree'`), does its work, runs the quality
  gate, then `pact merge`s its branch back into the epic branch it forked
  from. It never touches another epic's branch and never touches
  `master`.
- **`master` is held.** This build's merge-target decision (see the
  session that authored this doc) was explicit: agents land on epic
  branches only; a human reviews and merges the three epic branches into
  `master` by hand. Don't let a future session's fleet plan quietly
  upgrade that to "merge to master" without asking again — it's a
  different risk profile each time it comes up.

## The `pact merge` contract

```
pact merge <epic-branch> --verify "mise run ci" --agent <name>
```

- Runs under a mutex scoped to the target branch: only one agent merges
  into a given epic branch at a time. `pact lease ls`/`pact log` show who
  holds it.
- `--no-ff`, signed with `Pact-Agent` in the merge commit — that's what
  gives this repo the "~1 bead per commit" provenance
  `docs/pact-audit.md` measured in pact's own history and found missing
  in v1's single 6,167-line commit.
- `--verify` is not optional in practice even though the flag is
  optional in the CLI: every merge in this fleet passes
  `--verify "mise run ci"` (fmt-check + clippy + test). A failing verify
  reverts the merge and **keeps the mutex held** — deliberately, so nobody
  merges on top of a branch mid-failure. Treat a held mutex you didn't
  expect as "something broke, go look," not as a bug to route around.
- Default TTL (30m) is already generous — `pact merge --help` cites a
  median self-merge of 37s across real runs. Don't raise it to paper over
  a slow verify command; fix the verify command.

## Contended seams — mark them, don't be surprised by them

Three files get touched by more than one epic, by construction:

- **`src/main.rs`**: every epic adds its own CLI flag (`--levelset`,
  `--assets`, `--renderer`). To keep this additive instead of a
  three-way merge conflict when the epic branches eventually combine,
  the foundation phase adds a small `src/cli.rs` with an empty `Args`
  struct — each epic's bead adds *its own field* to that struct and
  *its own match arm*, never restructures what's there. Hand-rolled
  `std::env::args()` parsing, not a new dependency — three optional
  flags don't justify pulling in `clap`.
- **`src/events.rs`** / **`src/game.rs`**: frozen per the v2 spec, except
  Workstream C's one allowed seam (additive `GameEvent` fields/variants
  only, replay-hash test still green) and Workstream A's mapping code
  (LBreakout2 brick/bonus types → this game's own `BrickKind`/
  `PowerUpKind`, which lives in A's own `levelset.rs`, not in `game.rs`
  itself).
- **The Workstream B atlas is a cross-epic dependency for C**: Workstream
  C's brick-texturing bead reads `src/assets.rs`'s public atlas type.
  That bead is sequenced *after* Workstream B's `TextureSource`/atlas
  skeleton bead lands on `epic/textures` — cross-epic sequencing that a
  same-epic dependency graph alone won't express, so it's called out
  explicitly in the bd issue's description, not left implicit.

Per `docs/pact-audit.md`'s Finding 1: pact's event log records
`acquired`/`released`, not refusals — if two agents do end up racing one
of these seams, `pact audit`'s hold counts on that path are the only
signal available after the fact. Expect `src/main.rs` and `src/cli.rs`
to show up as contended in the post-run audit; that's the seam working
as designed, not a sign something went wrong.

## Attribution and commits, inside a worktree

pact 0.10's protocol block (see `AGENTS.md`) added guidance this fleet
depends on that the v1 build predates — worth restating here since it's
easy to miss inside a worktree specifically:

- **Export `BEADS_ACTOR=$PACT_AGENT` alongside `PACT_AGENT`, every
  agent, every shell.** This is the direct fix for the exact gap
  `docs/pact-audit.md` measured in v1: all 16 bd interactions attributed
  to one human git identity because nothing told bd which agent was
  acting. Without this line, this fleet reproduces that finding instead
  of fixing it.
- **Sign every commit** (not just the `pact merge` commit, which signs
  itself) **with `git commit --trailer Pact-Agent=$PACT_AGENT`** inside
  your own worktree, so `pact audit --check commit-correlation` can tell
  which agent made which commit, not just whether *someone* held a lease
  when it landed.
- **Commit before you release, not after** — a lease released while the
  work is still uncommitted breaks the record the log exists to produce.
- **Do not try to commit `.pact/events.jsonl` or `.pact/messages.jsonl`
  from a worktree.** Under the default shared coordination scope, every
  worktree resolves pact state to the main checkout — your worktree's
  copy is a stale tracked snapshot, and `git add` there finds nothing
  real to stage. The orchestrator, working from the main checkout,
  commits those for the whole fleet periodically (e.g. once per epic
  landing on `master`). If you're an agent reading this from inside a
  worktree: this is not your job, and trying will not do anything.

## Cross-epic dependencies: `pact watch`, not polling

The one place this fleet's independence claim has a real seam:
Workstream C's brick-texturing bead needs Workstream B's atlas type to
exist first, and B and C live on *different* epic branches — a same-epic
bd dependency can't express that. Use `pact watch add <path>` on the
file whose contract you depend on but don't own (e.g. C's bead watches
`src/assets.rs`) at task start. Its holder's next `lease release` sends
you the diff automatically — nobody has to remember to message you by
hand. Remember what this notice actually is in a worktree fleet: **a
contract notice, not a code delivery.** It tells you the branch the
change landed on; you don't have that code until that branch merges and
you merge from it. Read the diff to learn what the interface now says,
keep working against your own copy, and don't block waiting for the
file to change under you — it structurally can't, until the merges
happen.

## Dead-peer recovery

`pact lease sweep` reclaims holds whose holder process is gone —
recorded as recovery, not as a steal (no adversarial framing needed for
"the agent's process died"). Run it before assuming a long-held lease
means active, ongoing work. A worktree whose agent died mid-task also
needs its worktree removed (`git worktree remove --force`) once its
lease is swept — a dead worktree left behind will otherwise confuse the
next `git worktree list`.

## What this doesn't change

File leases, `pact msg`, and `bd` task tracking from `AGENTS.md`'s
protocol block still apply exactly as before *within* an epic's
worktrees — an agent still leases the paths it's about to write, still
checks `pact msg inbox` before starting, still claims/closes its bd
issue. Worktrees solve the commit-history problem; they don't replace
the announce-before-you-write discipline.
