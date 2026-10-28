#!/usr/bin/env python3
"""C API microbenchmarks: instructions inside each bench_* routine of
capibench.c (inclusive, callgrind --toggle-collect), robigo vs GNU R.

  crates/robigo-capi/bench/bench.py [--robigo PATH] [--out FILE]

GNU R runs in the robigo-oracle:4.5.0 image (R 4.5.0, Debian trixie, as CI);
the shared object is built there with R CMD SHLIB and loaded by both.
The no-op .Call is measured as the instructions of the R function noop()
(10,000 calls) minus nothing: it includes R's loop and call overhead.
"""
import argparse, json, os, re, shutil, subprocess, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
FUNCS = ["bench_alloc_small", "bench_alloc_medium", "bench_cons", "bench_protect", "bench_mkchar_hit",
         "bench_mkchar_miss", "bench_install", "bench_integer_loop", "bench_set_string_elt", "bench_preserve",
         "bench_eval", "bench_unwind_protect", "bench_error_caught"]
UID = f"{os.getuid()}:{os.getgid()}"


def docker(args, cwd):
    return subprocess.run(["docker", "run", "--rm", "-u", UID, "-e", "HOME=/tmp", "-e", "LC_ALL=C.UTF-8",
                           "-e", "R_DEFAULT_PACKAGES=NULL", "-v", f"{cwd}:{cwd}", "-w", cwd,
                           "robigo-oracle:4.5.0"] + args, capture_output=True, text=True)


def parse(out_file):
    ann = subprocess.run(["callgrind_annotate", "--inclusive=yes", "--threshold=100", out_file],
                         capture_output=True, text=True).stdout
    res = {}
    for line in ann.splitlines():
        m = re.match(r"\s*([\d,]+) \([^)]*\)\s+\S*capibench\.c:(bench_[a-z_]+)\b", line)
        if m and m.group(2) not in res:
            res[m.group(2)] = int(m.group(1).replace(",", ""))
    return res


def run(cmd, cwd, image=False):
    import glob
    for f in glob.glob(os.path.join(cwd, "cg.*.out")):
        os.unlink(f)
    out = os.path.join(cwd, "cg.%p.out")
    # R is a shell script that execs the real binary: follow children
    vg = ["valgrind", "--tool=callgrind", "--trace-children=yes", f"--callgrind-out-file={out}",
          "--toggle-collect=bench_*"]
    if image:
        p = docker(vg + cmd, cwd)
    else:
        env = dict(os.environ, R_DEFAULT_PACKAGES="NULL", LC_ALL="C.UTF-8")
        p = subprocess.run(vg + cmd, cwd=cwd, capture_output=True, text=True, env=env)
    if p.returncode != 0:
        raise SystemExit(p.stderr[-2000:])
    res = {}
    for f in glob.glob(os.path.join(cwd, "cg.*.out")):
        for k, v in parse(f).items():
            res[k] = max(v, res.get(k, 0))
    return res


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--robigo", default=os.path.join(ROOT, ".robigo", "bin", "robigo"))
    ap.add_argument("--out")
    a = ap.parse_args()
    d = tempfile.mkdtemp(prefix="capibench-", dir="/tmp")
    for f in ("capibench.c", "bench.R"):
        shutil.copy(os.path.join(HERE, f), os.path.join(d, f))
    p = docker(["R", "CMD", "SHLIB", "capibench.c"], d)
    if p.returncode != 0:
        raise SystemExit(p.stderr)
    gnur = run(["R", "--no-echo", "--no-restore", "-f", "bench.R"], d, image=True)
    robigo = run([a.robigo, "bench.R"], d)
    rows = {f: {"gnur": gnur.get(f), "robigo": robigo.get(f)} for f in FUNCS}
    print(f"{'routine':26} {'GNU R':>14} {'robigo':>14} {'robigo/R':>9}")
    for f in FUNCS:
        g, r = rows[f]["gnur"], rows[f]["robigo"]
        ratio = f"{r / g:.2f}" if g and r else "-"
        print(f"{f:26} {g or 0:>14,} {r or 0:>14,} {ratio:>9}")
    if a.out:
        json.dump(rows, open(a.out, "w"), indent=1)


if __name__ == "__main__":
    main()
