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
and valgrind packages and the measurement's inputs. Main's run saves its
usable records and the failures benchmarks/gnur-known-failures.txt names
(GNU R fails on flexclust, flexclust_no_s4 and volcano on every run, which
the old job inherited from main's artifact too); another failure is left
out, so a pull request measures that benchmark itself.

`fill` makes the records complete for the selected benchmarks. It keeps
every cached record (main's, its failures included), measures GNU R for the
selected benchmarks that have none, and measures once more those that then
fail. A failure measured here twice is accepted only for a benchmark that
benchmarks/gnur-known-failures.txt names: two failures on one runner do
not prove that main's record would have failed, and an unjudged answer is
a weaker gate. For any other benchmark `fill` exits 1, as it does when a
selected benchmark is still without a record. Then the base and the
candidate are measured against these records, so `wrong` is judged as
before.
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


KNOWN = os.path.join(os.path.dirname(HERE), "benchmarks", "gnur-known-failures.txt")


def known_failures(path=KNOWN):
    """The benchmarks GNU R fails on every main run."""
    try:
        with open(path) as f:
            return {line.split("#")[0].strip() for line in f} - {""}
    except OSError:
        return set()


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


def extract(perf_json, out, known=None):
    """A run's GNU R records for the cache: the usable ones, and the
    failures of the known-failure list. Another failure (a timeout on one
    main run, say) is left out, because a cache entry is immutable and
    would keep it for every later main run with the same key: a pull
    request then measures that benchmark itself, under fill's rule."""
    known = known_failures() if known is None else known
    recs = {name: {"gnur": r["gnur"]} for name, r in load(perf_json).items() if isinstance(r.get("gnur"), dict)}
    stamp(recs)
    failed = {n for n, r in recs.items() if not usable(r)}
    keep = {n: r for n, r in recs.items() if n not in failed or n in known}
    save(out, keep)
    print(f"perf-gnur: {len(keep)} GNU R records cached ({len(failed & known)} known failures;"
          f" {len(failed - known)} other failures left out: {', '.join(sorted(failed - known)) or 'none'})")
    if failed != known:  # (the list's own check, against what main fails now)
        print("::warning::GNU R's failures on main no longer match benchmarks/gnur-known-failures.txt: "
              f"failing but not listed {sorted(failed - known)}, listed but passing {sorted(known - failed)}")
    return 0


def stamp(recs, digests=None, code=None):
    """Mark each record with what it was measured from: its program's digest
    and the measuring code's (perf_suite.program_digests, code_digest)."""
    digests = perf.program_digests() if digests is None else digests
    code = perf.gnur_code_digest() if code is None else code
    for n, r in recs.items():
        r["program"] = digests.get(n)
        r["code"] = code
    return recs


def stale(have, want, digests, code, known=frozenset()):
    """{name: why} for the wanted benchmarks whose record cannot be used:
    none, a changed program, changed measuring code, or a failure that is no
    longer a known one. A record whose name matches but whose digests do
    not (one restored from an older cache entry, `restore-keys`) is
    measured again, never reused; so is a failure benchmarks/
    gnur-known-failures.txt no longer names, whose answer would otherwise
    go unjudged."""
    out = {}
    for n in want:
        r = have.get(n)
        if r is None:
            out[n] = "no record"
        elif not usable(r) and n not in known:
            out[n] = "failure no longer known"
        elif r.get("code") != code:
            out[n] = "changed measurement code"
        elif r.get("program") != digests.get(n):
            out[n] = "changed program"
    return out


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


def fill(path, rx=None, measure=measure, known=None, digests=None, code=None):
    known = known_failures() if known is None else known
    digests = perf.program_digests() if digests is None else digests
    code = perf.gnur_code_digest() if code is None else code
    have = load(path)
    want = selected(rx)
    why = stale(have, want, digests, code, known)
    for n in why:
        have.pop(n, None)
    missing = [n for n in want if n in why]
    counts = {k: sum(1 for v in why.values() if v == k)
              for k in ("changed program", "changed measurement code", "failure no longer known", "no record")}
    # (stdout: the job's step tees it into the summary)
    print(f"perf-gnur: {len(missing)} of {len(want)} selected benchmark(s) re-measured ("
          + ", ".join(f"{k} {v}" for k, v in counts.items()) + ")")
    fresh = []
    if missing:
        print(f"perf-gnur: measuring GNU R for {len(missing)} selected benchmark(s) without a usable record")
        have.update(stamp(measure(missing), digests, code))
        again = [n for n in missing if n in have and not usable(have[n])]
        if again:
            print(f"perf-gnur: GNU R failed on {len(again)}; measuring them once more: {', '.join(again)}")
            for n, r in stamp(measure(again), digests, code).items():
                if usable(r):
                    have[n] = r
        fresh = [n for n in missing if n in have and not usable(have[n]) and n not in known]
    save(path, have)
    failed = [n for n in want if n in have and not usable(have[n])]
    if failed:
        print(f"perf-gnur: GNU R fails on {len(failed)} selected benchmark(s), as recorded: {', '.join(failed)}"
              " (their answers are not judged, as on main)")
    if fresh:
        print(f"perf-gnur: GNU R failed twice here on {len(fresh)} benchmark(s) that main's records do not show"
              f" failing and benchmarks/gnur-known-failures.txt does not name: {', '.join(fresh)}."
              " Their answers would go unjudged: re-run the job (or, if GNU R now fails there for good, list them)")
        return 1
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
