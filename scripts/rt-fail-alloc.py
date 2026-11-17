#!/usr/bin/env python3
"""E9's injected allocation failure (docs/g3.md §15.3, §3 item 5) on robigo-rt.

    scripts/rt-fail-alloc.py [--points K] [--jobs J] [--bin BIN] [--slice] [SCRIPT.R ...]

For each script: one run without injection gives the output (stdout,
stderr, exit status) and the number of allocations the script makes after
base is loaded (the heap's `ROBIGO_GC_STATS` report); then one run per
point n with `ROBIGO_FAIL_ALLOC_AT=n`, which refuses that allocation once.
The code that made it retries (a collection, then the same request), so the
output must be the uninjected run's at every point: an effect committed
before the refusal and done again (a line printed twice, a warning
repeated), or lost, changes it, and so does a refusal some code does not
retry (an error, a panic). Every point is tried, or K points spread evenly
over the allocations with `--points K`. `--slice` adds every ratcheted
tests/rt case. Exits 1 on any point whose output differs.

BIN defaults to $ROBIGO_RT_BIN, then the target's release build.
"""
import argparse
import os
import re
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts"))
COUNT = re.compile(r"^robigo gc: allocations since ROBIGO_FAIL_ALLOC_AT was armed (\d+)", re.M)
DEFAULT = ["tests/rt/alloc/effects.R", "tests/rt/serialize-closures/unserialize-e9.R"]


def default_bin():
    import importlib.util

    spec = importlib.util.spec_from_file_location("ov", os.path.join(ROOT, "scripts", "rt-overshoot.py"))
    ov = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(ov)
    return ov.default_bin(), ov.ratcheted


def run(bin_, script, env_extra, libs):
    env = dict(os.environ, RT_CAPI_LIBS=libs, R_DEFAULT_PACKAGES="NULL", LANGUAGE="en", LC_ALL="C.UTF-8",
               TZ="UTC", **env_extra)
    env.pop("ROBIGO_GC_STATS", None) if "ROBIGO_GC_STATS" not in env_extra else None
    try:
        p = subprocess.run([bin_, script], cwd=ROOT, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                           timeout=600)
        return p.stdout, p.stderr, p.returncode
    except subprocess.TimeoutExpired:
        return b"", b"timeout", -999


def strip_stats(err):
    return b"".join(l for l in err.splitlines(keepends=True) if not l.startswith(b"robigo gc: "))


def sweep(bin_, script, points, jobs, libs):
    # the allocation count, with the knob armed past the end (it forces the
    # slow path as an injected run does, and refuses nothing)
    _, err, _ = run(bin_, script, {"ROBIGO_FAIL_ALLOC_AT": str(1 << 62), "ROBIGO_GC_STATS": "1"}, libs)
    m = COUNT.search(err.decode("utf-8", "replace"))
    if not m:
        return script, 0, 0, [("count", "no allocation count in the report")]
    total = int(m.group(1))
    want = run(bin_, script, {}, libs)
    if points and points < total:
        ns = sorted({1 + (k * (total - 1)) // max(points - 1, 1) for k in range(points)})
    else:
        ns = list(range(1, total + 1))

    def one(n):
        got = run(bin_, script, {"ROBIGO_FAIL_ALLOC_AT": str(n)}, libs)
        if got == want:
            return None
        what = []
        if got[2] != want[2]:
            what.append(f"exit {got[2]} (not {want[2]})")
        if got[0] != want[0]:
            what.append("stdout differs")
        if got[1] != want[1]:
            what.append("stderr: " + got[1].decode("utf-8", "replace").strip().splitlines()[-1][:200]
                        if got[1].strip() else "stderr differs")
        return n, "; ".join(what)

    with ThreadPoolExecutor(jobs) as ex:
        bad = [r for r in ex.map(one, ns) if r]
    return script, total, len(ns), bad


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("scripts", nargs="*")
    ap.add_argument("--bin")
    ap.add_argument("--points", type=int, default=0, help="points per script (0: every allocation)")
    ap.add_argument("--jobs", type=int, default=6)
    ap.add_argument("--slice", action="store_true", help="add every ratcheted tests/rt case")
    a = ap.parse_args()
    bin_default, ratcheted = default_bin()
    bin_ = a.bin or bin_default
    scripts = a.scripts or ([] if a.slice else DEFAULT)
    if a.slice:
        scripts += [s for s in ratcheted("rt") if s not in scripts]
    with tempfile.TemporaryDirectory() as tmp:
        libs = os.path.join(tmp, "libs")
        subprocess.run([os.path.join(ROOT, "scripts", "rt-capi-build.sh"), libs], check=True)
        print(f"rt-fail-alloc: {bin_}, {len(scripts)} scripts, "
              f"{'every allocation' if not a.points else f'{a.points} points each'}")
        failed = 0
        tried = 0
        for s in scripts:
            script, total, n, bad = sweep(bin_, s, a.points, a.jobs, libs)
            tried += n
            status = "ok" if not bad else f"{len(bad)} points differ"
            print(f"{script}: {total} allocations, {n} points tried: {status}")
            for p, what in bad[:10]:
                print(f"  ROBIGO_FAIL_ALLOC_AT={p}: {what}")
            failed += bool(bad)
    print(f"rt-fail-alloc: {tried} injected runs over {len(scripts)} scripts; {failed} scripts with a difference")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
