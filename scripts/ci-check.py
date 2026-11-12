#!/usr/bin/env python3
"""The `Check` job of .github/workflows/ci.yml: pass only when every job passed
or was skipped for a reason `select` gave (docs/verification.md, CI).

    NEEDS='${{ toJSON(needs) }}' scripts/ci-check.py [--pull-request]

Branch protection requires `Check` alone, so splitting or adding jobs needs
no ruleset change. A failed or cancelled job fails it, and so does a skip
nobody asked for: GitHub counts a skipped required check as passing.
"""
import json
import os
import sys

# Skipped when `select` reports heavy=false: the change cannot affect the binary.
HEAVY = {"build", "release-tests", "test", "corpora", "fuzz", "cran", "smoke", "perf"}


def allowed_skip(job, sel, pull_request):
    if job in HEAVY:
        return sel.get("heavy") == "false"
    if job == "package-gate":
        return not pull_request or sel.get("runtime") == "false"
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
    rows = verdicts(json.loads(os.environ["NEEDS"]), pull_request)
    for job, result, ok in rows:
        print(f"{'ok ' if ok else 'BAD'} {job}: {result}")
    bad = [job for job, _, ok in rows if not ok]
    if bad:
        print(f"::error::not passed: {', '.join(bad)}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
