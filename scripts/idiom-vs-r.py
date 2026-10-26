#!/usr/bin/env python3
"""Instructions per iteration of an R idiom, robigo (VM) against GNU R.

Usage: scripts/idiom-vs-r.py 'SETUP' 'BODY' [N] [--no-r] [--keep DIR]

Runs `g <- function() { for (i in 1:N) { BODY }; NULL }; g()` after SETUP,
for N and 2N iterations, under callgrind in robigo and in GNU R, and prints
the instruction count per iteration (the difference of the two runs over
N, so startup and setup cancel). Both run with R_DEFAULT_PACKAGES=NULL.
Example:

    scripts/idiom-vs-r.py 'f <- function(x) x' 'f(i)' 2000

Environment: ROBIGO (binary, default the shared release build), RSCRIPT_BIN
(GNU R's `exec/R`, default /usr/lib/R/bin/exec/R), R_HOME_DIR (default
/usr/lib/R), VALGRIND (default: from PATH). --keep DIR keeps the callgrind
files there (as NAME-N.cg) for scripts/callgrind-diff.py-style analysis.
"""
import argparse, os, re, shutil, subprocess, tempfile

ap = argparse.ArgumentParser()
ap.add_argument("setup")
ap.add_argument("body")
ap.add_argument("n", nargs="?", type=int, default=1000)
ap.add_argument("--no-r", action="store_true", help="measure robigo only")
ap.add_argument("--keep", help="directory for the callgrind output files")
args = ap.parse_args()

robigo = os.environ.get("ROBIGO", os.path.expanduser("~/.cache/cargo-target/release/robigo"))
r_bin = os.environ.get("RSCRIPT_BIN", "/usr/lib/R/bin/exec/R")
r_home = os.environ.get("R_HOME_DIR", "/usr/lib/R")
valgrind = os.environ.get("VALGRIND", "valgrind")
tmp = args.keep or tempfile.mkdtemp(prefix="idiom-")
os.makedirs(tmp, exist_ok=True)


def refs(cmd, out, env):
    p = subprocess.run(
        [valgrind, "--tool=callgrind", f"--callgrind-out-file={out}"] + cmd,
        env=dict(os.environ, R_DEFAULT_PACKAGES="NULL", **env),
        capture_output=True,
        text=True,
    )
    m = re.search(r"refs:\s+([\d,]+)", p.stderr)
    if not m:
        raise SystemExit(p.stderr[-2000:])
    return int(m.group(1).replace(",", ""))


def per_iteration(runner):
    counts = []
    for n in (args.n, 2 * args.n):
        script = os.path.join(tmp, f"idiom-{n}.R")
        with open(script, "w") as f:
            f.write(f"{args.setup}\ng <- function() {{ for (i in 1:{n}) {{ {args.body} }}; NULL }}\ng()\n")
        counts.append(runner(script, os.path.join(tmp, f"{runner.__name__}-{n}.cg")))
    return (counts[1] - counts[0]) // args.n


def robigo_run(script, out):
    return refs([robigo, script], out, {"ROBIGO_COMPILE": "1"})


def r_run(script, out):
    return refs([r_bin, "--vanilla", "--no-echo", "-f", script], out, {"R_HOME": r_home})


try:
    a = per_iteration(robigo_run)
    if args.no_r:
        print(f"robigo {a}")
    else:
        r = per_iteration(r_run)
        print(f"robigo {a}  GNU R {r}  ratio {a / r:.2f}")
finally:
    if not args.keep:
        shutil.rmtree(tmp, ignore_errors=True)
