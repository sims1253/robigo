#!/usr/bin/env python3
"""Test corpora: R code whose expected behaviour is GNU R 4.5.0's.

    scripts/corpus.py list
    scripts/corpus.py import NAME             # (re)vendor cases from upstream
    scripts/corpus.py record NAME [--jobs N]  # run GNU R, write expected.jsonl.gz
    scripts/corpus.py run NAME [--jobs N] [--filter RE] [--shard I/N] [--update]
    scripts/corpus.py show NAME ID            # source, expected and actual output
    scripts/corpus.py score [--markdown]      # pass counts of every corpus
    scripts/corpus.py absorb RESULTS.json...  # add passes from CI results to passing.txt

A corpus lives in tests/corpora/NAME/ (see tests/corpora/README.md):
cases (cases/*.R or cases.jsonl), expected.jsonl.gz (GNU R's stdout,
stderr and exit status per case), passing.txt (the ratchet). `run`
compares robigo's output with the expected bytes; a case passes only if
stdout, stderr and the exit status all match. Cases in passing.txt that
fail are regressions and make `run` exit with status 1. `--update` adds
new passes to passing.txt (it never removes any). Results of the last run
are written to .robigo/results/NAME.json.

The robigo binary is $ROBIGO, else .robigo/bin/robigo (scripts/build.sh),
else the release binary in cargo's target directory.
"""
import argparse
import base64
import concurrent.futures
import gzip
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import tomllib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CORPORA = os.path.join(ROOT, "tests", "corpora")
STATE = os.path.join(ROOT, ".robigo")
RSRC = os.environ.get("ROBIGO_RSRC", os.path.expanduser("~/.cache/robigo-research/rsrc/R-4.5.0"))

# The oracle's environment, for GNU R and robigo alike.
BASE_ENV = {"LANGUAGE": "en", "LC_ALL": "C.UTF-8", "TZ": "UTC", "R_ENVIRON_USER": "", "R_PROFILE_USER": ""}

# How each mode runs a case file. `{exe}` is Rscript/R or robigo.
MODES = {
    # Like `Rscript --vanilla FILE`: no echo.
    "script": {"gnur": ["Rscript", "--vanilla", "{file}"], "robigo": ["{robigo}", "--vanilla", "{file}"]},
    # Like R CMD check runs tests and examples: the transcript echoes each
    # top-level expression after a prompt.
    "echo": {
        "gnur": ["R", "--vanilla", "--quiet", "--no-save", "-f", "{file}"],
        "robigo": ["{robigo}", "--vanilla", "--quiet", "--no-save", "-f", "{file}"],
    },
}

# What differs from run to run in any implementation, masked as GNU R's own
# tools::Rdiff does for `make check` (R: tools/R/testing.R:Rdiff):
# addresses, `Time ...` lines, and the timings `proc.time()` prints.
ADDRESS = re.compile(rb"<(environment|bytecode|pointer|promise|weak reference|externalptr): 0x[0-9a-f]+>")
TIME_LINE = re.compile(rb"^Time .*$", re.M)
PROC_TIME = re.compile(rb"^(> proc\.time\(\)\n)[^\n]*\n[^\n]*(\n|$)", re.M)


def normalize(b):
    b = ADDRESS.sub(lambda m: b"<" + m.group(1) + b": 0x>", b)
    b = TIME_LINE.sub(b"Time <masked>", b)
    return PROC_TIME.sub(lambda m: m.group(1) + b"<proc.time masked>" + m.group(2), b)


def manifest():
    with open(os.path.join(CORPORA, "corpora.toml"), "rb") as f:
        return tomllib.load(f)


def corpus_conf(name):
    m = manifest()
    if name not in m:
        sys.exit(f"unknown corpus '{name}' (have: {', '.join(m)})")
    return m[name]


def cdir(name):
    return os.path.join(CORPORA, name)


# ---------------------------------------------------------------- cases

def load_cases(name):
    """[(id, source_bytes)] in a stable order."""
    d = cdir(name)
    jl = os.path.join(d, "cases.jsonl")
    out = []
    if os.path.exists(jl):
        with open(jl, encoding="utf-8") as f:
            for line in f:
                c = json.loads(line)
                out.append((c["id"], c["code"].encode("utf-8")))
    else:
        cases = os.path.join(d, "cases")
        for root, _, files in os.walk(cases):
            for fn in files:
                if fn.endswith(".R"):
                    p = os.path.join(root, fn)
                    with open(p, "rb") as f:
                        out.append((os.path.relpath(p, cases)[:-2], f.read()))
    out.sort()
    return out


def sha(b):
    return hashlib.sha256(b).hexdigest()[:16]


def shard_of(case_id, n):
    return int(hashlib.sha1(case_id.encode()).hexdigest(), 16) % n


# ------------------------------------------------------------- running

def run_case(name, conf, case_id, src, runtime, robigo=None):
    """Run one case; returns (stdout, stderr, status) as bytes/int."""
    mode = MODES[conf.get("mode", "script")]
    env = dict(os.environ, **BASE_ENV)
    env["R_DEFAULT_PACKAGES"] = conf.get("default_packages", "NULL")
    env["R_LIBS_USER"] = os.path.join(STATE, "empty-lib")
    os.makedirs(env["R_LIBS_USER"], exist_ok=True)
    for k, v in conf.get("env", {}).items():
        env[k] = v
    support = conf.get("support")
    # A working directory whose path depends only on the case, so output
    # that shows it (getwd(), normalizePath()) is the same in every run.
    tmp = os.path.join(tempfile.gettempdir(), "robigo-corpus", name, hashlib.sha1(case_id.encode()).hexdigest()[:12])
    shutil.rmtree(tmp, ignore_errors=True)
    os.makedirs(tmp)
    try:
        if support:
            sdir = os.path.join(cdir(name), support)
            for fn in os.listdir(sdir):
                os.symlink(os.path.join(sdir, fn), os.path.join(tmp, fn))
        file = os.path.join(tmp, "case.R")
        if os.path.lexists(file):
            os.unlink(file)
        with open(file, "wb") as f:
            f.write(src)
        env["HOME"] = tmp
        cmd = [a.format(file=file, robigo=robigo or "") for a in mode[runtime]]
        try:
            p = subprocess.run(cmd, cwd=tmp, env=env, capture_output=True, timeout=conf.get("timeout", 60))
            return p.stdout, p.stderr, p.returncode
        except subprocess.TimeoutExpired as e:
            return e.stdout or b"", (e.stderr or b"") + b"\n<robigo-corpus: timeout>\n", -1
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def enc(b):
    try:
        return {"t": b.decode("utf-8")}
    except UnicodeDecodeError:
        return {"b64": base64.b64encode(b).decode()}


def dec(d):
    return d["t"].encode("utf-8") if "t" in d else base64.b64decode(d["b64"])


def load_expected(name):
    p = os.path.join(cdir(name), "expected.jsonl.gz")
    if not os.path.exists(p):
        return {}
    out = {}
    with gzip.open(p, "rt", encoding="utf-8") as f:
        for line in f:
            e = json.loads(line)
            out[e["id"]] = e
    return out


def write_expected(name, exp):
    p = os.path.join(cdir(name), "expected.jsonl.gz")
    # mtime=0: the file only changes when its content does.
    with open(p, "wb") as raw, gzip.GzipFile(fileobj=raw, mode="wb", mtime=0) as gz:
        for k in sorted(exp):
            gz.write((json.dumps(exp[k], sort_keys=True, ensure_ascii=False) + "\n").encode("utf-8"))


def load_passing(name):
    p = os.path.join(cdir(name), "passing.txt")
    if not os.path.exists(p):
        return set()
    with open(p) as f:
        return {l.strip() for l in f if l.strip() and not l.startswith("#")}


def write_passing(name, ids):
    with open(os.path.join(cdir(name), "passing.txt"), "w") as f:
        for i in sorted(ids):
            f.write(i + "\n")


def robigo_path(arg):
    if arg:
        return os.path.abspath(arg)
    if os.environ.get("ROBIGO"):
        return os.environ["ROBIGO"]
    local = os.path.join(STATE, "bin", "robigo")
    if os.path.exists(local):
        return local
    try:
        md = json.loads(subprocess.run(["cargo", "metadata", "--format-version", "1", "--no-deps"], cwd=ROOT,
                                       capture_output=True, check=True).stdout)
        return os.path.join(md["target_directory"], "release", "robigo")
    except Exception:
        return os.path.join(ROOT, "target", "release", "robigo")


def pmap(fn, items, jobs, label):
    """Run fn over items in parallel, with a progress line on stderr."""
    out = []
    t0 = time.time()
    with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as ex:
        futs = [ex.submit(fn, it) for it in items]
        for k, fut in enumerate(concurrent.futures.as_completed(futs), 1):
            out.append(fut.result())
            if sys.stderr.isatty() and (k % 25 == 0 or k == len(futs)):
                rate = k / max(time.time() - t0, 1e-9)
                print(f"\r{label}: {k}/{len(futs)} ({rate:.1f}/s)", end="", file=sys.stderr, flush=True)
    if sys.stderr.isatty():
        print(file=sys.stderr)
    return out


# ------------------------------------------------------------- commands

def cmd_list(_):
    m = manifest()
    for name, conf in m.items():
        n = len(load_cases(name)) if os.path.isdir(cdir(name)) else 0
        print(f"{name:16} {n:6} cases  {conf.get('mode', 'script'):6}  {conf.get('description', '')}")


def cmd_record(a):
    conf = corpus_conf(a.name)
    cases = load_cases(a.name)
    if a.filter:
        rx = re.compile(a.filter)
        cases = [c for c in cases if rx.search(c[0])]
    old = load_expected(a.name)
    todo = cases if a.all else [c for c in cases if old.get(c[0], {}).get("sha") != sha(c[1])]
    ver = subprocess.run(["R", "--version"], capture_output=True, text=True).stdout.splitlines()[0]
    if "4.5.0" not in ver:
        sys.exit(f"the oracle must be GNU R 4.5.0, found: {ver}")

    def one(c):
        cid, src = c
        runs = [run_case(a.name, conf, cid, src, "gnur") for _ in range(a.repeat)]
        o, e, s = runs[0]
        rec = {"id": cid, "sha": sha(src), "out": enc(o), "err": enc(e), "status": s}
        # Lines that differ between runs (timings, dates) are masked; if the
        # runs differ in more than single lines, the case is unstable.
        for key, idx in (("out", 0), ("err", 1)):
            lines = [normalize(r[idx]).split(b"\n") for r in runs]
            if any(len(l) != len(lines[0]) for l in lines):
                rec["unstable"] = True
                continue
            mask = sorted({i for l in lines[1:] for i, (x, y) in enumerate(zip(lines[0], l)) if x != y})
            if mask:
                rec["mask_" + key] = mask
        if any(r[2] != s for r in runs):
            rec["unstable"] = True
        return rec

    recs = pmap(one, todo, a.jobs, f"record {a.name}")
    keep = {c[0] for c in cases} if not a.filter else None
    exp = {k: v for k, v in old.items() if keep is None or k in keep}
    for r in recs:
        exp[r["id"]] = r
    write_expected(a.name, exp)
    unstable = sum(1 for r in recs if r.get("unstable"))
    partly = sum(1 for r in recs if not r.get("unstable") and ("mask_out" in r or "mask_err" in r))
    print(f"recorded {len(recs)} case(s) of {a.name} with {ver}; {partly} with varying lines masked, "
          f"{unstable} unstable (excluded from scoring)")


def masked(b, mask):
    b = normalize(b)
    if not mask:
        return b
    lines = b.split(b"\n")
    for i in mask:
        if i < len(lines):
            lines[i] = b"<masked>"
    return b"\n".join(lines)


def classify(exp, o, e, s):
    """Which of stdout/stderr/status differ."""
    kinds = []
    if masked(o, exp.get("mask_out")) != masked(dec(exp["out"]), exp.get("mask_out")):
        kinds.append("stdout")
    if masked(e, exp.get("mask_err")) != masked(dec(exp["err"]), exp.get("mask_err")):
        kinds.append("stderr")
    if s != exp["status"]:
        kinds.append("status")
    return kinds


def cmd_run(a):
    conf = corpus_conf(a.name)
    robigo = robigo_path(a.robigo)
    if not os.path.exists(robigo):
        sys.exit(f"no robigo binary at {robigo}; run scripts/build.sh")
    exp = load_expected(a.name)
    if not exp:
        sys.exit(f"{a.name} has no expected output; run: scripts/corpus.py record {a.name}")
    cases = [c for c in load_cases(a.name) if c[0] in exp and not exp[c[0]].get("unstable")]
    stale = [c[0] for c in cases if exp[c[0]]["sha"] != sha(c[1])]
    if stale:
        sys.exit(f"{len(stale)} case(s) changed since recording (e.g. {stale[0]}); run: scripts/corpus.py record {a.name}")
    if a.filter:
        rx = re.compile(a.filter)
        cases = [c for c in cases if rx.search(c[0])]
    if a.shard:
        i, n = map(int, a.shard.split("/"))
        cases = [c for c in cases if shard_of(c[0], n) == i - 1]
    passing = load_passing(a.name)

    def one(c):
        cid, src = c
        t = time.time()
        o, e, s = run_case(a.name, conf, cid, src, "robigo", robigo)
        kinds = classify(exp[cid], o, e, s)
        r = {"id": cid, "pass": not kinds, "differs": kinds, "secs": round(time.time() - t, 3)}
        if kinds and a.keep_output:
            r["out"], r["err"], r["status"] = enc(o), enc(e), s
        return r

    res = sorted(pmap(one, cases, a.jobs, f"run {a.name}"), key=lambda r: r["id"])
    ok = {r["id"] for r in res if r["pass"]}
    ran = {r["id"] for r in res}
    regressions = sorted((passing & ran) - ok)
    new = sorted(ok - passing)
    count = lambda k: sum(1 for r in res if k not in r["differs"])
    os.makedirs(os.path.join(STATE, "results"), exist_ok=True)
    summary = {
        "corpus": a.name, "cases": len(res), "pass": len(ok), "stdout": count("stdout"),
        "stderr": count("stderr"), "status": count("status"), "regressions": regressions, "new": new,
    }
    out = os.path.join(STATE, "results", a.name + (f".shard{a.shard.replace('/', 'of')}" if a.shard else "") + ".json")
    with open(out, "w") as f:
        json.dump({"summary": summary, "results": res}, f, indent=0)
    print(f"{a.name}: {len(ok)}/{len(res)} pass; stdout {summary['stdout']}, stderr {summary['stderr']}, "
          f"status {summary['status']} match; {len(new)} new, {len(regressions)} regressions")
    for cid in regressions[:50]:
        print(f"  REGRESSION {cid}")
    if a.update and new:
        write_passing(a.name, passing | ok)
        print(f"added {len(new)} case(s) to tests/corpora/{a.name}/passing.txt")
    elif new and not a.quiet:
        print(f"  ({len(new)} new passes; --update records them)")
    return 1 if regressions else 0


def cmd_show(a):
    conf = corpus_conf(a.name)
    cases = dict(load_cases(a.name))
    if a.id not in cases:
        sys.exit(f"no case {a.id}")
    src = cases[a.id]
    exp = load_expected(a.name).get(a.id)
    print("== source\n" + src.decode("utf-8", "replace"))
    o, e, s = run_case(a.name, conf, a.id, src, "robigo", robigo_path(a.robigo))
    if exp:
        for label, want, got in (("stdout", dec(exp["out"]), o), ("stderr", dec(exp["err"]), e)):
            if normalize(want) == normalize(got):
                print(f"== {label}: same")
                continue
            import difflib
            print(f"== {label} (--- GNU R, +++ robigo)")
            for line in difflib.unified_diff(want.decode("utf-8", "replace").splitlines(),
                                             got.decode("utf-8", "replace").splitlines(), lineterm="", n=2):
                print(line)
        print(f"== status: GNU R {exp['status']}, robigo {s}")
    else:
        print("== (not recorded)\n" + o.decode("utf-8", "replace") + e.decode("utf-8", "replace"))


def cmd_score(a):
    rows = []
    for name, conf in manifest().items():
        if not os.path.isdir(cdir(name)):
            continue
        exp = load_expected(name)
        scored = sum(1 for e in exp.values() if not e.get("unstable"))
        rows.append((name, len(load_passing(name)), scored, conf.get("description", "")))
    if a.markdown:
        print("| corpus | passing | cases | |\n|---|---|---|---|")
        for n, p, t, d in rows:
            print(f"| {n} | {p} | {t} | {100 * p / max(t, 1):.1f}% |")
    else:
        for n, p, t, d in rows:
            print(f"{n:16} {p:6}/{t:<6} {100 * p / max(t, 1):5.1f}%  {d}")


def cmd_absorb(a):
    """Add the passes recorded in result files (e.g. CI artifacts) to the ratchets."""
    by = {}
    for fn in a.files:
        with open(fn) as f:
            doc = json.load(f)
        by.setdefault(doc["summary"]["corpus"], set()).update(r["id"] for r in doc["results"] if r["pass"])
    for name, ok in sorted(by.items()):
        exp = load_expected(name)
        ok = {i for i in ok if i in exp}
        old = load_passing(name)
        if ok - old:
            write_passing(name, old | ok)
        print(f"{name}: {len(ok - old)} new pass(es) added")


# --------------------------------------------------------------- import

def import_fastr(conf):
    """FastR's ExpectedTestOutput.test: `##TestName#options` then `#code`."""
    url = f"https://raw.githubusercontent.com/oracle/fastr/{conf['commit']}/{conf['path']}"
    text = subprocess.run(["curl", "-sSfL", url], capture_output=True, check=True).stdout.decode("utf-8")
    lines = text.split("\n")
    snippets = {}
    for i, line in enumerate(lines):
        if line.startswith("##com.oracle") and i + 1 < len(lines) and lines[i + 1].startswith("#"):
            test = line[2:].split("#")[0].removeprefix("com.oracle.truffle.r.test.")
            if any(test.startswith(x) for x in conf.get("exclude_prefix", [])):
                continue
            code = lines[i + 1][1:]
            cid = test.rsplit(".", 1)[0].replace(".", "/") + "/" + sha(code.encode())
            snippets.setdefault(code, {"id": cid, "code": code + "\n", "tests": []})["tests"].append(test)
    with open(os.path.join(cdir("fastr"), "cases.jsonl"), "w", encoding="utf-8") as f:
        for s in sorted(snippets.values(), key=lambda s: s["id"]):
            f.write(json.dumps(s, ensure_ascii=False) + "\n")
    print(f"fastr: {len(snippets)} distinct snippets")


def import_gnur_tests(conf):
    src = os.path.join(RSRC, "tests")
    d = cdir("gnur-tests")
    for sub in ("cases", "support"):
        shutil.rmtree(os.path.join(d, sub), ignore_errors=True)
        os.makedirs(os.path.join(d, sub))
    skip = set(conf.get("exclude", []))
    n = 0
    for fn in sorted(os.listdir(src)):
        p = os.path.join(src, fn)
        if not os.path.isfile(p):
            continue
        if fn.endswith(".R") and fn not in skip:
            shutil.copy(p, os.path.join(d, "cases", fn))
            n += 1
        elif not re.search(r"\.(R|Rout\.save|Rin|in|win|common|install)$|^Makefile|^README", fn):
            shutil.copy(p, os.path.join(d, "support", fn))
    print(f"gnur-tests: {n} scripts")


EXTRACT_EXAMPLES = r"""
args <- commandArgs(TRUE); out <- args[1]; pkgs <- args[-1]
attached <- c("base", "stats", "graphics", "grDevices", "utils", "datasets")
for (p in pkgs) {
  db <- tools::Rd_db(p)
  dir.create(file.path(out, p), recursive = TRUE, showWarnings = FALSE)
  for (f in names(db)) {
    tf <- tempfile()
    tools::Rd2ex(db[[f]], tf, commentDontrun = TRUE, commentDonttest = TRUE)
    if (!file.exists(tf)) next
    ex <- readLines(tf, encoding = "UTF-8")
    # As R CMD check's cleanEx() before each example (share/R/examples-header.R).
    pre <- c('RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)',
             if (!(p %in% attached)) sprintf("library(%s)", p))
    writeLines(c(pre, ex), file.path(out, p, sub("\\.Rd$", ".R", basename(f))), useBytes = TRUE)
  }
}
"""


def import_gnur_examples(conf):
    d = cdir("gnur-examples")
    shutil.rmtree(os.path.join(d, "cases"), ignore_errors=True)
    os.makedirs(os.path.join(d, "cases"))
    with tempfile.NamedTemporaryFile("w", suffix=".R", delete=False) as f:
        f.write(EXTRACT_EXAMPLES)
    env = dict(os.environ, **BASE_ENV, R_DEFAULT_PACKAGES="NULL")
    subprocess.run(["Rscript", "--vanilla", f.name, os.path.join(d, "cases")] + conf["packages"], check=True, env=env)
    os.unlink(f.name)
    n = sum(len(fs) for _, _, fs in os.walk(os.path.join(d, "cases")))
    print(f"gnur-examples: {n} help pages with examples")


IMPORTERS = {"fastr": import_fastr, "gnur-tests": import_gnur_tests, "gnur-examples": import_gnur_examples}


def cmd_import(a):
    conf = corpus_conf(a.name)
    os.makedirs(cdir(a.name), exist_ok=True)
    IMPORTERS[conf.get("importer", a.name)](conf)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list")
    p = sub.add_parser("import")
    p.add_argument("name")
    p = sub.add_parser("record")
    p.add_argument("name")
    p.add_argument("--jobs", type=int, default=os.cpu_count())
    p.add_argument("--repeat", type=int, default=2, help="runs per case to detect unstable output")
    p.add_argument("--all", action="store_true", help="re-record unchanged cases too")
    p.add_argument("--filter")
    p = sub.add_parser("run")
    p.add_argument("name")
    p.add_argument("--jobs", type=int, default=os.cpu_count())
    p.add_argument("--filter")
    p.add_argument("--shard")
    p.add_argument("--robigo")
    p.add_argument("--update", action="store_true")
    p.add_argument("--keep-output", action="store_true", help="store robigo's output of failing cases")
    p.add_argument("--quiet", action="store_true")
    p = sub.add_parser("show")
    p.add_argument("name")
    p.add_argument("id")
    p.add_argument("--robigo")
    p = sub.add_parser("score")
    p.add_argument("--markdown", action="store_true")
    p = sub.add_parser("absorb")
    p.add_argument("files", nargs="+")
    a = ap.parse_args()
    fn = {"list": cmd_list, "import": cmd_import, "record": cmd_record, "run": cmd_run, "show": cmd_show,
          "score": cmd_score, "absorb": cmd_absorb}[a.cmd]
    sys.exit(fn(a) or 0)


if __name__ == "__main__":
    main()
