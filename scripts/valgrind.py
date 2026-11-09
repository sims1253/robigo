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


HEAP_TOOLS = ("memcheck", "massif", "dhat")


def pick_robigo(tool):
    """$ROBIGO, else the build the tool needs: the heap tools see robigo's
    allocations only in .robigo/bin/robigo-vg (scripts/build-vg.sh); the
    counting tools measure the normal build, as the gate does."""
    if os.environ.get("ROBIGO"):
        return os.path.abspath(os.path.join(ROOT, os.environ["ROBIGO"]))
    vg_bin = os.path.join(ROOT, ".robigo", "bin", "robigo-vg")
    if tool in HEAP_TOOLS:
        if os.path.exists(vg_bin):
            return vg_bin
        print(f"note: no .robigo/bin/robigo-vg (scripts/build-vg.sh); {tool} cannot see the normal "
              "build's mimalloc heap", file=sys.stderr)
    return robigo_path(None)


def run(argv, cwd, env, timeout):
    print(f"$ {' '.join(argv)}", file=sys.stderr, flush=True)
    try:
        return subprocess.run(argv, cwd=cwd, env=env, capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        print(f"timeout after {timeout}s", file=sys.stderr)
        sys.exit(2)
    except FileNotFoundError:
        print(f"{argv[0]} not found (install valgrind, or set VALGRIND to its path)", file=sys.stderr)
        sys.exit(2)


def target(args, tmp):
    """(command, env, name) for the thing to profile; tmp is its cwd."""
    env = dict(os.environ, **perf.BASE_ENV)
    if args.bench:
        progs = [p for p in perf.programs(perf.conf()) if p[0] == args.bench]
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
        cmd = robigo_command(pick_robigo(args.tool), file)
    return cmd, env, name


def profile(a, valgrind, tool, flags, default_out):
    """Run TOOL over the target. Returns its data file in OUTDIR, or None
    (reported) when the run left none; the tool's stderr is kept beside it."""
    os.makedirs(OUTDIR, exist_ok=True)
    out = os.path.join(OUTDIR, os.path.basename(a.out or default_out))
    if os.path.exists(out):
        os.remove(out)  # a stale file must not pass for this run's output
    with tempfile.TemporaryDirectory(prefix="vg-") as tmp:
        cmd, env, _ = target(a, tmp)
        p = run([valgrind, f"--tool={tool}", *flags, f"--{tool}-out-file={out}", *a.vg_args, "--", *cmd],
                tmp, env, a.timeout)
    open(out + ".stderr", "wb").write(p.stderr)
    if not os.path.exists(out):
        print(f"no {tool} output at {out} (the run failed; its stderr is at {out}.stderr)", file=sys.stderr)
        return None
    if p.returncode:
        print(f"note: the profiled run exited {p.returncode}; the data may cover part of it "
              f"(stderr: {out}.stderr)", file=sys.stderr)
    return out


def annotate(env_var, default, out, opts, limit):
    """Print the first LIMIT characters of an annotator's report on OUT.
    False (reported) when the annotator is missing or fails."""
    prog = os.environ.get(env_var, default)
    try:
        text = subprocess.run([prog, out, *opts], capture_output=True, text=True, check=True).stdout
    except subprocess.CalledProcessError as e:
        print(f"{prog} failed: {e.stderr.strip()[:400]} (data kept at {out})", file=sys.stderr)
        return False
    except FileNotFoundError:
        print(f"{prog} not found (install valgrind's annotate tools; data kept at {out})", file=sys.stderr)
        return False
    print(text[:limit])
    return True


def cmd_memcheck(a, valgrind):
    flags = ["--leak-check=full", "--show-leak-kinds=definite,indirect", "--errors-for-leak-kinds=definite"]
    supp = os.path.join(ROOT, "scripts", "valgrind.supp")
    if os.path.exists(supp) and not any(f.startswith("--suppressions=") for f in a.vg_args):
        flags.append(f"--suppressions={supp}")
    if a.origins:
        flags.append("--track-origins=yes")
    with tempfile.TemporaryDirectory(prefix="vg-") as tmp:
        cmd, env, name = target(a, tmp)
        p = run([valgrind, "--tool=memcheck", *flags, *a.vg_args, "--", *cmd], tmp, env, a.timeout)
    os.makedirs(OUTDIR, exist_ok=True)
    full = os.path.join(OUTDIR, "memcheck-full.txt")
    text = p.stderr.decode("utf-8", "replace")
    open(full, "w", errors="replace").write(text)
    print("\n".join(text.splitlines()[-a.lines:]))
    summary = re.search(r"ERROR SUMMARY: (\d+) errors", text)
    lost = re.search(r"in use at exit: ([\d,]+) bytes in ([\d,]+) blocks", text)
    if not summary or int(summary.group(1)) or p.returncode:
        found = f"{summary.group(1)} memcheck error(s)" if summary else "no ERROR SUMMARY (valgrind failed)"
        print(f"{name}: {found}, exit {p.returncode} (full log: {full})")
        return 1
    tail = f", {lost.group(1)} bytes in {lost.group(2)} blocks still in use at exit" if lost else ""
    print(f"{name}: clean{tail} (full log: {full})")
    return 0


def cmd_cachegrind(a, valgrind):
    out = profile(a, valgrind, "cachegrind", ["--cache-sim=yes", "--branch-sim=yes"], "cachegrind.out")
    opts = [f"--show={CG_SHOW}", "--sort=Ir", "--show-percs=yes"]
    if a.source:
        opts += ["--annotate=yes", f"--context={a.context}"]
    if not out or not annotate("CG_ANNOTATE", "cg_annotate", out, opts, 20000):
        return 1
    print(f"cachegrind data: {out}")
    return 0


def cmd_callgrind(a, valgrind):
    out = profile(a, valgrind, "callgrind", [], "callgrind.out")
    opts = [f"--threshold={a.threshold}"] + (["--inclusive=yes"] if a.inclusive else [])
    if a.source:
        opts += [f"--context={a.context}"]
    if not out or not annotate("CALLGRIND_ANNOTATE", "callgrind_annotate", out, opts, 20000):
        return 1
    print(f"callgrind data: {out}")
    return 0


def cmd_massif(a, valgrind):
    out = profile(a, valgrind, "massif", ["--stacks=no", "--time-unit=B"], "massif.out")
    if not out:
        return 1
    peak = 0
    for line in open(out, errors="replace"):
        m = re.match(r"mem_heap_B=(\d+)", line)
        if m:
            peak = max(peak, int(m.group(1)))
    annotate("MS_PRINT", "ms_print", out, [], 8000)  # the peak below stands without it
    print(f"peak heap: {peak:,} bytes (massif data: {out})")
    return 0


def cmd_dhat(a, valgrind):
    out = profile(a, valgrind, "dhat", [], "dhat.json")
    if not out:
        return 1
    doc = json.load(open(out))
    ftbl = doc.get("ftbl", [])

    def frame_of(pp):
        fs = pp.get("fs")
        if not fs:
            return "?"
        s = ftbl[fs[0]] if fs[0] < len(ftbl) else "?"
        m = re.match(r"^[0-9a-fx]+: (.*) \((.*)\)$", s, re.IGNORECASE)
        return f"{m.group(1)} ({m.group(2)})" if m else s

    # DHAT's program points are a flat list (dh_view builds the tree), so
    # the sums below are valgrind's own "Total:".
    flat = list(doc.get("pps", []))
    flat.sort(key=lambda pp: pp.get("tb", 0), reverse=True)
    total = sum(pp.get("tb", 0) for pp in flat)
    blocks = sum(pp.get("tbk", 0) for pp in flat)
    reads = sum(pp.get("rb", 0) for pp in flat)
    writes = sum(pp.get("wb", 0) for pp in flat)
    at_end = sum(pp.get("eb", 0) for pp in flat)
    print(f"total allocated: {total:,} bytes in {blocks:,} blocks; "
          f"still alive at the end: {at_end:,} bytes; {doc.get('te', 0):,} {doc.get('tu', 'units')} run")
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
            p.add_argument("--context", type=int, default=8)
            p.add_argument("--out")
        if name in ("massif", "dhat"):
            p.add_argument("--out")
        if name == "callgrind":
            p.add_argument("--inclusive", action="store_true")
            p.add_argument("--threshold", type=float, default=99)
        if name == "memcheck":
            p.add_argument("--origins", action="store_true")
            p.add_argument("--lines", type=int, default=80)
        if name == "dhat":
            p.add_argument("--top", type=int, default=15)
        p.set_defaults(fn=fn)
    a = ap.parse_args()
    valgrind = perf.valgrind_path()
    if not a.bench and not a.script:
        ap.error("a SCRIPT.R or --bench NAME is required")
    sys.exit(a.fn(a, valgrind) or 0)


if __name__ == "__main__":
    main()
