#!/usr/bin/env python3
"""Measure robigo vs Rscript startup (instructions, wall time, peak RSS).

Runs inside the robigo-oracle:4.5.0 container. Scenarios:
  e1        -e '1'
  stats-lm  -e 'library(stats); stopifnot(is.function(lm))'
  dataset   -e 'stopifnot(nrow(mtcars) == 32L)'
Each with R_DEFAULT_PACKAGES=NULL and the default packages minus methods.

Usage: startup-perf.py [--robigo PATH] [--runs N] [--wall-runs N] [--filter ...]
       [--metric instr|wall|rss|all]
"""
import argparse, os, resource, statistics, subprocess, sys, tempfile, time

DEF_PKGS = "datasets,utils,grDevices,graphics,stats"

SCENARIOS = {
    "e1": "1",
    "stats-lm": "library(stats); stopifnot(is.function(lm))",
    "dataset": "stopifnot(nrow(mtcars) == 32L)",
}


def run(cmd, env):
    p = subprocess.run(cmd, env=env, capture_output=True)
    if p.returncode != 0:
        print("FAILED:", " ".join(cmd), file=sys.stderr)
        sys.stderr.write(p.stderr.decode()[:2000])
        sys.exit(1)
    return p


def instr(cmd, env):
    with tempfile.NamedTemporaryFile(suffix=".cg") as f:
        run(["valgrind", "--tool=callgrind", "--trace-children=yes",
             f"--callgrind-out-file={f.name}", *cmd], env)
        text = subprocess.run(
            ["callgrind_annotate", "--threshold=100", f.name],
            capture_output=True, text=True, check=True).stdout
    for line in text.splitlines():
        if "PROGRAM TOTALS" in line:
            return int(line.split()[0].replace(",", ""))
    raise RuntimeError("no PROGRAM TOTALS")


def wall(cmd, env, n):
    times = []
    for _ in range(n):
        before = time.perf_counter()
        run(cmd, env)
        after = time.perf_counter()
        times.append(after - before)
    return statistics.median(times)


def rss(cmd, env):
    pid = os.fork()
    if pid == 0:
        os.environ.clear()
        os.environ.update(env)
        os.dup2(os.open("/dev/null", os.O_WRONLY), 1)
        os.execvp(cmd[0], cmd)
        os._exit(127)
    _, status, ru = os.wait4(pid, 0)
    if os.waitstatus_to_exitcode(status) != 0:
        print("FAILED:", " ".join(cmd), file=sys.stderr)
        sys.exit(1)
    return ru.ru_maxrss  # KiB


ap = argparse.ArgumentParser()
ap.add_argument("--robigo", default=".robigo/bin/robigo")
ap.add_argument("--runs", type=int, default=1, help="instruction runs (median)")
ap.add_argument("--wall-runs", type=int, default=15)
ap.add_argument("--filter", default="")
ap.add_argument("--metric", default="all", choices=["instr", "wall", "rss", "all"])
args = ap.parse_args()

base_env = dict(os.environ)
base_env["R_HOME"] = "/usr/lib/R"
base_env["R_LIBS_USER"] = "--no-check"  # don't pick up host user libs
base_env["R_ENVIRON_USER"] = ""
base_env["R_PROFILE_USER"] = ""

print(f"{'scenario':<22} {'runtime':<8} {'pkgs':<5} {'instr':>15} {'wall_ms':>9} {'rss_MB':>8}")
for name, code in SCENARIOS.items():
    if args.filter and args.filter not in name:
        continue
    for pkgs in ["NULL", "def"]:
        if name == "dataset" and pkgs == "NULL":
            continue  # datasets is not attached without default packages
        env = dict(base_env, R_DEFAULT_PACKAGES="NULL" if pkgs == "NULL" else DEF_PKGS)
        outputs = []
        for runtime in ["robigo", "Rscript"]:
            if runtime == "robigo":
                cmd = [args.robigo, "-e", code]
            else:
                dp = "NULL" if pkgs == "NULL" else DEF_PKGS
                cmd = ["Rscript", f"--default-packages={dp}", "-e", code]
            result = run(cmd, env)
            outputs.append((result.stdout, result.stderr))
            i = w = r = ""
            if args.metric in ("instr", "all"):
                i = f"{statistics.median([instr(cmd, env) for _ in range(args.runs)]):,.0f}"
            if args.metric in ("wall", "all"):
                w = f"{wall(cmd, env, args.wall_runs) * 1000:9.1f}"
            if args.metric in ("rss", "all"):
                r = f"{rss(cmd, env) / 1024:8.1f}"
            print(f"{name:<22} {runtime:<8} {pkgs:<5} {i:>15} {w:>9} {r:>8}", flush=True)

        if outputs[0] != outputs[1]:
            raise RuntimeError(f"output differs for {name}/{pkgs}: {outputs!r}")
