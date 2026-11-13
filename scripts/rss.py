#!/usr/bin/env python3
"""Peak resident set size of R programs: robigo against GNU R.

    scripts/rss.py [--runs K] [--robigo BIN] [--robigo-env K=V ...] [--no-r] [--json OUT] FILE.R ...

Runs each FILE as `BIN --vanilla FILE` and as GNU R's `exec/R --vanilla
--no-echo -f FILE`, K times each (default 5), with R_DEFAULT_PACKAGES=NULL
and the oracle locale, and reports the median peak RSS in KiB with the
range. The peak is the child's `ru_maxrss`, read from `wait4` on that one
child: getrusage(RUSAGE_CHILDREN) per process, without other children
mixing in. Both programs must exit 0 and print the same output; a
difference is reported as `wrong`.

The G3 memory benchmarks (docs/ledger/g3.md, E2 and E4) are
benchmarks/suite/binarytrees.R and benchmarks/rt/mem/*.R.

Environment as scripts/idiom-vs-r.py: ROBIGO (or --robigo), RSCRIPT_BIN
(GNU R's exec/R), R_HOME_DIR.
"""
import argparse
import json
import os
import statistics
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from corpus import robigo_path  # noqa: E402
from perf import BASE_ENV  # noqa: E402


def peak(cmd, env):
    """(peak RSS in KiB, exit status, stdout) of one run."""
    p = subprocess.Popen(cmd, env=env, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    out = p.stdout.read()
    p.stdout.close()
    _, status, ru = os.wait4(p.pid, 0)
    p.returncode = os.waitstatus_to_exitcode(status)
    return ru.ru_maxrss, p.returncode, out.decode("utf-8", "replace")


def measure(cmd, env, runs):
    kib, outs = [], set()
    for _ in range(runs):
        k, status, out = peak(cmd, env)
        if status:
            raise SystemExit(f"{' '.join(cmd)} exited {status}")
        kib.append(k)
        outs.add(out)
    return {"median_kib": int(statistics.median(kib)), "min_kib": min(kib), "max_kib": max(kib),
            "out": outs.pop() if len(outs) == 1 else None}



def env_pair(kv):
    """argparse type for K=V: a value without '=' is an error, not a crash later."""
    k, sep, v = kv.partition("=")
    if not sep or not k:
        raise argparse.ArgumentTypeError(f"expected K=V, got {kv!r}")
    return kv


def positive_int(s):
    n = int(s)
    if n < 1:
        raise argparse.ArgumentTypeError(f"must be at least 1, got {n}")
    return n

def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("files", nargs="+")
    ap.add_argument("--runs", type=positive_int, default=5)
    ap.add_argument("--robigo", help="the robigo binary (default as scripts/corpus.py chooses it)")
    ap.add_argument("--robigo-env", action="append", default=[], metavar="K=V", type=env_pair,
                    help="environment for robigo's runs only, e.g. ROBIGO_JIT=0")
    ap.add_argument("--no-r", action="store_true", help="measure robigo only")
    ap.add_argument("--json", help="write the results here")
    a = ap.parse_args()

    robigo = robigo_path(a.robigo)
    r_bin = os.environ.get("RSCRIPT_BIN", "/usr/lib/R/bin/exec/R")
    r_home = os.environ.get("R_HOME_DIR", "/usr/lib/R")
    base = dict(os.environ, **BASE_ENV, R_DEFAULT_PACKAGES="NULL")
    renv = dict(base, **dict(kv.split("=", 1) for kv in a.robigo_env))
    genv = dict(base, R_HOME=r_home, LD_LIBRARY_PATH=os.pathsep.join(
        [os.path.join(r_home, "lib"), os.environ.get("LD_LIBRARY_PATH", "")]).rstrip(os.pathsep))

    results = {}
    print("| program | robigo KiB (range) | GNU R KiB (range) | robigo÷GNU R |")
    print("|---|---:|---:|---:|")
    for f in a.files:
        r = measure([robigo, "--vanilla", f], renv, a.runs)
        g = None if a.no_r else measure([r_bin, "--vanilla", "--no-echo", "-f", f], genv, a.runs)
        wrong = g is not None and (r["out"] is None or r["out"] != g["out"])
        results[f] = {"robigo": r, "gnur": g, "wrong": wrong}

        def cell(m):
            return "-" if m is None else f"{m['median_kib']:,} ({m['min_kib']:,}-{m['max_kib']:,})"
        ratio = "-" if g is None else f"{r['median_kib'] / g['median_kib']:.2f}"
        print(f"| {os.path.relpath(f)} | {cell(r)} | {cell(g)} | {ratio}{' **wrong**' if wrong else ''} |",
              flush=True)
    if a.json:
        meta = {"robigo": robigo, "robigo_env": a.robigo_env, "gnur": None if a.no_r else r_bin, "runs": a.runs}
        with open(a.json, "w") as fh:
            json.dump({"meta": meta, "results": results}, fh, indent=1, sort_keys=True)
    return 1 if any(v["wrong"] for v in results.values()) else 0


if __name__ == "__main__":
    sys.exit(main())
