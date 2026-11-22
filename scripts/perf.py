#!/usr/bin/env python3
"""Benchmarks: robigo against GNU R 4.5.0.

    scripts/perf.py list
    scripts/perf.py fetch                         # external suites, pinned
    scripts/perf.py run [--metric instr|time|both|cache] [--runtime robigo|gnur|both]
                        [--suite S] [--filter RE] [--jobs N] [--out FILE]
    scripts/perf.py compare BASE.json NEW.json [--threshold PCT]
    scripts/perf.py recheck BASE.json NEW.json [--robigo BIN]   # compare, re-measuring what is slower
    scripts/perf.py report FILE.json [--markdown]
    scripts/perf.py rt-bars --robigo-rt BIN [--main BIN] [--gnur FILE] [--cycles] [--out FILE]  # the M1 bars
    scripts/perf.py rt-gnur --out FILE    # GNU R's side of the bars, measured on this machine

Suites are listed in benchmarks/benchmarks.toml. Every program runs in the
oracle environment (C.UTF-8, TZ=UTC, R_DEFAULT_PACKAGES from the suite).
`instr` counts instructions with callgrind: exact and repeatable on one
machine, so it is the gate (`compare` fails if any benchmark gets more than
PCT% slower, default 0.5). `time` is the best of --runs wall times,
startup included. A program whose output differs from GNU R's is marked
`wrong`: being fast does not count if the answer is different.

Results go to --out (default .robigo/perf/latest.json). The robigo binary
is chosen as in scripts/corpus.py.

`rt-bars` is the same gate for robigo-rt (the perf-rt job): the rows of
benchmarks/rt/m1-bars.toml, measured as the M1 bars measure them
(scripts/idiom-vs-r.py --rows benchmarks/rt/calls-rows.toml, instructions
per iteration, N against 2N), and peak RSS (scripts/rss.py). It fails when
a bar marked met breaks against the committed GNU R and today's-robigo
numbers, when a regression the lead accepted passes its `ceiling`, or when a
row is more than the threshold slower than with --main, main's binary of the
merge's base, measured beside it on this machine. Its guard rows have no bar:
only main holds them (common VM paths no bar's row reaches).

GNU R's side of the bars is measured on the machine that gates, not taken
from another: `rt-gnur` measures GNU R's rows (instructions and estimated
cycles) and peak RSS, here, with the CPU's signature (`cpu_signature`), and
`rt-bars --gnur FILE` uses them, refusing a file measured on another CPU
signature or for another GNU R. The perf-rt job caches that file per
(oracle image, CPU signature), so that no bar mixes CPUs: glibc's CPU
dispatch moves GNU R's counts. Without --gnur, the bars use
benchmarks/rt/m1-bars.toml's numbers, measured on one developer machine.
--cycles adds estimated cycles under a pinned cache model (PINNED_CACHE),
reported and never failed on (docs/ledger/g3/2026-11-16-cycles-study.md).
"""
import argparse
import concurrent.futures
import glob
import hashlib
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


def rows_with_setups(doc):
    """The `[[row]]` tables of a rows file (benchmarks/rt/calls-rows.toml)
    with each one's `setup` resolved: its own, or with `setup_ref` the
    `[setups]` table's setup of that name, which rows share."""
    setups = doc.get("setups", {})
    out = []
    for r in doc["row"]:
        ref = r.get("setup_ref")
        if ref is not None and ref not in setups:
            raise KeyError(f"row `{r['name']}`: no setup named `{ref}` in the [setups] table")
        out.append(r if ref is None else {**r, "setup": setups[ref]})
    return out


def env_pair(kv):
    """argparse type for K=V: a value without '=' is an error, not a crash later."""
    k, sep, v = kv.partition("=")
    if not sep or not k:
        raise argparse.ArgumentTypeError(f"expected K=V, got {kv!r}")
    return kv


sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from corpus import pins, robigo_path  # noqa: E402
from scratch import scratch_dir  # noqa: E402

# The gate's threshold: a benchmark with more than this many percent more
# instructions than main's fails (`compare`, `recheck`, `rt-bars`).
THRESHOLD = 0.5
# Peak RSS is not exact (ru_maxrss moves by a few pages from run to run):
# `rt-bars`' threshold for it, in percent.
RSS_THRESHOLD = 2.0
RT_ROWS = os.path.join(ROOT, "benchmarks", "rt", "calls-rows.toml")
# The cache model estimated cycles are counted under. Callgrind otherwise
# takes the host's cache sizes, and a direct-mapped last level, so the same
# binary would count differently on every runner
# (docs/ledger/g3/2026-11-16-cycles-study.md, "Cycles depend on the host").
PINNED_CACHE = "--I1=32768,8,64 --D1=32768,8,64 --LL=33554432,16,64"
RT_BARS = os.path.join(ROOT, "benchmarks", "rt", "m1-bars.toml")


def rt_startup():
    """scripts/rt-startup.py as a module (its name is not an identifier):
    the starts' measurement and gate, perf.py rt-bars' [[startup]]."""
    import importlib.util
    spec = importlib.util.spec_from_file_location("rt_startup", os.path.join(ROOT, "scripts", "rt-startup.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod

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
                workdir = os.path.join(src, b.get("workdir", b["dir"]))
                out.append((f"{sname}/{b['name']}", sname,
                            DRIVER.format(dir=workdir, file=os.path.join(d, b["file"]), param=b["param"]), pk))
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


def crate_dirs(crate, root=ROOT):
    """The directories of `crate` and of every local crate it is built from,
    through `path` dependencies in [dependencies] and [build-dependencies]
    (target-specific ones too; `workspace = true` resolves through the root
    manifest), never [dev-dependencies]."""
    with open(os.path.join(root, "Cargo.toml"), "rb") as f:
        ws = tomllib.load(f).get("workspace", {})
    ws_deps = ws.get("dependencies", {})
    by_name = {}
    for m in ws.get("members", []):
        for d in glob.glob(os.path.join(root, m)):
            mf = os.path.join(d, "Cargo.toml")
            if os.path.isfile(mf):
                with open(mf, "rb") as f:
                    by_name[tomllib.load(f).get("package", {}).get("name")] = os.path.normpath(d)
    out, todo = [], [by_name[crate]]
    while todo:
        d = todo.pop()
        if d in out:
            continue
        out.append(d)
        with open(os.path.join(d, "Cargo.toml"), "rb") as f:
            man = tomllib.load(f)
        tables = [man.get("dependencies", {}), man.get("build-dependencies", {})]
        for t in man.get("target", {}).values():
            tables += [t.get("dependencies", {}), t.get("build-dependencies", {})]
        for table in tables:
            for name, spec in table.items():
                if not isinstance(spec, dict):
                    continue
                if spec.get("workspace"):
                    spec = ws_deps.get(name, {})
                    if "path" in spec:
                        todo.append(os.path.normpath(os.path.join(root, spec["path"])))
                elif "path" in spec:
                    todo.append(os.path.normpath(os.path.join(d, spec["path"])))
    return sorted(out)


def git(args, root=ROOT):
    """`git ARGS` in `root`: its stdout, or None without git or a repository."""
    try:
        p = subprocess.run(["git", *args], cwd=root, capture_output=True, text=True)
    except OSError:
        return None
    return p.stdout if p.returncode == 0 else None


def newest_source(dirs, root=ROOT):
    """(time, what) of the newest change to `dirs`: the last commit that
    touched them, or an uncommitted edit (its file's mtime), whichever is
    later. Commit times, not file mtimes, so that a fresh checkout of the
    same commit does not look newer than a binary built from it."""
    rel = [os.path.relpath(d, root) for d in dirs] + ["Cargo.lock"]
    best = (0.0, None)
    last = git(["log", "-1", "--format=%ct %h", "HEAD", "--", *rel], root)
    if last and last.strip():
        t, h = last.split()
        best = (float(t), f"commit {h}")
    # NUL-separated entries: paths unquoted, untracked files listed one by one;
    # a rename or copy is followed by its source, which is skipped
    entries = iter((git(["status", "--porcelain", "-z", "--untracked-files=all", "--", *rel], root) or "").split("\0"))
    for e in entries:
        if len(e) < 4:
            continue
        if "R" in e[:2] or "C" in e[:2]:
            next(entries, None)
        f = os.path.join(root, e[3:])
        if os.path.isfile(f):
            best = max(best, (os.path.getmtime(f), os.path.relpath(f, root) + " (uncommitted)"))
    return best


def build_id(path):
    """The GNU build ID of an ELF binary (`readelf -n`), or None."""
    try:
        p = subprocess.run(["readelf", "-n", path], capture_output=True, text=True)
    except OSError:
        return None
    m = re.search(r"Build ID: ([0-9a-f]+)", p.stdout)
    return m.group(1) if m else None


def binary_identity(path, crate=None, root=ROOT):
    """What a measurement says about the binary it measured: its sha256, GNU
    build ID and modification time, and this checkout's `git describe`. With
    `crate` (the package that builds the binary: robigo-rtapi for robigo-rt,
    robigo-cli for robigo), also the newest source of that crate's local
    dependency tree when it is newer than the binary: then the binary is not
    a build of this checkout (`cargo build -p robigo-rt` builds the library
    only; the robigo-rt binary comes from -p robigo-rtapi). A copy made with
    plain `cp` takes a new mtime and hides that: the sha256 and build ID in
    the report are what identify it."""
    # in bounded chunks: reading the whole binary would raise this process's
    # peak RSS, which the programs rss.py then starts inherit
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 16), b""):
            h.update(chunk)
    sha = h.hexdigest()
    mtime = os.path.getmtime(path)
    describe = git(["describe", "--always", "--dirty", "--tags"], root)
    ident = {"path": path, "sha256": sha, "build_id": build_id(path),
             "mtime": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(mtime)),
             "checkout": describe.strip() if describe else None, "stale": None}
    if crate:
        newest, what = newest_source(crate_dirs(crate, root), root)
        if what and newest > mtime:
            ident["stale"] = {"source": what, "mtime": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(newest))}
    return ident


def crate_of(binary):
    """The package that builds a robigo binary, by its file name."""
    name = os.path.basename(binary)
    return "robigo-rtapi" if name.startswith("robigo-rt") else "robigo-cli" if name.startswith("robigo") else None


def report_identity(label, ident, out=sys.stderr):
    """One header line per binary, and a warning for a stale one."""
    print(f"{label}: {ident['path']}: sha256 {ident['sha256'][:16]}, build ID {(ident['build_id'] or 'none')[:16]}, "
          f"modified {ident['mtime']}; checkout {ident['checkout']}", file=out)
    if ident["stale"]:
        print(f"warning: {label} {ident['path']} is older than {ident['stale']['source']} "
              f"({ident['stale']['mtime']}) of the crates it is built from: it is not a build of "
              f"this checkout's sources", file=out)


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


def cmd_run(a):
    c = conf()
    progs = programs(c, a.suite, a.filter)
    robigo = robigo_path(a.robigo)
    runtimes = ["robigo", "gnur"] if a.runtime == "both" else [a.runtime]
    valgrind = valgrind_path()
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
    if getattr(a, "oracle", None):
        with open(a.oracle) as f:
            oracle_results = json.load(f).get("results", {})
        missing_out = [p for p in progs if p[0] in oracle_results and "gnur" in oracle_results[p[0]] and "out" not in oracle_results[p[0]]["gnur"]]
        if missing_out:
            print(f"Recording missing GNU R outputs for {len(missing_out)} baseline benchmark(s)...", file=sys.stderr)
            for p in missing_out:
                r = run_one(p, "gnur", robigo, "time", 1, valgrind, a.timeout)
                oracle_results[p[0]]["gnur"]["out"] = r.get("out")
                oracle_results[p[0]]["gnur"]["status"] = r.get("status")
                oracle_results[p[0]]["gnur"]["_fallback"] = True
        for name in results:
            if "gnur" not in results[name] and name in oracle_results and "gnur" in oracle_results[name]:
                results[name]["gnur"] = oracle_results[name]["gnur"]

    for name, r in results.items():
        gn = r.get("gnur", {})
        oracle_failed = "gnur" in r and ("error" in gn or gn.get("status") != 0)
        if oracle_failed:
            r["oracle_warning"] = "GNU R measurement failed; ratio unavailable"
            print(f"WARNING {name}: {r['oracle_warning']} ({gn})", file=sys.stderr)
        if "robigo" in r and "gnur" in r and not oracle_failed and "error" not in r["robigo"]:
            wrong = (r["robigo"]["out"], r["robigo"]["status"]) != (gn.get("out"), gn.get("status"))
            if wrong and gn.get("_fallback") and a.metric != "time":
                prog = next((p for p in progs if p[0] == name), None)
                if prog:
                    r_gn = run_one(prog, "gnur", robigo, a.metric, 1, valgrind, a.timeout)
                    gn["out"] = r_gn.get("out")
                    gn["status"] = r_gn.get("status")
                    wrong = (r["robigo"]["out"], r["robigo"]["status"]) != (gn.get("out"), gn.get("status"))
            r["wrong"] = wrong
        if "gnur" in r and isinstance(r["gnur"], dict):
            r["gnur"].pop("_fallback", None)
        if "robigo" in r and isinstance(r["robigo"], dict):
            if "gnur" in r:
                r["robigo"].pop("out", None)
    out = a.out or os.path.join(STATE, "perf", "latest.json")
    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    doc = {"metric": a.metric, "robigo": robigo, "results": results}
    if a.filter or a.suite:
        # A narrowed run (pull requests, scripts/perf-select.py): the gate
        # then does not call the benchmarks left out "removed".
        doc["selection"] = {"filter": a.filter, "suite": a.suite}
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


def not_selected(doc, name):
    """True if `doc` is a narrowed run (`selection` recorded by `run
    --filter/--suite`) that left `name` out on purpose."""
    sel = doc.get("selection")
    if not sel:
        return False
    if sel.get("suite") and not name.startswith(sel["suite"] + "/"):
        return True
    return bool(sel.get("filter")) and not re.search(sel["filter"], name)


def load_instr(path, verb):
    """A result file that carries instruction counts; any other metric
    (time, or the diagnostic cache) exits 2 rather than gating on nothing."""
    with open(path) as f:
        doc = json.load(f)
    if doc.get("metric") not in (None, "instr", "both"):
        print(f"{verb} needs instruction counts: {path} was measured with "
              f"--metric {doc['metric']} (diagnostic only; the gate is instr)", file=sys.stderr)
        sys.exit(2)
    return doc


def cmd_compare(a):
    base = load_instr(a.base, "compare")["results"]
    new_doc = load_instr(a.new, "compare")
    new = new_doc.get("results", {})
    updated = False
    for name, r in new.items():
        b = base.get(name, {})
        gn = b.get("gnur", {})
        ro = r.get("robigo", {})
        if "out" in ro:
            oracle_failed = "error" in gn or gn.get("status", 0) != 0
            if "out" in gn and not oracle_failed and "error" not in ro:
                r["wrong"] = (ro["out"], ro.get("status", 0)) != (gn["out"], gn.get("status", 0))
            if "gnur" not in r and "gnur" in b:
                r["gnur"] = gn
            ro.pop("out", None)
            updated = True
        elif "gnur" in b and "gnur" not in r:
            r["gnur"] = gn
            updated = True
    if updated:
        with open(a.new, "w") as f:
            json.dump(new_doc, f, indent=1, sort_keys=True)
    worse, better, broke, fixed = [], [], [], []
    skipped = 0
    for name in sorted(base.keys() - new.keys()):
        if not_selected(new_doc, name):
            skipped += 1
            continue
        print(f"REMOVED {name} (absent from candidate; no regression comparison)")
    if skipped:
        detail = " ".join(f"{k}={v}" for k, v in new_doc["selection"].items() if v)
        print(f"{skipped} baseline benchmark(s) outside this run's selection "
              f"({detail}): not compared")
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
    base = load_instr(a.base, "recheck")["results"]
    doc = load_instr(a.new, "recheck")
    new = doc["results"]

    def slower(name):
        b = base.get(name, {}).get("robigo", {})
        n = new[name].get("robigo", {})
        return "instr" in b and "instr" in n and 100 * (n["instr"] - b["instr"]) / b["instr"] > a.threshold

    slow = {name for name in new if slower(name)}
    if slow:
        robigo = robigo_path(a.robigo)
        valgrind = valgrind_path()
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


# The heap shifts the address-sensitive rows are measured over (k unrelated
# strings before the row's setup; idiom-vs-r.py --shifts): their counts
# follow where the strings were allocated, so one layout's count is noise.
# The gate compares the medians (docs/ledger/g3/2026-11-17-address-rows.md).
ADDRESS_SHIFTS = (0, 300, 1000, 3000, 5000)


def rt_instr(binary, rows, jobs, outdir, tag, cycles=False, ranges=None):
    """Instructions per iteration of `rows` (names of
    benchmarks/rt/calls-rows.toml rows), as the M1 bars measure them:
    scripts/idiom-vs-r.py --rows, callgrind N against 2N; robigo-rt's
    (`binary`), or GNU R's (`binary` None: RSCRIPT_BIN). The rows are split
    over up to `jobs` parallel runs, the long ones (fib, binarytrees) first.
    With `cycles`, also the estimated cycles under PINNED_CACHE: ({row:
    instr}, {row: cycles}); without, {row: instr}. An address-sensitive row
    reports the median over ADDRESS_SHIFTS, and `ranges` (a dict) gets its
    {row: (lowest, highest)} count over them."""
    long = [r for r in ("fib", "binarytrees") if r in rows]
    order = long + [r for r in rows if r not in long]
    n = max(1, min(jobs, len(order)))
    groups = [order[i::n] for i in range(n)]
    script = os.path.join(ROOT, "scripts", "idiom-vs-r.py")
    who = "robigo" if binary else "gnur"
    env = dict(os.environ, VALGRIND_OPTS=PINNED_CACHE) if cycles else None

    def one(i):
        out = os.path.join(outdir, f"{tag}-{i}.json")
        rx = "^(" + "|".join(re.escape(r) for r in groups[i]) + ")$"
        side = ["--robigo", binary, "--no-r"] if binary else ["--no-robigo"]
        cmd = [sys.executable, script, "--rows", RT_ROWS, "--filter", rx, *side, "--json", out,
               "--shifts", ",".join(map(str, ADDRESS_SHIFTS)), *(["--cycles"] if cycles else [])]
        p = subprocess.run(cmd, capture_output=True, text=True, env=env)
        if p.returncode:
            raise SystemExit(f"rt-bars: {' '.join(cmd)} failed:\n{p.stderr[-3000:]}")
        with open(out) as f:
            return json.load(f)["rows"]

    instr, cyc = {}, {}
    with concurrent.futures.ThreadPoolExecutor(max_workers=n) as ex:
        for got in ex.map(one, range(n)):
            instr.update({name: v[f"{who}_instr"] for name, v in got.items()})
            cyc.update({name: v[f"{who}_est_cycles"] for name, v in got.items()})
            if ranges is not None:
                for name, v in got.items():
                    if "shifts" in v:
                        ks = [x[f"{who}_instr"] for x in v["shifts"].values()]
                        ranges[name] = (min(ks), max(ks))
    missing = sorted(set(rows) - set(instr))
    if missing:
        raise SystemExit(f"rt-bars: no measurement of {', '.join(missing)}")
    return (instr, cyc) if cycles else instr


def rt_rss(binary, files, runs, outdir, tag):
    """Peak RSS of each program in KiB, scripts/rss.py's median of `runs`:
    robigo-rt's (`binary`), or GNU R's (`binary` None). A bars file may
    name no program (jit-bars.toml): then there is nothing to run, and
    rss.py refuses a call without files."""
    if not files:
        return {}
    out = os.path.join(outdir, f"{tag}-rss.json")
    side = ["--robigo", binary, "--no-r"] if binary else ["--no-robigo"]
    cmd = [sys.executable, os.path.join(ROOT, "scripts", "rss.py"), "--runs", str(runs), *side,
           "--json", out, *files]
    p = subprocess.run(cmd, capture_output=True, text=True, cwd=ROOT)
    if p.returncode:
        raise SystemExit(f"rt-bars: {' '.join(cmd)} failed:\n{p.stderr[-3000:]}")
    with open(out) as f:
        results = json.load(f)["results"]
    return {name: results[name]["robigo" if binary else "gnur"]["median_kib"] for name in files}


def cpu_signature(cpuinfo="/proc/cpuinfo", cache_dir="/sys/devices/system/cpu/cpu0/cache"):
    """(model name, signature) of this machine's CPU. The signature hashes
    what the C library's CPU dispatch reads, and so what moves callgrind's
    counts: vendor, family, model, stepping, the feature flags and the cache
    sizes (glibc's memcpy tunables come from them)."""
    info = {}
    with open(cpuinfo) as f:
        for line in f:
            if not line.strip():
                break  # the first processor's block
            k, _, v = line.partition(":")
            info[k.strip()] = v.strip()
    caches = []
    for d in sorted(glob.glob(os.path.join(cache_dir, "index*"))):
        fields = []
        for x in ("level", "type", "size", "ways_of_associativity", "coherency_line_size"):
            try:
                with open(os.path.join(d, x)) as f:
                    fields.append(f.read().strip())
            except OSError:
                fields.append("?")
        caches.append(":".join(fields))
    parts = [info.get(k, "") for k in ("vendor_id", "cpu family", "model", "stepping", "model name")]
    parts += [" ".join(sorted(info.get("flags", "").split()))] + caches
    return info.get("model name", "unknown"), hashlib.sha256("\n".join(parts).encode()).hexdigest()[:16]


def gnur_version():
    """The version of the GNU R that RSCRIPT_BIN runs."""
    r_bin = os.environ.get("RSCRIPT_BIN", "/usr/lib/R/bin/exec/R")
    r_home = os.environ.get("R_HOME_DIR", "/usr/lib/R")
    with tempfile.NamedTemporaryFile("w", suffix=".R", delete=False) as f:
        f.write("cat(format(getRversion()))\n")
    env = dict(os.environ, **BASE_ENV, R_DEFAULT_PACKAGES="NULL", R_HOME=r_home,
               LD_LIBRARY_PATH=os.path.join(r_home, "lib"))
    p = subprocess.run([r_bin, "--vanilla", "--no-echo", "-f", f.name], env=env, capture_output=True, text=True)
    os.unlink(f.name)
    if p.returncode:
        raise SystemExit(f"rt-gnur: {r_bin} failed:\n{p.stderr[-2000:]}")
    return p.stdout.strip()


def cmd_rt_gnur(a):
    """GNU R's side of the bars on this machine: its rows' instructions and
    estimated cycles (PINNED_CACHE), its peak RSS, the CPU and its signature."""
    with open(a.bars, "rb") as f:
        spec = tomllib.load(f)
    rows = sorted({b["row"] for b in spec["bar"] if b["against"] == "gnur"})
    files = sorted({r["program"] for r in spec["rss"] if "gnur" in r["against"]})
    work = scratch_dir("rt-gnur-")
    version = gnur_version()
    instr, cycles = rt_instr(None, rows, a.jobs, work, "gnur", cycles=True)
    rss = rt_rss(None, files, a.runs, work, "gnur")
    # GNU R's starts, shown beside robigo-rt's (report-only)
    startup = rt_startup().run_gnur(valgrind=valgrind_path()) if spec.get("startup") else {}
    model, sig = cpu_signature()
    doc = {"version": version, "cpu": model, "cpu_signature": sig, "cache_model": PINNED_CACHE,
           "valgrind": subprocess.run([valgrind_path(), "--version"], capture_output=True, text=True).stdout.strip(),
           "instr": instr, "cycles": cycles, "rss": rss, "startup": startup}
    with open(a.out, "w") as f:
        json.dump(doc, f, indent=1, sort_keys=True)
    print(f"GNU R {version} on {model} (signature {sig}): {len(instr)} rows, {len(rss)} programs -> {a.out}")
    return 0


def rt_with_gnur(spec, doc):
    """The bars file `spec` with GNU R's side replaced by `rt-gnur`'s
    measurement `doc`: its instructions per row, and its peak RSS where it
    measured a program."""
    return {**spec, "gnur": {**spec["gnur"], "instr": doc["instr"],
                             "rss": {**spec["gnur"].get("rss", {}), **doc["rss"]}}}


def rt_gnur_errors(doc, pin, sig):
    """Why a GNU R file (`rt-gnur`'s) cannot gate here: measured for another
    GNU R than the rt pin, or on another CPU signature (a bar would mix
    CPUs)."""
    errors = []
    if doc.get("version") != pin:
        errors.append(f"it holds GNU R {doc.get('version')}'s numbers, but the rt pin is {pin}")
    if doc.get("cpu_signature") != sig:
        errors.append(f"it was measured on CPU signature {doc.get('cpu_signature')} ({doc.get('cpu')}), "
                      f"and this machine's is {sig}")
    if doc.get("cache_model") != PINNED_CACHE:
        errors.append(f"its cycles were counted under {doc.get('cache_model')}, not {PINNED_CACHE}")
    return errors


def pct(new, base):
    return 100 * (new - base) / base


def rt_acceptance_errors(bar):
    """What is wrong with a bar's acceptance, if it has one: `accepted` and
    `ceiling` go together, only on an unmet bar, with a record that exists
    and a ceiling above the bar. rt-bars refuses a bars file with any."""
    keys = sorted({"accepted", "ceiling"} & bar.keys())
    if not keys:
        return []
    row = bar["row"]
    if len(keys) == 1:
        return [f"`{row}`: `accepted` and `ceiling` go together, but it has only `{keys[0]}`"]
    errors = []
    if bar.get("met", True):
        errors.append(f"`{row}`: an accepted regression is unmet by definition, so it needs `met = false`")
    if not os.path.exists(os.path.join(ROOT, bar["accepted"])):
        errors.append(f"`{row}`: its acceptance record {bar['accepted']} does not exist")
    if not bar["ceiling"] > bar["max"]:
        errors.append(f"`{row}`: its ceiling {bar['ceiling']}x is not above its bar of {bar['max']}x")
    return errors


def rt_guard_errors(spec, row_names):
    """What is wrong with the bars file's guard rows: each is a row of
    benchmarks/rt/calls-rows.toml (`row_names`), named once, and not also a
    bar's row (a bar already holds its row to main). rt-bars refuses a bars
    file with any."""
    bars = {b["row"] for b in spec["bar"]}
    errors, seen = [], set()
    for g in spec.get("guard", []):
        row = g["row"]
        if row not in row_names:
            errors.append(f"guard `{row}`: no row of that name in benchmarks/rt/calls-rows.toml")
        if row in bars:
            errors.append(f"guard `{row}`: a bar's row already, which holds it to main")
        if row in seen:
            errors.append(f"guard `{row}`: named twice")
        seen.add(row)
    return errors


def rt_verdict(spec, instr, rss, main=None):
    """The rt-bars gate's findings: (failures, notes, table). `spec` is
    benchmarks/rt/m1-bars.toml; `instr` and `rss` this binary's numbers;
    `main` the same numbers of main's binary, measured on the same machine
    ({"instr": ..., "rss": ...}), or None for the bars only."""
    failures, notes, table = [], [], []
    for b in spec["bar"]:
        row, against, cap, met = b["row"], b["against"], b["max"], b.get("met", True)
        ref = spec[against]["instr"][row]
        v = instr[row]
        ratio = v / ref
        m = main["instr"][row] if main else None
        status = "ok" if ratio <= cap else ("BROKEN" if met else "not met yet")
        if met:
            if ratio > cap:
                failures.append(f"`{row}`: {ratio:.3f}x {against} breaks its bar of {cap}x")
        elif ratio <= cap:
            todo = ("the accepted regression is no longer needed: remove `accepted` and `ceiling` and set it to met"
                    if "accepted" in b else "set it to met")
            notes.append(f"`{row}`: {ratio:.3f}x {against} meets its bar of {cap}x, so {todo} in "
                         f"benchmarks/rt/m1-bars.toml")
        elif "ceiling" in b:
            # an accepted regression, unmet by definition, holds only up to its ceiling
            status = f"accepted up to {b['ceiling']}x"
            if ratio > b["ceiling"]:
                failures.append(f"`{row}`: {ratio:.3f}x {against} is above the ceiling of {b['ceiling']}x that its "
                                f"acceptance holds to ({b.get('accepted', 'no record named')})")
                status = "ABOVE CEILING"
        if m and pct(v, m) > THRESHOLD:
            failures.append(f"`{row}`: {pct(v, m):+.2f}% instructions against main ({m:,.1f} -> {v:,.1f})")
            status += ", SLOWER"
        table.append((row, v, m, against, ratio, cap, met, status))
    # a guard row has no reference: only main holds it, to the same 0.5% rule
    for g in spec.get("guard", []):
        row = g["row"]
        v = instr[row]
        m = main["instr"][row] if main else None
        status = "guard"
        if m and pct(v, m) > THRESHOLD:
            failures.append(f"`{row}`: {pct(v, m):+.2f}% instructions against main ({m:,.1f} -> {v:,.1f})")
            status += ", SLOWER"
        table.append((row, v, m, "main", None, None, True, status))
    for r in spec["rss"]:
        f = r["program"]
        for against in r["against"]:
            ref = spec[against]["rss"][f]
            if rss[f] > ref:
                failures.append(f"`{f}`: peak RSS {rss[f]:,} KiB, above {against}'s {ref:,}")
        m = main["rss"][f] if main else None
        if m and pct(rss[f], m) > RSS_THRESHOLD:
            failures.append(f"`{f}`: peak RSS {pct(rss[f], m):+.1f}% against main ({m:,} -> {rss[f]:,} KiB)")
    return failures, notes, table


def cmd_rt_bars(a):
    with open(a.bars, "rb") as f:
        spec = tomllib.load(f)
    pin = pins().get("rt")
    if spec["gnur"]["version"] != pin:
        print(f"rt-bars: {os.path.relpath(a.bars, ROOT)} holds GNU R {spec['gnur']['version']}'s numbers, but the "
              f"rt pin (scripts/oracle-versions) is {pin}: measure GNU R {pin} in its oracle image "
              f"({spec['gnur']['source']}) and update them")
        return 1
    errors = [e for b in spec["bar"] for e in rt_acceptance_errors(b)]
    with open(RT_ROWS, "rb") as f:
        errors += rt_guard_errors(spec, {r["name"] for r in tomllib.load(f)["row"]})
    if errors:
        for e in errors:
            print(f"rt-bars: {os.path.relpath(a.bars, ROOT)}: {e}")
        return 1
    model, sig = cpu_signature()
    gnur_source = f"{spec['gnur']['source']} (measured on another machine)"
    gnur_doc = None
    if a.gnur:
        with open(a.gnur) as f:
            doc = json.load(f)
        gnur_doc = doc
        errors = rt_gnur_errors(doc, pin, sig)
        if errors:
            for e in errors:
                print(f"rt-bars: {a.gnur}: {e}")
            return 1
        spec = rt_with_gnur(spec, doc)
        gnur_source = f"GNU R {doc['version']} measured on this CPU signature ({a.gnur})"
    rows = [b["row"] for b in spec["bar"]] + [g["row"] for g in spec.get("guard", [])]
    files = [r["program"] for r in spec["rss"]]
    work = scratch_dir("rt-bars-")

    ranges = {"new": {}, "main": {}}

    def measure(binary, tag):
        got = rt_instr(binary, rows, a.jobs, work, tag, cycles=a.cycles, ranges=ranges[tag])
        instr, cyc = got if a.cycles else (got, None)
        return instr, cyc, rt_rss(binary, files, a.runs, work, tag)

    # which binaries these are: printed in the header and kept in the JSON.
    # main's is another commit by design, so only robigo-rt is checked
    # against this checkout's sources
    idents = {"robigo-rt": binary_identity(a.robigo_rt, "robigo-rtapi")}
    if a.main:
        idents["main"] = binary_identity(a.main)
    instr, cycles, rss = measure(a.robigo_rt, "new")
    # main's binary is measured on this machine too: instruction counts differ
    # between CPUs (the C library's dispatch), so another machine's numbers
    # would hide or invent a slowdown
    main = None
    if a.main:
        mi, mc, mr = measure(a.main, "main")
        main = {"instr": mi, "rss": mr, "cycles": mc}
    # the starts ([[startup]], scripts/rt-startup.py): each binary from the
    # trees its own build writes, so one after the other in this checkout
    starts = [e["name"] for e in spec.get("startup", [])] if not getattr(a, "no_startup", False) else []
    su = rt_startup() if starts else None
    runs = getattr(a, "startup_runs", 9)
    # (the trees packed by perf-rt-build in CI, --trees and --main-trees; built
    # here otherwise, which rt-startup.py refuses in CI)
    t_new = getattr(a, "trees", None) or True
    t_main = getattr(a, "main_trees", None) or True
    s_new = su.run(a.robigo_rt, starts, ROOT, runs, trees=t_new, valgrind=valgrind_path()) if su else {}
    s_main = su.run(a.main, starts, ROOT, runs, trees=t_main, valgrind=valgrind_path()) \
        if su and a.main else {}
    out = a.out or os.path.join(STATE, "perf", "rt-bars.json")
    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    with open(out, "w") as f:
        json.dump({"instr": instr, "rss": rss, "cycles": cycles, "main": main, "cpu": model, "cpu_signature": sig,
                   "shift_ranges": ranges, "binaries": idents,
                   "startup": {"new": s_new, "main": s_main or None}}, f, indent=1, sort_keys=True)
    failures, notes, table = rt_verdict(spec, instr, rss, main)
    if su:
        failures += su.verdict(s_new, s_main)
    print("### robigo-rt's M1 bars (scripts/perf.py rt-bars)\n")
    print(f"CPU: {model}, signature {sig}.")
    for label, ident in idents.items():
        report_identity(label, ident, out=sys.stdout)
    print(f"Callgrind instructions per iteration; bars against {gnur_source} and today's robigo.")
    against = "main's binary of the merge's base, on this runner" if main else "none (the bars only)"
    print(f"Against main: {against}.")
    if a.cycles:
        print(f"Estimated cycles under `{PINNED_CACHE}`: reported only, never failed on.")
    print()
    cyc_cols = " est. cycles | cycles change |" if a.cycles else ""
    print(f"| row | instructions | main | change | against | ratio | bar | |{cyc_cols}")
    print("|---|---:|---:|---:|---|---:|---:|---|" + ("---:|---:|" if a.cycles else ""))
    for row, v, m, against, ratio, cap, met, status in table:
        change = f"{pct(v, m):+.2f}%" if m else ""
        main_s = f"{m:,.1f}" if m else ""
        cyc = ""
        if a.cycles:
            mc = main["cycles"][row] if main else None
            cyc = f" {cycles[row]:,.1f} | {f'{pct(cycles[row], mc):+.2f}%' if mc else ''} |"
        if cap is None:
            # a guard row: main is its only reference
            print(f"| {row} | {v:,.1f} | {main_s} | {change} | main | | guard | {status} |{cyc}")
            continue
        print(f"| {row} | {v:,.1f} | {main_s} | {change} | {against} | {ratio:.3f}x | "
              f"{'' if met else '('}{cap}x{'' if met else ')'} | {status} |{cyc}")
    if ranges["new"]:
        ks = ", ".join(map(str, ADDRESS_SHIFTS))
        print(f"\nAddress-sensitive rows: the median over heap shifts k ∈ {{{ks}}} (k unrelated strings before "
              "the setup), with the range over them:")
        for row, (lo, hi) in sorted(ranges["new"].items()):
            m = ranges["main"].get(row)
            print(f"- {row}: {lo:,.1f}–{hi:,.1f}" + (f"; main {m[0]:,.1f}–{m[1]:,.1f}" if m else ""))
    print("\n| program | peak RSS KiB | main | bars |")
    print("|---|---:|---:|---|")
    for r in spec["rss"]:
        f = r["program"]
        m = main["rss"][f] if main else None
        refs = ", ".join(f"{ag} {spec[ag]['rss'][f]:,}" for ag in r["against"])
        print(f"| {f} | {rss[f]:,} | {f'{m:,}' if m else ''} | ≤ {refs} |")
    if su:
        g = gnur_doc.get("startup") if gnur_doc else None
        print("\nStarts (`--vanilla -e 1`, R_DEFAULT_PACKAGES=NULL; scripts/rt-startup.py): callgrind's total "
              f"instructions, the median over environment shifts; peak anonymous memory and peak RSS, the median of "
              f"{runs} runs with address randomization off. Against main: instructions +{su.INSTR_LIMIT}%, "
              f"anonymous memory +{su.ANON_LIMIT}% (at least {su.ANON_FLOOR_KIB} KiB), peak RSS +{su.RSS_LIMIT}%. "
              + ("GNU R's beside them, measured on this runner, report only." if g else "GNU R's not measured here.")
              + "\n")
        for ln in su.table(spec["startup"], s_new, s_main, g):
            print(ln)
    print("\n(A bar in parentheses is not met yet: only regressions against main fail it, and an accepted "
          "regression's ceiling. A guard row has no bar: only regressions against main fail it.)")
    for n in notes:
        print(f"\nNOTE {n}")
    for x in failures:
        print(f"\nFAIL {x}")
    print(f"\n{len(failures)} failure(s); thresholds: {THRESHOLD}% instructions, {RSS_THRESHOLD}% peak RSS against main")
    return 1 if failures else 0


def cmd_list(_):
    for p in programs(conf()):
        print(p[0])


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list")
    sub.add_parser("fetch")
    p = sub.add_parser("run")
    p.add_argument("--metric", choices=["instr", "time", "both", "cache"], default="instr",
                   help="cache adds callgrind's simulated cache and branch counters "
                        "(diagnostic; never a gate input)")
    p.add_argument("--runtime", choices=["robigo", "gnur", "both"], default="both")
    p.add_argument("--suite")
    p.add_argument("--filter")
    p.add_argument("--jobs", type=int, default=os.cpu_count())
    p.add_argument("--runs", type=int, default=3)
    p.add_argument("--timeout", type=int, default=120, help="seconds per run (x60 under callgrind)")
    p.add_argument("--robigo")
    p.add_argument("--oracle", help="path to baseline perf.json providing GNU R oracle outputs and counts")
    p.add_argument("--out")
    p.add_argument("--merge", action="store_true", help="keep results of other runtimes from --out")
    p.add_argument("--quiet", action="store_true")
    p = sub.add_parser("compare")
    p.add_argument("base")
    p.add_argument("new")
    p.add_argument("--threshold", type=float, default=THRESHOLD)
    p = sub.add_parser("recheck", help="compare, re-measuring the benchmarks found slower once")
    p.add_argument("base")
    p.add_argument("new")
    p.add_argument("--threshold", type=float, default=THRESHOLD)
    p.add_argument("--robigo")
    p.add_argument("--jobs", type=int, default=os.cpu_count())
    p.add_argument("--timeout", type=int, default=120, help="seconds per run (x60 under callgrind)")
    p = sub.add_parser("report")
    p.add_argument("file")
    p.add_argument("--markdown", action="store_true")
    p = sub.add_parser("rt-bars", help="robigo-rt's M1 bars (benchmarks/rt/m1-bars.toml)")
    p.add_argument("--robigo-rt", required=True, help="the robigo-rt binary")
    p.add_argument("--main", help="main's robigo-rt (the merge's base), measured beside it")
    p.add_argument("--bars", default=RT_BARS)
    p.add_argument("--jobs", type=int, default=os.cpu_count())
    p.add_argument("--runs", type=int, default=3, help="peak RSS: scripts/rss.py runs per program")
    p.add_argument("--out", help="this run's numbers (default .robigo/perf/rt-bars.json)")
    p.add_argument("--gnur", help="GNU R's side, measured on this machine by `rt-gnur` (default: the bars file's)")
    p.add_argument("--cycles", action="store_true", help="also estimated cycles (PINNED_CACHE), reported only")
    p.add_argument("--startup-runs", type=int, default=9, help="the starts' memory runs per start (default 9)")
    p.add_argument("--trees", help="the R_HOME trees robigo-rt's build wrote, a tarball (rt-startup.py "
                                   "--pack-trees); built here when not given (not in CI)")
    p.add_argument("--main-trees", help="the same for main's binary")
    p.add_argument("--no-startup", action="store_true",
                   help="skip the starts (they rebuild the checkout's .robigo/rt-home-db per binary)")
    p = sub.add_parser("rt-gnur", help="GNU R's side of the M1 bars, measured on this machine")
    p.add_argument("--bars", default=RT_BARS)
    p.add_argument("--jobs", type=int, default=os.cpu_count())
    p.add_argument("--runs", type=int, default=3, help="peak RSS: scripts/rss.py runs per program")
    p.add_argument("--out", required=True)
    a = ap.parse_args()
    fn = {"list": cmd_list, "fetch": cmd_fetch, "run": cmd_run, "compare": cmd_compare, "recheck": cmd_recheck,
          "report": cmd_report, "rt-bars": cmd_rt_bars, "rt-gnur": cmd_rt_gnur}[a.cmd]
    sys.exit(fn(a) or 0)


if __name__ == "__main__":
    main()
