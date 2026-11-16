#!/usr/bin/env python3
"""E1's wall time under docs/g3.md §13's rule, for one program, one process:
at least 10 interleaved runs per runtime, each batched to at least 0.05 s,
medians compared, two runtimes within 2% tie (a tie does not count as
faster). scripts/rivals.py takes the fastest of N runs per runtime in turn,
so this wraps its Runner (containers started once, the clock read where the
program runs) and its programs list.

    python3 wall13.py --robigo BIN [--rounds 11] [--filter 'calls$'] --out FILE

GNU R is the 4.6.2 oracle image (§13's rival), not rivals.py's 4.5.0. The
round order rotates, so no runtime always runs first. A runtime's answer
must be GNU R's stdout, as rivals.py requires.
"""
import argparse
import json
import math
import os
import statistics
import sys
import tempfile

# docs/ledger/g3/data/<run>/wall13.py: the checkout is five directories up
ROOT = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), *[".."] * 5))
sys.path.insert(0, os.path.join(ROOT, "scripts"))
import rivals  # noqa: E402
from perf import conf  # noqa: E402

rivals.IMAGES["gnur"] = ("robigo-oracle:4.6.2", ["Rscript"])
IMPLS = ["robigo", "gnur", "rir", "fastr"]
BATCH_S = 0.05
TIE = 0.02


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--robigo", required=True)
    ap.add_argument("--rounds", type=int, default=11)
    ap.add_argument("--filter", default="calls$")
    ap.add_argument("--timeout", type=float, default=300.0)
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    if a.rounds < 10:
        sys.exit("§13 asks for at least 10 runs per runtime")
    progs = rivals.programs(conf(), a.filter)
    if len(progs) != 1:
        sys.exit(f"--filter must select one program, got {[p[0] for p in progs]}")
    name, shot, _, pk = progs[0]
    os.makedirs(rivals.RIVALS, exist_ok=True)
    doc = {"program": name, "robigo": os.path.abspath(a.robigo), "gnur_image": rivals.IMAGES["gnur"][0],
           "rounds": a.rounds, "batch_s": BATCH_S, "tie": TIE, "samples": {i: [] for i in IMPLS}, "batch": {}}
    with tempfile.TemporaryDirectory(dir=rivals.RIVALS) as shared:
        os.chmod(shared, 0o777)
        runner = rivals.Runner(IMPLS, shared)
        try:
            file = os.path.join(shared, "shot.R")
            with open(file, "w") as f:
                f.write(shot)

            def once(impl):
                secs, out, status = runner.run(impl, file, pk, a.timeout, a.robigo)
                if secs is None or status != 0:
                    sys.exit(f"{impl}: failed: {out[-300:]}")
                return secs, out

            # the answers, and each runtime's batch: k processes back to back,
            # so that a timed sample is at least BATCH_S
            answers = {}
            for impl in IMPLS:
                secs, out = once(impl)
                answers[impl] = out
                doc["batch"][impl] = max(1, math.ceil(BATCH_S / secs))
            for impl in IMPLS:
                if answers[impl] != answers["gnur"]:
                    sys.exit(f"{impl}: its answer is not GNU R's")
            for r in range(a.rounds):
                order = IMPLS[r % len(IMPLS):] + IMPLS[:r % len(IMPLS)]
                for impl in order:
                    k = doc["batch"][impl]
                    total = sum(once(impl)[0] for _ in range(k))
                    doc["samples"][impl].append(total / k)
                    print(f"round {r + 1:2} {impl:7} {total / k:.4f} s", file=sys.stderr, flush=True)
                with open(a.out, "w") as f:
                    json.dump(doc, f, indent=1)
        finally:
            runner.close()
    med = {i: statistics.median(doc["samples"][i]) for i in IMPLS}
    doc["median"] = med
    doc["robigo_vs"] = {}
    for i in IMPLS[1:]:
        ratio = med["robigo"] / med[i]
        verdict = "tie" if abs(ratio - 1) <= TIE else ("faster" if ratio < 1 else "slower")
        doc["robigo_vs"][i] = {"ratio": ratio, "verdict": verdict}
    with open(a.out, "w") as f:
        json.dump(doc, f, indent=1)
    print(f"{name}, {a.rounds} interleaved rounds, medians:")
    for i in IMPLS:
        xs = doc["samples"][i]
        print(f"  {i:7} median {med[i]:.4f} s  (min {min(xs):.4f}, max {max(xs):.4f}, batch {doc['batch'][i]})")
    for i, v in doc["robigo_vs"].items():
        print(f"  robigo-rt / {i}: {v['ratio']:.3f} ({v['verdict']})")


if __name__ == "__main__":
    main()
