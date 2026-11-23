#!/usr/bin/env python3
"""Instructions per iteration of an R idiom: robigo (VM), GNU R and the floor.

Usage: scripts/idiom-vs-r.py 'SETUP' 'BODY' [N] [--no-r] [--keep DIR]
       scripts/idiom-vs-r.py --idioms [REGEX] [--no-r]
       scripts/idiom-vs-r.py --floors [REGEX]
       scripts/idiom-vs-r.py --rows FILE [--filter REGEX] [--cycles] [--json OUT] [--no-r | --no-robigo]
       scripts/idiom-vs-r.py --rows FILE --allocs [--filter REGEX] [--json OUT]
       (all: [--robigo BIN] [--robigo-env K=V ...] [--gnur-env K=V ...])

Runs `g <- function() { for (i in 1:N) { BODY }; NULL }; g()` after SETUP,
for N and 2N iterations, under callgrind in robigo and in GNU R, and prints
the instruction count per iteration (the difference of the two runs over
N, so startup and setup cancel). Both run with R_DEFAULT_PACKAGES=NULL.
Example:

    scripts/idiom-vs-r.py 'f <- function(x) x' 'f(i)' 2000

--idioms runs the named idioms of benchmarks/idioms.toml and reports each
against its floor as well: the instructions per iteration of a plain C
version of the same work, measured the same way (callgrind, N and 2N), and
stored beside the idiom with the arithmetic that explains it. GNU R is a
rival; the floor is how far there is to go. --floors measures the C
versions again (cc -O2) and shows them beside the stored numbers, which is
how the stored floors are made and checked.

--rows FILE runs a rows file without floors (benchmarks/rt/calls-rows.toml,
G3's bars: `[[row]]` tables with name, setup, body, n). --cycles adds
estimated cycles per iteration (callgrind's cache and branch simulation,
scripts/perf.py's Ir + 10*D1m + 100*LLm model) to the instructions. --json
OUT writes every count with the binaries and environment used. --allocs
counts allocations per iteration instead (robigo-rt only, no callgrind):
objects and bytes by SEXPTYPE from the heap's ROBIGO_GC_STATS=1 report, N
against 2N, the bars' allocation metric (docs/ledger/g3.md).

--rows checks first that each row's window holds no collection: every row
runs once at N and once at 2N without callgrind, robigo-rt counting its
collections (ROBIGO_GC_STATS=1) and GNU R its own (gcinfo(TRUE), in a run of
its own, so the measured script is unchanged). robigo-rt's two counts must
be equal and zero, unless the row is marked `collects = true` (its work
collects by design, the jit rows): otherwise the N-to-2N difference holds a
collection, whose cost follows the live heap and not the row's loop, and the
row is refused before anything is measured, naming the row, its counts and
the boot. A row's `heap` table (R_VSIZE, R_NSIZE) sizes the heap of both
sides' runs, which is how an allocating row keeps its window clean. GNU R's
counts are reported, not enforced: its generational collector makes minor
collections inside most allocating rows' windows
(docs/ledger/g3/2026-11-23-row-collections.md). Both sides' counts are in
the table and the JSON.

The same runs refuse a row whose script fails on either side: it exits
non-zero, or prints an R error on stderr (a line starting `Error`). Every
failing row is named with the side, its exit status and the error's first
lines, before anything is measured: a row that stops at an error would
measure the error, and robigo-rt's heap report reads zero collections
from a failed run.

robigo runs as `BIN SCRIPT` with ROBIGO_COMPILE=1 (the VM); --robigo-env
K=V adds or overrides its environment, e.g. ROBIGO_JIT=0 for today's
robigo with the JIT off, and --gnur-env K=V GNU R's. --robigo BIN can be
robigo-rt.

Environment: ROBIGO (binary, default as scripts/perf.py chooses it),
RSCRIPT_BIN (GNU R's `exec/R`, default /usr/lib/R/bin/exec/R), R_HOME_DIR
(default /usr/lib/R), VALGRIND (default as scripts/perf.py chooses it), CC
(default cc). --keep DIR keeps the callgrind files there (as [IDIOM-]NAME-N.cg) for
scripts/callgrind-diff.py-style analysis.
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perf import env_pair, rows_with_setups  # noqa: E402
import tempfile
import tomllib

HERE = os.path.dirname(os.path.abspath(__file__))
IDIOMS = os.path.join(os.path.dirname(HERE), "benchmarks", "idioms.toml")
sys.path.insert(0, HERE)
import perf  # noqa: E402
from corpus import robigo_path  # noqa: E402

# The C version of an idiom: its fields fill this. SINK keeps a value or the
# memory behind a pointer alive without costing an instruction, so the
# compiler cannot drop work whose result nothing reads (nor print it, whose
# cost would vary with the value); n comes from argv, so it cannot fold the
# loop away either.
C_PROGRAM = """#include <stdlib.h>
#define SINK(x) __asm__ volatile("" : : "g"(x) : "memory")
{decls}
int main(int argc, char **argv) {{
    int n = atoi(argv[1]);
    {setup}
    for (int i = 1; i <= n; i++) {{ {body} }}
    double result = {result};
    SINK(result);
    return 0;
}}
"""
C_FLAGS = ["-O2"]


def load_idioms(rx=None):
    with open(IDIOMS, "rb") as f:
        idioms = tomllib.load(f)["idiom"]
    return [d for d in idioms if not rx or re.search(rx, d["name"])]


def c_source(idiom):
    c = idiom["c"]
    return C_PROGRAM.format(decls=c.get("decls", ""), setup=c.get("setup", ""), body=c["body"],
                            result=c["result"])


# The cycles study's pinned model (docs/ledger/g3/2026-11-16-cycles-study.md):
# 32 KiB 8-way L1s, a 32 MB 16-way last level, 64-byte lines.
PINNED_CACHE = ["--I1=32768,8,64", "--D1=32768,8,64", "--LL=33554432,16,64"]

class Meter:
    """Callgrind instruction counts per iteration, N against 2N."""

    def __init__(self, keep=None, robigo=None, robigo_env=(), cycles=False, need_robigo=True, identify=True,
                 cache_model=None, gnur_env=()):
        self.valgrind = perf.valgrind_path()
        self.robigo = robigo_path(robigo) if need_robigo else None
        # which binary this measures, in the report and the JSON (a stale
        # build is warned about: perf.binary_identity)
        self.robigo_identity = None
        if identify and self.robigo and os.path.exists(self.robigo):
            self.robigo_identity = perf.binary_identity(self.robigo, perf.crate_of(self.robigo))
            perf.report_identity("robigo", self.robigo_identity)
        self.robigo_env = {"ROBIGO_COMPILE": "1", **dict(kv.split("=", 1) for kv in robigo_env)}
        self.gnur_env = dict(kv.split("=", 1) for kv in gnur_env)
        self.cycles = cycles
        # callgrind's cache model: the host's (None) or a pinned one
        self.cache = PINNED_CACHE if cache_model == "pinned" else []
        self.r_bin = os.environ.get("RSCRIPT_BIN", "/usr/lib/R/bin/exec/R")
        self.r_home = os.environ.get("R_HOME_DIR", "/usr/lib/R")
        self.cc = os.environ.get("CC", "cc")
        self.keep = keep
        self.tmp = keep or tempfile.mkdtemp(prefix="idiom-")
        os.makedirs(self.tmp, exist_ok=True)

    def close(self):
        if not self.keep:
            shutil.rmtree(self.tmp, ignore_errors=True)

    def refs(self, cmd, out, env):
        """Instructions of one run, or (instructions, estimated cycles) with --cycles."""
        sim = ["--cache-sim=yes", "--branch-sim=yes", *self.cache] if self.cycles else []
        p = subprocess.run([self.valgrind, "--tool=callgrind", *sim, f"--callgrind-out-file={out}"] + cmd,
                           env={**os.environ, **perf.BASE_ENV, "R_DEFAULT_PACKAGES": "NULL", **env},
                           capture_output=True, text=True)
        m = re.search(r"refs:\s+([\d,]+)", p.stderr)
        if not m or p.returncode:
            raise SystemExit(f"{' '.join(cmd)} failed:\n{p.stderr[-2000:]}")
        ir = int(m.group(1).replace(",", ""))
        if not self.cycles:
            return ir
        c = perf.cache_counters(out)
        if c is None:
            raise SystemExit(f"{out}: callgrind wrote no summary line")
        return ir, c["est_cycles"]

    def per_iteration(self, name, n, run):
        files = [os.path.join(self.tmp, f"{name}-{k}.cg") for k in (n, 2 * n)]
        counts = [run(k, f) for k, f in zip((n, 2 * n), files)]
        # the loop's calls of the C library's allocator, per iteration
        # (cmd_rows' heap shifts)
        self.last_alloc_calls = (allocator_calls(files[1]) - allocator_calls(files[0])) / n
        if isinstance(counts[0], tuple):
            return tuple((b - a) / n for a, b in zip(*counts))
        return (counts[1] - counts[0]) / n

    def r_script(self, setup, body, k):
        script = os.path.join(self.tmp, f"idiom-{k}.R")
        with open(script, "w") as f:
            f.write(f"{setup}\ng <- function() {{ for (i in 1:{k}) {{ {body} }}; NULL }}\ng()\n")
        return script

    def robigo_iter(self, setup, body, n, tag="", env=None):
        """`env`: a row's `heap` sizing, over --robigo-env."""
        return self.per_iteration(f"{tag}robigo", n, lambda k, out: self.refs(
            [self.robigo, self.r_script(setup, body, k)], out, {**self.robigo_env, **(env or {})}))

    def gnur_run_env(self, env=None):
        """GNU R's environment: R_HOME and the library path that R's
        front-end script would set for exec/R, --gnur-env, and `env` (a
        row's `heap` sizing)."""
        return {"R_HOME": self.r_home, "LD_LIBRARY_PATH": os.pathsep.join(
            [os.path.join(self.r_home, "lib"), os.environ.get("LD_LIBRARY_PATH", "")]).rstrip(os.pathsep),
            **self.gnur_env, **(env or {})}

    def gnur_iter(self, setup, body, n, tag="", env=None):
        return self.per_iteration(f"{tag}gnur", n, lambda k, out: self.refs(
            [self.r_bin, "--vanilla", "--no-echo", "-f", self.r_script(setup, body, k)], out,
            self.gnur_run_env(env)))

    def robigo_collections(self, setup, body, k, env=None):
        """The collections robigo-rt makes in one run of the loop of `k`
        iterations (its heap's ROBIGO_GC_STATS=1 report), not under
        callgrind; None from a binary that makes no such report (today's
        robigo, not robigo-rt). RowFailed if the run fails (row_failure)."""
        script = self.r_script(setup, body, k)
        p = subprocess.run([self.robigo, script], capture_output=True, text=True,
                           env={**os.environ, **perf.BASE_ENV, "R_DEFAULT_PACKAGES": "NULL",
                                **self.robigo_env, **(env or {}), "ROBIGO_GC_STATS": "1"})
        if e := row_failure("robigo-rt", p):
            raise RowFailed(e)
        m = re.search(r"^robigo gc: collections (\d+)", p.stderr, re.M)
        return int(m.group(1)) if m else None

    def gnur_collections(self, setup, body, k, env=None):
        """The collections GNU R makes in one run of the loop of `k`
        iterations: gcinfo(TRUE)'s report of each one, in a run of its own
        (the measured script stays as it is), not under callgrind.
        RowFailed if the run fails (row_failure)."""
        script = os.path.join(self.tmp, f"gcinfo-{k}.R")
        with open(script, "w") as f:
            f.write(f"invisible(gcinfo(TRUE))\n{setup}\n"
                    f"g <- function() {{ for (i in 1:{k}) {{ {body} }}; NULL }}\ng()\n")
        p = subprocess.run([self.r_bin, "--vanilla", "--no-echo", "-f", script], capture_output=True, text=True,
                           env={**os.environ, **perf.BASE_ENV, "R_DEFAULT_PACKAGES": "NULL", **self.gnur_run_env(env)})
        if e := row_failure("GNU R", p):
            raise RowFailed(e)
        return len(re.findall(r"^Garbage collection \d+ ", p.stderr, re.M))

    def alloc_counts(self, script):
        """{SEXPTYPE or "total": (objects, bytes)} allocated by one run (ROBIGO_GC_STATS=1)."""
        p = subprocess.run([self.robigo, script], capture_output=True, text=True,
                           env={**os.environ, **perf.BASE_ENV, "R_DEFAULT_PACKAGES": "NULL",
                                **self.robigo_env, "ROBIGO_GC_STATS": "1"})
        if p.returncode:
            raise SystemExit(f"{self.robigo} {script} failed:\n{p.stderr[-2000:]}")
        out = {}
        for line in p.stderr.splitlines():
            if m := re.match(r"robigo gc: allocated (\d+) objects, (\d+) bytes", line):
                out["total"] = (int(m.group(1)), int(m.group(2)))
            elif m := re.match(r"robigo gc:\s+([A-Z]+SXP) (\d+) objects, (\d+) bytes", line):
                out[m.group(1)] = (int(m.group(2)), int(m.group(3)))
        if "total" not in out:
            raise SystemExit(f"{self.robigo} {script}: no ROBIGO_GC_STATS report (not robigo-rt?)")
        return out

    def alloc_iter(self, setup, body, n):
        """{SEXPTYPE or "total": (objects, bytes)} per iteration, N against 2N."""
        a, b = (self.alloc_counts(self.r_script(setup, body, k)) for k in (n, 2 * n))
        return {t: tuple((y - x) / n for x, y in zip(a.get(t, (0, 0)), b.get(t, (0, 0))))
                for t in sorted(set(a) | set(b))}

    def floor_iter(self, idiom):
        src = os.path.join(self.tmp, f"floor-{idiom['name']}.c")
        exe = src[:-2]
        with open(src, "w") as f:
            f.write(c_source(idiom))
        subprocess.run([self.cc, *C_FLAGS, "-o", exe, src], check=True)
        return self.per_iteration(f"{idiom['name']}-floor", idiom["n"], lambda k, out: self.refs([exe, str(k)], out, {}))


class RowFailed(Exception):
    """A row's script failed on one side: the side, the exit status and
    the R error's first lines (row_failure)."""


def row_failure(who, p):
    """Why one run of a row's script fails, or None: a non-zero exit, or an
    R error printed on stderr (a line starting `Error`), whatever the exit
    status says."""
    errs = [line for line in p.stderr.splitlines() if re.match(r"Error\b", line)]
    if not p.returncode and not errs:
        return None
    tail = "; ".join(errs[:2]) or (p.stderr.strip().splitlines() or ["no output"])[-1]
    return f"{who} exits {p.returncode}: {tail}"


def ratio(a, b):
    return f"{a / b:,.2f}" if b else "-"


def cmd_adhoc(a, m):
    r = m.robigo_iter(a.setup, a.body, a.n)
    if a.no_r:
        print(f"robigo {r:,.0f}")
    else:
        g = m.gnur_iter(a.setup, a.body, a.n)
        print(f"robigo {r:,.0f}  GNU R {g:,.0f}  ratio {r / g:.2f}")


def cmd_idioms(a, m):
    cols = ["idiom", "robigo", "GNU R", "floor", "robigo÷GNU R", "robigo÷floor"]
    print("| " + " | ".join(cols) + " |\n|---|" + "---:|" * (len(cols) - 1))
    for d in load_idioms(a.rx):
        tag = f"{d['name']}-"
        r = m.robigo_iter(d["setup"], d["body"], d["n"], tag)
        g = None if a.no_r else m.gnur_iter(d["setup"], d["body"], d["n"], tag)
        f = d["c"]["floor"]
        print(f"| {d['name']} | {r:,.0f} | {'-' if g is None else f'{g:,.0f}'} | {f:g} | "
              f"{'-' if g is None else ratio(r, g)} | {ratio(r, f)} |", flush=True)


def load_rows(path, rx=None):
    with open(path, "rb") as f:
        rows = rows_with_setups(tomllib.load(f))
    errors = [e for r in rows for e in row_errors(r)]
    if errors:
        raise SystemExit("\n".join(f"idiom-vs-r: {path}: {e}" for e in errors))
    return [r for r in rows if not rx or re.search(rx, r["name"])]


# the variables a row's `heap` table may set: GNU R's initial heap sizes
# (startup.c:R_SizeFromEnv), which robigo-rt's heap reads the same way
HEAP_VARS = ("R_VSIZE", "R_NSIZE")


def row_errors(row):
    """What is wrong with a row's `collects` and `heap` fields."""
    errors = []
    name = row.get("name", "?")
    if not isinstance(row.get("collects", False), bool):
        errors.append(f"row `{name}`: `collects` is true or false, not {row['collects']!r}")
    heap = row.get("heap", {})
    if not isinstance(heap, dict):
        errors.append(f"row `{name}`: `heap` is a table of {', '.join(HEAP_VARS)}, not {heap!r}")
    else:
        for k, v in heap.items():
            if k not in HEAP_VARS:
                errors.append(f"row `{name}`: `heap` sets {k}; it may set only {', '.join(HEAP_VARS)}")
            elif not isinstance(v, str) or not v:
                errors.append(f"row `{name}`: `heap.{k}` is a size string such as \"256M\", not {v!r}")
    return errors


def boot_of(env):
    """How robigo-rt boots under `env` (its run environment over the
    process's), for a refusal's message."""
    full = {**os.environ, **env}
    boot = full.get("ROBIGO_RT_BOOT") or "the default"
    jit = full.get("R_ENABLE_JIT")
    return f"boot {boot}" + (f", R_ENABLE_JIT={jit}" if jit is not None else "")


def collection_error(row, counts, n, env):
    """Why robigo-rt's collection counts (at N, at 2N) refuse `row`, or
    None. A row's window must hold no collection, unless it is marked
    `collects = true`."""
    a, b = counts
    if a is None or row.get("collects"):
        return None
    if a == b == 0:
        return None
    heap = row.get("heap") or {}
    sizing = ", ".join(f"{k}={v}" for k, v in heap.items()) or "the default heap"
    return (f"row `{row['name']}`: robigo-rt collected {a} times at N = {n} and {b} at 2N ({boot_of(env)}; "
            f"{sizing}): the N-to-2N difference must hold no collection, which would measure the collector "
            f"on the boot's live heap and not the row's loop. Give the row a `heap` table large enough "
            f"(e.g. heap = {{ R_VSIZE = \"256M\" }}) or, if its work collects by design, `collects = true`.")


# the C library's allocator: a loop that calls it follows the initial heap
ALLOCATOR = frozenset(("malloc", "calloc", "realloc", "free",
                       "__libc_malloc", "__libc_calloc", "__libc_realloc", "__libc_free"))


def follows_heap(row, allocs):
    """Whether a row's count can follow the initial heap, so that --shifts
    measures it once per shift: marked `address_sensitive`, or its loop
    calls the allocator at all (`allocs`: calls per iteration by side).
    There is no floor: how often a loop calls it does not bound how far a
    layout moves it (one realloc that extends in place in one heap can copy
    a large buffer in another), so one layout's count cannot exempt a row."""
    return bool(row.get("address_sensitive")) or any(v > 0 for v in allocs.values())


def allocator_calls(cg):
    """Calls of ALLOCATOR's functions in the callgrind file `cg`, under any
    of callgrind's name decorations. A name is the function's own, then any
    number of `'` segments (a recursion depth, `free'2`; a caller under
    --separate-callers, `malloc'worker`), and with --compress-mangled=yes
    the function's part, or a segment, is a reference to a name given
    before, `(7)'2`. The function is the part before the first `'`, a
    reference resolved."""
    names, cfn, n = {}, None, 0

    def function(name):
        for _ in range(8):
            head = name.split("'", 1)[0]
            m = re.fullmatch(r"\((\d+)\)", head)
            if not m or m.group(1) not in names:
                return head
            name = names[m.group(1)]
        return head

    # (as perf.py's cache_counters: a compressed name can hold a locale's
    # bytes from a path)
    with open(cg, errors="replace") as f:
        for ln in f:
            # (a name may be given on any function tag: fn=, cfn=, and frfn=
            # and jfn= under some options; only cfn= names the callee)
            if ln.startswith(("cfn=", "fn=", "frfn=", "jfn=")):
                m = re.match(r"(?:c|fr|j)?fn=\((\d+)\)(?: (.*))?", ln)
                if m:
                    if m.group(2):
                        names[m.group(1)] = m.group(2).strip()
                    name = names.get(m.group(1), "")
                else:
                    name = ln.split("=", 1)[1].strip()
                if ln.startswith("cfn="):
                    cfn = function(name)
            elif ln.startswith("calls=") and cfn in ALLOCATOR:
                n += int(ln[6:].split()[0])
    return n


def shifted(setup, k):
    """A row's setup after `k` unrelated strings: the heap shift of
    --shifts. k = 0 is the row as written."""
    return setup if k == 0 else f'zz <- paste0("z", seq_len({k}L))\n' + setup


def median(xs):
    xs = sorted(xs)
    n = len(xs)
    return xs[n // 2] if n % 2 else (xs[n // 2 - 1] + xs[n // 2]) / 2


def cmd_rows(a, m):
    """Each row's instructions (and estimated cycles) per iteration; no floors.

    With --shifts, a row is measured once per heap shift, after k unrelated
    strings, and reports the median, with every shift's count in the JSON
    (docs/ledger/g3/2026-11-17-address-rows.md), when its count can follow
    the initial heap:
    - a row marked `address_sensitive`, whose count follows where its
      strings were allocated (GNU R's and robigo-rt's string hash is an
      address hash, unique.c:cshash);
    - a row whose loop calls the C library's allocator (ALLOCATOR, counted
      in its first measurement): malloc's paths follow the heap's initial
      state, which the binary's layout and the script's path length move
      (#974's s3, docs/ledger/g3/2026-11-23-startup-rows.md).
      follows_heap() decides."""
    both = not a.no_r and not a.no_robigo
    rows = load_rows(a.rows, a.filter)
    renv = {**m.robigo_env}
    # each row's collections at N and 2N, k = 0, checked before anything is
    # measured: a failing script refuses the row on either side
    # (row_failure), robigo-rt's collections refuse it (collection_error),
    # GNU R's are reported
    colls = {}
    errors = []
    for d in rows:
        heap = d.get("heap") or {}
        c = {}
        try:
            if not a.no_robigo:
                c["robigo"] = [m.robigo_collections(d["setup"], d["body"], k, heap) for k in (d["n"], 2 * d["n"])]
            if not a.no_r:
                c["gnur"] = [m.gnur_collections(d["setup"], d["body"], k, heap) for k in (d["n"], 2 * d["n"])]
        except RowFailed as e:
            where = f" ({boot_of({**renv, **heap})})" if str(e).startswith("robigo-rt") else ""
            errors.append(f"row `{d['name']}`: {e}{where}: the script must run clean on both sides to be measured")
            continue
        if "robigo" in c:
            if c["robigo"][0] is None:
                print(f"idiom-vs-r: {m.robigo} reports no collections (not robigo-rt?): the rows' windows are "
                      "not checked", file=sys.stderr)
            elif e := collection_error(d, c["robigo"], d["n"], {**renv, **heap}):
                errors.append(e)
        colls[d["name"]] = c
    if errors:
        raise SystemExit("\n".join(f"idiom-vs-r: {e}" for e in errors))
    cols = ["row", "group"]
    if not a.no_robigo:
        cols += ["robigo instr"] + (["robigo est. cycles"] if a.cycles else [])
    if not a.no_r:
        cols += ["GNU R instr"] + (["GNU R est. cycles"] if a.cycles else []) + (["robigo÷GNU R"] if both else [])
    cols += ["collections N→2N"]
    print("| " + " | ".join(cols) + " |\n|---|---|" + "---:|" * (len(cols) - 3) + "---|")
    out = {}
    for d in rows:
        tag = f"{d['name']}-"
        heap = d.get("heap") or {}
        rs, gs = [], []
        allocs = {}

        def one(k):
            setup = shifted(d["setup"], k)
            r = g = None
            if not a.no_robigo:
                if k:
                    # a shift's strings can move the collector's cycle into
                    # the window: checked again
                    try:
                        c = [m.robigo_collections(setup, d["body"], j, heap) for j in (d["n"], 2 * d["n"])]
                    except RowFailed as e:
                        raise SystemExit(f"idiom-vs-r: row `{d['name']}`: {e} (heap shift k = {k})")
                    if e := collection_error(d, c, d["n"], {**renv, **heap}):
                        raise SystemExit(f"idiom-vs-r: {e} (heap shift k = {k})")
                r = m.robigo_iter(setup, d["body"], d["n"], f"{tag}{k}-", heap)
                allocs.setdefault("robigo", getattr(m, "last_alloc_calls", 0))
            if not a.no_r:
                g = m.gnur_iter(setup, d["body"], d["n"], f"{tag}{k}-", heap)
                allocs.setdefault("gnur", getattr(m, "last_alloc_calls", 0))
            rs.append((r if a.cycles else (r, None)) if r is not None else (None, None))
            gs.append((g if a.cycles else (g, None)) if g is not None else (None, None))
        one(0)
        # the other shifts, for a row whose count can follow the initial heap
        shift = a.shifts and follows_heap(d, allocs)
        ks = [0] + [k for k in a.shifts if k != 0] if shift else [0]
        for k in ks[1:]:
            one(k)

        def mid(xs, i):
            v = [x[i] for x in xs if x[i] is not None]
            return median(v) if v else None
        ri, rc, gi, gc = mid(rs, 0), mid(rs, 1), mid(gs, 0), mid(gs, 1)
        cells = [d["name"], d.get("group", "") + (f", median of {len(ks)} heap shifts" if len(ks) > 1 else "")]
        if not a.no_robigo:
            cells += [f"{ri:,.0f}"] + ([f"{rc:,.0f}"] if a.cycles else [])
        if not a.no_r:
            cells += [f"{gi:,.0f}"] + ([f"{gc:,.0f}"] if a.cycles else []) + ([ratio(ri, gi)] if both else [])
        c = colls[d["name"]]
        cells += ["; ".join(f"{'robigo' if s == 'robigo' else 'GNU R'} {'-' if v[0] is None else f'{v[0]}→{v[1]}'}"
                            for s, v in c.items()) + (" (collects)" if d.get("collects") else "")]
        print("| " + " | ".join(cells) + " |", flush=True)
        out[d["name"]] = {"n": d["n"], "robigo_instr": ri, "robigo_est_cycles": rc,
                          "gnur_instr": gi, "gnur_est_cycles": gc,
                          # the loop's allocator calls per iteration, at k = 0
                          "allocator_calls": allocs,
                          # each side's collections at N and at 2N, k = 0
                          "collections": c, "collects": bool(d.get("collects")), "heap": heap}
        if len(ks) > 1:
            # every shift's count: the median above is what the row reports
            out[d["name"]]["shifts"] = {str(k): {"robigo_instr": r[0], "robigo_est_cycles": r[1],
                                                 "gnur_instr": g[0], "gnur_est_cycles": g[1]}
                                        for k, r, g in zip(ks, rs, gs)}
    if a.json:
        meta = {"rows": a.rows, "robigo": m.robigo, "robigo_identity": m.robigo_identity, "robigo_env": m.robigo_env,
                "gnur": None if a.no_r else m.r_bin, "gnur_env": None if a.no_r else m.gnur_env,
                "valgrind": m.valgrind, "cycles": a.cycles, "shifts": a.shifts}
        with open(a.json, "w") as f:
            json.dump({"meta": meta, "rows": out}, f, indent=1, sort_keys=True)
            f.write("\n")


def cmd_allocs(a, m):
    """Each row's allocations per iteration: objects and bytes, in all and by SEXPTYPE."""
    print("| row | group | objects | bytes | by type (objects) |\n|---|---|---:|---:|---|")
    out = {}
    for d in load_rows(a.rows, a.filter):
        c = m.alloc_iter(d["setup"], d["body"], d["n"])
        objs, nbytes = c.pop("total")
        # 12 significant digits: `g`'s default 6 would round a byte count
        kinds = ", ".join(f"{t} {o:,.12g}" for t, (o, _) in c.items() if o)
        print(f"| {d['name']} | {d.get('group', '')} | {objs:,.12g} | {nbytes:,.12g} | {kinds} |", flush=True)
        out[d["name"]] = {"n": d["n"], "objects": objs, "bytes": nbytes,
                          "by_type": {t: {"objects": o, "bytes": b} for t, (o, b) in c.items()}}
    if a.json:
        meta = {"rows": a.rows, "robigo": m.robigo, "robigo_identity": m.robigo_identity, "robigo_env": m.robigo_env,
                "allocs": True}
        with open(a.json, "w") as f:
            json.dump({"meta": meta, "rows": out}, f, indent=1, sort_keys=True)
            f.write("\n")


def cmd_floors(a, m):
    cc = subprocess.run([m.cc, "--version"], capture_output=True, text=True).stdout.split("\n")[0]
    print(f"{m.cc} {' '.join(C_FLAGS)}: {cc}")
    for d in load_idioms(a.rx):
        got = m.floor_iter(d)
        stored = d["c"]["floor"]
        note = "" if abs(got - stored) < 0.05 else "  (differs: update benchmarks/idioms.toml)"
        print(f"{d['name']:16} stored {stored:g}  measured {got:g}{note}", flush=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("setup", nargs="?")
    ap.add_argument("body", nargs="?")
    ap.add_argument("n", nargs="?", type=int, default=1000)
    ap.add_argument("--idioms", dest="rx", nargs="?", const="", default=None, metavar="REGEX",
                    help="the named idioms of benchmarks/idioms.toml, with their floors")
    ap.add_argument("--floors", nargs="?", const="", default=None, metavar="REGEX",
                    help="measure the idioms' C floors again")
    ap.add_argument("--rows", metavar="FILE", help="a rows file without floors (benchmarks/rt/calls-rows.toml)")
    ap.add_argument("--filter", metavar="REGEX", help="with --rows: the rows whose name matches")
    ap.add_argument("--cycles", action="store_true", help="with --rows: estimated cycles too (cache simulation)")
    ap.add_argument("--cache-model", choices=["host", "pinned"], default="host",
                    help="with --cycles: callgrind's cache model, the host's or the pinned 32 MB 16-way one "
                         "(docs/ledger/g3/2026-11-16-cycles-study.md)")
    ap.add_argument("--shifts", type=lambda v: [int(k) for k in v.split(",")], metavar="K,K,...",
                    help="with --rows: measure a row whose count can follow the initial heap (address_sensitive, "
                         "or its loop calls the C library's allocator) after each of these heap shifts "
                         "(k unrelated strings; 0 = as written) and report the median")
    ap.add_argument("--allocs", action="store_true",
                    help="with --rows: allocations per iteration (robigo-rt's ROBIGO_GC_STATS), no callgrind")
    ap.add_argument("--json", metavar="OUT", help="with --rows: write the counts here")
    ap.add_argument("--robigo", metavar="BIN", help="the robigo binary (default: $ROBIGO, as scripts/perf.py)")
    ap.add_argument("--robigo-env", action="append", default=[], metavar="K=V", type=env_pair,
                    help="environment for robigo's runs, e.g. ROBIGO_JIT=0")
    ap.add_argument("--gnur-env", action="append", default=[], metavar="K=V", type=env_pair,
                    help="environment for GNU R's runs, e.g. R_VSIZE=1G")
    ap.add_argument("--no-r", action="store_true", help="measure robigo only")
    ap.add_argument("--no-robigo", action="store_true", help="with --rows: measure GNU R only")
    ap.add_argument("--keep", help="directory for the callgrind output files")
    a = ap.parse_args()
    if a.no_robigo and (a.no_r or a.rows is None or a.allocs):
        ap.error("--no-robigo goes with --rows (not --allocs), and not with --no-r")
    if a.shifts and (a.rows is None or a.allocs):
        ap.error("--shifts goes with --rows (not --allocs)")
    if a.rx is None and a.floors is None and a.rows is None and (a.setup is None or a.body is None):
        ap.error("give SETUP and BODY, --idioms, --floors or --rows")
    # --floors measures C versions only: no robigo binary to name
    m = Meter(a.keep, a.robigo, a.robigo_env, a.cycles and a.rows is not None, need_robigo=not a.no_robigo,
              identify=a.floors is None, cache_model=a.cache_model, gnur_env=a.gnur_env)
    try:
        if a.rows is not None and a.allocs:
            cmd_allocs(a, m)
        elif a.rows is not None:
            cmd_rows(a, m)
        elif a.floors is not None:
            a.rx = a.floors
            cmd_floors(a, m)
        elif a.rx is not None:
            cmd_idioms(a, m)
        else:
            cmd_adhoc(a, m)
    finally:
        m.close()


if __name__ == "__main__":
    main()
