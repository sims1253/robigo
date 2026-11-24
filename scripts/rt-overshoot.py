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
- benchmarks/rt/mem-report/*.R, report-only: each program's first line
  says which builtin family it measures and the PR expected to cure it
  (`# E4 report-only: FAMILY; cured by: PR`). Their overshoot is printed in
  a table of its own and never fails the step (one that gives no report,
  or times out, still does: a missing number is worse than a red step); a program moves to
  benchmarks/rt/mem/ when its cure lands, and that move is the cure's
  proof;
- every row of benchmarks/rt/calls-rows.toml, as the N loop
  scripts/idiom-vs-r.py runs (`SETUP; g <- function() { for (i in 1:N)
  { BODY }; NULL }; g()`).

Each runs under the default boot, but those BOOTS names, each with its
reason (printed with the table).

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
sys.path.insert(0, os.path.join(ROOT, "scripts"))
from perf import rows_with_setups  # noqa: E402
BOUND_FLOOR = 32 << 20
REPORT_DIR = os.path.join("benchmarks", "rt", "mem-report")
REPORT_HEADER = re.compile(r"^# E4 report-only: (.+?); cured by: (.+)$")

COLLECTIONS = re.compile(r"^robigo gc: collections (\d+)", re.M)
OVERSHOOT = re.compile(
    r"^robigo gc: overshoot max (\d+) bytes \(live (\d+) bytes after it\), total (\d+) bytes, "
    r"over max\(32 MiB, 10% live\) (\d+)$",
    re.M,
)
DUE_AT_EXIT = re.compile(
    r"^robigo gc: due at exit: (\d+) bytes since \(live (\d+) bytes\), over max\(32 MiB, 10% live\) (\d)$", re.M
)


# The programs that run under a boot other than the default, each with the
# boot and why. Every other program boots the default (rdb) and is held to
# the bound there; a named one must still pass under its boot. INTERIM(L5):
# an entry goes when its cure lands, and M1's loader cannot be deleted while
# one names it (docs/ledger/g3/2026-11-23-l5-flip.md, the deletion list).
# None today: benchmarks/rt/mem/strings1e6.R, the one entry the flip made
# (paste0's loop had no safepoint, so under rdb it overshot 34.1 MB past a
# 33.55 MB bound), runs under rdb since paste collects between its elements
# (Cx::fill_strings, B's paste poll): 48 bytes.
BOOTS = {}


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


def report_header(path):
    """(family, cure) from a report-only program's first line, else None."""
    with open(os.path.join(ROOT, path)) as f:
        m = REPORT_HEADER.match(f.readline().rstrip("\n"))
    return (m.group(1), m.group(2)) if m else None


def report_only():
    """{name: (family, cure)} of benchmarks/rt/mem-report's programs; a
    program there without the header is an error."""
    d = os.path.join(ROOT, REPORT_DIR)
    out = {}
    for f in sorted(os.listdir(d)) if os.path.isdir(d) else []:
        if f.endswith(".R"):
            p = os.path.join(REPORT_DIR, f)
            h = report_header(p)
            if h is None:
                sys.exit(f"rt-overshoot: {p} has no '# E4 report-only: FAMILY; cured by: PR' first line")
            out[p] = h
    return out


def verdict(res, report):
    """The step's exit status: 1 if a gated program passed the bound or
    any program gave no report (a report-only one too); a report-only
    program's overshoot never fails it."""
    over = [r for r in res if "error" not in r and r["over"] and r["program"] not in report]
    errors = [r for r in res if "error" in r]
    return 1 if over or errors else 0


def programs(tmp, report=None):
    """(name, path) of every program to run; `report` is report_only()'s
    snapshot (read here when not given)."""
    progs = [(p, p) for p in ratcheted("rt") + ratcheted("r")]
    for f in ["benchmarks/suite/calls.R", "benchmarks/suite/fib.R", "benchmarks/suite/binarytrees.R"]:
        progs.append((f, f))
    mem = os.path.join(ROOT, "benchmarks", "rt", "mem")
    progs += [(os.path.join("benchmarks/rt/mem", f), os.path.join("benchmarks/rt/mem", f))
              for f in sorted(os.listdir(mem)) if f.endswith(".R")]
    progs += [(p, p) for p in (report_only() if report is None else report)]
    with open(os.path.join(ROOT, "benchmarks", "rt", "calls-rows.toml"), "rb") as f:
        rows = rows_with_setups(tomllib.load(f))
    # (the jit rows compile through the compiler namespace, which robigo-rt
    # cannot load until V4 step 3: run them here from then on)
    rows = [r for r in rows if r.get("group") != "jit"]
    for r in rows:
        path = os.path.join(tmp, f"row-{r['name']}.R")
        with open(path, "w") as f:
            f.write(f"{r.get('setup', '')}\ng <- function() {{ for (i in 1:{r['n']}) {{ {r['body']} }}; NULL }}\ng()\n")
        progs.append((f"calls-rows:{r['name']}", path))
    return progs


def run(bin_, name, path, profile, libs):
    env = dict(os.environ, ROBIGO_GC_STATS="1", ROBIGO_RT_PROFILE=profile, RT_CAPI_LIBS=libs,
               R_DEFAULT_PACKAGES="NULL", LANGUAGE="en", LC_ALL="C.UTF-8", TZ="UTC")
    boot = BOOTS.get(name, (None, ""))[0]
    if boot:
        env["ROBIGO_RT_BOOT"] = boot
    try:
        p = subprocess.run([bin_, path], cwd=ROOT, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                           timeout=1800)
        err = p.stderr.decode("utf-8", "replace")
    except subprocess.TimeoutExpired:
        return {"program": name, "profile": profile, "boot": boot, "error": "timeout"}
    m = OVERSHOOT.search(err)
    c = COLLECTIONS.search(err)
    if not m or not c:
        return {"program": name, "profile": profile, "boot": boot, "error": "no report (exit %d)" % p.returncode}
    r = {"program": name, "profile": profile, "boot": boot, "exit": p.returncode, "collections": int(c.group(1)), "max": int(m.group(1)),
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
    # one snapshot of the report-only programs, for the run and the verdict
    report = report_only()
    with tempfile.TemporaryDirectory() as tmp:
        libs = os.path.join(tmp, "libs")
        subprocess.run([os.path.join(ROOT, "scripts", "rt-capi-build.sh"), libs], check=True)
        progs = programs(tmp, report)
        jobs = [(n, p, prof) for n, p in progs for prof in a.profiles.split(",")]
        with ThreadPoolExecutor(a.jobs) as ex:
            res = list(ex.map(lambda j: run(bin_, j[0], j[1], j[2], libs), jobs))
    errors = [r for r in res if "error" in r]
    ok = [r for r in res if "error" not in r]
    ok.sort(key=lambda r: -r["ratio"])
    reported = [r for r in ok if r["program"] in report]
    ok = [r for r in ok if r["program"] not in report]
    print(f"rt-overshoot: {bin_}, {len(progs)} programs x {len(a.profiles.split(','))} profiles")
    print()
    print("| program | profile | boot | collections | max overshoot | live after | bound | max / bound | over |")
    print("|---|---|---|---:|---:|---:|---:|---:|---:|")
    # (the worst, and every program BOOTS names, wherever it ranks)
    for r in [r for i, r in enumerate(ok) if i < a.top or r["boot"]]:
        print(f"| {r['program']} | {r['profile']} | {r['boot'] or 'default'} | {r['collections']} | {r['max']:,} | "
              f"{r['live']:,} | {bound(r['live']):,} | {r['ratio']:.4f} | {r['over']} |")
    for name, (boot, why) in sorted(BOOTS.items()):
        print(f"boot {boot}: {name}: {why} (INTERIM(L5))")
    over = [r for r in ok if r["over"]]
    due = [r for r in ok if r["exit_due"]]
    print()
    print(f"runs: {len(res)}; gated runs with a report: {len(ok)}; over the bound: {len(over)}; "
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
    if reported:
        print()
        print("report-only (benchmarks/rt/mem-report: its overshoot never fails the step; no report does)")
        print()
        print("| program | profile | family | cured by | collections | max overshoot | bound | max / bound |")
        print("|---|---|---|---|---:|---:|---:|---:|")
        for r in reported:
            fam, cure = report[r["program"]]
            print(f"| {r['program']} | {r['profile']} | {fam} | {cure} | {r['collections']} | {r['max']:,} | "
                  f"{bound(r['live']):,} | {r['ratio']:.4f} |")
        print()
        print(f"report-only runs: {len(reported)}; over the bound: {sum(1 for r in reported if r['over'])}")
    if a.json:
        with open(a.json, "w") as f:
            json.dump({"bin": bin_, "runs": res}, f, indent=1)
    return verdict(res, report)


if __name__ == "__main__":
    sys.exit(main())
