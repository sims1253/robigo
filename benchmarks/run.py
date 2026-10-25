#!/usr/bin/env python3
"""Time rr (interpreter and bytecode VM) against GNU R's Rscript.

Usage: benchmarks/run.py [path/to/rr] [names...]

Each script runs three times per configuration; the best wall time is
reported, including process startup (~0.03 s for both). Outputs must be
identical across all three runs, or the row is marked.
"""
import os
import subprocess
import sys
import time

here = os.path.dirname(os.path.abspath(__file__))
rr = sys.argv[1] if len(sys.argv) > 1 else "rr"
names = sys.argv[2:] or sorted(
    f[:-2] for f in os.listdir(here) if f.endswith(".R") and f != "vm-ops.R"
)
env = dict(os.environ, LANGUAGE="en", LC_ALL="C.UTF-8", TZ="UTC", R_DEFAULT_PACKAGES="NULL")


def run(cmd, extra):
    best, out = None, None
    for _ in range(3):
        t = time.perf_counter()
        p = subprocess.run(cmd, env=dict(env, **extra), capture_output=True, cwd=here)
        d = time.perf_counter() - t
        best = d if best is None else min(best, d)
        out = p.stdout + p.stderr
    return best, out


print(f"{'script':12} {'R':>7} {'rr':>7} {'rr-vm':>7}")
for n in names:
    f = n + ".R"
    r, ro = run(["Rscript", f], {})
    a, ao = run([rr, f], {"RR_COMPILE": "0"})
    b, bo = run([rr, f], {"RR_COMPILE": "1"})
    mark = "" if ro == ao == bo else "  (output differs)"
    print(f"{n:12} {r:7.2f} {a:7.2f} {b:7.2f}{mark}")
