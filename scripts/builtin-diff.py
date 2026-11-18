#!/usr/bin/env python3
"""A4: robigo-rt's builtins against GNU R 4.6.2 on generated inputs, out of
process (docs/g3.md §10, §16 "ROBIGO_BUILTIN_DIFF"; docs/verification.md,
"Builtin differential").

    scripts/builtin-diff.py run                  # every row; fails on a failure not in known/
    scripts/builtin-diff.py run --only seq.int   # one builtin's score and its failing calls
    scripts/builtin-diff.py run --seed N ...     # another generated set (known/ does not apply)
    scripts/builtin-diff.py run --update         # drop known failures that pass now; add new rows' lists
    scripts/builtin-diff.py show seq.int 412     # one case: its call and both records
    scripts/builtin-diff.py plant --plant-rt BIN # the planted-divergence check (the bar)
    scripts/builtin-diff.py check-rows           # every registry builtin has a row or a reason
    scripts/builtin-diff.py oracle ...           # GNU R's side (in the oracle image)
    scripts/builtin-diff.py key                  # the cache key of GNU R's side (CI)
    scripts/builtin-diff.py ci                   # rt-ci.py's step: run, then plant

Each row of tests/builtin-diff/rows.toml becomes a batch of cases (made by
scripts/builtin-diff/gen.R in GNU R 4.6.2). Each case runs in both runtimes
under scripts/builtin-diff/driver.R, and its records' bytes are compared.
GNU R's side is cached by a hash of everything it depends on, under
--data (default ~/.cache/robigo-research/builtin-diff). When a row's data is
missing it is made in the oracle image with docker, or, in CI, by the
builtin-diff-data job.

A case is one of:
  pass            the records are byte-identical
  mismatch        they differ (the field is named when an R can read them)
  not-implemented robigo-rt's record says "is not implemented yet"
  crash, timeout  robigo-rt died or hung on it (it costs that case only)
  resource        GNU R ran out of memory under the cap: excluded
  oracle-crash    GNU R died or hung on it, or gave two answers in two runs: excluded

Both runtimes run with a 2 GiB address-space cap per process, no network,
and a scratch directory as working, home and temporary directory.
"""
import argparse
import concurrent.futures
import hashlib
import json
import os
import re
import resource
import shutil
import subprocess
import sys
import tempfile
import time
import tomllib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
HERE = os.path.join(ROOT, "scripts", "builtin-diff")
GEN, DRIVER, RENDER = (os.path.join(HERE, f) for f in ("gen.R", "driver.R", "render.R"))
ROWS = os.path.join(ROOT, "tests", "builtin-diff", "rows.toml")
KNOWN = os.path.join(ROOT, "tests", "builtin-diff", "known")
NAMES_C = "src/main/names.c"
REGISTRY = os.path.join(ROOT, "crates", "robigo-rtbuiltins", "src", "registry.rs")
DEFAULT_DATA = os.path.expanduser("~/.cache/robigo-research/builtin-diff")
# the image GNU R's side runs in (docker; CI names its GHCR reference)
IMAGE = os.environ.get("ROBIGO_BUILTIN_DIFF_ORACLE", "robigo-oracle:4.6.2")


def oracle_id():
    """The oracle image's identity in the cache key: the content tag
    scripts/ci-image.sh derives from the image's definition, the same in CI
    (whatever the registry reference) and locally."""
    if not hasattr(oracle_id, "v"):
        p = subprocess.run([os.path.join(ROOT, "scripts", "ci-image.sh"), "refs"], cwd=ROOT,
                           stdout=subprocess.PIPE, text=True)
        m = re.search(r"^oracle_4_6_2=\S+:(\S+)$", p.stdout, re.M)
        if p.returncode != 0 or not m:
            sys.exit("builtin-diff: scripts/ci-image.sh refs gave no oracle_4_6_2 reference")
        oracle_id.v = m.group(1)
    return oracle_id.v
# the environment both runtimes run in (part of the cache key)
ENV = {"LC_ALL": "C.UTF-8", "LANGUAGE": "en", "TZ": "UTC", "R_DEFAULT_PACKAGES": "NULL"}
JIT = {"off": "0", "on": "3"}
MEM_CAP = 2 << 30
# what GNU R's side is made of beyond gen.R and driver.R (part of the key):
# 2, two runs per batch (a case whose records differ is unstable); 3, the
# umask below and the case directory's own entry in `<id>.files`
ORACLE_VERSION = 3
# every runtime process's umask (file modes are compared; part of the key)
UMASK = 0o022
CASE_TIMEOUT = 10.0
RESOURCE = [b"cannot allocate vector of size", b"memory exhausted", b"vector memory limit of",
            b"cannot allocate memory block", b"vector memory exhausted"]
NOT_YET = b"is not implemented yet"
EXCLUDED = ("resource", "oracle-crash")
# --timing: seconds per phase, summed over rows and jobs
TIMES = {}


def timed(phase, t0):
    TIMES[phase] = TIMES.get(phase, 0.0) + time.monotonic() - t0
FAILS = ("mismatch", "not-implemented", "crash", "timeout")


# ---- rows, names.c and the registry ------------------------------------------

def r_str(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n") + '"'


def r_chr(xs):
    return "c(" + ", ".join(r_str(x) for x in xs) + ")" if xs else "character()"


def names_c(src):
    """names.c's rows: name -> (eval, arity)."""
    out = {}
    with open(os.path.join(src, NAMES_C), encoding="latin-1") as f:
        for m in re.finditer(r'^\{"([^"]+)",\s*\w+,\s*-?\d+,\s*(\d+),\s*(-?\d+),', f.read(), re.M):
            out[m.group(1)] = (int(m.group(2)), int(m.group(3)))
    return out


def registry():
    """The registry's rows: name -> effect class."""
    with open(REGISTRY) as f:
        text = f.read()
    out = {}
    for m in re.finditer(r'^\s*(?:row|op!)\(\s*"([^"]+)"[^\n]*?,\s*(Pure|Env|Calls|Rng|External)\s*\)', text, re.M):
        out[m.group(1)] = m.group(2)
    return out


def r_source():
    p = subprocess.run([os.path.join(ROOT, "scripts", "r-src.sh")], cwd=ROOT, stdout=subprocess.PIPE, text=True)
    if p.returncode != 0:
        sys.exit("builtin-diff: scripts/r-src.sh failed (GNU R 4.6.2's sources)")
    return p.stdout.strip()


class Row:
    def __init__(self, name, spec, nc, reg):
        self.name = name
        self.spec = spec
        if name not in nc:
            sys.exit(f"builtin-diff: row [{name}] is not in GNU R's names.c")
        ev, self.arity = nc[name]
        if ev % 10 == 0 and not spec.get("special"):
            # (its arguments reach it as the case's symbols a1, a2, ...: the
            # same expressions in both runtimes, which a row opts into)
            sys.exit(f"builtin-diff: row [{name}] is a special (names.c eval {ev}): `special = true` runs it")
        self.internal = (ev // 10) % 10 == 1
        self.effect = reg.get(name) or spec.get("effect")
        if reg.get(name) and spec.get("effect") and spec["effect"] != reg[name]:
            sys.exit(f"builtin-diff: row [{name}] says effect {spec['effect']}, the registry {reg[name]}")
        if self.effect is None:
            sys.exit(f"builtin-diff: row [{name}] is not in the registry yet, so it needs `effect`")
        if self.effect not in ("Pure", "Rng", "Env", "Calls", "External"):
            sys.exit(f"builtin-diff: row [{name}] has an unknown effect {self.effect}")
        # External cases each run in a process and directory of their own
        self.isolated = self.effect == "External"
        self.jits = ["off", "on"] if spec.get("jit") == "both" else ["off"]

    def r_spec(self, seed):
        s = self.spec
        dom = ", ".join(f"{r_str(k)} = {r_chr(v)}" for k, v in s.get("domain", {}).items())
        typ = ", ".join(f"{r_str(k)} = {r_str(v)}" for k, v in s.get("typical", {}).items())
        return (f"list(name = {r_str(self.name)}, internal = {'TRUE' if self.internal else 'FALSE'}, "
                f"arity = {self.arity}L, effect = {r_str(self.effect)}, "
                f"args = {r_chr(s['args']) if 'args' in s else 'NULL'}, typical = list({typ}), domain = list({dom}), "
                f"extra = {r_chr(s.get('extra', []))}, budget = {int(s.get('budget', 150 if self.effect == 'External' else 2000))}L, "
                f"wrapper_budget = {int(s.get('wrapper_budget', 300))}L, seed = {int(seed)}L)")

    def key(self, seed, jit):
        h = hashlib.sha256()
        for p in (GEN, DRIVER):
            with open(p, "rb") as f:
                h.update(f.read())
        h.update(self.r_spec(seed).encode())
        h.update(json.dumps([ENV, JIT[jit], oracle_id(), ORACLE_VERSION, MEM_CAP, UMASK], sort_keys=True).encode())
        return h.hexdigest()[:24]

    def variant(self, jit):
        return self.name if jit == "off" else f"{self.name}@jit"


def load_rows():
    with open(ROWS, "rb") as f:
        doc = tomllib.load(f)
    uncovered = doc.pop("uncovered", {})
    return doc, uncovered


# ---- running a batch ---------------------------------------------------------

def unshare_works():
    try:
        return subprocess.run(["unshare", "-rn", "true"], stderr=subprocess.DEVNULL).returncode == 0
    except FileNotFoundError:
        return False


def cap():
    resource.setrlimit(resource.RLIMIT_AS, (MEM_CAP, MEM_CAP))
    os.umask(UMASK)


class Unusable(Exception):
    pass


def one_run(cmd, e, cwd, out, start, ncases, timeout):
    """One driver process from case `start` on: the next case without a
    record, whether it was killed for a hang, its stderr and exit status.
    Raises Unusable when the driver refused the runtime or the runtime died
    before its first case (no case is to blame)."""
    # a case is done when its marker is: the driver writes it after the
    # record, so a crash while writing the record leaves the case lost
    have = lambda i: os.path.exists(os.path.join(out, f"{i:06d}.ok"))
    started = os.path.join(out, "started")
    if os.path.exists(started):
        os.remove(started)
    err = tempfile.TemporaryFile()
    sout = open(os.path.join(out, f"stdout.{start}"), "wb")
    p = subprocess.Popen(cmd, env=e, cwd=cwd, stdout=sout, stderr=err, preexec_fn=cap)
    nxt, since = start, time.monotonic()
    timed_out = False
    while p.poll() is None:
        time.sleep(0.05)
        while nxt <= ncases and have(nxt):
            nxt, since = nxt + 1, time.monotonic()
        if time.monotonic() - since > timeout:
            p.kill()
            p.wait()
            timed_out = True
    while nxt <= ncases and have(nxt):
        nxt += 1
    sout.close()
    split_stdout(os.path.join(out, f"stdout.{start}"), out)
    err.seek(0)
    text = err.read().decode("utf-8", "replace")
    err.close()
    m = re.search(r"^Error: (driver unusable on .*)$", text, re.M)
    if m:
        raise Unusable(m.group(1))
    if not os.path.exists(started) and not timed_out:
        raise Unusable(f"{cmd[-3] if len(cmd) >= 3 else cmd[0]} did not start (exit status {p.returncode}): "
                       + " | ".join(text.strip().splitlines()[-3:]))
    return nxt, timed_out, text, p.returncode


def note_lost(out, i, timed_out, text, status, lost):
    lost[i] = "timeout" if timed_out else "crash"
    tail = "\n".join(text.splitlines()[-20:])
    with open(os.path.join(out, f"{i:06d}.lost"), "w") as f:
        f.write(f"{lost[i]} (exit status {status})\n{tail}\n")


def run_batch(cmd, env, batch, out, ncases, scratch, timeout=CASE_TIMEOUT):
    """Run the driver over the batch, restarting after a crash or a hang at
    the next case: {id: "crash" | "timeout"} for the cases with no record."""
    os.makedirs(out, exist_ok=True)
    lost = {}
    start = 1
    while start <= ncases:
        e = {**env, "A4_BATCH": batch, "A4_OUT": out, "A4_FROM": str(start),
             "HOME": scratch, "TMPDIR": scratch}
        nxt, timed_out, text, status = one_run(cmd, e, scratch, out, start, ncases, timeout)
        if nxt > ncases:
            break
        note_lost(out, nxt, timed_out, text, status, lost)
        start = nxt + 1
    return lost


# An External row's cases each run in a process of their own, in a fresh
# directory at the same absolute path in GNU R's container and on the host,
# under an environment made from nothing but ENV and these (so Sys.getenv()
# and the paths a case sees are the same for both runtimes).
CLEAN_PATH = "/usr/local/bin:/usr/bin:/bin"


def remove_tree(d):
    """rm -rf, after giving the owner back every permission a case's
    Sys.chmod may have taken (each directory's before it is listed)."""
    def unlock(p):
        os.chmod(p, 0o700)
        for x in os.listdir(p):
            q = os.path.join(p, x)
            if os.path.isdir(q) and not os.path.islink(q):
                unlock(q)
    if not os.path.lexists(d):
        return
    if os.path.isdir(d) and not os.path.islink(d):
        unlock(d)
    shutil.rmtree(d)


def case_root(key):
    return f"/tmp/robigo-a4-{os.getuid()}-{key}"


def tree(d):
    """A directory's files: [path, kind, mode, size, sha256], sorted, the
    directory itself first as "." (Sys.chmod("."))."""
    out = [[".", "dir", os.lstat(d).st_mode & 0o7777, 0, ""]]
    for root, dirs, files in os.walk(d):
        for name in sorted(dirs) + sorted(files):
            p = os.path.join(root, name)
            rel = os.path.relpath(p, d)
            st = os.lstat(p)
            if os.path.islink(p):
                out.append([rel, "link", 0, 0, os.readlink(p)])
            elif os.path.isdir(p):
                out.append([rel, "dir", st.st_mode & 0o7777, 0, ""])
            else:
                with open(p, "rb") as f:
                    h = hashlib.sha256(f.read()).hexdigest()
                out.append([rel, "file", st.st_mode & 0o7777, st.st_size, h])
    return out[:1] + sorted(out[1:])


def run_isolated(cmd, env, batch, out, ncases, key, rsrc=None, timeout=CASE_TIMEOUT):
    """An External row: one process per case in its own directory, whose
    files after the case go to `<id>.files`."""
    os.makedirs(out, exist_ok=True)
    root = case_root(key)
    lost = {}
    base = {k: env[k] for k in (*ENV, "R_ENABLE_JIT", "A4_LABEL") if k in env}
    for i in range(1, ncases + 1):
        remove_tree(root)
        cwd, home, tmp = (os.path.join(root, x) for x in ("case", "home", "tmp"))
        for d in (cwd, home, tmp):
            os.makedirs(d)
        # the modes UMASK gives, whatever the harness's own umask (the
        # directory's mode is part of `<id>.files`)
        for d in (root, cwd, home, tmp):
            os.chmod(d, 0o777 & ~UMASK)
        if rsrc:
            # robigo-rt finds GNU R's sources under HOME (no ROBIGO_RSRC in Sys.getenv())
            os.makedirs(os.path.join(home, ".cache", "robigo-research"))
            os.symlink(os.path.dirname(rsrc), os.path.join(home, ".cache", "robigo-research", "rsrc"))
        e = {**base, "PATH": CLEAN_PATH, "HOME": home, "TMPDIR": tmp, "A4_BATCH": batch, "A4_OUT": out,
             "A4_FROM": str(i), "A4_TO": str(i)}
        nxt, timed_out, text, status = one_run(cmd, e, cwd, out, i, i, timeout)
        if nxt <= i:
            note_lost(out, i, timed_out, text, status, lost)
        with open(os.path.join(out, f"{i:06d}.files"), "w") as f:
            json.dump(tree(cwd), f)
    remove_tree(root)
    return lost


MARK = re.compile(rb"\x01A4 (\d+)\x01\n")


def split_stdout(path, out):
    """A run's stdout, split at the driver's markers into `<id>.out` (the
    output each case printed; none when it printed nothing)."""
    with open(path, "rb") as f:
        data = f.read()
    os.remove(path)
    ms = list(MARK.finditer(data))
    for k, m in enumerate(ms):
        end = ms[k + 1].start() if k + 1 < len(ms) else len(data)
        if end > m.end():
            with open(os.path.join(out, f"{int(m.group(1)):06d}.out"), "wb") as f:
                f.write(data[m.end():end])


def made(rds):
    """The files an External case left in its directory (empty otherwise)."""
    p = rds[:-4] + ".files"
    if not os.path.exists(p):
        return []
    with open(p) as f:
        return json.load(f)


def printed(rds):
    """What the case whose record is `rds` printed."""
    p = rds[:-4] + ".out"
    if not os.path.exists(p):
        return b""
    with open(p, "rb") as f:
        return f.read()


def read_cases(d):
    """cases.tsv: [(id, kind, call)]."""
    out = []
    with open(os.path.join(d, "cases.tsv"), encoding="utf-8", errors="replace") as f:
        for line in f:
            i, kind, call = line.rstrip("\n").split("\t", 2)
            out.append((int(i), kind, call))
    return out


# ---- GNU R's side ------------------------------------------------------------

def oracle_dir(data, row, seed, jit):
    return os.path.join(data, row.variant(jit), row.key(seed, jit))


def have_oracle(data, row, seed, jit):
    return os.path.exists(os.path.join(oracle_dir(data, row, seed, jit), "done"))


def cmd_oracle(a):
    """GNU R's side for the rows given (run with GNU R 4.6.2's Rscript on the
    PATH: in the oracle image)."""
    doc, _ = load_rows()
    nc, reg = names_c(a.r_src or r_source()), registry()
    rows = [Row(n, doc[n], nc, reg) for n in (sorted(doc) if a.all else a.rows)]
    todo = [(r, j) for r in rows for j in r.jits if not have_oracle(a.data, r, a.seed, j)]
    if not todo:
        return 0
    work = tempfile.mkdtemp(prefix="a4-oracle-")
    spec = os.path.join(work, "spec.R")
    gens = {}
    for r, j in todo:
        gens.setdefault(r.name, r)
    with open(spec, "w") as f:
        f.write("rows <- list(\n" + ",\n".join(r.r_spec(a.seed) for r in gens.values()) + "\n)\n")
    gen_out = os.path.join(work, "gen")
    p = subprocess.run(["Rscript", "--vanilla", GEN, spec, gen_out], env={**os.environ, **ENV}, cwd=work)
    if p.returncode != 0:
        sys.exit("builtin-diff: gen.R failed")
    prefix = ["unshare", "-rn"] if unshare_works() else []

    def one(rj):
        r, j = rj
        d = oracle_dir(a.data, r, a.seed, j)
        if os.path.exists(d):
            shutil.rmtree(d)
        os.makedirs(d)
        for f in ("batch.rds", "cases.tsv"):
            shutil.copy(os.path.join(gen_out, r.name, f), d)
        scratch = tempfile.mkdtemp(prefix="a4-scratch-")
        n = len(read_cases(d))
        env = {**os.environ, **ENV, "R_ENABLE_JIT": JIT[j], "A4_LABEL": "GNU R"}
        cmd = [*prefix, "Rscript", "--vanilla", DRIVER]
        batch = os.path.join(d, "batch.rds")

        def go(out):
            if r.isolated:
                return run_isolated(cmd, env, batch, out, n, r.key(a.seed, j))
            return run_batch(cmd, env, batch, out, n, scratch)
        lost = go(os.path.join(d, "records"))
        # a second run: a case whose two records differ (GNU R reading memory
        # it should not, on an input its wrapper never passes; a clock or a
        # process id) has no answer
        again = os.path.join(scratch, "again")
        lost2 = go(again)
        for i in range(1, n + 1):
            if i in lost:
                continue
            a_, b_ = (os.path.join(x, f"{i:06d}.rds") for x in (os.path.join(d, "records"), again))
            if i in lost2 or not os.path.exists(b_) or open(a_, "rb").read() != open(b_, "rb").read() \
                    or printed(a_) != printed(b_) or made(a_) != made(b_):
                lost[i] = "unstable"
        shutil.rmtree(scratch, ignore_errors=True)
        with open(os.path.join(d, "done"), "w") as f:
            json.dump({"lost": lost}, f)
        return r.variant(j), n, len(lost)

    with concurrent.futures.ThreadPoolExecutor(a.jobs) as ex:
        for v, n, nl in ex.map(one, todo):
            print(f"oracle: {v}: {n} cases, {nl} lost", flush=True)
    shutil.rmtree(work, ignore_errors=True)
    return 0


def ensure_oracle(a, rows):
    missing = sorted({r.name for r in rows for j in r.jits if not have_oracle(a.data, r, a.seed, j)})
    if not missing:
        return
    if a.no_docker or not shutil.which("docker"):
        sys.exit("builtin-diff: no GNU R records for " + ", ".join(missing) +
                 f" in {a.data} (CI: the builtin-diff-data job makes them; locally: drop --no-docker)")
    os.makedirs(a.data, exist_ok=True)
    src = r_source()
    print(f"builtin-diff: GNU R's side for {len(missing)} row(s), in {IMAGE}", flush=True)
    cmd = ["docker", "run", "--rm", "--network", "none", "--memory", f"{a.jobs * 2 + 1}g",
           "--security-opt", "seccomp=unconfined", "-u", f"{os.getuid()}:{os.getgid()}",
           "-v", f"{ROOT}:{ROOT}:ro", "-v", f"{src}:{src}:ro", "-v", f"{a.data}:{a.data}", "-e", "HOME=/tmp",
           "-e", f"ROBIGO_BUILTIN_DIFF_ORACLE={IMAGE}",
           IMAGE, "python3", os.path.join(ROOT, "scripts", "builtin-diff.py"), "oracle",
           "--data", a.data, "--seed", str(a.seed), "--jobs", str(a.jobs), "--r-src", src, *missing]
    if subprocess.run(cmd).returncode != 0:
        sys.exit("builtin-diff: GNU R's side failed")


# ---- robigo-rt's side, comparison, score -------------------------------------

def classify(o, r, lost_o, lost_r, i):
    """One case's class from GNU R's and robigo-rt's record files."""
    if i in lost_o or not os.path.exists(o):
        return "oracle-crash"
    ob = open(o, "rb").read()
    if any(p in ob for p in RESOURCE):
        return "resource"
    if i in lost_r or not os.path.exists(r):
        return lost_r.get(i, "crash")
    rb = open(r, "rb").read()
    if ob == rb and printed(o) == printed(r) and made(o) == made(r):
        return "pass"
    if NOT_YET in rb:
        return "not-implemented"
    return "mismatch"


def run_rt(a, row, jit, rt_bin, prefix):
    d = oracle_dir(a.data, row, a.seed, jit)
    cases = read_cases(d)
    with open(os.path.join(d, "done")) as f:
        lost_o = {int(k): v for k, v in json.load(f)["lost"].items()}
    scratch = tempfile.mkdtemp(prefix="a4-rt-")
    out = os.path.join(scratch, "records")
    env = {k: v for k, v in os.environ.items() if k != "R_HOME"}
    env.update(ENV, R_ENABLE_JIT=JIT[jit], A4_LABEL="robigo-rt", ROBIGO_RSRC=a.rsrc)
    work = os.path.join(scratch, "cwd")
    os.makedirs(work)
    t0 = time.monotonic()
    if row.isolated:
        lost_r = run_isolated([*prefix, rt_bin, "--vanilla", DRIVER], env, os.path.join(d, "batch.rds"), out,
                              len(cases), row.key(a.seed, jit), rsrc=a.rsrc)
    else:
        lost_r = run_batch([*prefix, rt_bin, "--vanilla", DRIVER], env, os.path.join(d, "batch.rds"), out,
                           len(cases), work)
    timed("robigo-rt: the driver over the batch", t0)
    t0 = time.monotonic()
    res = {}
    for i, kind, call in cases:
        res[i] = classify(os.path.join(d, "records", f"{i:06d}.rds"), os.path.join(out, f"{i:06d}.rds"),
                          lost_o, lost_r, i)
    timed("harness: comparing the records", t0)
    return {"row": row, "jit": jit, "cases": cases, "res": res, "oracle": os.path.join(d, "records"),
            "rt": out, "scratch": scratch}


def rscript():
    """An R that can read the records (any R >= 4; only for showing diffs)."""
    if shutil.which("Rscript"):
        return ["Rscript", "--vanilla"]
    if shutil.which("docker"):
        return ["docker", "run", "--rm", "-i", "-u", f"{os.getuid()}:{os.getgid()}", "-v", "/tmp:/tmp",
                "-v", f"{ROOT}:{ROOT}:ro", "-v", f"{tempfile.gettempdir()}:{tempfile.gettempdir()}",
                IMAGE, "Rscript", "--vanilla"]
    return None


def diff_fields(pairs):
    """render(), with "stdout" for the pairs whose records are equal (what
    differs is what the case printed)."""
    out = render(pairs)
    for i, o, x in pairs:
        with open(o, "rb") as f1, open(x, "rb") as f2:
            if f1.read() == f2.read():
                if printed(o) != printed(x):
                    out[i] = ("stdout", repr(printed(o)[:150]), repr(printed(x)[:150]))
                else:
                    out[i] = ("files", json.dumps(made(o))[:150], json.dumps(made(x))[:150])
    return out


def render(pairs):
    """For [(id, oracle, rt)]: {id: (field, GNU R's, robigo-rt's)}."""
    r = rscript()
    if not r or not pairs:
        return {}
    with tempfile.NamedTemporaryFile("w", suffix=".tsv", delete=False, dir=tempfile.gettempdir()) as f:
        for i, o, x in pairs:
            f.write(f"{i}\t{o}\t{x}\n")
        lst = f.name
    t0 = time.monotonic()
    p = subprocess.run([*r, RENDER, lst], stdout=subprocess.PIPE, text=True, errors="replace")
    timed("render.R: naming the fields of the mismatches", t0)
    os.unlink(lst)
    out = {}
    for line in p.stdout.splitlines():
        parts = line.split("\t")
        if len(parts) == 4:
            out[int(parts[0])] = (parts[1], parts[2], parts[3])
    return out


def known_path(variant):
    return os.path.join(KNOWN, f"{variant}.txt")


def read_known(variant, key):
    p = known_path(variant)
    if not os.path.exists(p):
        return None
    ids, hdr = set(), None
    with open(p) as f:
        for line in f:
            if line.startswith("#"):
                m = re.search(r"\bkey ([0-9a-f]+)", line)
                if m:
                    hdr = m.group(1)
            elif line.strip():
                ids.add(int(line.split()[0]))
    if hdr != key:
        sys.exit(f"builtin-diff: {os.path.relpath(p, ROOT)} is for another generator, row or seed "
                 f"(key {hdr}, now {key}): regenerate it with --regen-known, in a PR titled `builtin-diff:` "
                 "whose text shows each builtin's pass/total before and after")
    return ids


def write_known(variant, key, r, fails):
    os.makedirs(KNOWN, exist_ok=True)
    c = counts(r["res"])
    with open(known_path(variant), "w") as f:
        f.write(f"# builtin-diff known failures: {variant}, seed 0, key {key}\n")
        f.write(f"# {score_line(variant, c)}\n")
        for i in sorted(fails):
            f.write(f"{i}\n")


def counts(res):
    c = {k: 0 for k in ("pass", *FAILS, *EXCLUDED)}
    for v in res.values():
        c[v] += 1
    return c


def score_line(variant, c):
    total = sum(c[k] for k in ("pass", *FAILS))
    return (f"{variant}: {c['pass']} / {total} (not-implemented {c['not-implemented']}, crash {c['crash']}, "
            f"timeout {c['timeout']}, resource {c['resource']} excluded, oracle-crash {c['oracle-crash']} excluded)")


def report(r, known, show):
    """Print one row's score and its failing calls; the new failures' ids."""
    variant = r["row"].variant(r["jit"])
    c = counts(r["res"])
    fails = sorted(i for i, v in r["res"].items() if v in FAILS)
    new = [i for i in fails if known is None or i not in known]
    print(score_line(variant, c))
    calls = {i: (kind, call) for i, kind, call in r["cases"]}
    new_set = set(new)
    mism = [i for i in fails if r["res"][i] == "mismatch"]
    # every failing call, the new ones (not in known/) first
    shown = ([i for i in new] + [i for i in fails if i not in set(new)])[:show]
    detail = diff_fields([(i, os.path.join(r["oracle"], f"{i:06d}.rds"), os.path.join(r["rt"], f"{i:06d}.rds"))
                     for i in mism])
    if mism:
        if detail:
            byf = {}
            for i in mism:
                f = detail.get(i, ("?",))[0]
                byf[f] = byf.get(f, 0) + 1
            print("  mismatches by field: " + ", ".join(f"{k} {v}" for k, v in sorted(byf.items(), key=lambda x: -x[1])))
        else:
            print("  mismatches by field: (no R to read the records here)")
    if known is not None:
        fixed = sorted(known - set(fails))
        print(f"  known failures {len(known)}, new {len(new)}, fixed {len(fixed)}")
    for i in shown:
        kind, call = calls[i]
        why = r["res"][i]
        mark = "NEW " if i in new_set else "    "
        line = f"  {mark}#{i} {kind:7} {why:15} {call}"
        if why == "mismatch" and i in detail:
            f, o, x = detail[i]
            line += f"\n      {f}: GNU R {o}\n      {' ' * len(f)}  robigo-rt {x}"
        elif why in ("crash", "timeout"):
            lostf = os.path.join(r["rt"], f"{i:06d}.lost")
            if os.path.exists(lostf):
                tail = [t for t in open(lostf, errors="replace").read().strip().splitlines()[1:] if t.strip()]
                line += "".join(f"\n      {t[:200]}" for t in tail[-3:])
        print(line)
    if len(fails) > len(shown):
        print(f"  ... {len(fails) - len(shown)} more (--show N, or scripts/builtin-diff.py show {r['row'].name} ID)")
    return new, fails


def cmd_run(a):
    doc, _ = load_rows()
    nc, reg = names_c(r_source()), registry()
    names = a.only or sorted(doc)
    for n in names:
        if n not in doc:
            sys.exit(f"builtin-diff: no row [{n}] in tests/builtin-diff/rows.toml")
    rows = [Row(n, doc[n], nc, reg) for n in names]
    t0 = time.monotonic()
    ensure_oracle(a, rows)
    timed("GNU R's side (cache misses only)", t0)
    rt_bin = a.robigo_rt or default_rt()
    a.rsrc = r_source()
    prefix = ["unshare", "-rn"] if unshare_works() else []
    jobs = [(r, j) for r in rows for j in r.jits]
    if a.timing:
        # robigo-rt's start (self-test and the batch's readRDS), with no case
        t0 = time.monotonic()
        d = oracle_dir(a.data, rows[0], a.seed, rows[0].jits[0])
        with tempfile.TemporaryDirectory() as tmp:
            env = {k: v for k, v in os.environ.items() if k != "R_HOME"}
            env.update(ENV, R_ENABLE_JIT="0", A4_LABEL="robigo-rt", ROBIGO_RSRC=a.rsrc, A4_BATCH=os.path.join(d, "batch.rds"),
                       A4_OUT=tmp, A4_FROM="999999", HOME=tmp, TMPDIR=tmp)
            subprocess.run([rt_bin, "--vanilla", DRIVER], env=env, cwd=tmp, preexec_fn=cap)
        print(f"timing: robigo-rt's start with {rows[0].name}'s batch: {time.monotonic() - t0:.2f} s")
    t_all = time.monotonic()
    status = 0
    try:
        with concurrent.futures.ThreadPoolExecutor(a.jobs) as ex:
            results = list(ex.map(lambda rj: run_rt(a, rj[0], rj[1], rt_bin, prefix), jobs))
    except Unusable as e:
        print(f"builtin-diff: {e}", file=sys.stderr)
        return 3
    for r in results:
        variant = r["row"].variant(r["jit"])
        key = r["row"].key(a.seed, r["jit"])
        known = read_known(variant, key) if a.seed == 0 and not a.regen_known else None
        new, fails = report(r, known, a.show)
        rel = os.path.relpath(known_path(variant), ROOT)
        if a.seed == 0 and (a.regen_known or (a.update and known is None)):
            # a new row starts with the failures it has (or a builtin-diff: PR rewrites them)
            write_known(variant, key, r, fails)
            print(f"  wrote {rel} ({len(fails)} failures)")
            continue
        if a.update and known is not None and known - set(fails):
            # the ratchet only shrinks: fixed cases leave, new ones never enter
            write_known(variant, key, r, known & set(fails))
            print(f"  {rel}: {len(known - set(fails))} fixed, removed")
        if a.seed != 0:
            new = fails     # no known list applies to another seed
        elif known is None:
            print(f"  no {rel}: a new row's list is written by --update")
            new = fails
        if new:
            status = 1
        if getattr(a, "reuse", None) is not None and r["row"].name in PLANT_ROWS and r["jit"] == "off":
            a.reuse[r["row"].name] = r    # the plant check's runs on this binary (ci)
        elif not a.keep:
            shutil.rmtree(r["scratch"], ignore_errors=True)
        else:
            print(f"  robigo-rt's records: {r['rt']}")
    if a.timing:
        n = sum(len(r["cases"]) for r in results)
        print(f"timing: {len(results)} rows, {n} cases, {time.monotonic() - t_all:.1f} s wall at {a.jobs} jobs")
        for k, v in sorted(TIMES.items()):
            print(f"timing:   {k}: {v:.1f} s")
    if a.seed == 0 and not a.only:
        stale = [f[:-4] for f in os.listdir(KNOWN) if f.endswith(".txt")] if os.path.isdir(KNOWN) else []
        gone = [v for v in stale if v.removesuffix("@jit") not in doc]
        if gone:
            print("builtin-diff: known lists without a row: " + ", ".join(gone))
            status = 1
    return status


def default_rt():
    p = os.path.join(os.environ.get("CARGO_TARGET_DIR", os.path.join(ROOT, "target")), "release", "robigo-rt")
    if not os.path.exists(p):
        sys.exit(f"builtin-diff: no robigo-rt at {p} (cargo build --release -p robigo-rtapi --bin robigo-rt, "
                 "or --robigo-rt BIN)")
    return p


def cmd_show(a):
    doc, _ = load_rows()
    nc, reg = names_c(r_source()), registry()
    row = Row(a.name, doc[a.name], nc, reg)
    jit = "on" if a.jit else "off"
    ensure_oracle(a, [row])
    rt_bin = a.robigo_rt or default_rt()
    a.rsrc = r_source()
    r = run_rt(a, row, jit, rt_bin, ["unshare", "-rn"] if unshare_works() else [])
    kind, call = {i: (k, c) for i, k, c in r["cases"]}[a.id]
    print(f"#{a.id} {kind}: {call}\nclass: {r['res'][a.id]}")
    o, x = (os.path.join(r[s], f"{a.id:06d}.rds") for s in ("oracle", "rt"))
    rs = rscript()
    if rs:
        subprocess.run([*rs, RENDER, "--full", o, x])
    shutil.rmtree(r["scratch"], ignore_errors=True)
    return 0


# ---- the planted-divergence check --------------------------------------------

# each plant: the row, and the class (and field) its new failures must have
PLANTS = {
    "nchar": ("mismatch", {"value", "error"}),
    "seq_len": ("mismatch", {"conds"}),
    "names": ("mismatch", {"env"}),
    "abs": ("crash", None),
    "set.seed": ("mismatch", {"seed"}),
    "nzchar": ("mismatch", {"stdout"}),
    "assign": ("mismatch", {"env"}),
    "do.call": ("mismatch", {"log", "conds"}),
    "Sys.getenv": ("mismatch", {"files"}),
}
CONTROL = ["rep_len"]
PLANT_ROWS = {*PLANTS, *CONTROL}


def cmd_plant(a):
    """The bar: with the plant binary each planted row gains failures of the
    planted kind (a crash costing exactly its own cases), and the control
    rows' results do not change."""
    doc, _ = load_rows()
    nc, reg = names_c(r_source()), registry()
    rows = [Row(n, doc[n], nc, reg) for n in [*PLANTS, *CONTROL]]
    ensure_oracle(a, rows)
    prefix = ["unshare", "-rn"] if unshare_works() else []
    base_rt = a.robigo_rt or default_rt()
    a.rsrc = r_source()
    ok = True
    try:
        for row in rows:
            base = (getattr(a, "reuse", None) or {}).get(row.name) or run_rt(a, row, "off", base_rt, prefix)
            plant = run_rt(a, row, "off", a.plant_rt, prefix)
            changed = sorted(i for i in base["res"] if base["res"][i] != plant["res"][i])
            if row.name in CONTROL:
                good = not changed
                msg = "unchanged" if good else f"{len(changed)} cases changed"
            else:
                want, fields = PLANTS[row.name]
                newf = [i for i in changed if base["res"][i] == "pass" and plant["res"][i] in FAILS]
                kinds = {plant["res"][i] for i in newf}
                good = bool(newf) and kinds == {want}
                if good and fields:
                    det = diff_fields([(i, os.path.join(plant["oracle"], f"{i:06d}.rds"),
                                   os.path.join(plant["rt"], f"{i:06d}.rds")) for i in newf[:50]])
                    got = {d[0] for d in det.values()}
                    good = bool(got) and got <= fields
                    kinds = kinds | {f"field {g}" for g in got}
                if good and want == "crash":
                    # every case but the crashing ones still has its record
                    lost = [i for i, v in plant["res"].items() if v in ("crash", "timeout")]
                    good = set(lost) == set(newf)
                msg = f"{len(newf)} new failures ({', '.join(sorted(kinds))}); expected {want}" + \
                      (f" in {', '.join(sorted(fields))}" if fields else "")
            print(f"{'ok  ' if good else 'FAIL'} {row.name}: {msg}")
            ok &= good
            for r in (base, plant):
                shutil.rmtree(r["scratch"], ignore_errors=True)
    except Unusable as e:
        print(f"builtin-diff: {e}", file=sys.stderr)
        return 3
    return 0 if ok else 1


# ---- coverage ----------------------------------------------------------------

# Whether a registry builtin with neither a row nor a reason, or an
# [uncovered] name the registry lacks, fails check-rows. False while the
# builtin PRs queued before A4 landed drain: they are then warnings, listed
# in the summary. A later PR sets it to True once the foreman says that
# queue has drained.
UNLISTED_FAILS = False


def cmd_check_rows(a):
    doc, uncovered = load_rows()
    reg = registry()
    listed = {n for names in uncovered.values() for n in names}
    both = sorted(set(doc) & listed)
    missing = sorted(n for n in reg if n not in doc and n not in listed)
    stale = sorted(n for n in listed if n not in reg)
    for n in both:
        print(f"builtin-diff: [{n}] has a row and is listed as uncovered")
    level = "" if UNLISTED_FAILS else "warning: "
    for n in missing:
        print(f"builtin-diff: {level}registry builtin '{n}' has no row in tests/builtin-diff/rows.toml "
              "and no reason under [uncovered]")
    for n in stale:
        print(f"builtin-diff: {level}[uncovered] names '{n}', which is not in the registry")
    print(f"builtin-diff: {len(doc)} rows, {len(listed)} uncovered, {len(reg)} registry builtins"
          + (f"; warnings: {len(missing)} without a row or reason, {len(stale)} uncovered not in the registry"
             if not UNLISTED_FAILS and (missing or stale) else ""))
    return 1 if both or (UNLISTED_FAILS and (missing or stale)) else 0


def cmd_key(a):
    doc, _ = load_rows()
    nc, reg = names_c(a.r_src or r_source()), registry()
    h = hashlib.sha256()
    for n in sorted(doc):
        r = Row(n, doc[n], nc, reg)
        for j in r.jits:
            h.update(f"{r.variant(j)}={r.key(0, j)}\n".encode())
    print(h.hexdigest()[:24])
    return 0


PLANT_FEATURE = "robigo-rtbuiltins/diff-plant"


def cmd_ci(a):
    """rt-ci.py's step: every row against the release binary the steps before
    built, then the plant check with a build of robigo-rt with the plants."""
    target = os.environ.get("CARGO_TARGET_DIR", os.path.join(ROOT, "target"))
    work = os.path.join(ROOT, ".robigo", "builtin-diff")
    os.makedirs(work, exist_ok=True)
    rel = os.path.join(target, "release", "robigo-rt")
    build = ["cargo", "build", "--release", "--locked", "-p", "robigo-rtapi", "--bin", "robigo-rt"]
    if subprocess.run(build, cwd=ROOT).returncode != 0:
        return 1
    base = os.path.join(work, "robigo-rt")
    shutil.copy2(rel, base)
    a.robigo_rt, a.only, a.show, a.update, a.regen_known, a.keep, a.timing = base, None, 5, False, False, False, True
    a.reuse = {}    # run's results for the plant rows, so plant need not run them again
    status = cmd_check_rows(a)
    status = cmd_run(a) or status
    if subprocess.run([*build, "--features", PLANT_FEATURE], cwd=ROOT).returncode != 0:
        return 1
    plant = os.path.join(work, "robigo-rt-plant")
    shutil.copy2(rel, plant)
    a.plant_rt = plant
    print("builtin-diff: the planted divergences", flush=True)
    return status or cmd_plant(a)


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)

    def common(p):
        p.add_argument("--data", default=os.environ.get("ROBIGO_BUILTIN_DIFF_DATA", DEFAULT_DATA))
        p.add_argument("--seed", type=int, default=0)
        p.add_argument("--jobs", type=int, default=4)
        p.add_argument("--robigo-rt", help="the robigo-rt binary (default: the release build)")
        p.add_argument("--no-docker", action="store_true", help="fail instead of making GNU R's side")
    p = sub.add_parser("run")
    common(p)
    p.add_argument("--only", action="append", help="this builtin's row (repeatable)")
    p.add_argument("--show", type=int, default=20, help="failing calls shown per row")
    p.add_argument("--update", action="store_true", help="remove fixed known failures; write new rows' lists")
    p.add_argument("--regen-known", action="store_true", help="rewrite the known lists (a `builtin-diff:` PR)")
    p.add_argument("--keep", action="store_true", help="keep robigo-rt's records")
    p.add_argument("--timing", action="store_true", help="the seconds each phase took")
    p = sub.add_parser("show")
    common(p)
    p.add_argument("name")
    p.add_argument("id", type=int)
    p.add_argument("--jit", action="store_true")
    p = sub.add_parser("plant")
    common(p)
    p.add_argument("--plant-rt", required=True, help="robigo-rt built with robigo-rtbuiltins/diff-plant")
    sub.add_parser("check-rows")
    p = sub.add_parser("ci")
    common(p)
    p = sub.add_parser("oracle")
    common(p)
    p.add_argument("--r-src")
    p.add_argument("--all", action="store_true", help="every row")
    p.add_argument("rows", nargs="*")
    p = sub.add_parser("key")
    p.add_argument("--r-src")
    a = ap.parse_args(argv)
    if hasattr(a, "data"):
        a.data = os.path.abspath(a.data)
    return {"run": cmd_run, "show": cmd_show, "plant": cmd_plant, "check-rows": cmd_check_rows,
            "oracle": cmd_oracle, "key": cmd_key, "ci": cmd_ci}[a.cmd](a)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
