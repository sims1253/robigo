#!/usr/bin/env python3
"""Benchmarks: robigo against GNU R 4.5.0.

    scripts/perf.py list
    scripts/perf.py fetch                         # external suites, pinned
    scripts/perf.py run [--metric instr|time|both] [--runtime robigo|gnur|both]
                        [--suite S] [--filter RE] [--jobs N] [--out FILE]
    scripts/perf.py compare BASE.json NEW.json [--threshold PCT]
    scripts/perf.py recheck BASE.json NEW.json [--robigo BIN]   # compare, re-measuring what is slower
    scripts/perf.py report FILE.json [--markdown]

Suites are listed in benchmarks/benchmarks.toml. Every program runs in the
oracle environment (C.UTF-8, TZ=UTC, R_DEFAULT_PACKAGES from the suite).
`instr` counts instructions with callgrind: exact and repeatable on one
machine, so it is the gate (`compare` fails if any benchmark gets more than
PCT% slower, default 0.5). `time` is the best of --runs wall times,
startup included. A program whose output differs from GNU R's is marked
`wrong`: being fast does not count if the answer is different.

Results go to --out (default .robigo/perf/latest.json). The robigo binary
is chosen as in scripts/corpus.py.
"""
import argparse
import concurrent.futures
import json
import math
import os
import re
import subprocess
import sys
import tempfile
import time
import tomllib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
STATE = os.path.join(ROOT, ".robigo")
CACHE = os.environ.get("ROBIGO_BENCH_CACHE", os.path.expanduser("~/.cache/robigo-bench"))
BASE_ENV = {"LANGUAGE": "en", "LC_ALL": "C.UTF-8", "TZ": "UTC", "R_ENVIRON_USER": "", "R_PROFILE_USER": ""}
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from corpus import robigo_path  # noqa: E402

DRIVER = """setwd({dir!r})
source({file!r})
.n <- {param}
.r <- if (length(formals(execute)) > 0) execute(.n) else {{ for (.i in seq_len(.n)) .v <- execute(); .v }}
if (exists("verifyResult", mode = "function")) cat("verified:", isTRUE(verifyResult(.r, .n)), "\\n")
"""


def conf():
    with open(os.path.join(ROOT, "benchmarks", "benchmarks.toml"), "rb") as f:
        return tomllib.load(f)


def suite_src(name, s):
    if "dir" in s:
        return os.path.join(ROOT, s["dir"])
    return os.path.join(CACHE, name, s["commit"])


def programs(c, suite=None, rx=None):
    """[(name, suite, script_text, default_packages)]"""
    out = []
    for sname, s in c["suites"].items():
        if suite and sname != suite:
            continue
        src = suite_src(sname, s)
        pk = s.get("default_packages", "utils,stats")
        if "dir" in s:
            for fn in sorted(os.listdir(src)):
                if fn.endswith(".R"):
                    with open(os.path.join(src, fn)) as f:
                        out.append((f"{sname}/{fn[:-2]}", sname, f.read(), pk))
        else:
            for b in s["benchmarks"]:
                d = os.path.join(src, b["dir"])
                out.append((f"{sname}/{b['name']}", sname,
                            DRIVER.format(dir=d, file=os.path.join(d, b["file"]), param=b["param"]), pk))
    if rx:
        out = [p for p in out if re.search(rx, p[0])]
    return out


def cmd_fetch(_):
    for sname, s in conf()["suites"].items():
        if "repo" not in s:
            continue
        dst = suite_src(sname, s)
        if os.path.isdir(dst):
            print(f"{sname}: have {s['commit'][:12]}")
            continue
        os.makedirs(dst)
        url = s["repo"].replace("https://github.com/", "https://codeload.github.com/") + f"/tar.gz/{s['commit']}"
        tar = subprocess.run(["curl", "-sSfL", url], capture_output=True, check=True).stdout
        subprocess.run(["tar", "xz", "--strip-components=1", "-C", dst], input=tar, check=True)
        print(f"{sname}: fetched {s['commit'][:12]}")


def r_home():
    return subprocess.run(["R", "RHOME"], capture_output=True, text=True, check=True).stdout.strip()


def command(runtime, robigo, file, instr):
    if runtime == "robigo":
        return [robigo, "--vanilla", file]
    if instr:
        # Under valgrind run R's executable directly (Rscript would exec it).
        return [os.path.join(r_home(), "bin", "exec", "R"), "--vanilla", "--no-echo", "--no-restore", "-f", file]
    return ["Rscript", "--vanilla", file]


def run_one(prog, runtime, robigo, metric, runs, valgrind, timeout, profile=None):
    name, _, text, pk = prog
    env = dict(os.environ, **BASE_ENV, R_DEFAULT_PACKAGES=pk, R_HOME=r_home())
    res = {}
    with tempfile.TemporaryDirectory(prefix="robigo-bench-") as tmp:
        file = os.path.join(tmp, "bench.R")
        with open(file, "w") as f:
            f.write(text)
        out = None
        if metric in ("time", "both"):
            best = None
            for _ in range(runs):
                t = time.perf_counter()
                try:
                    p = subprocess.run(command(runtime, robigo, file, False), cwd=tmp, env=env, capture_output=True,
                                       timeout=timeout)
                except subprocess.TimeoutExpired:
                    return {"error": "timeout"}
                d = time.perf_counter() - t
                best = d if best is None else min(best, d)
                out = (p.stdout, p.returncode)
            res["secs"] = round(best, 4)
        if metric in ("instr", "both"):
            cg = profile or os.path.join(tmp, "cg.out")
            try:
                p = subprocess.run([valgrind, "--tool=callgrind", f"--callgrind-out-file={cg}"] +
                                   command(runtime, robigo, file, True), cwd=tmp, env=env, capture_output=True,
                                   timeout=timeout * 60)
            except subprocess.TimeoutExpired:
                return {"error": "timeout"}
            m = re.search(rb"refs:\s+([\d,]+)", p.stderr)
            if not m:
                return {"error": "valgrind: " + p.stderr[-300:].decode("utf-8", "replace")}
            res["instr"] = int(m.group(1).replace(b",", b""))
            # valgrind's own messages are on stderr; stdout is the program's.
            out = (p.stdout, p.returncode)
        res["status"] = out[1]
        res["out"] = out[0].decode("utf-8", "replace")
    return res


def cmd_run(a):
    c = conf()
    progs = programs(c, a.suite, a.filter)
    robigo = robigo_path(a.robigo)
    runtimes = ["robigo", "gnur"] if a.runtime == "both" else [a.runtime]
    valgrind = os.environ.get("VALGRIND", os.path.expanduser("~/.cache/rr-tools/bin/valgrind"))
    if not os.path.exists(valgrind):
        valgrind = "valgrind"
    jobs = a.jobs if a.metric == "instr" else 1  # wall times need an idle machine
    tasks = [(p, rt) for p in progs for rt in runtimes]
    results = {p[0]: {} for p in progs}

    def go(t):
        p, rt = t
        return p[0], rt, run_one(p, rt, robigo, a.metric, a.runs, valgrind, a.timeout)

    with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as ex:
        for name, rt, r in ex.map(go, tasks):
            results[name][rt] = r
            if not a.quiet:
                show = r.get("error") or " ".join(f"{k}={r[k]}" for k in ("instr", "secs") if k in r)
                print(f"{name:40} {rt:7} {show}", file=sys.stderr, flush=True)
    for name, r in results.items():
        gn = r.get("gnur", {})
        oracle_failed = "gnur" in r and ("error" in gn or gn.get("status") != 0)
        if oracle_failed:
            r["oracle_warning"] = "GNU R measurement failed; ratio unavailable"
            print(f"WARNING {name}: {r['oracle_warning']} ({gn})", file=sys.stderr)
        if "robigo" in r and "gnur" in r and not oracle_failed and "error" not in r["robigo"]:
            r["wrong"] = (r["robigo"]["out"], r["robigo"]["status"]) != (r["gnur"]["out"], r["gnur"]["status"])
        for rt in r.values():
            if isinstance(rt, dict):
                rt.pop("out", None)
    out = a.out or os.path.join(STATE, "perf", "latest.json")
    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    doc = {"metric": a.metric, "robigo": robigo, "results": results}
    if os.path.exists(out) and a.merge:
        with open(out) as f:
            old = json.load(f)
        for k, v in old.get("results", {}).items():
            doc["results"].setdefault(k, {}).update({rt: x for rt, x in v.items() if rt not in doc["results"][k]})
    with open(out, "w") as f:
        json.dump(doc, f, indent=1, sort_keys=True)
    report(doc, False)


def geomean(xs):
    xs = [x for x in xs if x > 0]
    return math.exp(sum(math.log(x) for x in xs) / len(xs)) if xs else float("nan")


def report(doc, markdown):
    rows = []
    for name, r in sorted(doc["results"].items()):
        ro, gn = r.get("robigo", {}), r.get("gnur", {})
        row = [name]
        for k in ("instr", "secs"):
            a, b = ro.get(k), gn.get(k)
            oracle_ok = "error" not in gn and gn.get("status", 0) == 0
            row += [a, b, (a / b) if a and b and oracle_ok else None]
        err = ro.get("error") or ("robigo failed" if ro.get("status", 0) != 0 else "") or ("wrong output" if r.get("wrong") else "")
        if r.get("oracle_warning") or "error" in gn or gn.get("status", 0) != 0:
            err = (err + "; " if err else "") + "GNU R measurement failed; ratio unavailable"
        rows.append(row + [err])
    if markdown:
        print("| benchmark | robigo instr | GNU R instr | ratio | robigo s | GNU R s | ratio | |")
        print("|---|---|---|---|---|---|---|---|")
    for row in rows:
        f = lambda x, fmt: "" if x is None else format(x, fmt)
        cells = [row[0], f(row[1], ","), f(row[2], ","), f(row[3], ".2f"), f(row[4], ".3f"), f(row[5], ".3f"),
                 f(row[6], ".2f"), row[7]]
        print(("| " + " | ".join(cells) + " |") if markdown else
              f"{cells[0]:40} {cells[1]:>16} {cells[2]:>16} {cells[3]:>6} {cells[4]:>8} {cells[5]:>8} {cells[6]:>6} {cells[7]}")
    for i, label in ((3, "instructions"), (6, "wall time")):
        ok = [r[i] for r in rows if r[i] is not None and not r[7]]
        if ok:
            faster = sum(1 for x in ok if x < 1)
            print(f"\n{label}: robigo/GNU R geometric mean {geomean(ok):.3f} over {len(ok)} correct programs; "
                  f"faster on {faster}")
    bad = [r[0] for r in rows if r[7]]
    if bad:
        print(f"not counted ({len(bad)}): " + ", ".join(bad))


def cmd_report(a):
    with open(a.file) as f:
        report(json.load(f), a.markdown)


def cmd_compare(a):
    with open(a.base) as f:
        base = json.load(f)["results"]
    with open(a.new) as f:
        new = json.load(f)["results"]
    worse, better, broke, fixed = [], [], [], []
    for name in sorted(base.keys() - new.keys()):
        print(f"REMOVED {name} (absent from candidate; no regression comparison)")
    for name, r in sorted(new.items()):
        b = base.get(name, {}).get("robigo", {})
        n = r.get("robigo", {})
        base_failed = "error" in b or b.get("status", 0) != 0 or base.get(name, {}).get("wrong")
        if "error" in n or n.get("status", 0) != 0 or r.get("wrong"):
            if not base_failed:
                broke.append(name)
            else:
                print(f"UNSUPPORTED {name} (failed on base and candidate)")
            continue
        if name not in base:
            print(f"ADDED {name} (absent from base; no regression comparison)")
        if base_failed:
            # Not counted before (a wrong answer is not fast): the base
            # count measured a different run, often one that stopped early.
            if name in base:
                fixed.append(name)
            continue
        if "instr" in b and "instr" in n:
            d = 100 * (n["instr"] - b["instr"]) / b["instr"]
            if d > a.threshold:
                worse.append((name, d))
            elif d < -a.threshold:
                better.append((name, d))
    for name, d in better:
        print(f"faster  {name:40} {d:+.2f}%")
    for name, d in worse:
        print(f"SLOWER  {name:40} {d:+.2f}%")
    for name in broke:
        print(f"BROKEN  {name} (was correct, now fails or prints a different result)")
    for name in fixed:
        print(f"FIXED   {name} (failed or printed a different result, now correct)")
    print(f"{len(better)} faster, {len(worse)} slower (> {a.threshold}%), {len(broke)} broken, {len(fixed)} fixed")
    return 1 if worse or broke else 0


def cmd_recheck(a):
    """`compare`, after measuring every benchmark it finds slower once more
    (`--robigo`) and keeping the lower of the two counts: a regression is
    slower in both runs, a one-off outlier is not. The result file is
    updated in place."""
    with open(a.base) as f:
        base = json.load(f)["results"]
    with open(a.new) as f:
        doc = json.load(f)
    new = doc["results"]

    def slower(name):
        b = base.get(name, {}).get("robigo", {})
        n = new[name].get("robigo", {})
        return "instr" in b and "instr" in n and 100 * (n["instr"] - b["instr"]) / b["instr"] > a.threshold

    slow = {name for name in new if slower(name)}
    if slow:
        robigo = robigo_path(a.robigo)
        valgrind = os.environ.get("VALGRIND", os.path.expanduser("~/.cache/rr-tools/bin/valgrind"))
        if not os.path.exists(valgrind):
            valgrind = "valgrind"
        progs = [p for p in programs(conf()) if p[0] in slow]
        with concurrent.futures.ThreadPoolExecutor(max_workers=a.jobs) as ex:
            again = list(ex.map(lambda p: (p[0], run_one(p, "robigo", robigo, "instr", 1, valgrind, a.timeout)), progs))
        for name, r in again:
            if "instr" not in r:
                continue
            first = new[name]["robigo"]["instr"]
            new[name]["robigo"]["instr"] = min(first, r["instr"])
            print(f"re-measured {name}: {first:,} then {r['instr']:,}")
        with open(a.new, "w") as f:
            json.dump(doc, f, indent=1, sort_keys=True)
    return cmd_compare(a)


def cmd_list(_):
    for p in programs(conf()):
        print(p[0])


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list")
    sub.add_parser("fetch")
    p = sub.add_parser("run")
    p.add_argument("--metric", choices=["instr", "time", "both"], default="instr")
    p.add_argument("--runtime", choices=["robigo", "gnur", "both"], default="both")
    p.add_argument("--suite")
    p.add_argument("--filter")
    p.add_argument("--jobs", type=int, default=os.cpu_count())
    p.add_argument("--runs", type=int, default=3)
    p.add_argument("--timeout", type=int, default=120, help="seconds per run (x60 under callgrind)")
    p.add_argument("--robigo")
    p.add_argument("--out")
    p.add_argument("--merge", action="store_true", help="keep results of other runtimes from --out")
    p.add_argument("--quiet", action="store_true")
    p = sub.add_parser("compare")
    p.add_argument("base")
    p.add_argument("new")
    p.add_argument("--threshold", type=float, default=0.5)
    p = sub.add_parser("recheck", help="compare, re-measuring the benchmarks found slower once")
    p.add_argument("base")
    p.add_argument("new")
    p.add_argument("--threshold", type=float, default=0.5)
    p.add_argument("--robigo")
    p.add_argument("--jobs", type=int, default=os.cpu_count())
    p.add_argument("--timeout", type=int, default=120, help="seconds per run (x60 under callgrind)")
    p = sub.add_parser("report")
    p.add_argument("file")
    p.add_argument("--markdown", action="store_true")
    a = ap.parse_args()
    fn = {"list": cmd_list, "fetch": cmd_fetch, "run": cmd_run, "compare": cmd_compare, "recheck": cmd_recheck,
          "report": cmd_report}[a.cmd]
    sys.exit(fn(a) or 0)


if __name__ == "__main__":
    main()
