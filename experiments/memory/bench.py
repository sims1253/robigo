#!/usr/bin/env python3
"""Measurement harness for the memory-manager bake-off.

For every workload x variant (rc, gc):
  - a native pass (repeated): wall time (median), peak RSS, longest
    collector pause, heap reserved, checksum;
  - a callgrind pass: total instruction count (Ir).

The two variants must agree on every checksum: the workloads are written
once, generic over the Mem trait, so a mismatch means a manager bug and
the run is refused.

Usage:
  bench.py run [--scale 1.0] [--reps 5] [--workloads a,b,...] [--json OUT]
  bench.py table RESULTS.json...
"""

import json
import argparse
import pathlib
import statistics
import subprocess
import sys
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
BIN = pathlib.Path.home() / ".cache/cargo-target/release/mmbench"
VALGRIND = pathlib.Path.home() / ".cache/rr-tools/bin/valgrind"

WORKLOADS = ["binarytrees", "fib", "lapply", "listgrowth", "strings", "cycles", "capi"]
VARIANTS = ["rc", "gc"]


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


def callgrind_ir(variant, workload, scale):
    with tempfile.TemporaryDirectory(prefix="mmcg") as td:
        cg = pathlib.Path(td) / "cg.out"
        cmd = [
            VALGRIND,
            "--tool=callgrind",
            "--cache-sim=no",
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
        for line in cg.read_text().splitlines():
            if line.startswith("summary:"):
                # "summary: Ir 12345"
                return int(line.split()[-1])
    raise RuntimeError(f"no summary in callgrind output for {variant}/{workload}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("command", choices=["run", "table"])
    ap.add_argument("--scale", type=float, default=1.0)
    ap.add_argument("--reps", type=int, default=5)
    ap.add_argument("--skip-ir", action="store_true", help="native numbers only")
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
        print(f"| workload | variant | wall ms | Ir (M) | rss MB | max pause µs | collections | heap MB |")
        print("|---|---|---|---|---|---|---|---|")
        for w in workloads:
            for v in VARIANTS:
                r = rows.get((w, v))
                if not r:
                    continue
                ir = f"{r['ir'] / 1e6:.1f}" if r.get("ir") else "-"
                print(
                    f"| {w} | {v} | {r['wall_ms_median']:.1f} | {ir} "
                    f"| {r['rss_mb']:.1f} | {r['max_pause_us']:.0f} "
                    f"| {r['collections']} | {r['heap_mb']:.1f} |"
                )
        # variant ratios, rc/gc
        print()
        print("| workload | wall rc/gc | Ir rc/gc | rss rc/gc |")
        print("|---|---|---|---|")
        for w in workloads:
            rc, gc = rows.get((w, "rc")), rows.get((w, "gc"))
            if not (rc and gc):
                continue
            irr = f"{rc['ir'] / gc['ir']:.3f}" if rc.get("ir") and gc.get("ir") else "-"
            print(
                f"| {w} | {rc['wall_ms_median'] / gc['wall_ms_median']:.3f} "
                f"| {irr} | {rc['rss_mb'] / gc['rss_mb']:.3f} |"
            )
        return 0

    workloads = [w for w in args.workloads.split(",") if w]
    results = []
    for w in workloads:
        checks = {}
        for v in VARIANTS:
            runs = [run_once(v, w, args.scale) for _ in range(max(1, args.reps))]
            check = runs[0]["check"]
            checks[v] = check
            walls = sorted(r["wall_ms"] for r in runs)
            best = min(runs, key=lambda r: r["rss_mb"])
            results.append(
                {
                    "workload": w,
                    "variant": v,
                    "scale": args.scale,
                    "check": check,
                    "wall_ms_median": statistics.median(walls),
                    "wall_ms_min": walls[0],
                    "rss_mb": best["rss_mb"],
                    "max_pause_us": best["max_pause_us"],
                    "collections": best["collections"],
                    "gc_ms": best["gc_ms"],
                    "heap_mb": best["heap_mb"],
                    "allocs": best["allocs"],
                    "frees": best["frees"],
                    "live_mb": best["live_mb"],
                }
            )
        if checks["rc"] != checks["gc"]:
            print(f"REFUSED: {w} checksum mismatch rc={checks['rc']} gc={checks['gc']}", file=sys.stderr)
            return 1
        if not args.skip_ir:
            for v in VARIANTS:
                ir = callgrind_ir(v, w, args.scale)
                for r in results:
                    if r["workload"] == w and r["variant"] == v:
                        r["ir"] = ir
        rcs = [r for r in results if r["workload"] == w]
        for r in rcs:
            ir = f" Ir={r['ir'] / 1e6:.1f}M" if r.get("ir") else ""
            print(
                f"{w}/{r['variant']}: wall={r['wall_ms_median']:.1f}ms{ir} "
                f"rss={r['rss_mb']:.1f}MB pause={r['max_pause_us']:.0f}µs "
                f"cols={r['collections']} heap={r['heap_mb']:.1f}MB",
                file=sys.stderr,
            )

    doc = {
        "scale": args.scale,
        "reps": args.reps,
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
