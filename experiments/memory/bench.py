#!/usr/bin/env python3
"""Measurement harness for the memory-manager bake-off.

For every workload x variant (rc, gc):
  - a native pass (repeated, variants interleaved so both see the same
    machine load): wall time (min/median/max), peak RSS, longest
    collector pause, heap reserved, checksum;
  - a callgrind pass with --cache-sim=yes: instructions (Ir), D1 and LL
    read/write misses, and an estimated cycle count (deterministic,
    load-independent: the cache model is a fixed machine).

Estimated cycles = Ir + 10*(D1mr + D1mw) + 100*(LLmr + LLmw), where
LLmr/LLmw are DLmr/DLmw. The formula charges ~10 cycles per L1 miss that
hits lower cache and ~100 per last-level miss (a rough desktop/shared-LLC
machine; the sensitivity to these two constants is visible in the ratios
and can be recomputed from the raw counters in the results files).

The two variants must agree on every checksum: the workloads are written
once, generic over the Mem trait, so a mismatch means a manager bug and
the run is refused.

Usage:
  bench.py run [--scale 1.0] [--reps 5] [--workloads a,b,...] [--json OUT]
               [--skip-ir] [--skip-cache]
  bench.py table RESULTS.json...
"""

import json
import argparse
import os
import pathlib
import statistics
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
BIN = pathlib.Path.home() / ".cache/cargo-target/release/mmbench"
VALGRIND = pathlib.Path.home() / ".cache/rr-tools/bin/valgrind"
# callgrind outputs live under the worktree's ignored .robigo/ (not /tmp,
# which is a small tmpfs); deleted after parsing.
CGDIR = HERE / ".robigo" / "cg"

WORKLOADS = ["binarytrees", "fib", "lapply", "listgrowth", "strings", "cycles", "capi"]
VARIANTS = ["rc", "gc"]

# cycle cost of a D1 miss that resolves in a lower cache, and of an LL miss
CYC_D1 = 10
CYC_LL = 100


def est_cycles(ir, d1mr, d1mw, llmr, llmw):
    return ir + CYC_D1 * (d1mr + d1mw) + CYC_LL * (llmr + llmw)


def run_once(variant, workload, scale):
    out = subprocess.run(
        [BIN, variant, workload, str(scale)],
        capture_output=True,
        text=True,
        timeout=600,
    )
    if out.returncode != 0:
        raise RuntimeError(
            f"{variant}/{workload} failed ({out.returncode}):\n{out.stderr[-2000:]}"
        )
    return json.loads(out.stdout)


def callgrind_run(variant, workload, scale, cache):
    """Run one callgrind pass. cache=True adds --cache-sim=yes and returns
    the full counter set; cache=False returns only Ir."""
    CGDIR.mkdir(parents=True, exist_ok=True)
    cg = CGDIR / f"cg-{variant}-{workload}-{scale}.out"
    cmd = [
        VALGRIND,
        "--tool=callgrind",
        f"--cache-sim={'yes' if cache else 'no'}",
        "--branch-sim=no",
        f"--callgrind-out-file={cg}",
        BIN,
        variant,
        workload,
        str(scale),
    ]
    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=3600)
    if proc.returncode != 0:
        raise RuntimeError(
            f"callgrind {variant}/{workload} failed:\n{proc.stderr[-2000:]}"
        )
    events = None
    values = None
    try:
        for line in cg.read_text().splitlines():
            if line.startswith("events:"):
                events = line.split()[1:]
            elif line.startswith("summary:"):
                values = [int(x) for x in line.split()[1:]]
    finally:
        cg.unlink(missing_ok=True)
    if not events or not values:
        raise RuntimeError(f"no summary in callgrind output for {variant}/{workload}")
    c = dict(zip(events, values))
    if cache:
        return {
            "ir": c["Ir"],
            "d1mr": c["D1mr"],
            "d1mw": c["D1mw"],
            "llmr": c["DLmr"],
            "llmw": c["DLmw"],
        }
    return {"ir": c["Ir"]}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("command", choices=["run", "table"])
    ap.add_argument("--scale", type=float, default=1.0)
    ap.add_argument("--reps", type=int, default=5)
    ap.add_argument("--skip-ir", action="store_true", help="no callgrind pass at all")
    ap.add_argument("--skip-cache", action="store_true", help="callgrind without --cache-sim (Ir only)")
    ap.add_argument("--workloads", default=",".join(WORKLOADS))
    ap.add_argument("--json", default=None, help="append results to this file")
    ap.add_argument("files", nargs="*", help="results files for 'table'")
    args = ap.parse_args()

    if args.command == "table":
        rows = {}
        for f in args.files:
            doc = json.loads(pathlib.Path(f).read_text())
            # a bare {"results": ...} or the {"runs": [...]} format the
            # run command appends to; later files and later runs win
            runs = doc["runs"] if "runs" in doc else [doc]
            for run in runs:
                for r in run["results"]:
                    key = (r["workload"], r["variant"])
                    rows[key] = r  # later runs win
        workloads = sorted({k[0] for k in rows})

        def num(r, k, div=1.0):
            v = r.get(k)
            return f"{v / div:.1f}" if v is not None else "-"

        print("| workload | variant | wall min | wall med | wall max | Ir (M) | D1mr (k) | D1mw (k) | LLmr (k) | LLmw (k) | est cyc (M) | rss MB | max pause µs | collections | heap MB |")
        print("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|")
        for w in workloads:
            for v in VARIANTS:
                r = rows.get((w, v))
                if not r:
                    continue
                print(
                    f"| {w} | {v} "
                    f"| {r['wall_ms_min']:.1f} | {r['wall_ms_median']:.1f} | {r['wall_ms_max']:.1f} "
                    f"| {num(r, 'ir', 1e6)} | {num(r, 'd1mr', 1e3)} | {num(r, 'd1mw', 1e3)} "
                    f"| {num(r, 'llmr', 1e3)} | {num(r, 'llmw', 1e3)} | {num(r, 'est_cycles', 1e6)} "
                    f"| {r['rss_mb']:.1f} | {r['max_pause_us']:.0f} "
                    f"| {r['collections']} | {r['heap_mb']:.1f} |"
                )
        # variant ratios, rc/gc
        print()
        print("| workload | wall rc/gc | Ir rc/gc | D1m rc/gc | LLm rc/gc | est cyc rc/gc | rss rc/gc |")
        print("|---|---|---|---|---|---|---|")
        for w in workloads:
            rc, gc = rows.get((w, "rc")), rows.get((w, "gc"))
            if not (rc and gc):
                continue

            def ratio(a, b, k):
                return f"{a[k] / b[k]:.3f}" if a.get(k) and b.get(k) else "-"

            d1 = (
                f"{(rc['d1mr'] + rc['d1mw']) / (gc['d1mr'] + gc['d1mw']):.3f}"
                if rc.get("d1mr") is not None and gc.get("d1mr") is not None
                else "-"
            )
            ll = (
                f"{(rc['llmr'] + rc['llmw']) / (gc['llmr'] + gc['llmw']):.3f}"
                if rc.get("llmr") is not None and gc.get("llmr") is not None
                else "-"
            )
            print(
                f"| {w} | {ratio(rc, gc, 'wall_ms_median')} "
                f"| {ratio(rc, gc, 'ir')} | {d1} | {ll} "
                f"| {ratio(rc, gc, 'est_cycles')} | {ratio(rc, gc, 'rss_mb')} |"
            )
        return 0

    workloads = [w for w in args.workloads.split(",") if w]
    results = []
    load0 = os.getloadavg()
    print(f"# load before: {load0[0]:.2f} {load0[1]:.2f} {load0[2]:.2f}", file=sys.stderr)
    for w in workloads:
        checks = {}
        walls = {v: [] for v in VARIANTS}
        best = {}
        # interleaved: variant pairs inside each rep, so rc and gc see the
        # same machine load
        for _ in range(max(1, args.reps)):
            for v in VARIANTS:
                r = run_once(v, w, args.scale)
                checks[v] = r["check"]
                walls[v].append(r["wall_ms"])
                cur = best.get(v)
                if cur is None or r["rss_mb"] < cur["rss_mb"]:
                    best[v] = r
        if checks["rc"] != checks["gc"]:
            print(f"REFUSED: {w} checksum mismatch rc={checks['rc']} gc={checks['gc']}", file=sys.stderr)
            return 1
        for v in VARIANTS:
            ws = sorted(walls[v])
            b = best[v]
            results.append(
                {
                    "workload": w,
                    "variant": v,
                    "scale": args.scale,
                    "check": checks[v],
                    "wall_ms_median": statistics.median(ws),
                    "wall_ms_min": ws[0],
                    "wall_ms_max": ws[-1],
                    "wall_ms_spread": ws[-1] / ws[0],
                    "rss_mb": b["rss_mb"],
                    "max_pause_us": b["max_pause_us"],
                    "collections": b["collections"],
                    "gc_ms": b["gc_ms"],
                    "heap_mb": b["heap_mb"],
                    "allocs": b["allocs"],
                    "frees": b["frees"],
                    "live_mb": b["live_mb"],
                }
            )
        if not args.skip_ir:
            for v in VARIANTS:
                c = callgrind_run(v, w, args.scale, cache=not args.skip_cache)
                for r in results:
                    if r["workload"] == w and r["variant"] == v:
                        r.update(c)
                        if "d1mr" in c:
                            r["est_cycles"] = est_cycles(
                                c["ir"], c["d1mr"], c["d1mw"], c["llmr"], c["llmw"]
                            )
        for r in results:
            if r["workload"] != w:
                continue
            extra = ""
            if r.get("ir"):
                extra += f" Ir={r['ir'] / 1e6:.1f}M"
                if r.get("d1mr") is not None:
                    extra += (
                        f" D1m={((r['d1mr'] + r['d1mw']) / 1e3):.0f}k"
                        f" LLm={((r['llmr'] + r['llmw']) / 1e3):.0f}k"
                        f" cyc={r['est_cycles'] / 1e6:.1f}M"
                    )
            print(
                f"{w}/{r['variant']}: wall={r['wall_ms_median']:.1f}ms{extra} "
                f"rss={r['rss_mb']:.1f}MB pause={r['max_pause_us']:.0f}µs "
                f"cols={r['collections']} heap={r['heap_mb']:.1f}MB",
                file=sys.stderr,
            )
    load1 = os.getloadavg()
    print(f"# load after: {load1[0]:.2f} {load1[1]:.2f} {load1[2]:.2f}", file=sys.stderr)

    doc = {
        "scale": args.scale,
        "reps": args.reps,
        "load_before": load0,
        "load_after": load1,
        "results": results,
    }
    if args.json:
        path = pathlib.Path(args.json)
        old = json.loads(path.read_text()) if path.exists() else {"runs": []}
        old.setdefault("runs", []).append(doc)
        path.write_text(json.dumps(old, indent=1))
    else:
        print(json.dumps(doc, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
