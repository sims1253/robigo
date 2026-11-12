#!/usr/bin/env python3
"""Exact instruction costs of a workload, from two callgrind runs: by function and by cause.

Usage: scripts/callgrind-diff.py WORK.R BASE.R [UNITS] [--incl] [--top N] [--causes]
       scripts/callgrind-diff.py --bench NAME [--top N] [--causes]

Runs robigo under callgrind on WORK.R and BASE.R (which should differ only
in the amount of work, e.g. fib(21) vs fib(20)) and prints the per-function
instruction difference divided by UNITS (e.g. the number of extra calls).
Startup and other fixed costs cancel out. Inclusive costs (--incl) are
unreliable under deep recursion.

--bench NAME measures a scripts/perf.py benchmark the way the gate runs it,
against an empty program with the same default packages: what is left is
the benchmark's own work, startup excluded.

--causes adds where the instructions go by cause (dispatch and evaluation,
guards and type checks, allocation and GC, attributes, JIT-generated code,
builtins and numerics, ...): benchmarks/causes.toml maps callgrind's
`file:function [object]` lines to causes, as data. Each row is a work item.

Environment: ROBIGO (binary, default as scripts/perf.py chooses it),
VALGRIND and CALLGRIND_ANNOTATE (default as scripts/perf.py chooses
valgrind, and the callgrind_annotate beside it), ROBIGO_COMPILE (passed on).
"""
import argparse
import os
import re
import subprocess
import sys
import tempfile
import tomllib

HERE = os.path.dirname(os.path.abspath(__file__))
CAUSES = os.path.join(os.path.dirname(HERE), "benchmarks", "causes.toml")
OTHER = "Other"
SMALL = 0.001  # causes below this share of the total share one row
sys.path.insert(0, HERE)
import perf  # noqa: E402
from corpus import robigo_path  # noqa: E402

LINE = re.compile(r"\s*([\d,]+)\s+\([^)]*\)\s+(.*)$")


def short_location(line):
    """`file:function [object]` with the file and object paths shortened to
    what benchmarks/causes.toml matches: crates/..., library/... (std),
    <crate>-<version>/... (a dependency) and the object's file name."""
    m = re.match(r"(.*?):(.*?)(?: \[([^]]*)\])?$", line)
    if not m:
        return line
    file, func, obj = m.groups()
    for rx in (r".*/(crates/robigo-.*)", r"/rustc/[0-9a-f]+/(library/.*)", r".*/registry/src/[^/]+/(.*)"):
        f = re.match(rx, file)
        if f:
            file = f.group(1)
            break
    func = re.sub(r"'\d+$", "", func)  # recursion levels
    return f"{file}:{func}" + (f" [{os.path.basename(obj)}]" if obj else "")


def parse_annotate(text):
    """callgrind_annotate's output: (program total, {short location: cost})."""
    total, costs = 0, {}
    for line in text.splitlines():
        m = LINE.match(line)
        if not m:
            continue
        n = int(m.group(1).replace(",", ""))
        if "PROGRAM TOTALS" in line:
            total = n
            continue
        loc = short_location(m.group(2))
        costs[loc] = costs.get(loc, 0) + n
    return total, costs


def function_of(loc):
    """The function of a location, by name only (files differ with inlining)."""
    return re.sub(r" \[[^]]*\]$", "", loc).split(":", 1)[-1]


def load_causes(path=CAUSES):
    with open(path, "rb") as f:
        return [(c["name"], [re.compile(p) for p in c["patterns"]]) for c in tomllib.load(f)["cause"]]


def cause_of(loc, causes):
    if loc.startswith("("):  # a remainder row of its own
        return loc
    for name, pats in causes:
        if any(p.search(loc) for p in pats):
            return name
    return OTHER


def by_cause(diff, causes):
    """{cause: (instructions, [(instructions, location), ...] largest first)}."""
    out = {}
    for loc, v in diff.items():
        c = out.setdefault(cause_of(loc, causes), [0, []])
        c[0] += v
        c[1].append((v, loc))
    return {k: (v, sorted(locs, reverse=True)) for k, (v, locs) in out.items()}


def cause_table(diff, total, units, causes):
    rows = sorted(by_cause(diff, causes).items(), key=lambda kv: -kv[1][0])
    lines = ["| Cause | Instructions | Share | Largest function |", "|---|---:|---:|---|"]
    small = [v for _, (v, _) in rows if abs(v) < SMALL * total]
    for name, (v, locs) in rows:
        if abs(v) < SMALL * total:
            continue
        top = function_of(locs[0][1])[:90] if locs and locs[0][0] > 0 else ""
        lines.append(f"| {name} | {v / units:,.0f} | {100 * v / max(total, 1):.1f}% | `{top}` |")
    if small:
        lines.append(f"| {len(small)} smaller causes | {sum(small) / units:,.0f} | "
                     f"{100 * sum(small) / max(total, 1):.1f}% | |")
    lines.append(f"| **Total** | **{total / units:,.0f}** | 100% | |")
    return "\n".join(lines)


class Profiler:
    def __init__(self, incl=False):
        self.binary = robigo_path(None)
        self.valgrind = perf.valgrind_path()
        beside = os.path.join(os.path.dirname(self.valgrind), "callgrind_annotate")
        self.annotate = os.environ.get("CALLGRIND_ANNOTATE") or (beside if os.path.exists(beside)
                                                                  else "callgrind_annotate")
        self.incl = incl

    def run(self, script, out, env, args=(), cwd=None):
        subprocess.run([self.valgrind, "--tool=callgrind", f"--callgrind-out-file={out}", self.binary, *args,
                        script], env=env, cwd=cwd, capture_output=True, check=True)
        opts = ["--threshold=100", "--auto=no"] + (["--inclusive=yes"] if self.incl else [])
        return parse_annotate(subprocess.run([self.annotate, *opts, out], capture_output=True, text=True,
                                             check=True).stdout)


def profiles(a, p, d):
    """(work, base) profiles: two scripts, or a benchmark and an empty program."""
    if a.bench:
        progs = [x for x in perf.programs(perf.conf()) if x[0] == a.bench]
        if not progs:
            sys.exit(f"no benchmark named {a.bench!r} (scripts/perf.py list)")
        _, _, text, pk = progs[0]
        env = dict(os.environ, **perf.BASE_ENV, R_DEFAULT_PACKAGES=pk, R_HOME=perf.r_home())
        files = []
        for name, body in (("work", text), ("base", "")):
            os.makedirs(os.path.join(d, name))
            files.append(os.path.join(d, name, "bench.R"))
            with open(files[-1], "w") as f:
                f.write(body)
        # as perf.py runs it: in a directory of its own
        return [p.run(f, os.path.join(os.path.dirname(f), "cg.out"), env, ["--vanilla"], os.path.dirname(f))
                for f in files]
    env = dict(os.environ, R_DEFAULT_PACKAGES=os.environ.get("R_DEFAULT_PACKAGES", "NULL"))
    return [p.run(os.path.abspath(s), os.path.join(d, f"{n}.cg"), env) for s, n in ((a.work, "work"), (a.base, "base"))]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("work", nargs="?")
    ap.add_argument("base", nargs="?")
    ap.add_argument("units", nargs="?", type=float, default=1.0)
    ap.add_argument("--bench", help="a scripts/perf.py benchmark, against an empty program")
    ap.add_argument("--incl", action="store_true")
    ap.add_argument("--top", type=int, default=40)
    ap.add_argument("--causes", action="store_true", help="also split the instructions by cause")
    a = ap.parse_args()
    if not a.bench and not (a.work and a.base):
        ap.error("give WORK.R and BASE.R, or --bench NAME")
    if a.causes and a.incl:
        ap.error("--causes adds up exclusive costs; leave out --incl")
    with tempfile.TemporaryDirectory() as d:
        (tw, w), (tb, b) = profiles(a, Profiler(a.incl), d)
    diff = tw - tb
    what = a.bench or "work"
    print(f"total: {what} {tw:,}  base {tb:,}  per unit {diff / a.units:,.0f}")
    locs = {k: w.get(k, 0) - b.get(k, 0) for k in set(w) | set(b)}
    funcs = {}
    for k, v in locs.items():
        funcs[function_of(k)] = funcs.get(function_of(k), 0) + v
    for k, v in sorted(funcs.items(), key=lambda kv: -kv[1])[:a.top]:
        if v <= 0:
            break
        print(f"{v / a.units:12,.1f} {100 * v / max(diff, 1):6.1f}%  {k[:140]}")
    if a.causes:
        # Lines under callgrind_annotate's threshold are not listed: the
        # table's total is what it lists, and the remainder is its own row.
        listed = sum(locs.values())
        if diff - listed:
            locs["(below callgrind_annotate's threshold)"] = diff - listed
        print()
        print(cause_table(locs, diff, a.units, load_causes()))


if __name__ == "__main__":
    main()
