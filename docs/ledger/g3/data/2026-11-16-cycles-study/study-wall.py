#!/usr/bin/env python3
"""Wall time per iteration of benchmarks/rt/calls-rows.toml rows, for the
estimated-cycles study (docs/ledger/g3/2026-11-16-m1-exit-lead-decisions.md,
decision (c)), under docs/g3.md §13's protocol: each sample batched to at
least 0.05 s (a row's loop is sized to about TARGET seconds), at least 10
interleaved rounds over every (binary, row), medians.

A sample is one round's (T(2K) - T(K)) / K, the process wall times of the
row's loop script with K and 2K iterations, the same script and N-against-2N
method scripts/idiom-vs-r.py counts instructions with, so that startup and
setup cancel. Standalone: it needs only Python 3.11 and the rows file.

    study-wall.py --rows calls-rows.toml --bin NAME=PATH ... [--row NAME ...]
                  [--rounds 11] [--target 0.3] [--env K=V ...] --out FILE
"""
import argparse
import json
import math
import os
import statistics
import subprocess
import sys
import tempfile
import time
import tomllib

BASE_ENV = {"LANGUAGE": "en", "LC_ALL": "C.UTF-8", "TZ": "UTC", "R_ENVIRON_USER": "", "R_PROFILE_USER": "",
            "R_DEFAULT_PACKAGES": "NULL", "ROBIGO_COMPILE": "1"}


def script(d, setup, body, k):
    p = os.path.join(d, f"loop-{k}.R")
    with open(p, "w") as f:
        f.write(f"{setup}\ng <- function() {{ for (i in 1:{k}) {{ {body} }}; NULL }}\ng()\n")
    return p


def run(binary, path, env):
    t = time.perf_counter()
    p = subprocess.run([binary, path], env=env, capture_output=True)
    dt = time.perf_counter() - t
    if p.returncode:
        raise RuntimeError(p.stderr.decode("utf-8", "replace")[-500:])
    return dt


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rows", required=True)
    ap.add_argument("--bin", action="append", required=True, help="NAME=PATH")
    ap.add_argument("--row", action="append")
    ap.add_argument("--rounds", type=int, default=11)
    ap.add_argument("--target", type=float, default=0.3, help="seconds of a row's K-iteration loop")
    ap.add_argument("--env", action="append", default=[])
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    if a.rounds < 10:
        sys.exit("§13 asks for at least 10 runs")
    rows = {r["name"]: r for r in tomllib.load(open(a.rows, "rb"))["row"]}
    names = a.row or list(rows)
    bins = dict(b.split("=", 1) for b in a.bin)
    env = {**os.environ, **BASE_ENV, **dict(e.split("=", 1) for e in a.env)}
    work = tempfile.mkdtemp(prefix="study-wall-")
    ref = list(bins)[-1]
    doc = {"rows": {}, "bins": bins, "env": a.env, "rounds": a.rounds, "target": a.target,
           "host": os.uname().nodename}
    try:
        with open("/proc/cpuinfo") as f:
            doc["cpu"] = next(l.split(":", 1)[1].strip() for l in f if l.startswith("model name"))
    except (OSError, StopIteration):
        doc["cpu"] = "?"
    # K per row: calibrated on the last binary so that K iterations take
    # about --target seconds (and so a sample is far above 0.05 s)
    plan = {}
    for n in names:
        r = rows[n]
        d = os.path.join(work, n)
        os.makedirs(d, exist_ok=True)
        k = max(1, int(r.get("n", 100)))
        while True:
            t1 = run(bins[ref], script(d, r["setup"], r["body"], k), env)
            t2 = run(bins[ref], script(d, r["setup"], r["body"], 2 * k), env)
            loop = t2 - t1
            if loop >= a.target / 4 or k > 10**8:
                break
            k *= 4
        k = max(1, math.ceil(k * a.target / max(loop, 1e-6)))
        plan[n] = k
        print(f"{n}: K = {k}", file=sys.stderr, flush=True)
    pairs = [(b, n) for n in names for b in bins]
    samples = {f"{b}|{n}": [] for b, n in pairs}
    failed = set()
    for rd in range(a.rounds):
        order = pairs[rd % len(pairs):] + pairs[:rd % len(pairs)]
        for b, n in order:
            if (b, n) in failed:
                continue
            r, k = rows[n], plan[n]
            d = os.path.join(work, n)
            s1, s2 = script(d, r["setup"], r["body"], k), script(d, r["setup"], r["body"], 2 * k)
            try:
                # alternate which of K and 2K runs first
                if rd % 2:
                    t2, t1 = run(bins[b], s2, env), run(bins[b], s1, env)
                else:
                    t1, t2 = run(bins[b], s1, env), run(bins[b], s2, env)
            except RuntimeError as e:
                print(f"{b} {n}: failed: {e}", file=sys.stderr)
                failed.add((b, n))
                continue
            samples[f"{b}|{n}"].append((t2 - t1) / k)
        print(f"round {rd + 1} done", file=sys.stderr, flush=True)
        doc["samples"] = samples
        doc["k"] = plan
        with open(a.out, "w") as f:
            json.dump(doc, f, indent=1)
    doc["median_ns"] = {key: statistics.median(v) * 1e9 for key, v in samples.items() if v}
    doc["iqr_rel"] = {}
    for key, v in samples.items():
        if len(v) >= 4:
            q = statistics.quantiles(v, n=4)
            doc["iqr_rel"][key] = (q[2] - q[0]) / statistics.median(v)
    doc["failed"] = sorted(f"{b}|{n}" for b, n in failed)
    with open(a.out, "w") as f:
        json.dump(doc, f, indent=1)
    for key in sorted(doc["median_ns"]):
        print(f"{key:40} {doc['median_ns'][key]:14.1f} ns  IQR {doc['iqr_rel'].get(key, 0):.3f}")


if __name__ == "__main__":
    main()
