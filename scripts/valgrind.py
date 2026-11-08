#!/usr/bin/env python3
"""Run an R script (or a benchmark) under the valgrind tools perf.py does not cover.

    scripts/valgrind.py memcheck SCRIPT.R        # invalid accesses + leaks (Rc cycles)
    scripts/valgrind.py cachegrind SCRIPT.R      # Ir, cache misses, branch misses
    scripts/valgrind.py callgrind SCRIPT.R       # Ir per function (callgrind_annotate)
    scripts/valgrind.py massif SCRIPT.R          # heap growth over time (ms_print)
    scripts/valgrind.py dhat SCRIPT.R            # allocation profile (.dhat.json)

Every tool also takes `--bench NAME` (a scripts/perf.py benchmark, run with
the gate's environment) instead of a script file, and `--runtime gnur` to
profile GNU R the same way for comparison. Outputs land in .robigo/valgrind/.
`--source` adds source-line attribution (cachegrind/callgrind, cg_annotate).
"""
import argparse
import json
import os
import re
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUTDIR = os.path.join(ROOT, ".robigo", "valgrind")
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import perf  # noqa: E402
from corpus import robigo_path  # noqa: E402

CG_SHOW = "Ir,Bc,Bcm,Bi,Bim,Dr,Dw,D1mr,D1mw,DLmr,DLmw"


def robigo_command(robigo, file):
    return [robigo, "--vanilla", file]


def pick_robigo():
    """Under valgrind the tracked build is the point: prefer
    .robigo/bin/robigo-vg (scripts/build-vg.sh) over the normal binary."""
    if os.environ.get("ROBIGO"):
        return os.environ["ROBIGO"]
    vg_bin = os.path.join(ROOT, ".robigo", "bin", "robigo-vg")
    if os.path.exists(vg_bin):
        return vg_bin
    return robigo_path(None)


def run(args, argv, out, cwd, env, timeout):
    print(f"$ {' '.join(argv)}", file=sys.stderr, flush=True)
    try:
        p = subprocess.run(argv, cwd=cwd, env=env, capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        print(f"timeout after {timeout}s", file=sys.stderr)
        sys.exit(2)
    if out:
        open(out + ".stderr", "wb").write(p.stderr)
    return p


def target(args, tmp):
    """(command, cwd, env) for the thing to profile."""
    env = dict(os.environ, **perf.BASE_ENV)
    if args.bench:
        progs = [p for p in perf.programs(perf.conf(), filter=args.bench) if p[0] == args.bench]
        if not progs:
            print(f"no benchmark named {args.bench!r} (scripts/perf.py list)", file=sys.stderr)
            sys.exit(2)
        name, _, text, pk = progs[0]
        file = os.path.join(tmp, "bench.R")
        open(file, "w").write(text)
        env["R_DEFAULT_PACKAGES"] = pk
        env["R_HOME"] = perf.r_home()
    else:
        name = os.path.relpath(args.script, ROOT)
        file = os.path.abspath(args.script)
        env["R_DEFAULT_PACKAGES"] = "NULL"
    if args.runtime == "gnur":
        cmd = perf.command("gnur", "", file, instr=True)
        env["R_HOME"] = perf.r_home()
        name = "gnur/" + name
    else:
        cmd = robigo_command(pick_robigo(), file)
    return cmd, tmp, env, name


def cmd_memcheck(a, valgrind):
    flags = ["--leak-check=full", "--show-leak-kinds=definite,indirect", "--errors-for-leak-kinds=definite"]
    supp = os.path.join(ROOT, "scripts", "valgrind.supp")
    if os.path.exists(supp) and not any("suppressions" in f for f in a.vg_args):
        flags.append(f"--suppressions={supp}")
    if a.origins:
        flags.append("--track-origins=yes")
    with tempfile.TemporaryDirectory(prefix="vg-") as tmp:
        cmd, cwd, env, name = target(a, tmp)
        p = run(a, [valgrind, "--tool=memcheck", *flags, *a.vg_args, "--", *cmd], None, cwd, env, a.timeout)
    os.makedirs(OUTDIR, exist_ok=True)
    full = os.path.join(OUTDIR, "memcheck-full.txt")
    text = p.stderr.decode("utf-8", "replace")
    open(full, "w", errors="replace").write(text)
    print(text[-a.lines:])
    errors = int(re.search(r"ERROR SUMMARY: (\d+) errors", text).group(1)) if "ERROR SUMMARY" in text else -1
    lost = re.search(r"in use at exit: ([\d,]+) bytes in ([\d,]+) blocks", text)
    if errors or p.returncode:
        print(f"{name}: {errors} memcheck error(s), exit {p.returncode}")
        return 1
    tail = f", {lost.group(1)} bytes in {lost.group(2)} blocks still in use at exit" if lost else ""
    print(f"{name}: clean{tail} (full log: .robigo/valgrind memcheck stderr shown above)")
    return 0


def annotated(a, valgrind, tool, out):
    with tempfile.TemporaryDirectory(prefix="vg-") as tmp:
        cmd, cwd, env, name = target(a, tmp)
        run(a, [valgrind, f"--tool={tool}", f"--{tool}-out-file={out}", *a.vg_args, "--", *cmd],
            out, cwd, env, a.timeout)
    return name


def cmd_cachegrind(a, valgrind):
    os.makedirs(OUTDIR, exist_ok=True)
    out = os.path.join(OUTDIR, os.path.basename(a.out or "cachegrind.out"))
    with tempfile.TemporaryDirectory(prefix="vg-") as tmp:
        cmd, cwd, env, name = target(a, tmp)
        run(a, [valgrind, "--tool=cachegrind", "--cache-sim=yes", "--branch-sim=yes",
                f"--cachegrind-out-file={out}", *a.vg_args, "--", *cmd], out, cwd, env, a.timeout)
    annotate = os.environ.get("CG_ANNOTATE", "cg_annotate")
    opts = [f"--show={CG_SHOW}", "--sort=Ir", "--show-percs=yes"]
    if a.source:
        opts += ["--annotate", f"--context={a.context}"]
    try:
        text = subprocess.run([annotate, out] + opts, capture_output=True, text=True, check=True).stdout
    except subprocess.CalledProcessError as e:
        print(f"cg_annotate failed: {e.stderr.strip()[:400]}", file=sys.stderr)
        return 1
    print(text[:20000])
    print(f"cachegrind data: {out}")
    return 0


def cmd_callgrind(a, valgrind):
    os.makedirs(OUTDIR, exist_ok=True)
    out = os.path.join(OUTDIR, os.path.basename(a.out or "callgrind.out"))
    name = annotated(a, valgrind, "callgrind", out)
    annotate = os.environ.get("CALLGRIND_ANNOTATE", "callgrind_annotate")
    opts = [f"--threshold={a.threshold}"] + (["--inclusive=yes"] if a.inclusive else [])
    if a.source:
        opts += [f"--context={a.context}"]
    text = subprocess.run([annotate, out] + opts, capture_output=True, text=True, check=True).stdout
    print(text[:20000])
    print(f"callgrind data: {out}")
    return 0


def cmd_massif(a, valgrind):
    os.makedirs(OUTDIR, exist_ok=True)
    out = os.path.join(OUTDIR, os.path.basename(a.out or "massif.out"))
    with tempfile.TemporaryDirectory(prefix="vg-") as tmp:
        cmd, cwd, env, name = target(a, tmp)
        run(a, [valgrind, "--tool=massif", "--stacks=no", "--time-unit=B", f"--massif-out-file={out}",
                *a.vg_args, "--", *cmd], out, cwd, env, a.timeout)
    peak, peak_at = 0, 0
    for line in open(out, errors="replace"):
        m = re.match(r"mem_heap_B=(\d+)", line)
        if m:
            b = int(m.group(1))
            if b > peak:
                peak, peak_at = b, 0
        m = re.match(r"mem_heapsize_B=(\d+)", line)
        if m and int(m.group(1)) and peak:
            peak_at = int(m.group(1))
    ms = os.environ.get("MS_PRINT", "ms_print")
    text = subprocess.run([ms, out], capture_output=True, text=True).stdout
    print(text[:8000])
    print(f"peak heap: {peak:,} bytes (massif data: {out}; full graph: {ms} {out})")
    return 0


def dhat_pp_total(pp):
    return pp.get("total", 0)


def cmd_dhat(a, valgrind):
    os.makedirs(OUTDIR, exist_ok=True)
    out = os.path.join(OUTDIR, os.path.basename(a.out or "dhat.json"))
    with tempfile.TemporaryDirectory(prefix="vg-") as tmp:
        cmd, cwd, env, name = target(a, tmp)
        run(a, [valgrind, "--tool=dhat", f"--dhat-out-file={out}", *a.vg_args, "--", *cmd], None, cwd, env, a.timeout)
    doc = json.load(open(out))
    ftbl = doc.get("ftbl", [])

    def frame_of(pp):
        fs = pp.get("fs")
        if not fs:
            return "?"
        s = ftbl[fs[0]] if fs[0] < len(ftbl) else "?"
        m = re.match(r"^[0-9a-fx]+: (.*) \((.*)\)$", s)
        return f"{m.group(1)} ({m.group(2)})" if m else s

    flat = []

    def walk(pps):
        for pp in pps[:64]:
            flat.append(pp)
            walk(pp.get("pps", []))

    walk(doc.get("pps", []))
    flat.sort(key=lambda pp: pp.get("tb", 0), reverse=True)
    total = sum(pp.get("tb", 0) for pp in flat)
    blocks = sum(pp.get("tbk", 0) for pp in flat)
    reads = sum(pp.get("rb", 0) for pp in flat)
    writes = sum(pp.get("wb", 0) for pp in flat)
    print(f"total allocated: {total:,} bytes in {blocks:,} blocks "
          f"({doc.get('tg', 0):,} bytes still alive at the end; {doc.get('te', 0):,} {doc.get('tu', 'units')} run)")
    print(f"reads: {reads:,} B; writes: {writes:,} B")
    print("\ntop allocation sites by total bytes:")
    for pp in flat[:a.top]:
        print(f"  {pp.get('tb', 0):>14,} B {pp.get('tbk', 0):>10,} blks  {frame_of(pp)}")
    view = "/usr/lib/valgrind/dh_view.html"
    print(f"\nviewer: xdg-open {view}  then open {out}")
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="tool", required=True)
    for name, fn in (("memcheck", cmd_memcheck), ("cachegrind", cmd_cachegrind),
                     ("callgrind", cmd_callgrind), ("massif", cmd_massif), ("dhat", cmd_dhat)):
        p = sub.add_parser(name)
        p.add_argument("script", nargs="?")
        p.add_argument("--bench", help="a scripts/perf.py benchmark name (scripts/perf.py list)")
        p.add_argument("--runtime", choices=["robigo", "gnur"], default="robigo")
        p.add_argument("--vg-arg", dest="vg_args", action="append", default=[],
                       help="extra valgrind tool flag, e.g. --vg-arg=--scheduling-fair=no")
        p.add_argument("--timeout", type=int, default=3600)
        if name in ("cachegrind", "callgrind"):
            p.add_argument("--source", action="store_true", help="annotate source lines (slow)")
            p.add_argument("--threshold", type=float, default=99)
            p.add_argument("--context", type=int, default=8)
            p.add_argument("--out")
        if name in ("massif", "dhat"):
            p.add_argument("--out")
        if name == "callgrind":
            p.add_argument("--inclusive", action="store_true")
        if name == "memcheck":
            p.add_argument("--origins", action="store_true")
            p.add_argument("--lines", type=int, default=80)
        if name == "dhat":
            p.add_argument("--top", type=int, default=15)
        p.set_defaults(fn=fn)
    a = ap.parse_args()
    valgrind = perf.valgrind_path()
    if not (os.path.exists(valgrind) or valgrind == "valgrind"):
        valgrind = "valgrind"
    if not a.bench and not a.script:
        ap.error("a SCRIPT.R or --bench NAME is required")
    sys.exit(a.fn(a, valgrind) or 0)


if __name__ == "__main__":
    main()
