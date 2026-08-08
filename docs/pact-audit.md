# pact field-test audit — arkanoid-rs 15-agent build

Ground-truth notes on an external diagnosis of pact's coordination during
this repo's build (bootstrap → 5 milestones, 15 bd issues + 1 follow-up,
16 pact agent identities, one shared checkout, ~2 hours). Written by the
orchestrating session, which has first-hand knowledge the diagnosis's
author (working from repo artifacts alone) couldn't have had. Structure:
the one correction that matters, then what independently checks out
against the actual data, then what neither of us can verify from here.

## The correction: the single 6,167-line commit was policy, not a pact gap

The diagnosis reads the commit history — `feat(game): implement
milestones 1-5`, one commit, 6,167 insertions — and concludes agents
"reasonably deferred" to one integration commit because "in a shared
checkout, the git index is the one shared resource pact doesn't cover."
That causal story is wrong for this repo, and it's checkable: **no agent
ever attempted a commit, successful or not.**

What actually happened, in order:

1. Before launching any agent, the orchestrating session asked the human
   operator an explicit question: commit after each milestone (the
   spec's own instruction) or leave everything uncommitted for review.
   The human chose "leave uncommitted."
2. Every one of the 15 agent prompts contained a hard instruction: *"Do
   NOT run `git add` or `git commit` — leave the working tree modified
   for human review."* This was a top-down policy decision made before
   the run started, for a reason unrelated to git-index contention: this
   repo's own `CLAUDE.md`/`AGENTS.md` default to a conservative
   git policy (no commits without explicit instruction), and the human's
   answer to (1) confirmed applying it here.
3. After the run finished, the human separately, explicitly asked for
   two commits in two later turns — "commit this" (the mise-task diff)
   and "commit the rest as one commit" (everything else, verbatim,
   in those words). The orchestrating session complied literally. The
   6,167-line commit is a direct rendering of an explicit human
   instruction to squash, not an emergent workaround for anything pact
   or git couldn't do.

So this repo contains **zero evidence about whether pact can or should
coordinate concurrent commits** — the question never came up, because
committing was off the table for every agent from the start. The
underlying suggestions (a lease on a reserved key like
`.pact/internal/git-index` to serialize per-task commits; worktrees as
the right topology when independent history matters more than
shared-file speed) may still be worth doing, but they should be argued
on their own merits, not as "the field test proved agents need this."

One knock-on nuance for **Finding 1** in the same diagnosis: the 8
holds/8 agents on `src/render.rs` and 6/6 on `src/game.rs` were not
organic contention arbitrated by pact's acquire/steal/retry path either.
The orchestrating workflow script sequenced every stage itself (explicit
`parallel()`/sequential `await` groups matching a hand-built dependency
graph of which files each stage touches) *before* any agent ran pact
commands — pact's leases were a confirmation/safety-net layer underneath
an already-serialized schedule, not the mechanism doing the serializing.
Zero `refused`-type events in this log is consistent both with "pact
doesn't record refusals" (true, see below) *and* with "no agent in this
run ever actually raced another for the same path" (also true, as far
as the orchestrator can tell — every acquire's log line reported "no
active leases" / "last released by <agent>", never a live holder).
This run is weak evidence about how pact behaves under genuine racing
conditions, because the calling harness never generated any. The
instrumentation gap (below) is real and worth fixing regardless; it just
isn't "proven to matter" by this specific repo the way the diagnosis
frames it.

## What checks out

Verified directly against this repo's `.beads/`, `.pact/events.jsonl`,
`pact audit`, `pact log`, and `AGENTS.md`:

- **94% claim-then-close rate**: 15 of 16 beads went `in_progress` →
  `closed`; one (`arkanoid-p4f`) went straight `open` → `closed`,
  skipping `bd update --claim`. Confirmed via
  `.beads/interactions.jsonl`.
- **`chain_hash` + `detail` on every event**: confirmed on inspection —
  every `acquired`/`released` line carries both, and `detail` strings
  are genuinely self-documenting (e.g. `"Bootstrap Cargo project: cargo
  init, add wgpu/winit/glyphon/bytemuck/rand..."`).
- **TTL discipline**: `ttl_secs: 2700` on every lease, 26/26 completed
  holds inside TTL, max hold 14m56s, zero renewals, zero stale leases
  (`pact doctor` / `pact audit` both clean).
- **No `refused`/`wait`-kind events exist anywhere in the log** —
  confirmed: `grep -o '"kind":"[a-z_]*"' .pact/events.jsonl` returns only
  `acquired` (26) and `released` (26). The diagnosis's core observation
  for Finding 1 — pact's schema has no event for a denied acquire — is
  accurate independent of the causal-weight question above.
- **Only 3 pact messages in the entire run**, none triggered by a lease
  refusal (two are an agent proactively documenting an interface change
  for downstream agents; one is a completion note to `human`). Consistent
  with "either nobody was refused, or the message-on-refusal step was
  never exercised" — and per the point above, the orchestrator can
  confirm it's the former for this run.
- **`src/render.rs`: 8 holds by 8 agents; `src/game.rs`: 6 by 6** —
  matches current `pact audit` output exactly (includes the later
  `arkanoid-sla` follow-up task, which took an 8th `render.rs` lease
  after the original 15-agent run).
- **All 16 bd interactions attributed to one human git identity**, never
  to the individual `agent-*` pact identities that did the work —
  confirmed via `.beads/interactions.jsonl`'s `actor` field. `bd
  update --claim` / `bd close` were run directly by agents under the
  ambient git identity; pact's own attribution fix doesn't reach calls
  that don't go through pact.
- **`AGENTS.md` has three duplicated "Quick Reference" bd blocks** in
  this fresh repo (confirmed at lines 16, 55, 110) — the duplication the
  diagnosis flags as propagating from `pact init`'s template does show
  up here.

## What can't be checked from this repo

Everything the diagnosis states about pact's *own* development history —
the 87% historical skip rate, 136 commits at ~1 bead/commit, the 1800s
vs. 2700s TTL recommendation trail — describes pact's own repository,
which isn't part of arkanoid-rs. Take those as asserted, not
re-verified here.

## Bottom line

Finding 1's proposed fix (log `refused` events with holder/TTL/detail,
teach `audit` to report contention and message-on-contention adherence)
stands on its own regardless of the causal-weight question — it's a real
gap in what the event schema can express. Finding 2's proposed fix
(a reserved-key lease serializing per-task commits, or recommending
worktrees when independent history matters) is a reasonable feature to
build, but shouldn't be justified by this repo's single commit, which
was a human's explicit choice made after every agent had already been
told not to touch git at all.
