#!/usr/bin/env python3
"""The cost of a start: `--vanilla -e 1` with R_DEFAULT_PACKAGES=NULL, of
robigo-rt and of GNU R.

    scripts/rt-startup.py --robigo-rt BIN [--checkout DIR] [--trees] [--starts A,B] [--runs K] [--json OUT]
    scripts/rt-startup.py --gnur [--runs K] [--json OUT]

With --trees, the checkout's R_HOME trees are built first with BIN
(scripts/rt-home.sh, scripts/rt-home-db.sh --robigo-rt BIN): each binary
boots from the base database its own build writes, as it ships.

perf.py rt-bars gates robigo-rt's starts against main's binary (the
[[startup]] entries of benchmarks/rt/m1-bars.toml), and perf.py rt-gnur
records GNU R's beside them, report-only.

Each start has three numbers:
- `instr`: callgrind's total instructions of the whole process;
- `anon_kib`: peak anonymous memory, in KiB (RssAnon, see below);
- `rss_kib`: total peak RSS, `VmHWM`, in KiB.

Each is the median over `runs` runs (instructions: over ENV_SHIFTS, as a
row's count over its heap shifts, below).

**Anonymous memory.** Linux keeps a high-water mark of the whole RSS
(VmHWM) but none of its anonymous part, and a 20 ms start is too short to
sample /proc. The process is stopped at its exit, before the kernel tears
its memory down (ptrace's PTRACE_EVENT_EXIT), and the peak anonymous memory
is VmHWM less the file-backed and shared pages mapped there. RssAnon itself
is no measure at that point: robigo-rt has freed most of its heap by then
(1.3-1.7 MB left of a 4.6-6.8 MB peak). The file-backed pages are the
binary's and its libraries', which stay mapped and only grow, so the
difference is the anonymous part at the moment of the total peak; a page
faulted in after the peak makes it read low, never high
(docs/ledger/g3/2026-11-23-startup-rows.md has the spread).
The file-backed part of the RSS is the binary's and its libraries' pages,
faulted in around the code that ran. It follows code layout (#959: 45.8 KB
more code, +0.2–0.4 MB of RSS, anonymous memory unchanged), so it is
reported but gated loosely.

**Environment shifts.** A start's instruction count can follow the
process's initial heap and stack, which the environment's size moves, as
the s3 row followed its script path's length (#974). Each start is
measured under ENV_SHIFTS, padding variables of these lengths, and the
median is the count.
"""
import argparse
import concurrent.futures
import ctypes
import json
import os
import re
import signal
import statistics
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)

# a start by name: the environment it runs with, over BASE
STARTS = {
    "interim": {},
    "rdb-jit": {"ROBIGO_RT_BOOT": "rdb"},
    "rdb-nojit": {"ROBIGO_RT_BOOT": "rdb", "R_ENABLE_JIT": "0"},
    # reserved for L5: default packages loaded, the translation cache on
    # (defined when they can run)
    "rdb-pkgs": None,
    "rdb-cache": None,
}
GNUR_STARTS = {"jit": {}, "nojit": {"R_ENABLE_JIT": "0"}}
BASE = {"R_DEFAULT_PACKAGES": "NULL", "LANGUAGE": "en", "LC_ALL": "C.UTF-8", "TZ": "UTC",
        "R_ENVIRON_USER": "", "R_PROFILE_USER": ""}
# the padding variable's lengths of the environment shifts
ENV_SHIFTS = (0, 100, 1000)

# the gate against main's binary (perf.py rt-bars), in percent: instructions
# as the rows' THRESHOLD; the anonymous peak at least ANON_FLOOR_KIB, five
# pages being 1% of a 2 MB start (H's interim measurements); total peak RSS
# loosely, since its file-backed part follows code layout
INSTR_LIMIT = 0.5
ANON_LIMIT = 1.0
ANON_FLOOR_KIB = 64
RSS_LIMIT = 5.0

PTRACE_TRACEME, PTRACE_CONT, PTRACE_SETOPTIONS = 0, 7, 0x4200
PTRACE_O_TRACEEXIT, PTRACE_EVENT_EXIT = 0x40, 6
# personality(2)'s flag that turns address-space randomization off (setarch -R)
ADDR_NO_RANDOMIZE = 0x0040000
libc = ctypes.CDLL(None, use_errno=True)
libc.ptrace.argtypes = [ctypes.c_long, ctypes.c_long, ctypes.c_void_p, ctypes.c_void_p]
libc.ptrace.restype = ctypes.c_long


def ptrace(req, pid, data=0):
    if libc.ptrace(req, pid, None, ctypes.c_void_p(data)) == -1:
        e = ctypes.get_errno()
        raise OSError(e, f"ptrace({req}): {os.strerror(e)}")


def status_kib(pid):
    """/proc/PID/status's memory lines, KiB by name."""
    out = {}
    with open(f"/proc/{pid}/status") as f:
        for ln in f:
            m = re.match(r"^(Vm\w+|Rss\w+):\s+(\d+) kB", ln)
            if m:
                out[m.group(1)] = int(m.group(2))
    return out


def memory(cmd, env):
    """(peak anonymous KiB, VmHWM KiB, exit status, stdout) of one run of
    `cmd`, stopped at PTRACE_EVENT_EXIT to read /proc: the anonymous peak
    is VmHWM less the file-backed and shared pages still mapped there."""
    r, w = os.pipe()
    pid = os.fork()
    if pid == 0:
        try:
            os.close(r)
            os.dup2(w, 1)
            fd = os.open(os.devnull, os.O_WRONLY)
            os.dup2(fd, 2)
            ptrace(PTRACE_TRACEME, 0)
            # one layout for every run: with randomized addresses the pages a
            # start touches, and so its peak RSS, vary by run
            if libc.personality(ADDR_NO_RANDOMIZE) == -1:
                os.write(w, b"rt-startup: personality(ADDR_NO_RANDOMIZE) refused\n")
                os._exit(126)
            os.execve(cmd[0], cmd, env)
        finally:
            os._exit(127)
    os.close(w)
    # the stop at exec
    _, st = os.waitpid(pid, 0)
    if not os.WIFSTOPPED(st):
        msg = os.read(r, 4096).decode(errors="replace").strip()
        raise SystemExit(f"rt-startup: {cmd[0]} did not start under ptrace" + (f": {msg}" if msg else ""))
    ptrace(PTRACE_SETOPTIONS, pid, PTRACE_O_TRACEEXIT)
    ptrace(PTRACE_CONT, pid)
    seen = None
    chunks = []
    os.set_blocking(r, False)
    while True:
        try:
            while (b := os.read(r, 65536)):
                chunks.append(b)
        except BlockingIOError:
            pass
        _, st = os.waitpid(pid, 0)
        if os.WIFEXITED(st) or os.WIFSIGNALED(st):
            break
        sig = os.WSTOPSIG(st)
        if sig == signal.SIGTRAP and st >> 16 == PTRACE_EVENT_EXIT:
            seen = status_kib(pid)
            ptrace(PTRACE_CONT, pid)
        else:
            # pass any other signal on
            ptrace(PTRACE_CONT, pid, 0 if sig == signal.SIGTRAP else sig)
    os.set_blocking(r, True)
    while (b := os.read(r, 65536)):
        chunks.append(b)
    os.close(r)
    if seen is None:
        raise SystemExit(f"rt-startup: no exit stop for {cmd[0]}")
    anon = seen["VmHWM"] - seen["RssFile"] - seen.get("RssShmem", 0)
    return anon, seen["VmHWM"], os.waitstatus_to_exitcode(st), b"".join(chunks)


def instructions(cmd, env, valgrind):
    """callgrind's total instructions of one run of `cmd`."""
    with tempfile.TemporaryDirectory(prefix="rt-startup-") as d:
        out = os.path.join(d, "cg")
        p = subprocess.run([valgrind, "--tool=callgrind", f"--callgrind-out-file={out}", *cmd], env=env,
                           stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
        if p.returncode:
            raise SystemExit(f"rt-startup: {' '.join(cmd)} under callgrind exited {p.returncode}:\n{p.stderr[-2000:]}")
        with open(out) as f:
            for ln in f:
                if ln.startswith("summary:") or ln.startswith("totals:"):
                    return int(ln.split()[1])
    raise SystemExit(f"rt-startup: no total in callgrind's output of {' '.join(cmd)}")


def measure(cmd, env, runs, valgrind):
    """One start's numbers: {"instr", "instr_shifts", "anon_kib", "rss_kib",
    "anon_runs", "rss_runs"}."""
    def one(k):
        return str(k), instructions(cmd, dict(env, ROBIGO_STARTUP_PAD="x" * k) if k else env, valgrind)

    with concurrent.futures.ThreadPoolExecutor(max_workers=len(ENV_SHIFTS)) as ex:
        shifts = dict(ex.map(one, ENV_SHIFTS))
    anon, rss = [], []
    want = None
    for _ in range(runs):
        a, h, status, out = memory(cmd, env)
        if status != 0:
            raise SystemExit(f"rt-startup: {' '.join(cmd)} exited {status}")
        if want is None:
            want = out
        elif out != want:
            raise SystemExit(f"rt-startup: {' '.join(cmd)} printed differently between runs")
        anon.append(a)
        rss.append(h)
    return {"instr": statistics.median(shifts.values()), "instr_shifts": shifts,
            "anon_kib": statistics.median(anon), "rss_kib": statistics.median(rss),
            "anon_runs": anon, "rss_runs": rss}


def defined():
    return [n for n, e in STARTS.items() if e is not None]


def build_trees(binary, checkout):
    """The checkout's R_HOME trees, the database one written by `binary`."""
    for cmd in (["scripts/rt-home.sh"], ["scripts/rt-home-db.sh", "--robigo-rt", os.path.abspath(binary)]):
        p = subprocess.run(cmd, cwd=checkout, capture_output=True, text=True)
        if p.returncode:
            raise SystemExit(f"rt-startup: {' '.join(cmd)} failed:\n{(p.stdout + p.stderr)[-3000:]}")


def run(binary, starts=None, checkout=ROOT, runs=9, trees=False, valgrind=None):
    """robigo-rt's starts (`starts`, default every defined one) of `binary`:
    {start: measure()'s numbers}."""
    names = starts or defined()
    for n in names:
        if STARTS.get(n) is None:
            raise SystemExit(f"rt-startup: no start '{n}' (defined: {', '.join(defined())})")
    if trees:
        build_trees(binary, checkout)
    env0 = clean_env()
    return {n: measure([os.path.abspath(binary), "--vanilla", "-e", "1"],
                       dict(env0, ROBIGO_RT_CHECKOUT=os.path.abspath(checkout), **STARTS[n]), runs,
                       valgrind or os.environ.get("VALGRIND", "valgrind"))
            for n in names}


def run_gnur(runs=9, valgrind=None):
    """GNU R's starts (GNUR_STARTS) of RSCRIPT_BIN in R_HOME_DIR."""
    exe = os.environ.get("RSCRIPT_BIN")
    home = os.environ.get("R_HOME_DIR")
    if not exe or not home:
        raise SystemExit("rt-startup: GNU R's starts need RSCRIPT_BIN (GNU R's exec/R) and R_HOME_DIR")
    res = {}
    for n, extra in GNUR_STARTS.items():
        env = dict(clean_env(), R_HOME=home, **extra)
        lib = os.path.join(home, "lib")
        env["LD_LIBRARY_PATH"] = lib + (":" + env["LD_LIBRARY_PATH"] if env.get("LD_LIBRARY_PATH") else "")
        res[n] = measure([exe, "--vanilla", "--no-echo", "-e", "1"], env, runs,
                         valgrind or os.environ.get("VALGRIND", "valgrind"))
    return res


def clean_env():
    env = {k: v for k, v in os.environ.items() if not k.startswith(("R_", "ROBIGO_"))}
    env.update(BASE)
    return env


def anon_limit_kib(main_kib):
    return max(main_kib * ANON_LIMIT / 100, ANON_FLOOR_KIB)


def verdict(new, main):
    """The startup gate's failures against main's numbers: a list of
    messages, one per start and number over its limit."""
    fails = []
    for n, v in new.items():
        m = main.get(n)
        if not m:
            continue
        if v["instr"] > m["instr"] * (1 + INSTR_LIMIT / 100):
            fails.append(f"start {n}: {v['instr']:,.0f} instructions, {pct(v['instr'], m['instr']):+.2f}% against "
                         f"main's {m['instr']:,.0f} (limit +{INSTR_LIMIT}%)")
        lim = anon_limit_kib(m["anon_kib"])
        if v["anon_kib"] > m["anon_kib"] + lim:
            fails.append(f"start {n}: peak anonymous memory {v['anon_kib']:,.0f} KiB, "
                         f"{v['anon_kib'] - m['anon_kib']:+,.0f} KiB against main's {m['anon_kib']:,.0f} "
                         f"(limit +{lim:,.0f} KiB: {ANON_LIMIT}%, at least {ANON_FLOOR_KIB} KiB)")
        if v["rss_kib"] > m["rss_kib"] * (1 + RSS_LIMIT / 100):
            fails.append(f"start {n}: peak RSS {v['rss_kib']:,.0f} KiB, {pct(v['rss_kib'], m['rss_kib']):+.2f}% "
                         f"against main's {m['rss_kib']:,.0f} (limit +{RSS_LIMIT}%)")
    return fails


def pct(a, b):
    return (a - b) / b * 100


def table(entries, new, main=None, gnur=None):
    """The startup table, markdown lines: `entries` the bars file's
    [[startup]] (name, and the GNU R start it is shown beside)."""
    lines = ["| start | instructions | main | change | anon KiB | main | change | peak RSS KiB | main | change "
             "| GNU R instructions | anon KiB | peak RSS KiB |",
             "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|"]
    for e in entries:
        n = e["name"]
        v = new[n]
        m = (main or {}).get(n)
        g = (gnur or {}).get(e.get("gnur", ""))
        cells = [n, f"{v['instr']:,.0f}"]
        for k in ("instr", "anon_kib", "rss_kib"):
            if k != "instr":
                cells.append(f"{v[k]:,.0f}")
            cells += [f"{m[k]:,.0f}", f"{pct(v[k], m[k]):+.2f}%"] if m else ["", ""]
        cells += [f"{g['instr']:,.0f} ({e['gnur']})", f"{g['anon_kib']:,.0f}", f"{g['rss_kib']:,.0f}"] if g \
            else ["", "", ""]
        lines.append("| " + " | ".join(cells) + " |")
    return lines


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    side = ap.add_mutually_exclusive_group(required=True)
    side.add_argument("--robigo-rt", help="robigo-rt's binary")
    side.add_argument("--gnur", action="store_true",
                      help="GNU R's starts: RSCRIPT_BIN (exec/R) and R_HOME_DIR, as idiom-vs-r.py")
    ap.add_argument("--checkout", default=ROOT,
                    help="ROBIGO_RT_CHECKOUT for robigo-rt: its .robigo/rt-home and rt-home-db trees")
    ap.add_argument("--trees", action="store_true",
                    help="build the checkout's R_HOME trees with BIN first (the database one is BIN's)")
    ap.add_argument("--starts", help="robigo-rt's starts to measure, comma-separated (default: all defined)")
    ap.add_argument("--runs", type=int, default=9, help="memory runs per start (default 9)")
    ap.add_argument("--valgrind", default=os.environ.get("VALGRIND", "valgrind"))
    ap.add_argument("--json", help="write the numbers here")
    a = ap.parse_args()
    if a.robigo_rt:
        res = run(a.robigo_rt, a.starts.split(",") if a.starts else None, a.checkout, a.runs, a.trees, a.valgrind)
    else:
        res = run_gnur(a.runs, a.valgrind)
    for n, v in res.items():
        print(f"{n}: {v['instr']:,.0f} instructions (shifts {', '.join(f'{x:,}' for x in v['instr_shifts'].values())}), "
              f"anon {v['anon_kib']:,.0f} KiB, peak RSS {v['rss_kib']:,.0f} KiB")
    if a.json:
        with open(a.json, "w") as f:
            json.dump(res, f, indent=1, sort_keys=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
