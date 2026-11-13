#!/usr/bin/env python3
"""Instructions per iteration of an R idiom: robigo (VM), GNU R and the floor.

Usage: scripts/idiom-vs-r.py 'SETUP' 'BODY' [N] [--no-r] [--keep DIR]
       scripts/idiom-vs-r.py --idioms [REGEX] [--no-r]
       scripts/idiom-vs-r.py --floors [REGEX]
       scripts/idiom-vs-r.py --rows FILE [--filter REGEX] [--cycles] [--json OUT] [--no-r]
       (all: [--robigo BIN] [--robigo-env K=V ...])

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
OUT writes every count with the binaries and environment used.

robigo runs as `BIN SCRIPT` with ROBIGO_COMPILE=1 (the VM); --robigo-env
K=V adds or overrides its environment, e.g. ROBIGO_JIT=0 for today's
robigo with the JIT off. --robigo BIN can be robigo-rt.

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
from perf import env_pair  # noqa: E402
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


class Meter:
    """Callgrind instruction counts per iteration, N against 2N."""

    def __init__(self, keep=None, robigo=None, robigo_env=(), cycles=False):
        self.valgrind = perf.valgrind_path()
        self.robigo = robigo_path(robigo)
        self.robigo_env = {"ROBIGO_COMPILE": "1", **dict(kv.split("=", 1) for kv in robigo_env)}
        self.cycles = cycles
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
        sim = ["--cache-sim=yes", "--branch-sim=yes"] if self.cycles else []
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
        counts = [run(k, os.path.join(self.tmp, f"{name}-{k}.cg")) for k in (n, 2 * n)]
        if isinstance(counts[0], tuple):
            return tuple((b - a) / n for a, b in zip(*counts))
        return (counts[1] - counts[0]) / n

    def r_script(self, setup, body, k):
        script = os.path.join(self.tmp, f"idiom-{k}.R")
        with open(script, "w") as f:
            f.write(f"{setup}\ng <- function() {{ for (i in 1:{k}) {{ {body} }}; NULL }}\ng()\n")
        return script

    def robigo_iter(self, setup, body, n, tag=""):
        return self.per_iteration(f"{tag}robigo", n, lambda k, out: self.refs(
            [self.robigo, self.r_script(setup, body, k)], out, self.robigo_env))

    def gnur_iter(self, setup, body, n, tag=""):
        return self.per_iteration(f"{tag}gnur", n, lambda k, out: self.refs(
            [self.r_bin, "--vanilla", "--no-echo", "-f", self.r_script(setup, body, k)], out,
            # R's front-end script would set the library path for exec/R.
            {"R_HOME": self.r_home, "LD_LIBRARY_PATH": os.pathsep.join(
                [os.path.join(self.r_home, "lib"), os.environ.get("LD_LIBRARY_PATH", "")]).rstrip(os.pathsep)}))

    def floor_iter(self, idiom):
        src = os.path.join(self.tmp, f"floor-{idiom['name']}.c")
        exe = src[:-2]
        with open(src, "w") as f:
            f.write(c_source(idiom))
        subprocess.run([self.cc, *C_FLAGS, "-o", exe, src], check=True)
        return self.per_iteration(f"{idiom['name']}-floor", idiom["n"], lambda k, out: self.refs([exe, str(k)], out, {}))


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
        rows = tomllib.load(f)["row"]
    return [r for r in rows if not rx or re.search(rx, r["name"])]


def cmd_rows(a, m):
    """Each row's instructions (and estimated cycles) per iteration; no floors."""
    cols = ["row", "group", "robigo instr"] + (["robigo est. cycles"] if a.cycles else [])
    if not a.no_r:
        cols += ["GNU R instr"] + (["GNU R est. cycles"] if a.cycles else []) + ["robigo÷GNU R"]
    print("| " + " | ".join(cols) + " |\n|---|---|" + "---:|" * (len(cols) - 2))
    out = {}
    for d in load_rows(a.rows, a.filter):
        tag = f"{d['name']}-"
        r = m.robigo_iter(d["setup"], d["body"], d["n"], tag)
        g = None if a.no_r else m.gnur_iter(d["setup"], d["body"], d["n"], tag)
        ri, rc = r if a.cycles else (r, None)
        gi, gc = (g if a.cycles else (g, None)) if g is not None else (None, None)
        cells = [d["name"], d.get("group", ""), f"{ri:,.0f}"] + ([f"{rc:,.0f}"] if a.cycles else [])
        if not a.no_r:
            cells += [f"{gi:,.0f}"] + ([f"{gc:,.0f}"] if a.cycles else []) + [ratio(ri, gi)]
        print("| " + " | ".join(cells) + " |", flush=True)
        out[d["name"]] = {"n": d["n"], "robigo_instr": ri, "robigo_est_cycles": rc,
                          "gnur_instr": gi, "gnur_est_cycles": gc}
    if a.json:
        meta = {"rows": a.rows, "robigo": m.robigo, "robigo_env": m.robigo_env,
                "gnur": None if a.no_r else m.r_bin, "valgrind": m.valgrind, "cycles": a.cycles}
        with open(a.json, "w") as f:
            json.dump({"meta": meta, "rows": out}, f, indent=1, sort_keys=True)


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
    ap.add_argument("--json", metavar="OUT", help="with --rows: write the counts here")
    ap.add_argument("--robigo", metavar="BIN", help="the robigo binary (default: $ROBIGO, as scripts/perf.py)")
    ap.add_argument("--robigo-env", action="append", default=[], metavar="K=V", type=env_pair,
                    help="environment for robigo's runs, e.g. ROBIGO_JIT=0")
    ap.add_argument("--no-r", action="store_true", help="measure robigo only")
    ap.add_argument("--keep", help="directory for the callgrind output files")
    a = ap.parse_args()
    if a.rx is None and a.floors is None and a.rows is None and (a.setup is None or a.body is None):
        ap.error("give SETUP and BODY, --idioms, --floors or --rows")
    m = Meter(a.keep, a.robigo, a.robigo_env, a.cycles and a.rows is not None)
    try:
        if a.rows is not None:
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
