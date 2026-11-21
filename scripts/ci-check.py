#!/usr/bin/env python3
"""The `Check` job of .github/workflows/ci.yml: pass only when every job passed
or was skipped for a reason `select` gave (docs/verification.md, CI).

    NEEDS='${{ toJSON(needs) }}' CANCELLED='${{ cancelled() }}' scripts/ci-check.py [--pull-request]

Branch protection requires `Check` alone, so splitting or adding jobs needs
no ruleset change. A failed or cancelled job fails it, and so does a skip
nobody asked for: GitHub counts a skipped required check as passing. So
does a cancelled run (CANCELLED=true), even when every job it waited for
had passed: the run was cancelled before it ended.
"""
import json
import os
import sys

# Skipped when `select` reports heavy=false: the change cannot affect the binary.
HEAVY = {"build", "release-tests", "test", "counters", "corpora", "fuzz", "cran", "smoke", "perf"}


# Skipped on a stacked pull request (select's `stacked`), beside HEAVY and
# the package gate: only select, lint and rt (with rdb-data) run there.
STACKED_SKIP = {"miri", "bytecode-fidelity", "base-subset", "perf-rt-build", "perf-rt", "rt-corpora"}


def allowed_skip(job, sel, pull_request):
    if sel.get("stacked") == "true" and (job in HEAVY or job in STACKED_SKIP or job == "package-gate"):
        return True
    if job in HEAVY:
        return sel.get("heavy") == "false"
    if job in ("rt", "rt-gc-move", "rt-feature-builds", "rdb-data", "builtin-diff-data", "base-subset",
               "perf-rt-build", "perf-rt", "rt-corpora"):
        # G3's runtime crates (with L1's rdb data and A4's GNU R records for
        # their tests), robigo-rt's base subset, its M1 bars and its corpus
        # column: skipped only when nothing but docs changed
        return sel.get("rt") == "false"
    if job == "package-gate":
        return not pull_request or sel.get("runtime") == "false"
    if job == "perf-base":
        # perf's base build: pull requests with the heavy jobs only
        return not pull_request or sel.get("heavy") == "false"
    return False


def verdicts(needs, pull_request):
    """(job, result, ok) for every job in `needs`."""
    sel = (needs.get("select") or {}).get("outputs") or {}
    rows = []
    for job, r in sorted(needs.items()):
        result = r.get("result")
        ok = result == "success" or (result == "skipped" and allowed_skip(job, sel, pull_request))
        rows.append((job, result, ok))
    return rows


def main(argv):
    pull_request = "--pull-request" in argv
    if os.environ.get("CANCELLED") == "true":
        # (superseded by a newer push, or cancelled by hand: either way not a
        # run that may pass, whatever its jobs' results)
        print("::error::the run was cancelled")
        return 1
    rows = verdicts(json.loads(os.environ["NEEDS"]), pull_request)
    for job, result, ok in rows:
        print(f"{'ok ' if ok else 'BAD'} {job}: {result}")
    bad = [job for job, _, ok in rows if not ok]
    if bad:
        print(f"::error::not passed: {', '.join(bad)}")
        return 1
    sel = (json.loads(os.environ["NEEDS"]).get("select") or {}).get("outputs") or {}
    if sel.get("stacked") == "true":
        # a retarget starts no run: green here would let it merge into main untested
        print("::error::stacked pull request: the heavy jobs did not run. Retarget it to main, then push "
              "(or close and reopen it) for the full run")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
