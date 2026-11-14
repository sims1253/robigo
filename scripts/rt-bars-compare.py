#!/usr/bin/env python3
"""Compare the M1 decision bars' runs of two variants (docs/ledger/g3.md).

    scripts/rt-bars-compare.py --a NAME DIR [DIR ...] --b NAME DIR [DIR ...] [--rows R1,R2,...]

Each DIR is one run of scripts/rt-bars.sh (its JSON files). For each variant
the reproduction check of the bars' protocol: per row, the spread of the
runs (max ÷ min − 1), flagged when it exceeds 0.1% in instructions or 0.5%
in estimated cycles (such a row needs five runs and their median). Then,
per row, the median of each variant's runs, b ÷ a, and the geomean of b ÷ a
over --rows (default: every row both have), for instructions and estimated
cycles; and the allocations per iteration of each (from the first run's
allocs.json). Prints Markdown.
"""
import argparse
import glob
import json
import math
import os
import statistics

ORDER = ["plain", "defaults", "named", "env-fn", "superassign", "s3", "fib", "binarytrees",
         "d2s-assign", "d2s-eval", "d2s-callee-assign", "d2s-delayed"]
METRICS = [("robigo_instr", "instr", 0.1), ("robigo_est_cycles", "est. cycles", 0.5)]


def load(d):
    rows = {}
    for f in sorted(glob.glob(os.path.join(d, "*.json"))):
        if not f.endswith("allocs.json"):
            rows.update(json.load(open(f))["rows"])
    return rows


def allocs(d):
    f = os.path.join(d, "allocs.json")
    return json.load(open(f))["rows"] if os.path.exists(f) else {}


def spread(v):
    return (max(v) - min(v)) / min(v) * 100


def gm(v):
    return math.exp(sum(map(math.log, v)) / len(v))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--a", nargs="+", required=True, metavar=("NAME", "DIR"))
    ap.add_argument("--b", nargs="+", required=True, metavar=("NAME", "DIR"))
    ap.add_argument("--rows", help="comma-separated rows for the geomean")
    a = ap.parse_args()
    va = (a.a[0], [load(d) for d in a.a[1:]], allocs(a.a[1]))
    vb = (a.b[0], [load(d) for d in a.b[1:]], allocs(a.b[1]))
    common = [r for r in ORDER if all(r in x for x in va[1] + vb[1])]
    common += sorted(r for r in set.intersection(*(set(x) for x in va[1] + vb[1])) if r not in ORDER)
    sel = a.rows.split(",") if a.rows else common
    missing = [r for r in sel if r not in common]
    if missing:
        print(f"**Missing rows** (preliminary, decides nothing): {', '.join(missing)}\n")
    for name, runs, _ in (va, vb):
        print(f"Reproduction, `{name}` ({len(runs)} runs): spread max ÷ min − 1\n")
        print("| row | " + " | ".join(f"run {i + 1} {m}" for i in range(len(runs)) for _, m, _ in METRICS)
              + " | instr spread | cycles spread |")
        print("|---|" + "---:|" * (len(runs) * 2 + 2))
        for r in common:
            cells = [f"{x[r][k]:,.1f}" for x in runs for k, _, _ in METRICS]
            sp = [spread([x[r][k] for x in runs]) for k, _, _ in METRICS]
            flag = " (over)" if sp[0] > 0.1 or sp[1] > 0.5 else ""
            print(f"| {r} | " + " | ".join(cells) + f" | {sp[0]:.3f}% | {sp[1]:.3f}%{flag} |")
        print()
    print(f"Medians and `{vb[0]}` ÷ `{va[0]}`\n")
    print(f"| row | {va[0]} instr | {vb[0]} instr | ÷ | {va[0]} est. cycles | {vb[0]} est. cycles | ÷ |")
    print("|---|---:|---:|---:|---:|---:|---:|")
    ratios = {k: [] for k, _, _ in METRICS}
    for r in common:
        cells = []
        for k, _, _ in METRICS:
            x = statistics.median(v[r][k] for v in va[1])
            y = statistics.median(v[r][k] for v in vb[1])
            if r in sel:
                ratios[k].append(y / x)
            cells += [f"{x:,.1f}", f"{y:,.1f}", f"{y / x:.4f}"]
        print(f"| {r} | " + " | ".join(cells) + " |")
    if all(ratios.values()):
        g = [gm(ratios[k]) for k, _, _ in METRICS]
        print(f"| geomean ({', '.join(r for r in sel if r in common)}) | | | {g[0]:.4f} | | | {g[1]:.4f} |")
    print()
    if va[2] and vb[2]:
        print("Allocations per iteration (objects, bytes; by SEXPTYPE)\n")
        print(f"| row | {va[0]} objects | {va[0]} bytes | {vb[0]} objects | {vb[0]} bytes | {va[0]} by type | {vb[0]} by type |")
        print("|---|---:|---:|---:|---:|---|---|")
        t = lambda z: ", ".join(f"{k} {v['objects']:g}" for k, v in sorted(z["by_type"].items()) if v["objects"])
        for r in common:
            if r in va[2] and r in vb[2]:
                x, y = va[2][r], vb[2][r]
                print(f"| {r} | {x['objects']:g} | {x['bytes']:g} | {y['objects']:g} | {y['bytes']:g} | {t(x)} | {t(y)} |")


if __name__ == "__main__":
    main()
