#!/usr/bin/env python3
"""Time rr (interpreter and bytecode VM) against GNU R's Rscript.

Usage: benchmarks/run.py [--rr path/to/rr] [--runs N] [names...]

Runs the programs in benchmarks/suite (or the named ones) with the oracle
environment and R_DEFAULT_PACKAGES=utils,stats. Reports the best wall
time of N runs (default 3), including process startup, and the ratio
rr/R (below 1 means rr is faster). A row is marked if outputs differ.
"""
import argparse
import os
import subprocess
import time

here = os.path.dirname(os.path.abspath(__file__))
suite = os.path.join(here, "suite")
ap = argparse.ArgumentParser()
ap.add_argument("--rr", default=os.path.expanduser("~/.cache/cargo-target/release/rr"))
ap.add_argument("--runs", type=int, default=3)
ap.add_argument("names", nargs="*")
args = ap.parse_args()
names = args.names or sorted(f[:-2] for f in os.listdir(suite) if f.endswith(".R"))
env = dict(os.environ, LANGUAGE="en", LC_ALL="C.UTF-8", TZ="UTC", R_DEFAULT_PACKAGES="utils,stats")


def run(cmd, extra):
    best, out = None, b""
    for _ in range(args.runs):
        t = time.perf_counter()
        p = subprocess.run(cmd, env=dict(env, **extra), capture_output=True, cwd="/tmp")
        d = time.perf_counter() - t
        best = d if best is None else min(best, d)
        out = p.stdout + p.stderr
    return best, out


print(f"{'program':14} {'R':>7} {'rr':>7} {'rr-vm':>7} {'vm/R':>6}")
for n in names:
    f = os.path.join(suite, n + ".R")
    r, ro = run(["Rscript", f], {})
    a, ao = run([args.rr, f], {"RR_COMPILE": "0"})
    b, bo = run([args.rr, f], {"RR_COMPILE": "1"})
    mark = "" if ro == ao == bo else "  (output differs)"
    print(f"{n:14} {r:7.2f} {a:7.2f} {b:7.2f} {b / r:6.2f}{mark}", flush=True)
