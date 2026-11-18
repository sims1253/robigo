#!/usr/bin/env python3
"""GNU R's side of the perf gate, kept apart from robigo's numbers so that
a pull request's `perf` job need not measure it again.

    scripts/perf-gnur.py extract PERF_JSON OUT        # main: GNU R's records of a run
    scripts/perf-gnur.py fill GNUR_JSON [FILTER]      # a pull request: complete them

GNU R's records give a pull request two things: the answer robigo's must
equal (`out` and `status`, which decide `wrong`), and the instruction
counts its ratio is reported against. The answer does not depend on the
CPU, but the counts do (glibc picks its routines by CPU), so CI keeps these
records in a cache keyed by the oracle image, the CPU signature, the libc6
and valgrind packages and the measurement's inputs. Main's run saves every
record it made, failures included, so a pull request sees exactly the
outcome the old job inherited from main's artifact (GNU R fails on a few
benchmarks on every run: flexclust, flexclust_no_s4, volcano).

`fill` makes the records complete for the selected benchmarks. It keeps
every cached record, measures GNU R for the selected benchmarks that have
none, and measures once more those that then fail (a timeout must not
become the oracle), keeping the failure only if GNU R fails again, as
main's record would have it. It exits 1 when a selected benchmark is still
without a record. Then the base and the candidate are measured against
these records, so `wrong` is judged for each as before.
"""
import json
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import perf  # noqa: E402


def usable(rec):
    """A GNU R record that can judge an answer and give a ratio."""
    g = rec.get("gnur") if isinstance(rec, dict) else None
    return isinstance(g, dict) and "out" in g and "error" not in g and g.get("status", 0) == 0 and "instr" in g


def load(path):
    try:
        with open(path) as f:
            return json.load(f).get("results", {})
    except (OSError, ValueError):
        return {}


def save(path, results):
    with open(path, "w") as f:
        json.dump({"results": results}, f, indent=1, sort_keys=True)


def extract(perf_json, out):
    """Every GNU R record of a run, failed ones too."""
    keep = {name: {"gnur": r["gnur"]} for name, r in load(perf_json).items() if isinstance(r.get("gnur"), dict)}
    save(out, keep)
    print(f"perf-gnur: {len(keep)} GNU R records ({sum(1 for r in keep.values() if not usable(r))} failed)")
    return 0


def selected(rx=None):
    return [p[0] for p in perf.programs(perf.conf(), None, rx or None)]


def measure(names):
    """GNU R's records for `names`, measured here (perf.py run --runtime gnur)."""
    if not names:
        return {}
    rx = "^(" + "|".join(re.escape(n) for n in names) + ")$"
    with tempfile.TemporaryDirectory() as d:
        out = os.path.join(d, "gnur.json")
        subprocess.run([sys.executable, os.path.join(HERE, "perf.py"), "run", "--metric", "instr", "--runtime", "gnur",
                        "--jobs", str(os.cpu_count() or 1), "--quiet", "--filter", rx, "--out", out,
                        # (GNU R alone runs no robigo; perf.py still takes one)
                        *(["--robigo", "dist/robigo"] if os.path.exists("dist/robigo") else [])], check=True)
        return {n: {"gnur": r["gnur"]} for n, r in load(out).items() if isinstance(r.get("gnur"), dict)}


def fill(path, rx=None, measure=measure):
    have = load(path)
    want = selected(rx)
    missing = [n for n in want if n not in have]
    if missing:
        print(f"perf-gnur: measuring GNU R for {len(missing)} selected benchmark(s) without a record")
        have.update(measure(missing))
        again = [n for n in missing if n in have and not usable(have[n])]
        if again:
            print(f"perf-gnur: GNU R failed on {len(again)}; measuring them once more: {', '.join(again)}")
            for n, r in measure(again).items():
                if usable(r):
                    have[n] = r
    save(path, have)
    failed = [n for n in want if n in have and not usable(have[n])]
    if failed:
        print(f"perf-gnur: GNU R fails on {len(failed)} selected benchmark(s), as recorded: {', '.join(failed)}"
              " (their answers are not judged, as on main)")
    still = [n for n in want if n not in have]
    if still:
        print(f"perf-gnur: no GNU R record for {len(still)} selected benchmark(s): {', '.join(still[:5])}")
        return 1
    print(f"perf-gnur: GNU R records for all {len(want)} selected benchmark(s)")
    return 0


def main(argv):
    if len(argv) == 3 and argv[0] == "extract":
        return extract(argv[1], argv[2])
    if len(argv) in (2, 3) and argv[0] == "fill":
        return fill(argv[1], argv[2] if len(argv) == 3 else None)
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
