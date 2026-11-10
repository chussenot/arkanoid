#!/usr/bin/env python3
"""Summarise BRYX playtest logs (`--playtest-log`, docs/playtest.md).

usage: scripts/playtest-summary.py LOG.csv [LOG.csv ...]

Prints, for each condition (player, and rally scoring off or on):
  - the runs, how they ended, and how far they got;
  - per level: plays, median seconds, median last-tenth hunt and its
    share of the level, lives lost per play, median longest rally;
  - the levels that drag: a hunt over a quarter of the level, or a median
    over twice the condition's median level.

Standard library only; reads every file given, so one file per session
or one shared file both work.
"""

import csv
import statistics
import sys
from collections import defaultdict

HUNT_FLAG = 0.25
SLOW_FLAG = 2.0


def median(values):
    return statistics.median(values) if values else 0.0


def load(paths):
    rows = []
    for path in paths:
        with open(path, newline="") as f:
            for row in csv.DictReader(f):
                for key in ("secs", "hunt_secs"):
                    row[key] = float(row[key])
                for key in ("run", "level", "lives_lost", "longest_rally", "bricks", "capsules", "score"):
                    row[key] = int(row[key])
                rows.append(row)
    return rows


def condition(row):
    rally = "rally scoring" if row["rally_score"] == "true" else "baseline"
    return f"{row['player']}, {rally}"


def summarise(name, rows):
    print(f"\n== {name} ==")
    runs = {(r["session"], r["run"]) for r in rows}
    ends = defaultdict(int)
    for r in rows:
        if r["outcome"] in ("victory", "game_over", "abandoned"):
            ends[r["outcome"]] += 1
    furthest = max(r["level"] for r in rows)
    print(
        f"{len(runs)} runs, {len(rows)} levels played; ended: "
        + (", ".join(f"{k} {v}" for k, v in sorted(ends.items())) or "none yet")
        + f"; furthest level {furthest}"
    )
    finished = [r for r in rows if r["outcome"] in ("cleared", "warped", "victory")]
    level_median = median([r["secs"] for r in finished])
    print(
        f"per finished level: median {level_median:.0f} s, "
        f"{sum(r['lives_lost'] for r in rows) / len(rows):.2f} lives lost, "
        f"median longest rally {median([r['longest_rally'] for r in rows]):.0f}, "
        f"{sum(r['capsules'] for r in rows) / len(rows):.1f} capsules caught"
    )

    by_level = defaultdict(list)
    for r in rows:
        by_level[(r["mode"], r["level"], r["name"])].append(r)
    print(f"\n{'level':<28} {'plays':>5} {'secs':>6} {'hunt':>6} {'hunt%':>6} {'lives':>6} {'rally':>6}")
    flagged = []
    for (mode, level, lname), plays in sorted(by_level.items()):
        done = [p for p in plays if p["outcome"] != "abandoned"]
        secs = median([p["secs"] for p in done])
        hunt = median([p["hunt_secs"] for p in done])
        share = hunt / secs if secs else 0.0
        lives = sum(p["lives_lost"] for p in plays) / len(plays)
        rally = median([p["longest_rally"] for p in plays])
        label = f"{level:>2} {lname or mode}"[:28]
        print(f"{label:<28} {len(plays):>5} {secs:>6.0f} {hunt:>6.0f} {share:>6.0%} {lives:>6.2f} {rally:>6.0f}")
        if share > HUNT_FLAG:
            flagged.append(f"{label}: the last tenth takes {share:.0%} of the level")
        if level_median and secs > SLOW_FLAG * level_median:
            flagged.append(f"{label}: {secs:.0f} s, over {SLOW_FLAG:.0f}x the median level")
    if flagged:
        print("\nlevels that drag:")
        for line in flagged:
            print(f"  - {line}")


def main(paths):
    if not paths:
        print(__doc__)
        return 2
    rows = load(paths)
    if not rows:
        print("no rows: play a level with --playtest-log first")
        return 1
    groups = defaultdict(list)
    for r in rows:
        groups[condition(r)].append(r)
    for name in sorted(groups):
        summarise(name, groups[name])
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
