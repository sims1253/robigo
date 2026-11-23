"""The old runtime's benchmark suites and how one program is measured
(scripts/perf.py run, the `perf` job): the suites, their programs, the
command and the callgrind run of one program, which GNU R's records in the
job's cache depend on (scripts/perf-gnur.py). perf.py re-exports every
name, so `perf.<name>` stays the API; robigo-rt's rows and starts are
perf.py's own."""

import os
import re
import subprocess
import tempfile
import time
import tomllib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CACHE = os.environ.get("ROBIGO_BENCH_CACHE", os.path.expanduser("~/.cache/robigo-bench"))

BASE_ENV = {"LANGUAGE": "en", "LC_ALL": "C.UTF-8", "TZ": "UTC", "R_ENVIRON_USER": "", "R_PROFILE_USER": ""}

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


def suite_files(sname, s):
    """A directory suite's programs: the paths of its *.R files, sorted
    (what `programs` runs, and what GNU R's records depend on)."""
    src = suite_src(sname, s)
    return [os.path.join(src, fn) for fn in sorted(os.listdir(src)) if fn.endswith(".R")]


def programs(c, suite=None, rx=None):
    """[(name, suite, script_text, default_packages)]"""
    out = []
    for sname, s in c["suites"].items():
        if suite and sname != suite:
            continue
        src = suite_src(sname, s)
        pk = s.get("default_packages", "utils,stats")
        if "dir" in s:
            for path in suite_files(sname, s):
                with open(path) as f:
                    out.append((f"{sname}/{os.path.basename(path)[:-2]}", sname, f.read(), pk))
        else:
            for b in s["benchmarks"]:
                d = os.path.join(src, b["dir"])
                workdir = os.path.join(src, b.get("workdir", b["dir"]))
                out.append((f"{sname}/{b['name']}", sname,
                            DRIVER.format(dir=workdir, file=os.path.join(d, b["file"]), param=b["param"]), pk))
    if rx:
        out = [p for p in out if re.search(rx, p[0])]
    return out


def gnur_inputs(c=None):
    """The files GNU R's records in the `perf` job depend on, relative to the
    checkout, sorted: the suites' list (an external suite's pinned commit and
    parameters are in it), every directory suite's programs (`suite_files`,
    the listing `programs` runs), the known failures, and the scripts that
    measure and keep them. The job's cache key hashes these and nothing else
    (scripts/perf-gnur.py; a change to robigo-rt's rows does not drop it)."""
    c = c or conf()
    out = [os.path.join(ROOT, "benchmarks", "benchmarks.toml"),
           os.path.join(ROOT, "scripts", "perf.py"), os.path.join(ROOT, "scripts", "perf-gnur.py")]
    # (optional: perf-gnur.py reads none as no known failure)
    known = os.path.join(ROOT, "benchmarks", "gnur-known-failures.txt")
    if os.path.exists(known):
        out.append(known)
    for sname, s in c["suites"].items():
        if "dir" in s:
            out += suite_files(sname, s)
    return sorted(os.path.relpath(p, ROOT) for p in out)

def r_home():
    return subprocess.run(["R", "RHOME"], capture_output=True, text=True, check=True).stdout.strip()


def command(runtime, robigo, file, instr):
    if runtime == "robigo":
        return [robigo, "--vanilla", file]
    if instr:
        # Under valgrind run R's executable directly (Rscript would exec it).
        return [os.path.join(r_home(), "bin", "exec", "R"), "--vanilla", "--no-echo", "--no-restore", "-f", file]
    return ["Rscript", "--vanilla", file]

def valgrind_path():
    """$VALGRIND, the private build if there is one, or the one on PATH."""
    private = os.path.expanduser("~/.cache/rr-tools/bin/valgrind")
    return os.environ.get("VALGRIND") or (private if os.path.exists(private) else "valgrind")


def cache_counters(cgfile):
    """The cache and branch counters of a callgrind run with cache and
    branch simulation on (diagnostic: `--metric cache`; the gate stays
    instruction counts). Estimated cycles are experiments/memory/bench.py's
    sensitivity model, Ir + 10*D1m + 100*LLm — a rough machine, not a
    measurement (docs/ledger/benchmarks.md's formula, docs/ledger/memory.md
    for what it explained)."""
    events, summary = None, None
    with open(cgfile, errors="replace") as f:
        for line in f:
            if line.startswith("events: "):
                events = line[8:].split()
            elif line.startswith("summary: "):
                try:
                    summary = [int(x) for x in line[9:].split()]
                except ValueError:
                    return None
    if not events or not summary or len(events) != len(summary):
        return None
    c = dict(zip(events, summary))
    est = c.get("Ir", 0) + 10 * (c.get("D1mr", 0) + c.get("D1mw", 0)) \
        + 100 * (c.get("DLmr", 0) + c.get("DLmw", 0))
    return {k.lower(): c.get(k, 0) for k in
            ("Ir", "I1mr", "ILmr", "Dr", "D1mr", "DLmr", "Dw", "D1mw", "DLmw",
             "Bc", "Bcm", "Bi", "Bim")} | {"est_cycles": est}


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
        if metric in ("instr", "both", "cache"):
            cg = profile or os.path.join(tmp, "cg.out")
            sim = ["--cache-sim=yes", "--branch-sim=yes"] if metric == "cache" else []
            try:
                p = subprocess.run([valgrind, "--tool=callgrind", *sim,
                                    f"--callgrind-out-file={cg}"] +
                                   command(runtime, robigo, file, True), cwd=tmp, env=env, capture_output=True,
                                   timeout=timeout * 60)
            except subprocess.TimeoutExpired:
                return {"error": "timeout"}
            m = re.search(rb"refs:\s+([\d,]+)", p.stderr)
            if not m:
                return {"error": "valgrind: " + p.stderr[-300:].decode("utf-8", "replace")}
            res["instr"] = int(m.group(1).replace(b",", b""))
            if metric == "cache":
                res["cache"] = cache_counters(cg)
                if res["cache"] is None:
                    return {"error": "callgrind wrote no summary line (cache counters)"}
            # valgrind's own messages are on stderr; stdout is the program's.
            out = (p.stdout, p.returncode)
        res["status"] = out[1]
        res["out"] = out[0].decode("utf-8", "replace")
    return res
