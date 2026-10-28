#!/usr/bin/env python3
"""Parse-throughput measurements for robigo-syntax (docs/ledger/parser.md).

Two metrics over the parse corpora's manifests:

  wall     MB/s of `parse-batch --parse-only` (read + transcode + parse +
           tree checksum; best of N runs), per corpus and in total.
  callgrind  instructions per source byte, from `callgrind --tool=callgrind
           --collect-atstart=no` over `--parse-only` (I/O syscalls are not
           counted, so this isolates the parse work).

Usage:
  scripts/parse-perf.py wall   [CORPUS...] [--runs N]
  scripts/parse-perf.py cg     [CORPUS...] [--runs N]

Corpora are the manifests of scripts/parse-corpus.py (default: all local).
The binary is built into $CARGO_TARGET_DIR (a private one is strongly
advised; the shared dir can bleed another worktree's rlib into a bin
build).
"""

import argparse
import os
import re
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CACHE = Path(os.environ.get("ROBIGO_PARSE_CACHE", Path.home() / ".cache/robigo-research/parse-oracle"))
VALGRIND = Path.home() / ".cache/rr-tools/bin/valgrind"
CORPORA = ["rlib", "rtests", "fastr", "gnur-tests", "gnur-examples", "cran"]


def build():
    subprocess.run(
        ["cargo", "build", "--release", "-p", "robigo-syntax", "--bin", "parse-batch"],
        check=True, cwd=ROOT,
    )
    target = os.environ.get("CARGO_TARGET_DIR", str(Path.home() / ".cache/cargo-target"))
    binp = Path(target) / "release/parse-batch"
    if not binp.exists():
        raise SystemExit(f"parse-batch not found at {binp}")
    return binp


def manifest(name):
    m = CACHE / f"manifests/{name}.abs.tsv"
    if not m.exists():
        m = CACHE / f"manifests/{name}.tsv"
    if not m.exists():
        raise SystemExit(f"no manifest for {name!r} (run scripts/parse-corpus.py list)")
    return m


def corpus_bytes(name):
    total = 0
    for line in manifest(name).read_text().splitlines():
        p = line.split("\t")[1]
        try:
            total += os.path.getsize(p)
        except OSError:
            pass
    return total


def out_file(name):
    d = CACHE / "perf"
    d.mkdir(parents=True, exist_ok=True)
    return d / f"{name}.out"


def run_once(binp, name, extra=()):
    out = out_file(name)
    t0 = time.perf_counter()
    subprocess.run(
        [str(binp), *extra, str(manifest(name)), str(out)],
        check=True, cwd=ROOT, stdout=subprocess.DEVNULL,
    )
    return time.perf_counter() - t0


def cmd_wall(binp, names, runs):
    print(f"{'corpus':14} {'MB':>7} {'best s':>8} {'MB/s':>8}")
    total_b = 0.0
    total_t = 0.0
    for name in names:
        nbytes = corpus_bytes(name)
        best = min(run_once(binp, name, ["--parse-only"]) for _ in range(runs))
        total_b += nbytes
        total_t += best
        print(f"{name:14} {nbytes / 1e6:7.1f} {best:8.3f} {nbytes / best / 1e6:8.2f}")
    if len(names) > 1:
        print(f"{'total':14} {total_b / 1e6:7.1f} {total_t:8.3f} {total_b / total_t / 1e6:8.2f}")


CG_TOTAL = re.compile(rb"I\s+refs:\s+([\d,]+)")


def cmd_cg(binp, names, runs):
    print(f"{'corpus':14} {'MB':>7} {'runs':>12} {'best instr':>14} {'instr/kB':>10}")
    total_b = 0
    total_i = 0
    for name in names:
        nbytes = corpus_bytes(name)
        best = None
        for _ in range(runs):
            r = subprocess.run(
                [str(VALGRIND), "--tool=callgrind", "--callgrind-out-file=/dev/null",
                 str(binp), "--parse-only", str(manifest(name)), str(out_file(name))],
                check=True, cwd=ROOT, capture_output=True,
            )
            m = CG_TOTAL.search(r.stderr)
            if not m:
                raise SystemExit("callgrind total not found:\n" + r.stderr[-2000:].decode())
            i = int(m.group(1).replace(b",", b""))
            best = i if best is None else min(best, i)
        total_b += nbytes
        total_i += best
        print(f"{name:14} {nbytes / 1e6:7.1f} {runs:12} {best:14,} {best / (nbytes / 1000):10.1f}")
    if len(names) > 1:
        print(f"{'total':14} {total_b / 1e6:7.1f} {'':12} {total_i:14,} {total_i / (total_b / 1000):10.1f}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("metric", choices=["wall", "cg"])
    ap.add_argument("corpora", nargs="*", help=f"one of {CORPORA} (default: all local)")
    ap.add_argument("--runs", type=int, default=3)
    args = ap.parse_args()
    names = args.corpora or [n for n in CORPORA if manifest(n).exists()]
    binp = build()
    if args.metric == "wall":
        cmd_wall(binp, names, args.runs)
    else:
        cmd_cg(binp, names, args.runs)


if __name__ == "__main__":
    main()
