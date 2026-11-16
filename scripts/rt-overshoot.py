#!/usr/bin/env python3
"""E4's overshoot (docs/g3.md §15.3) on every slice robigo-rt runs at M1.

    scripts/rt-overshoot.py [--profiles eval,bc] [--json OUT] [--bin BIN] [--jobs J]

Runs each program under `ROBIGO_GC_STATS=1`, once per profile
(`ROBIGO_RT_PROFILE`), and reads the heap's report on stderr: the largest
overshoot (bytes allocated from the allocation that made a collection due
to the collection that served it), how many overshoots passed max(32 MiB,
10% of the live heap after that collection), and a collection still due at
exit with the bytes allocated since. The programs:

- every ratcheted case: tests/rt (tests/rt/passing-rt) and the tests/r
  cases robigo-rt ratchets (tests/r/passing-rt), their package C from
  scripts/rt-capi-build.sh;
- benchmarks/suite/{calls,fib,binarytrees}.R and benchmarks/rt/mem/*.R;
- every row of benchmarks/rt/calls-rows.toml, as the N loop
  scripts/idiom-vs-r.py runs (`SETUP; g <- function() { for (i in 1:N)
  { BODY }; NULL }; g()`).

Prints a table of the worst programs and a summary; exits 1 if any
overshoot passes the bound. A program's exit status does not matter (a
case may end in an error): only the report does, and a missing report
fails. BIN defaults to $ROBIGO_RT_BIN, then the target's release build.
"""
import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
import tomllib
from concurrent.futures import ThreadPoolExecutor

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BOUND_FLOOR = 32 << 20

COLLECTIONS = re.compile(r"^robigo gc: collections (\d+)", re.M)
OVERSHOOT = re.compile(
    r"^robigo gc: overshoot max (\d+) bytes \(live (\d+) bytes after it\), total (\d+) bytes, "
    r"over max\(32 MiB, 10% live\) (\d+)$",
    re.M,
)
DUE_AT_EXIT = re.compile(
    r"^robigo gc: due at exit: (\d+) bytes since \(live (\d+) bytes\), over max\(32 MiB, 10% live\) (\d)$", re.M
)


def bound(live):
    return max(BOUND_FLOOR, live // 10)


def default_bin():
    b = os.environ.get("ROBIGO_RT_BIN")
    if b:
        return b
    target = os.environ.get("CARGO_TARGET_DIR")
    if not target:
        meta = subprocess.run(["cargo", "metadata", "--format-version", "1", "--no-deps"], cwd=ROOT,
                              stdout=subprocess.PIPE, text=True, check=True)
        target = json.loads(meta.stdout)["target_directory"]
    return os.path.join(target, "release", "robigo-rt")


def ratcheted(suite):
    """The cases of tests/<suite> with a marker in tests/<suite>/passing-rt."""
    marks = os.path.join(ROOT, "tests", suite, "passing-rt")
    out = []
    for d, _, files in os.walk(marks):
        for f in files:
            if f.endswith(".R.pass"):
                rel = os.path.relpath(os.path.join(d, f[: -len(".pass")]), marks)
                out.append(os.path.join("tests", suite, rel))
    return sorted(out)


def programs(tmp):
    """(name, path) of every program to run."""
    progs = [(p, p) for p in ratcheted("rt") + ratcheted("r")]
    for f in ["benchmarks/suite/calls.R", "benchmarks/suite/fib.R", "benchmarks/suite/binarytrees.R"]:
        progs.append((f, f))
    mem = os.path.join(ROOT, "benchmarks", "rt", "mem")
    progs += [(os.path.join("benchmarks/rt/mem", f), os.path.join("benchmarks/rt/mem", f))
              for f in sorted(os.listdir(mem)) if f.endswith(".R")]
    with open(os.path.join(ROOT, "benchmarks", "rt", "calls-rows.toml"), "rb") as f:
        rows = tomllib.load(f)["row"]
    for r in rows:
        path = os.path.join(tmp, f"row-{r['name']}.R")
        with open(path, "w") as f:
            f.write(f"{r.get('setup', '')}\ng <- function() {{ for (i in 1:{r['n']}) {{ {r['body']} }}; NULL }}\ng()\n")
        progs.append((f"calls-rows:{r['name']}", path))
    return progs


def run(bin_, name, path, profile, libs):
    env = dict(os.environ, ROBIGO_GC_STATS="1", ROBIGO_RT_PROFILE=profile, RT_CAPI_LIBS=libs,
               R_DEFAULT_PACKAGES="NULL", LANGUAGE="en", LC_ALL="C.UTF-8", TZ="UTC")
    try:
        p = subprocess.run([bin_, path], cwd=ROOT, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                           timeout=1800)
        err = p.stderr.decode("utf-8", "replace")
    except subprocess.TimeoutExpired:
        return {"program": name, "profile": profile, "error": "timeout"}
    m = OVERSHOOT.search(err)
    c = COLLECTIONS.search(err)
    if not m or not c:
        return {"program": name, "profile": profile, "error": "no report (exit %d)" % p.returncode}
    r = {"program": name, "profile": profile, "exit": p.returncode, "collections": int(c.group(1)), "max": int(m.group(1)),
         "live": int(m.group(2)), "total": int(m.group(3)), "over": int(m.group(4)), "exit_due": None}
    d = DUE_AT_EXIT.search(err)
    if d:
        r["exit_due"] = {"since": int(d.group(1)), "live": int(d.group(2)), "over": int(d.group(3))}
        r["over"] += int(d.group(3))
    worst = r["max"] / bound(r["live"])
    if r["exit_due"]:
        worst = max(worst, r["exit_due"]["since"] / bound(r["exit_due"]["live"]))
    r["ratio"] = worst
    return r


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--bin", default=None)
    ap.add_argument("--profiles", default="eval,bc")
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--json")
    ap.add_argument("--top", type=int, default=15)
    a = ap.parse_args()
    bin_ = a.bin or default_bin()
    if not os.access(bin_, os.X_OK):
        sys.exit(f"rt-overshoot: no robigo-rt at {bin_}")
    with tempfile.TemporaryDirectory() as tmp:
        libs = os.path.join(tmp, "libs")
        subprocess.run([os.path.join(ROOT, "scripts", "rt-capi-build.sh"), libs], check=True)
        progs = programs(tmp)
        jobs = [(n, p, prof) for n, p in progs for prof in a.profiles.split(",")]
        with ThreadPoolExecutor(a.jobs) as ex:
            res = list(ex.map(lambda j: run(bin_, j[0], j[1], j[2], libs), jobs))
    errors = [r for r in res if "error" in r]
    ok = [r for r in res if "error" not in r]
    ok.sort(key=lambda r: -r["ratio"])
    print(f"rt-overshoot: {bin_}, {len(progs)} programs x {len(a.profiles.split(','))} profiles")
    print()
    print("| program | profile | collections | max overshoot | live after | bound | max / bound | over |")
    print("|---|---|---:|---:|---:|---:|---:|---:|")
    for r in ok[: a.top]:
        print(f"| {r['program']} | {r['profile']} | {r['collections']} | {r['max']:,} | {r['live']:,} | "
              f"{bound(r['live']):,} | {r['ratio']:.4f} | {r['over']} |")
    over = [r for r in ok if r["over"]]
    due = [r for r in ok if r["exit_due"]]
    print()
    print(f"runs: {len(res)}; with a report: {len(ok)}; over the bound: {len(over)}; "
          f"a collection due at exit: {len(due)}; largest max/bound: {ok[0]['ratio'] if ok else 0:.4f}")
    for r in errors:
        print(f"no result: {r['program']} ({r['profile']}): {r['error']}")
    # (a case may end in an error by design; a benchmark that does measured
    # less than it was meant to)
    for r in ok:
        if r["exit"] != 0 and not r["program"].startswith("tests/"):
            print(f"exit status {r['exit']}: {r['program']} ({r['profile']}): its overshoot covers what ran")
    for r in over:
        print(f"OVER: {r['program']} ({r['profile']}): max {r['max']:,} bytes, live {r['live']:,}, "
              f"exit {r['exit_due']}")
    if a.json:
        with open(a.json, "w") as f:
            json.dump({"bin": bin_, "runs": res}, f, indent=1)
    return 1 if over or errors else 0


if __name__ == "__main__":
    sys.exit(main())
