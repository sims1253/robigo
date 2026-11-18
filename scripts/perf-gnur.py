#!/usr/bin/env python3
"""GNU R's side of the perf gate, kept apart from robigo's numbers so that
a pull request's `perf` job need not measure it again.

    scripts/perf-gnur.py extract PERF_JSON OUT     # main: GNU R's records of a run
    scripts/perf-gnur.py covers GNUR_JSON [FILTER] # exit 0 when every selected
                                                   # benchmark has a usable record

GNU R's records give a pull request two things: the answer robigo's must
equal (`out` and `status`, which decide `wrong`), and the instruction
counts its ratio is reported against. The answer does not depend on the
CPU, but the counts do (glibc picks its routines by CPU), so CI keeps these
records in a cache keyed by the oracle image, the CPU signature, the libc6
and valgrind packages and the measurement's inputs. Main's run saves it, a
pull request restores it, and on a miss, or when the records do not cover
every benchmark it selected, the pull request measures GNU R itself.
"""
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import perf  # noqa: E402


def usable(rec):
    g = rec.get("gnur") if isinstance(rec, dict) else None
    return isinstance(g, dict) and "out" in g and "error" not in g and g.get("status", 0) == 0 and "instr" in g


def extract(perf_json, out):
    with open(perf_json) as f:
        doc = json.load(f)
    keep = {name: {"gnur": r["gnur"]} for name, r in doc.get("results", {}).items() if usable(r)}
    with open(out, "w") as f:
        json.dump({"results": keep}, f, indent=1, sort_keys=True)
    print(f"perf-gnur: {len(keep)} GNU R records")
    return 0


def missing(gnur_json, rx=None):
    """The selected benchmarks without a usable GNU R record."""
    try:
        with open(gnur_json) as f:
            have = json.load(f).get("results", {})
    except (OSError, ValueError):
        have = {}
    return [p[0] for p in perf.programs(perf.conf(), None, rx or None) if not usable(have.get(p[0], {}))]


def main(argv):
    if len(argv) == 3 and argv[0] == "extract":
        return extract(argv[1], argv[2])
    if len(argv) in (2, 3) and argv[0] == "covers":
        miss = missing(argv[1], argv[2] if len(argv) == 3 else None)
        if miss:
            print(f"perf-gnur: no GNU R record for {len(miss)} selected benchmark(s), e.g. {', '.join(miss[:3])}")
            return 1
        print("perf-gnur: GNU R's records cover every selected benchmark")
        return 0
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
