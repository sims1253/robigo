#!/usr/bin/env python3
"""Test corpora: R code whose expected behaviour is GNU R's (4.5.0 by default).

    scripts/corpus.py list
    scripts/corpus.py import NAME             # (re)vendor cases from upstream
    scripts/corpus.py record NAME [--jobs N]  # run GNU R, write expected.jsonl.gz
    scripts/corpus.py run NAME [--jobs N] [--filter RE] [--shard I/N] [--update] [--tier T] [--net-fixture]
    scripts/corpus.py show NAME ID            # source, expected and actual output
    scripts/corpus.py score [--markdown]      # pass counts of every corpus
    scripts/corpus.py absorb RESULTS.json...  # add passes from CI results to the ratchet
    scripts/corpus.py absorb --from-run [ID]  # the same, fetching a main push run's corpus artifacts (gh)
    scripts/corpus.py absorb ... --dry-run    # list what would be added (run those here first)

`record`, `run` and `show` take `--r-version V` (default $ROBIGO_R_VERSION,
else 4.5.0): the GNU R version whose recordings they write or compare with.
4.5.0's are the corpus's `expected_file`, which the ratchet (`passing/`)
holds the old runtime to; another version's are `R-V/<expected_file>`,
with the same case ids. A run against another version only reports: it
writes NAME.R-V.json and cannot update the ratchet.

A corpus lives in tests/corpora/NAME/ (see tests/corpora/README.md):
cases (cases/*.R or cases.jsonl), expected.jsonl.gz (GNU R's stdout,
stderr and exit status per case), passing/ (the ratchet: one empty marker
file per case, `<id>.pass`). `run` compares robigo's output with the
expected bytes; a case passes only if stdout, stderr and the exit status
all match. Cases in the ratchet that fail are regressions and make `run`
exit with status 1. `--update` ratchets new passes (it never removes any).
Results of the last run are written to .robigo/results/NAME.json.

`run` and `show` take `--runtime robigo|rt` (default robigo). `rt` runs
robigo-rt, G3's runtime (`--robigo`, else $ROBIGO_RT_BIN, else the target's
release robigo-rt), against the recordings of its own pin (`rt` in
scripts/oracle-versions), with its own ratchet, `passing-rt/`. It refuses,
with the reason, the corpora robigo-rt cannot run yet: those with a build
step. Each
failing case records the first thing it found missing (`blocked`: a
primitive or `.Internal` stub, another "not implemented yet", or a function
base does not bind), and the run prints them ranked by the cases each
blocks.

`--tier interp|vm|jit` runs every case in one execution tier (the AST
interpreter, the bytecode VM without the JIT, or both; unset is `jit`).
The VM and the JIT must not change behaviour, so a ratcheted case that
fails with `--tier vm` is a regression (CI runs it: the VM is the top
tier on wasm32). `interp` is a diagnostic: GNU R's own AST interpreter
differs from its bytecode in a few places robigo reproduces (the
`value =` argument of a complex assignment's error call, what `trace()`
prints), and GNU R never runs base's code in it. `--update` and `absorb`
only take default runs.

`--net-fixture` (CI) serves a corpus's `net_fixture` (corpora.toml: a
directory, the files it must hold and the hosts it stands in for) on
127.0.0.1:80 and points the hosts there in /etc/hosts for the run, so cases
that fetch from those hosts need no network. It runs only as root inside a
container (/.dockerenv, or CI=true), so it never edits a host's
/etc/hosts; the file is restored afterwards, on an error or SIGTERM/SIGINT
too.

The robigo binary is $ROBIGO, else .robigo/bin/robigo (scripts/build.sh),
else the release binary in cargo's target directory.
"""
import argparse
import base64
import concurrent.futures
import contextlib
import functools
import gzip
import hashlib
import http.server
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import tomllib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CORPORA = os.path.join(ROOT, "tests", "corpora")
STATE = os.path.join(ROOT, ".robigo")
RSRC = os.environ.get("ROBIGO_RSRC", os.path.expanduser("~/.cache/robigo-research/rsrc/R-4.5.0"))

# The pinned GNU R versions, NAME=VERSION lines: the one list the conformance
# runner and the other scripts read too.
PINS = os.path.join(ROOT, "scripts", "oracle-versions")


def pins():
    """scripts/oracle-versions' NAME -> X.Y.Z, in the format its header
    states (and crates/robigo-conformance's `pin_in` reads): LF lines, each
    empty, a comment (`#` first) or NAME=X.Y.Z, each NAME once."""
    out = {}
    with open(PINS, newline="") as f:
        for n, line in enumerate(f.read().split("\n"), 1):
            if not line or line.startswith("#"):
                continue
            m = re.fullmatch(r"([a-z]+)=([0-9]+\.[0-9]+\.[0-9]+)", line)
            if not m:
                crlf = " (CRLF?)" if line.endswith("\r") else ""
                raise SystemExit(f"{PINS}:{n}: not NAME=X.Y.Z, a # comment or empty{crlf}: {line!r}")
            if m[1] in out:
                raise SystemExit(f"{PINS}:{n}: {m[1]} is pinned twice")
            out[m[1]] = m[2]
    return out


# The GNU R version the old runtime is gated on (`gated`): the ratchet holds
# runs against its recordings, which sit at a corpus's `expected_file`.
# Another version V's sit at R-V/<expected_file> (docs/verification.md,
# "Oracles").
GATED_R_VERSION = pins()["gated"]
# The versions there are oracles for: scripts/build-oracle.sh and the
# oracle images build them from these pinned tarballs.
R_TARBALLS = os.path.join(ROOT, "scripts", "r-tarballs.sha256")


def oracle_versions():
    with open(R_TARBALLS) as f:
        return [m.group(1) for m in (re.fullmatch(r"[0-9a-f]{64}  R-(.+)\.tar\.gz", l.strip()) for l in f) if m]


def r_version(arg=None):
    """The GNU R version to record or compare with: `--r-version`, else
    $ROBIGO_R_VERSION, else 4.5.0. Only pinned oracle versions qualify: a
    host's R of another version must never record."""
    v = arg or os.environ.get("ROBIGO_R_VERSION") or GATED_R_VERSION
    if v not in oracle_versions():
        sys.exit(f"R {v} is not an oracle version (have: {', '.join(oracle_versions())}; {os.path.relpath(R_TARBALLS, ROOT)})")
    return v


def expected_file(conf, version):
    """A corpus's recordings for GNU R `version`, relative to its directory."""
    f = conf.get("expected_file", "expected.jsonl.gz")
    return f if version == GATED_R_VERSION else os.path.join(f"R-{version}", f)


# The oracle's environment, for GNU R and robigo alike.
BASE_ENV = {"LANGUAGE": "en", "LC_ALL": "C.UTF-8", "TZ": "UTC", "R_ENVIRON_USER": "", "R_PROFILE_USER": ""}

# The execution tiers `run --tier` selects (crates/robigo-runtime/src/vm.rs,
# jit.rs). Unset, robigo runs the VM with the JIT, which is `jit`.
TIERS = {
    "interp": {"ROBIGO_COMPILE": "0", "ROBIGO_JIT": "0"},
    "vm": {"ROBIGO_COMPILE": "1", "ROBIGO_JIT": "0"},
    "jit": {"ROBIGO_COMPILE": "1", "ROBIGO_JIT": "1"},
}

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
# gc() cell counts describe GNU R allocator state, which robigo does not share.
ADDRESS = re.compile(rb"<(environment|bytecode|pointer|promise|weak reference|externalptr): 0x[0-9a-f]+>")
TIME_LINE = re.compile(rb"^Time .*$", re.M)
PROC_TIME = re.compile(rb"^(> proc\.time\(\)\n)[^\n]*\n[^\n]*(\n|$)", re.M)

GC_HEADER = re.compile(
    rb"^[ \t]+used \(Mb\) gc trigger \(Mb\)(?: limit \(Mb\))? max used \(Mb\)(?=\nNcells[ \t])",
    re.M,
)
GC_CELLS = re.compile(
    rb"^(Ncells|Vcells)[ \t]+(?:NA|[0-9]+(?:\.[0-9]+)?)(?:[ \t]+(?:NA|[0-9]+(?:\.[0-9]+)?)){5}(?:(?:[ \t]+(?:NA|[0-9]+(?:\.[0-9]+)?)){2})?[ \t]*$",
    re.M,
)


def normalize(b):
    b = ADDRESS.sub(lambda m: b"<" + m.group(1) + b": 0x>", b)
    b = TIME_LINE.sub(b"Time <masked>", b)
    b = GC_HEADER.sub(b"<gc header masked>", b)
    b = GC_CELLS.sub(lambda m: m.group(1) + b" <gc masked>", b)
    return PROC_TIME.sub(lambda m: m.group(1) + b"<proc.time masked>" + m.group(2), b)


# Lines between these markers are not compared, as GNU R's tools::Rdiff
# drops them for `R CMD check` (R: tools/R/testing.R:Rdiff, documented in
# ?Rdiff): output that is allowed to differ, such as today's date.
IGNORE_BEGIN, IGNORE_END = b"> ## IGNORE_RDIFF_BEGIN", b"> ## IGNORE_RDIFF_END"


def rdiff_ignore(b):
    """`txt[cumsum(txt == BEGIN) <= cumsum(txt == END)]`: drops each BEGIN
    marker and what follows it, keeps the END marker."""
    if IGNORE_BEGIN not in b:
        return b
    out, begins, ends = [], 0, 0
    for line in b.split(b"\n"):
        begins += line == IGNORE_BEGIN
        ends += line == IGNORE_END
        if begins <= ends:
            out.append(line)
    return b"\n".join(out)


def manifest():
    with open(os.path.join(CORPORA, "corpora.toml"), "rb") as f:
        return tomllib.load(f)


def corpus_conf(name, profile=None):
    m = manifest()
    if name not in m:
        sys.exit(f"unknown corpus '{name}' (have: {', '.join(m)})")
    conf = m[name]
    if profile:
        if profile not in conf.get("profiles", {}):
            sys.exit(f"unknown profile {profile!r} for corpus {name!r}")
        conf = dict(conf, **conf["profiles"][profile])
    return conf


def cdir(name):
    return os.path.join(CORPORA, name)


def passing_dir(name, runtime="robigo"):
    return os.path.join(cdir(name), RUNTIMES[runtime]["ratchet"])


# Ratchet marker suffix: `passing/<case id>.pass`, one file per case.
MARKER = ".pass"

# The runtimes `run` and `show` compare with GNU R: today's robigo, held to
# the gated version's recordings by `passing/`, and robigo-rt, G3's runtime,
# held to the rt pin's by `passing-rt/` (docs/ledger/g3/2026-11-17-rc-rt-corpora.md).
RUNTIMES = {"robigo": {"ratchet": "passing", "pin": "gated"}, "rt": {"ratchet": "passing-rt", "pin": "rt"}}


def ensure_natives():
    """GNU R's base-package natives built from the R sources
    (scripts/build-natives.sh) serve the base packages when present."""
    natives = os.path.join(STATE, "natives")
    if "ROBIGO_NATIVES" not in os.environ and os.path.isdir(natives):
        os.environ["ROBIGO_NATIVES"] = natives


def build_corpus(name, conf, version, gnur=False):
    """A per-corpus build step: the corpus's `build` key names a script in
    its directory, run once before cases (the capi corpus compiles its C
    cases with R CMD SHLIB; built artifacts are gitignored). `version` is
    the GNU R that builds (ROBIGO_R_VERSION for the script); `gnur` builds
    for GNU R, never with robigo (capi: R CMD SHLIB, not robigo's)."""
    script = conf.get("build")
    if not script:
        return
    env = dict(os.environ, **BASE_ENV, ROBIGO_R_VERSION=version)
    if gnur:
        env["ROBIGO"] = "/nonexistent"
    p = os.path.join(cdir(name), script)
    subprocess.run(["sh", p], cwd=cdir(name), check=True, env=env)


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

def run_case(name, conf, case_id, src, runtime, robigo=None, tier=None):
    """Run one case; returns (stdout, stderr, status) as bytes/int."""
    mode = MODES[conf.get("mode", "script")]
    env = dict(os.environ, **BASE_ENV)
    env["R_DEFAULT_PACKAGES"] = conf.get("default_packages", "NULL")
    env["R_LIBS_USER"] = os.path.join(STATE, "empty-lib")
    os.makedirs(env["R_LIBS_USER"], exist_ok=True)
    for k, v in conf.get("env", {}).items():
        env[k] = v
    if tier:
        env.update(TIERS[tier])
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
        # R's tempdir() goes here too, so it is removed with the case even
        # when the process is killed on a timeout.
        env["TMPDIR"] = tmp
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


def per_case(filename):
    """An `expected_file` without the `.jsonl.gz` suffix names a directory
    holding one `<id>.json` per case, so that branches adding cases never
    touch the same file (like the ratchet markers)."""
    return not filename.endswith(".jsonl.gz")


def load_expected(name, filename="expected.jsonl.gz"):
    p = os.path.join(cdir(name), filename)
    if not os.path.exists(p):
        return {}
    out = {}
    if per_case(filename):
        for root, _, files in os.walk(p):
            for fn in sorted(files):
                if fn.endswith(".json"):
                    with open(os.path.join(root, fn), encoding="utf-8") as f:
                        e = json.load(f)
                    out[e["id"]] = e
        return out
    with gzip.open(p, "rt", encoding="utf-8") as f:
        for line in f:
            e = json.loads(line)
            out[e["id"]] = e
    return out


def write_expected(name, exp, filename="expected.jsonl.gz"):
    p = os.path.join(cdir(name), filename)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    if per_case(filename):
        # One file per case: only the cases whose record changed are
        # rewritten, and a case no longer recorded loses its file.
        want = {os.path.join(p, k + ".json"): exp[k] for k in exp}
        for root, _, files in os.walk(p) if os.path.isdir(p) else ():
            for fn in files:
                f = os.path.join(root, fn)
                if fn.endswith(".json") and f not in want:
                    os.remove(f)
        for f, e in want.items():
            text = json.dumps(e, sort_keys=True, ensure_ascii=False, indent=1) + "\n"
            if os.path.exists(f):
                with open(f, encoding="utf-8") as old:
                    if old.read() == text:
                        continue
            os.makedirs(os.path.dirname(f), exist_ok=True)
            with open(f, "w", encoding="utf-8") as out:
                out.write(text)
        return
    # mtime=0: the file only changes when its content does.
    with open(p, "wb") as raw, gzip.GzipFile(fileobj=raw, mode="wb", mtime=0) as gz:
        for k in sorted(exp):
            gz.write((json.dumps(exp[k], sort_keys=True, ensure_ascii=False) + "\n").encode("utf-8"))


def host_dependent(conf):
    """Cases whose output shows the host (`host_dependent` in corpora.toml:
    kernel, CPU count, installed tools, packages or locales, or memory a
    runner may not have), so GNU R's recorded output holds only on the
    machine that recorded it. They are neither run nor scored, and a
    ratchet marker of one holds nothing while it is listed."""
    return set(conf.get("host_dependent", []))


def load_passing(name, runtime="robigo"):
    """The ratchet: case ids that must keep passing.

    Each id is one empty marker file, `passing/<id>.pass` (an id with `/`
    nests directories). One file per case, so two branches that ratchet
    different cases add different files and never conflict on a shared line.
    The old single `passing.txt` is still read while branches migrate
    (`scripts/ratchet.py migrate`); a marker that names no case is an
    error, where a dangling list entry used to be a silent no-op."""
    d = passing_dir(name, runtime)
    ids = set()
    if os.path.isdir(d):
        for root, _, files in os.walk(d):
            for fn in files:
                p = os.path.join(root, fn)
                if not fn.endswith(MARKER):
                    sys.exit(f"{p}: not a ratchet marker (markers are named <case>{MARKER})")
                ids.add(os.path.relpath(p, d)[: -len(MARKER)].replace(os.sep, "/"))
    legacy = os.path.join(cdir(name), "passing.txt")
    if runtime == "robigo" and os.path.exists(legacy):
        print(f"note: {os.path.relpath(legacy, ROOT)} is deprecated; run scripts/ratchet.py migrate",
              file=sys.stderr)
        with open(legacy) as f:
            ids |= {l.strip() for l in f if l.strip() and not l.startswith("#")}
    dangling = sorted(ids - {cid for cid, _ in load_cases(name)})
    if dangling:
        sys.exit(f"{name}: {len(dangling)} ratchet marker(s) name no case, e.g. {dangling[0]}; "
                 "remove the marker or restore the case")
    return ids


def add_passing(name, ids, runtime="robigo"):
    """Add case ids to the ratchet as marker files; never removes any."""
    d = passing_dir(name, runtime)
    added = []
    for cid in sorted(ids):
        parts = cid.split("/")
        if not cid or cid.startswith("/") or ".." in parts:
            sys.exit(f"refusing to ratchet {cid!r}: not a relative case id")
        p = os.path.join(d, *parts) + MARKER
        if os.path.exists(p):
            continue
        os.makedirs(os.path.dirname(p), exist_ok=True)
        open(p, "w").close()
        added.append(cid)
    return added


def robigo_rt_path(arg):
    """robigo-rt: `--robigo`, else $ROBIGO_RT_BIN, else the release build in
    cargo's target directory (as scripts/rt-overshoot.py finds it), with
    base's database tree written by that binary (scripts/rt_trees.py: the
    cases name no boot, which boots rdb once it is the default)."""
    p = _robigo_rt_binary(arg)
    if os.path.exists(p):
        sys.path.insert(0, os.path.join(ROOT, "scripts"))
        import rt_trees
        rt_trees.ensure(p, ROOT)
    return p


def _robigo_rt_binary(arg):
    if arg:
        return os.path.abspath(arg)
    if os.environ.get("ROBIGO_RT_BIN"):
        return os.environ["ROBIGO_RT_BIN"]
    target = os.environ.get("CARGO_TARGET_DIR")
    if not target:
        try:
            md = json.loads(subprocess.run(["cargo", "metadata", "--format-version", "1", "--no-deps"], cwd=ROOT,
                                           capture_output=True, check=True).stdout)
            target = md["target_directory"]
        except Exception:
            target = os.path.join(ROOT, "target")
    return os.path.join(target, "release", "robigo-rt")


def rt_unsupported(name, conf):
    """Why robigo-rt cannot run a corpus yet, or None."""
    if conf.get("build"):
        return f"{name} has a build step ({conf['build']}), which robigo-rt cannot run yet"
    return None


# What a failing robigo-rt case hit first: the stubs of crates/robigo-rt's
# base.rs, any other "not implemented yet", and a function base does not
# bind (robigo-rt binds only part of base until L5).
BLOCKED = [
    (re.compile(rb"robigo: the primitive '([^']+)' is not implemented yet"), "primitive {}"),
    (re.compile(rb"robigo: \.Internal\(([^()\s]+)\(\)\) is not implemented yet"), ".Internal {}"),
    (re.compile(rb"robigo: (.{1,120}?) is not implemented yet"), "not implemented: {}"),
    (re.compile(rb'could not find function "([^"]+)"'), "unbound {}"),
]


def blocked(err):
    """The first missing thing a case's stderr names, or None."""
    hits = []
    for i, (rx, label) in enumerate(BLOCKED):
        m = rx.search(err)
        if m:
            # the earliest message; at one position, the most specific pattern
            hits.append((m.start(), i, label.format(m.group(1).decode("utf-8", "replace"))))
    return min(hits)[2] if hits else None


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
    conf = corpus_conf(a.name, a.profile)
    version = r_version(getattr(a, "r_version", None))
    ver = subprocess.run(["R", "--version"], capture_output=True, text=True).stdout.splitlines()[0]
    if not re.match(rf"R version {re.escape(version)}(?![0-9.])", ver):
        sys.exit(f"the oracle must be GNU R {version}, found: {ver}")
    build_corpus(a.name, conf, version, gnur=True)
    cases = load_cases(a.name)
    if a.filter:
        rx = re.compile(a.filter)
        cases = [c for c in cases if rx.search(c[0])]
    efile = expected_file(conf, version)
    old = load_expected(a.name, efile)
    todo = cases if a.all else [c for c in cases if old.get(c[0], {}).get("sha") != sha(c[1])]

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
        # The previous recording was made on another machine: lines that
        # differ between the two machines (host name, kernel, core count,
        # installed programs) are masked like lines that differ between
        # runs. A different line count keeps this platform's recording.
        prev = old.get(cid) if a.mask_against_previous else None
        if prev and not rec.get("unstable") and prev.get("status") == s:
            for key, idx in (("out", 0), ("err", 1)):
                now = normalize(runs[0][idx]).split(b"\n")
                then = normalize(dec(prev[key])).split(b"\n")
                if len(now) != len(then):
                    continue
                mask = set(rec.get("mask_" + key, [])) | {i for i, (x, y) in enumerate(zip(now, then)) if x != y}
                if mask:
                    rec["mask_" + key] = sorted(mask)
        # Rdiff ignores these blocks by content. Do not retain line-index
        # masks inside them: a different environment size can move an END
        # marker onto a masked index and prevent the block from closing.
        for key, idx in (("out", 0), ("err", 1)):
            mask_key = "mask_" + key
            if mask_key not in rec:
                continue
            begins = ends = 0
            ignored = set()
            for n, line in enumerate(normalize(runs[0][idx]).split(b"\n")):
                begins += line == IGNORE_BEGIN
                ends += line == IGNORE_END
                if begins > ends or line in (IGNORE_BEGIN, IGNORE_END):
                    ignored.add(n)
            mask = [n for n in rec[mask_key] if n not in ignored]
            if mask:
                rec[mask_key] = mask
            else:
                del rec[mask_key]
        return rec

    recs = pmap(one, todo, a.jobs, f"record {a.name}")
    keep = {c[0] for c in cases} if not a.filter else None
    exp = {k: v for k, v in old.items() if keep is None or k in keep}
    for r in recs:
        exp[r["id"]] = r
    write_expected(a.name, exp, efile)
    unstable = sum(1 for r in recs if r.get("unstable"))
    partly = sum(1 for r in recs if not r.get("unstable") and ("mask_out" in r or "mask_err" in r))
    print(f"recorded {len(recs)} case(s) of {a.name} with {ver}; {partly} with varying lines masked, "
          f"{unstable} unstable (excluded from scoring)")


def masked(b, mask):
    """What is compared: normalized, recorded masks applied (their line
    numbers count the normalized output), IGNORE_RDIFF blocks dropped."""
    b = normalize(b)
    if not mask:
        return rdiff_ignore(b)
    lines = b.split(b"\n")
    for i in mask:
        if i < len(lines):
            lines[i] = b"<masked>"
    return rdiff_ignore(b"\n".join(lines))


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


def control(name, exp, cases, passing):
    """Positive controls before a run counts anything: a changed stdout
    must be noticed, and no ratcheted case may compare nothing (masks that erase all of a non-empty stdout).
    Without them a broken comparison looks like a clean run."""
    probes = [cid for cid, _ in cases if dec(exp[cid]["out"]).strip()][:20]
    if probes:
        if not any(classify(exp[c], dec(exp[c]["out"]) + b"\nrobigo corpus control\n", dec(exp[c]["err"]),
                            exp[c]["status"]) for c in probes):
            sys.exit(f"{name}: CONTROL FAILED: a changed stdout compares equal ({probes[0]}); the comparison is broken")
    vacuous = sorted(cid for cid, _ in cases if cid in passing and dec(exp[cid]["out"]).strip()
                     and not masked(dec(exp[cid]["out"]), exp[cid].get("mask_out")).strip())
    if vacuous:
        sys.exit(f"{name}: CONTROL FAILED: {len(vacuous)} ratcheted case(s) compare nothing (their masks erase "
                 f"all of stdout), e.g. {vacuous[0]}")


class FixtureHandler(http.server.SimpleHTTPRequestHandler):
    """Serves the fixture's files. HTTP/1.1, so a connection stays open
    after a request that asks for `Connection: Keep-Alive`, as the real
    server's does: gnur-tests' sockets.R sends HEAD, then GET, on one
    socket. HEAD's headers are GET's, so the case's first read of the GET
    response (as long as HEAD's) takes the headers alone."""
    protocol_version = "HTTP/1.1"

    def log_message(self, format, *args):
        pass


def start_fixture_server(root, port=80):
    """Serve `root` on 127.0.0.1:`port` (0: any free port) in a daemon
    thread; the caller shuts the server down."""
    try:
        srv = http.server.ThreadingHTTPServer(("127.0.0.1", port), functools.partial(FixtureHandler, directory=root))
    except OSError as e:
        sys.exit(f"--net-fixture needs root and a free 127.0.0.1:{port}: {e}")
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv


def fixture_preflight(name, conf):
    """`run --net-fixture`'s checks before anything changes: the corpus's
    `net_fixture` (None if it has none), only as root inside a container,
    with every file it lists."""
    fx = conf.get("net_fixture")
    if not fx:
        return None
    if os.geteuid() != 0 or not (os.path.exists("/.dockerenv") or os.environ.get("CI") == "true"):
        sys.exit("--net-fixture needs root inside a container (/.dockerenv, or CI=true): "
                 "it serves 127.0.0.1:80 and edits /etc/hosts")
    root = os.path.join(cdir(name), fx["root"])
    missing = [f for f in fx["files"] if not os.path.isfile(os.path.join(root, f))]
    if missing:
        sys.exit(f"{name}: the net fixture lacks {', '.join(missing)} under {os.path.relpath(root, ROOT)}")
    return fx


@contextlib.contextmanager
def net_fixture(name, fx, hosts_file="/etc/hosts"):
    """`run --net-fixture`: `fx` (a corpus's `net_fixture`) served on
    127.0.0.1:80, its hosts mapped there in `hosts_file`, for the block.
    The block only appends to the file, so cutting it back to its old
    length restores it, in one call: os.replace cannot, since Docker
    bind-mounts /etc/hosts (EBUSY). It is restored when the block ends, on
    an exception too, and at once on SIGTERM or SIGINT."""
    size = srv = None

    def restore():
        if size is not None:
            os.truncate(hosts_file, size)

    def on_signal(signum, frame):
        restore()
        raise KeyboardInterrupt if signum == signal.SIGINT else SystemExit(128 + signum)

    handlers = {s: signal.signal(s, on_signal) for s in (signal.SIGTERM, signal.SIGINT)}
    try:
        size = os.path.getsize(hosts_file)
        srv = start_fixture_server(os.path.join(cdir(name), fx["root"]))
        with open(hosts_file, "a") as f:
            f.write("\n127.0.0.1 " + " ".join(fx["hosts"]) + "\n")
        yield
    finally:
        try:
            restore()
        finally:
            for s, h in handlers.items():
                signal.signal(s, h)
            if srv is not None:
                srv.shutdown()
                srv.server_close()


def rt_sources():
    """robigo-rt loads base from the pinned GNU R sources (scripts/r-src.sh).
    Cases run with HOME set to their own directory, so the sources' path
    goes through ROBIGO_RSRC, which robigo-rt reads before HOME."""
    if not os.environ.get("ROBIGO_RSRC"):
        p = subprocess.run([os.path.join(ROOT, "scripts", "r-src.sh")], capture_output=True, text=True)
        if p.returncode:
            sys.exit(f"robigo-rt needs GNU R's sources: scripts/r-src.sh failed:\n{p.stderr[-2000:]}")
        os.environ["ROBIGO_RSRC"] = p.stdout.strip()


def cmd_run(a):
    conf = corpus_conf(a.name, a.profile)
    runtime = getattr(a, "runtime", None) or "robigo"
    rt = runtime == "rt"
    pinned = pins()[RUNTIMES[runtime]["pin"]]
    version = r_version(getattr(a, "r_version", None) or (pinned if rt else None))
    # the ratchet holds runs against its runtime's pin only
    gated = version == pinned
    if rt and (why := rt_unsupported(a.name, conf)):
        sys.exit(f"--runtime rt: {why}")
    if rt and a.tier:
        sys.exit("--tier selects the old runtime's execution tiers; robigo-rt has none")
    if a.profile and a.update:
        sys.exit("profile runs cannot update the shared ratchet; activate the profile first")
    if a.tier and a.update:
        sys.exit("--tier runs cannot update the ratchet, which records default runs")
    if not gated and a.update:
        sys.exit(f"the {RUNTIMES[runtime]['ratchet']}/ ratchet records runs against GNU R {pinned}, not {version}")
    robigo = robigo_rt_path(a.robigo) if rt else robigo_path(a.robigo)
    if not os.path.exists(robigo):
        sys.exit(f"no {'robigo-rt' if rt else 'robigo'} binary at {robigo}; "
                 + ("build it with cargo build --release -p robigo-rtapi (the package of the robigo-rt binary)" if rt else "run scripts/build.sh"))
    if rt:
        rt_sources()
        # robigo-rt ignores an inherited R_HOME (GNU R's) with GNU R's
        # warning on stdout, in every case's output: none for its runs
        os.environ.pop("R_HOME", None)
    fx = fixture_preflight(a.name, conf) if getattr(a, "net_fixture", False) else None
    # the build step (capi: robigo CMD SHLIB) uses the binary under test
    os.environ["ROBIGO"] = os.path.abspath(robigo)
    ensure_natives()
    # what robigo loads (cran-purer's library) is built as for the runtime
    # under test, which is gated on 4.5.0, whatever the recordings compared
    build_corpus(a.name, conf, GATED_R_VERSION)
    exp = load_expected(a.name, expected_file(conf, version))
    vflag = "" if version == GATED_R_VERSION else f" --r-version {version}"
    if not exp:
        sys.exit(f"{a.name} has no expected output for GNU R {version}; run: scripts/corpus.py record {a.name}{vflag}")
    skip = host_dependent(conf)
    cases = [c for c in load_cases(a.name) if c[0] in exp and not exp[c[0]].get("unstable") and c[0] not in skip]
    stale = [c[0] for c in cases if exp[c[0]]["sha"] != sha(c[1])]
    if stale:
        sys.exit(f"{len(stale)} case(s) changed since recording (e.g. {stale[0]}); "
                 f"run: scripts/corpus.py record {a.name}{vflag}")
    # A run against another version only reports: each ratchet holds its pin's.
    passing = load_passing(a.name, runtime) if gated else set()
    control(a.name, exp, cases, passing)
    if a.filter:
        rx = re.compile(a.filter)
        cases = [c for c in cases if rx.search(c[0])]
    if a.shard:
        i, n = map(int, a.shard.split("/"))
        cases = [c for c in cases if shard_of(c[0], n) == i - 1]

    def one(c):
        cid, src = c
        t = time.time()
        o, e, s = run_case(a.name, conf, cid, src, "robigo", robigo, a.tier)
        kinds = classify(exp[cid], o, e, s)
        r = {"id": cid, "pass": not kinds, "differs": kinds, "secs": round(time.time() - t, 3)}
        if rt and kinds and (b := blocked(e)):
            r["blocked"] = b
        if kinds and a.keep_output:
            r["out"], r["err"], r["status"] = enc(o), enc(e), s
        return r

    with net_fixture(a.name, fx) if fx else contextlib.nullcontext():
        res = sorted(pmap(one, cases, a.jobs, f"run {a.name}"), key=lambda r: r["id"])
    ok = {r["id"] for r in res if r["pass"]}
    ran = {r["id"] for r in res}
    regressions = sorted((passing & ran) - ok)
    new = sorted(ok - passing)
    count = lambda k: sum(1 for r in res if k not in r["differs"])
    os.makedirs(os.path.join(STATE, "results"), exist_ok=True)
    summary = {
        "corpus": a.name, "profile": a.profile, "tier": a.tier, "cases": len(res), "pass": len(ok), "stdout": count("stdout"),
        "stderr": count("stderr"), "status": count("status"), "regressions": regressions, "new": new,
    }
    if rt:
        summary["runtime"] = "rt"
        ranked = {}
        for r in res:
            if r.get("blocked"):
                ranked[r["blocked"]] = ranked.get(r["blocked"], 0) + 1
        summary["blocked"] = dict(sorted(ranked.items(), key=lambda kv: (-kv[1], kv[0])))
    if not gated:
        summary["r_version"] = version
        summary["new"] = new = []
    out = os.path.join(STATE, "results", a.name + (".rt" if rt else "") + (f".{a.profile}" if a.profile else "") + (f".R-{version}" if not gated else "") + (f".{a.tier}" if a.tier else "") + (f".shard{a.shard.replace('/', 'of')}" if a.shard else "") + ".json")
    with open(out, "w") as f:
        json.dump({"summary": summary, "results": res}, f, indent=0)
    label = (a.name + (" on robigo-rt" if rt else "") + (f" [{a.tier}]" if a.tier else "")
             + ("" if gated else f" against GNU R {version} (not ratcheted)"))
    print(f"{label}: {len(ok)}/{len(res)} pass; stdout {summary['stdout']}, stderr {summary['stderr']}, "
          f"status {summary['status']} match; {len(new)} new, {len(regressions)} regressions")
    for cid in regressions[:50]:
        print(f"  REGRESSION {cid}")
    if rt and summary["blocked"] and not a.quiet:
        print("  most cases blocked by (the first missing thing each failing case hit):")
        for what, n in list(summary["blocked"].items())[:25]:
            print(f"  {n:7,}  {what}")
    if a.update and new:
        added = add_passing(a.name, new, runtime)
        print(f"ratcheted {len(added)} case(s) under tests/corpora/{a.name}/{RUNTIMES[runtime]['ratchet']}/")
    elif new and not a.quiet:
        print(f"  ({len(new)} new passes; --update records them)")
    return 1 if regressions else 0


def cmd_show(a):
    conf = corpus_conf(a.name, a.profile)
    rt = getattr(a, "runtime", None) == "rt"
    version = r_version(getattr(a, "r_version", None) or (pins()["rt"] if rt else None))
    if rt and (why := rt_unsupported(a.name, conf)):
        sys.exit(f"--runtime rt: {why}")
    if rt and a.tier:
        sys.exit("--tier selects the old runtime's execution tiers; robigo-rt has none")
    if rt:
        rt_sources()
        # robigo-rt ignores an inherited R_HOME (GNU R's) with GNU R's
        # warning on stdout, in every case's output: none for its runs
        os.environ.pop("R_HOME", None)
    cases = dict(load_cases(a.name))
    if a.id not in cases:
        sys.exit(f"no case {a.id}")
    src = cases[a.id]
    exp = load_expected(a.name, expected_file(conf, version)).get(a.id)
    print("== source\n" + src.decode("utf-8", "replace"))
    o, e, s = run_case(a.name, conf, a.id, src, "robigo", robigo_rt_path(a.robigo) if rt else robigo_path(a.robigo),
                       a.tier)
    if exp:
        for label, key, want, got in (("stdout", "out", dec(exp["out"]), o), ("stderr", "err", dec(exp["err"]), e)):
            if masked(want, exp.get("mask_" + key)) == masked(got, exp.get("mask_" + key)):
                print(f"== {label}: same")
                continue
            import difflib
            print(f"== {label} (--- GNU R {version}, +++ robigo)")
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
        exp = load_expected(name, expected_file(conf, GATED_R_VERSION))
        skip = host_dependent(conf)
        ids = {k for k, e in exp.items() if not e.get("unstable") and k not in skip}
        # robigo-rt's column: its ratchet against its pin's recordings
        rt_exp = load_expected(name, expected_file(conf, pins()["rt"]))
        rt_ids = {k for k, e in rt_exp.items() if not e.get("unstable") and k not in skip}
        rt = (len(load_passing(name, "rt") & rt_ids), len(rt_ids)) if rt_ids else None
        rows.append((name, len(load_passing(name) & ids), len(ids), rt, conf.get("description", "")))
    pct = lambda p, t: f"{100 * p / max(t, 1):.1f}%"
    if a.markdown:
        print("| corpus | passing | cases | | robigo-rt | rt cases | |\n|---|---|---|---|---|---|---|")
        for n, p, t, rt, d in rows:
            rt_s = f"{rt[0]} | {rt[1]} | {pct(*rt)}" if rt else "not recorded | | "
            print(f"| {n} | {p} | {t} | {pct(p, t)} | {rt_s} |")
    else:
        for n, p, t, rt, d in rows:
            rt_s = f"rt {rt[0]:6}/{rt[1]:<6} {pct(*rt):>6}" if rt else f"rt {'not recorded':>21}"
            print(f"{n:16} {p:6}/{t:<6} {pct(p, t):>6}  {rt_s}  {d}")


def gh(*args):
    """`gh ARGS` (the GitHub CLI, for this checkout's repository): stdout."""
    p = subprocess.run(["gh", *args], cwd=ROOT, stdout=subprocess.PIPE, text=True)
    if p.returncode != 0:
        sys.exit(f"corpus: gh {' '.join(args)} failed")
    return p.stdout


def corpus_artifacts(run):
    """The names of a CI run's corpus result artifacts that have not expired."""
    out = gh("api", "--paginate", f"repos/{{owner}}/{{repo}}/actions/runs/{run}/artifacts",
             "-q", '.artifacts[] | select(.expired | not) | .name')
    return sorted(n for n in out.split() if n.startswith("corpus-"))


RUNS_SEARCHED = 200


def main_run(run):
    """The CI run to absorb: RUN, which must be a push run on main (a pull
    request's results are of its own code), or the newest completed one
    with corpus artifacts."""
    if run is not None:
        info = json.loads(gh("run", "view", str(run), "--json", "headBranch,event,status,headSha"))
        if info["headBranch"] != "main" or info["event"] != "push":
            sys.exit(f"corpus: run {run} is a {info['event']} run on {info['headBranch']}, not a push to main")
        return str(run), info["headSha"]
    # (gh pages through the API for --limit; artifacts live 14 days, so the
    # newest RUNS_SEARCHED runs cover them unless main sees more pushes)
    runs = json.loads(gh("run", "list", "--workflow", "ci.yml", "--branch", "main", "--event", "push",
                         "--status", "completed", "--limit", str(RUNS_SEARCHED), "--json", "databaseId,headSha"))
    for r in runs:
        if corpus_artifacts(r["databaseId"]):
            return str(r["databaseId"]), r["headSha"]
    sys.exit(f"corpus: none of the newest {len(runs)} completed push runs on main has corpus artifacts")


def run_results(run, into):
    """Download a run's corpus artifacts into `into`: the result files among
    them (default runs of a corpus only: a profile's or a tier's results
    are not the ratchet's, and the conformance runner's are no corpus's)."""
    corpora = manifest()
    files = []
    for name in corpus_artifacts(run):
        d = os.path.join(into, name)
        gh("run", "download", run, "-n", name, "-D", d)
        for root, _, fs in os.walk(d):
            for f in sorted(fs):
                if not f.endswith(".json"):
                    continue
                fn = os.path.join(root, f)
                with open(fn) as fh:
                    doc = json.load(fh)
                s = doc.get("summary") if isinstance(doc, dict) else None
                if not s or "results" not in doc:
                    continue
                if s.get("corpus") not in corpora:
                    print(f"{name}/{f}: results of {s.get('corpus')!r}, which is no corpus, skipped")
                    continue
                if s.get("profile") or s.get("tier"):
                    print(f"{name}/{f}: a {'profile' if s.get('profile') else 'tier'} run's results, skipped")
                    continue
                files.append(fn)
    return files


def cmd_absorb(a):
    """Add the passes recorded in result files (e.g. CI artifacts) to the ratchets."""
    if getattr(a, "from_run", None) is not None:
        if a.files:
            sys.exit("corpus: absorb takes result files or --from-run, not both")
        run, sha = main_run(None if a.from_run == "latest" else a.from_run)
        with tempfile.TemporaryDirectory(prefix="corpus-absorb-") as tmp:
            a.files = run_results(run, tmp)
            if not a.files:
                sys.exit(f"corpus: run {run} has no corpus results")
            print(f"absorbing run {run} (main at {sha[:9]}): {len(a.files)} result file(s)")
            a.from_run = None
            return cmd_absorb(a)
    if not a.files:
        sys.exit("corpus: absorb needs result files or --from-run")
    by = {}
    for fn in a.files:
        with open(fn) as f:
            doc = json.load(f)
        if doc["summary"].get("profile"):
            sys.exit(f"{fn}: profile results cannot update the shared ratchet; activate the profile first")
        if doc["summary"].get("tier"):
            sys.exit(f"{fn}: --tier results cannot update the ratchet, which records default runs")
        # each runtime's results go to its own ratchet, held to its own pin
        runtime = doc["summary"].get("runtime", "robigo")
        if runtime not in RUNTIMES:
            sys.exit(f"{fn}: results of an unknown runtime {runtime!r}")
        pinned = pins()[RUNTIMES[runtime]["pin"]]
        if doc["summary"].get("r_version", pinned) != pinned:
            sys.exit(f"{fn}: results against GNU R {doc['summary']['r_version']} cannot update the "
                     f"{RUNTIMES[runtime]['ratchet']}/ ratchet, which records runs against {pinned}")
        by.setdefault((doc["summary"]["corpus"], runtime), set()).update(r["id"] for r in doc["results"] if r["pass"])
    for (name, runtime), ok in sorted(by.items()):
        conf = corpus_conf(name)
        exp = load_expected(name, expected_file(conf, pins()[RUNTIMES[runtime]["pin"]]))
        ok = {i for i in ok if i in exp}
        old = load_passing(name, runtime)
        label = f"{name}{' (rt)' if runtime == 'rt' else ''}"
        if getattr(a, "dry_run", False):
            # what would be added, to run here before ratcheting (a pass on CI
            # can depend on CI's environment)
            new = sorted(ok - old)
            print(f"{label}: {len(new)} new pass(es) would be added")
            for cid in new:
                print(f"  {cid}")
            continue
        added = add_passing(name, ok - old, runtime)
        print(f"{label}: {len(added)} new pass(es) added")


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
    fn = IMPORTERS.get(conf.get("importer", a.name))
    if fn is None:
        # A corpus with its own importer: scripts/import-<importer>.py NAME.
        sys.exit(subprocess.run([sys.executable, os.path.join(ROOT, "scripts", "import-" + conf["importer"] + ".py"),
                                 a.name], check=False).returncode)
    fn(conf)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list")
    p = sub.add_parser("import")
    p.add_argument("name")
    p = sub.add_parser("record")
    p.add_argument("name")
    p.add_argument("--profile", help="named corpus configuration and oracle recording")
    p.add_argument("--jobs", type=int, default=os.cpu_count())
    p.add_argument("--repeat", type=int, default=2, help="runs per case to detect unstable output")
    p.add_argument("--all", action="store_true", help="re-record unchanged cases too")
    p.add_argument("--filter")
    p.add_argument("--mask-against-previous", action="store_true",
                   help="also mask lines that differ from the existing recording (made on another machine)")
    p.add_argument("--r-version", help="the GNU R version recording (default $ROBIGO_R_VERSION, else 4.5.0)")
    p = sub.add_parser("run")
    p.add_argument("name")
    p.add_argument("--profile", help="named corpus configuration and oracle recording")
    p.add_argument("--jobs", type=int, default=os.cpu_count())
    p.add_argument("--filter")
    p.add_argument("--shard")
    p.add_argument("--robigo")
    p.add_argument("--update", action="store_true")
    p.add_argument("--tier", choices=sorted(TIERS), help="execution tier to run every case in (default: robigo's)")
    p.add_argument("--keep-output", action="store_true", help="store robigo's output of failing cases")
    p.add_argument("--net-fixture", action="store_true",
                   help="serve the corpus's net_fixture on 127.0.0.1:80 for its hosts (root in a container: CI's)")
    p.add_argument("--r-version", help="compare with this GNU R version's recordings (default $ROBIGO_R_VERSION, "
                   "else 4.5.0; with --runtime rt, the rt pin)")
    p.add_argument("--runtime", choices=sorted(RUNTIMES), default="robigo",
                   help="robigo (today's, ratchet passing/) or rt (robigo-rt, ratchet passing-rt/)")
    p.add_argument("--quiet", action="store_true")
    p = sub.add_parser("show")
    p.add_argument("name")
    p.add_argument("--profile", help="named corpus configuration and oracle recording")
    p.add_argument("id")
    p.add_argument("--robigo")
    p.add_argument("--tier", choices=sorted(TIERS), help="execution tier to run the case in (default: robigo's)")
    p.add_argument("--r-version", help="show this GNU R version's recording (default $ROBIGO_R_VERSION, else 4.5.0; "
                   "with --runtime rt, the rt pin)")
    p.add_argument("--runtime", choices=sorted(RUNTIMES), default="robigo")
    p = sub.add_parser("score")
    p.add_argument("--markdown", action="store_true")
    p = sub.add_parser("absorb")
    p.add_argument("files", nargs="*")
    p.add_argument("--from-run", nargs="?", const="latest", metavar="ID",
                   help="absorb a CI push run on main's corpus artifacts (default: the newest with any)")
    p.add_argument("--dry-run", action="store_true", help="list the passes that would be added; write nothing")
    a = ap.parse_args()
    fn = {"list": cmd_list, "import": cmd_import, "record": cmd_record, "run": cmd_run, "show": cmd_show,
          "score": cmd_score, "absorb": cmd_absorb}[a.cmd]
    sys.exit(fn(a) or 0)


if __name__ == "__main__":
    main()
