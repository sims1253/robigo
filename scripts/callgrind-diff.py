#!/usr/bin/env python3
"""Exact instruction costs of a workload, from two callgrind runs.

Usage: scripts/callgrind-diff.py WORK.R BASE.R [UNITS] [--incl] [--top N]

Runs robigo under callgrind on WORK.R and BASE.R (which should differ only
in the amount of work, e.g. fib(21) vs fib(20)) and prints the per-function
instruction difference divided by UNITS (e.g. the number of extra calls).
Startup and other fixed costs cancel out. Inclusive costs (--incl) are
unreliable under deep recursion.

Environment: ROBIGO (binary, default the shared release build), VALGRIND
and CALLGRIND_ANNOTATE (default: from PATH), ROBIGO_COMPILE (passed on).
"""
import argparse, os, re, subprocess, sys, tempfile

ap = argparse.ArgumentParser()
ap.add_argument("work")
ap.add_argument("base")
ap.add_argument("units", nargs="?", type=float, default=1.0)
ap.add_argument("--incl", action="store_true")
ap.add_argument("--top", type=int, default=40)
args = ap.parse_args()
binary = os.environ.get("ROBIGO", os.path.expanduser("~/.cache/cargo-target/release/robigo"))
valgrind = os.environ.get("VALGRIND", "valgrind")
annotate = os.environ.get("CALLGRIND_ANNOTATE", "callgrind_annotate")


def profile(script, out):
    env = dict(os.environ, R_DEFAULT_PACKAGES=os.environ.get("R_DEFAULT_PACKAGES", "NULL"))
    subprocess.run([valgrind, "--tool=callgrind", f"--callgrind-out-file={out}", binary, script],
                   env=env, capture_output=True, check=True)
    opts = ["--threshold=100", "--auto=no"] + (["--inclusive=yes"] if args.incl else [])
    text = subprocess.run([annotate, *opts, out], capture_output=True, text=True, check=True).stdout
    total, costs = 0, {}
    for line in text.splitlines():
        m = re.match(r"\s*([\d,]+)\s+\([^)]*\)\s+(.*)$", line)
        if not m:
            continue
        n = int(m.group(1).replace(",", ""))
        if "PROGRAM TOTALS" in line:
            total = n
            continue
        name = re.sub(r" \[[^]]*\]$", "", m.group(2))
        name = name.split(":", 1)[-1] if ":" in name else name  # by function, not file
        name = re.sub(r"'\d+$", "", name)  # recursion levels
        costs[name] = costs.get(name, 0) + n
    return total, costs


with tempfile.TemporaryDirectory() as d:
    tw, w = profile(args.work, os.path.join(d, "work.cg"))
    tb, b = profile(args.base, os.path.join(d, "base.cg"))
diff = tw - tb
print(f"total: work {tw:,}  base {tb:,}  per unit {diff / args.units:,.0f}")
rows = sorted(((w.get(k, 0) - b.get(k, 0), k) for k in set(w) | set(b)), reverse=True)
for v, k in rows[:args.top]:
    if v <= 0:
        break
    print(f"{v / args.units:12,.1f} {100 * v / max(diff, 1):6.1f}%  {k[:140]}")
