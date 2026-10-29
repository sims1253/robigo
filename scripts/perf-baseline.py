#!/usr/bin/env python3
"""Download the perf baseline of a pull request: main's `perf-main` artifact
for the commit the PR's merge commit is built on.

    scripts/perf-baseline.py BASE_SHA OUTDIR [--wait-minutes N]

A pull request's CI runs on GitHub's test merge of the PR into main; its
first parent BASE_SHA is the main commit whose numbers the PR must be
compared with. "The latest successful main run" is not that commit when
main moved after the merge was made, or when that main run has not finished
yet: then the PR is blamed (or credited) for what main itself changed.

If BASE_SHA's main run is still going, wait for its `perf-main` artifact
(the perf job uploads it before the whole run ends). If BASE_SHA has no main
run with one (it failed, or main moved by a push that skipped CI), fall back
to the nearest ancestor that has one and say so. Prints the commit used;
writes it to OUTDIR/BASELINE. Needs `gh` with GH_TOKEN and GITHUB_REPOSITORY.
"""
import argparse
import json
import os
import subprocess
import sys
import time

REPO = os.environ.get("GITHUB_REPOSITORY", "sims1253/robigo")
WORKFLOW = ".github/workflows/ci.yml"


def gh(*args):
    return json.loads(subprocess.run(["gh", "api", *args], capture_output=True, text=True, check=True).stdout)


def main_runs(sha):
    """CI runs of main for `sha`, newest first."""
    runs = gh(f"repos/{REPO}/actions/runs?head_sha={sha}&branch=main&event=push&per_page=20")["workflow_runs"]
    return [r for r in runs if r.get("path") == WORKFLOW]


def has_perf_main(run_id):
    arts = gh(f"repos/{REPO}/actions/runs/{run_id}/artifacts?per_page=100")["artifacts"]
    return any(a["name"] == "perf-main" and not a.get("expired") for a in arts)


def find(sha, wait_s):
    """The id of a main run of `sha` with a perf-main artifact (waiting while
    one is in progress), or None."""
    deadline = time.time() + wait_s
    while True:
        runs = main_runs(sha)
        for r in runs:
            if has_perf_main(r["id"]):
                return r["id"]
        if not any(r["status"] != "completed" for r in runs) or time.time() > deadline:
            return None
        print(f"waiting for main's run of {sha[:7]} to publish perf-main ...", flush=True)
        time.sleep(60)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("sha")
    ap.add_argument("outdir")
    ap.add_argument("--wait-minutes", type=float, default=50)
    ap.add_argument("--ancestors", type=int, default=30, help="how far back to fall back")
    a = ap.parse_args()
    used, note = a.sha, ""
    run = find(a.sha, a.wait_minutes * 60)
    if run is None:
        # Nearest ancestor on main with a baseline (first parents only).
        parent = a.sha
        for _ in range(a.ancestors):
            parents = gh(f"repos/{REPO}/commits/{parent}")["parents"]
            if not parents:
                break
            parent = parents[0]["sha"]
            run = find(parent, 0)
            if run is not None:
                used = parent
                note = f" (no perf-main for {a.sha[:7]}: compared with its ancestor {parent[:7]}," \
                       " so main's own changes in between are included)"
                break
    if run is None:
        print(f"no baseline on main for {a.sha[:7]} or its ancestors")
        return 0
    os.makedirs(a.outdir, exist_ok=True)
    subprocess.run(["gh", "run", "download", str(run), "-R", REPO, "-n", "perf-main", "-D", a.outdir], check=True)
    with open(os.path.join(a.outdir, "BASELINE"), "w") as f:
        f.write(used + "\n")
    print(f"baseline: main {used[:7]} (run {run}){note}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
