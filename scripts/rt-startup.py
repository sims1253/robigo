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
sample /proc. Anonymous memory grows only by page faults and shrinks only by
system calls, so the process is stopped at each system call under ptrace and
its anonymous memory read there from the page tables (smaps_rollup's
`Anonymous`): the largest reading is the peak (memory() says when it is).
Peak RSS (VmHWM) comes from other runs, stopped only at their exit: the
system-call stops move it by up to 7%.
The file-backed part of the RSS is the binary's and its libraries' pages,
faulted in around the code that ran. It follows code layout (#959: 45.8 KB
more code, +0.2–0.4 MB of RSS, anonymous memory unchanged), so peak RSS is
reported but gated loosely. Address randomization is off for the measured
process (personality(ADDR_NO_RANDOMIZE), as setarch -R): with it on, the
pages a start touches vary by run.

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
import threading

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

PTRACE_TRACEME, PTRACE_CONT, PTRACE_SYSCALL, PTRACE_SETOPTIONS, PTRACE_GETEVENTMSG = 0, 7, 24, 0x4200, 0x4201
PTRACE_O_TRACESYSGOOD, PTRACE_O_TRACECLONE, PTRACE_O_TRACEEXEC, PTRACE_O_TRACEEXIT = 0x1, 0x8, 0x10, 0x40
PTRACE_EVENT_CLONE, PTRACE_EVENT_EXEC, PTRACE_EVENT_EXIT = 3, 4, 6
# waitpid's flag for a traced thread as well as a process
WALL = 0x40000000
# a run's limit in seconds: a start that loops is killed, not waited on
TIMEOUT = 300
# personality(2)'s flag that turns address-space randomization off (setarch -R)
ADDR_NO_RANDOMIZE = 0x0040000
libc = ctypes.CDLL(None, use_errno=True)
libc.ptrace.argtypes = [ctypes.c_long, ctypes.c_long, ctypes.c_void_p, ctypes.c_void_p]
libc.ptrace.restype = ctypes.c_long


def event_msg(pid):
    """PTRACE_GETEVENTMSG: at a clone stop, the new thread's id."""
    msg = ctypes.c_ulong()
    if libc.ptrace(PTRACE_GETEVENTMSG, pid, None, ctypes.byref(msg)) == -1:
        e = ctypes.get_errno()
        raise OSError(e, f"ptrace(GETEVENTMSG): {os.strerror(e)}")
    return msg.value


def ptrace(req, pid, data=0):
    if libc.ptrace(req, pid, None, ctypes.c_void_p(data)) == -1:
        e = ctypes.get_errno()
        raise OSError(e, f"ptrace({req}): {os.strerror(e)}")


def anon_kib(pid):
    """The anonymous memory mapped resident now, in KiB: smaps_rollup's
    `Anonymous`, which the kernel counts by walking the page tables. (RssAnon
    in /proc/PID/status is a batched per-CPU counter, behind by up to a
    batch of pages per CPU.)"""
    with open(f"/proc/{pid}/smaps_rollup") as f:
        for ln in f:
            if ln.startswith("Anonymous:"):
                return int(ln.split()[1])
    raise SystemExit(f"rt-startup: no Anonymous line in /proc/{pid}/smaps_rollup")


def status_kib(pid):
    """/proc/PID/status's memory lines, KiB by name."""
    out = {}
    with open(f"/proc/{pid}/status") as f:
        for ln in f:
            m = re.match(r"^(Vm\w+|Rss\w+):\s+(\d+) kB", ln) or re.match(r"^(Threads):\s+(\d+)$", ln)
            if m:
                out[m.group(1)] = int(m.group(2))
    return out


def memory(cmd, env, syscalls=True, timeout=TIMEOUT):
    """(peak anonymous KiB, peak RSS KiB, exit status, stdout) of one run of
    `cmd` under ptrace; the exit status is negative for a fatal signal, as
    subprocess's.

    Anonymous memory grows only by page faults and shrinks only by system
    calls (munmap, madvise, brk, mremap, exit). So with `syscalls` the child
    is stopped at every system call (PTRACE_SYSCALL) and its anonymous
    memory read there from the page tables (anon_kib()): the largest reading
    is the peak. That holds for one thread with no swap:
    - memory the kernel takes back by itself between two stops (swapping
      out, splitting an underused huge page) makes the reading low, by what
      it missed; perf-rt's runners have no swap;
    - a thread the child starts would not be seen, so a start that starts
      one is refused at the thread's creation (PTRACE_O_TRACECLONE).

    Peak RSS is VmHWM, read at the exit stop (PTRACE_EVENT_EXIT, before the
    kernel tears the memory down). Take it from a run without `syscalls`,
    whose stops do not change which file pages are faulted in (measure()).
    Signals other than ptrace's own go on to the child (a group-stop is
    resumed: a start has no job control). A run longer than `timeout`
    seconds is killed, through a pidfd, and reported."""
    out = tempfile.TemporaryFile()
    pid = None
    pidfd = None
    timer = None
    threads = []
    # the timer's state, under its lock: whether it fired, whether the run is
    # over (then it sends nothing)
    clock = {"lock": threading.Lock(), "fired": False, "over": False}

    def expire():
        with clock["lock"]:
            if clock["over"]:
                return
            clock["fired"] = True
            try:
                # through a pidfd: never another process that took the pid
                signal.pidfd_send_signal(pidfd, signal.SIGKILL)
            except ProcessLookupError:
                pass

    try:
        pid = os.fork()
        if pid == 0:
            try:
                os.dup2(out.fileno(), 1)
                fd = os.open(os.devnull, os.O_WRONLY)
                os.dup2(fd, 2)
                ptrace(PTRACE_TRACEME, 0)
                # one layout for every run: with randomized addresses the pages
                # a start touches, and so its peak RSS, vary by run
                if libc.personality(ADDR_NO_RANDOMIZE) == -1:
                    os._exit(126)
                os.execve(cmd[0], cmd, env)
            finally:
                os._exit(127)
        pidfd = os.pidfd_open(pid)
        # the stop at the first exec (a SIGTRAP, before the options are set)
        _, st = os.waitpid(pid, 0)
        if not os.WIFSTOPPED(st):
            code = os.waitstatus_to_exitcode(st)
            pid = None
            why = "personality(ADDR_NO_RANDOMIZE) refused" if code == 126 else f"exit status {code}"
            raise SystemExit(f"rt-startup: {cmd[0]} did not start under ptrace: {why}")
        ptrace(PTRACE_SETOPTIONS, pid,
               PTRACE_O_TRACEEXIT | PTRACE_O_TRACEEXEC | PTRACE_O_TRACECLONE | PTRACE_O_TRACESYSGOOD)
        timer = threading.Timer(timeout, expire)
        timer.start()
        anon, hwm = 0, None
        go = PTRACE_SYSCALL if syscalls else PTRACE_CONT
        ptrace(go, pid)
        while True:
            _, st = os.waitpid(pid, 0)
            if os.WIFEXITED(st) or os.WIFSIGNALED(st):
                pid = None
                break
            sig, event = os.WSTOPSIG(st), st >> 16
            if sig == signal.SIGTRAP | 0x80:
                # a system call's entry or exit
                anon = max(anon, anon_kib(pid))
                ptrace(go, pid)
            elif sig == signal.SIGTRAP and event == PTRACE_EVENT_CLONE:
                # a second thread, refused as it is made: its memory would not
                # be seen (the new thread, traced too, is reaped with the child)
                threads.append(event_msg(pid))
                raise SystemExit(f"rt-startup: {cmd[0]} started a thread; the anonymous peak is measured "
                                 "for one (memory())")
            elif sig == signal.SIGTRAP and event == PTRACE_EVENT_EXIT:
                hwm = status_kib(pid)["VmHWM"]
                if syscalls:
                    anon = max(anon, anon_kib(pid))
                ptrace(PTRACE_CONT, pid)
            elif sig == signal.SIGTRAP and event == PTRACE_EVENT_EXEC:
                # a later exec: ptrace's stop, not a signal
                ptrace(go, pid)
            else:
                # a signal for the child (a real SIGTRAP among them): it goes on
                ptrace(go, pid, sig)
        with clock["lock"]:
            clock["over"] = True
            fired = clock["fired"]
        if fired:
            raise SystemExit(f"rt-startup: {' '.join(cmd)} ran over {timeout} s")
        if hwm is None:
            raise SystemExit(f"rt-startup: no exit stop for {cmd[0]}")
        out.seek(0)
        return anon, hwm, os.waitstatus_to_exitcode(st), out.read()
    finally:
        with clock["lock"]:
            clock["over"] = True
        if timer:
            timer.cancel()
            timer.join()
        if pid:
            reap(pid, threads)
        if pidfd is not None:
            os.close(pidfd)
        out.close()


def reap(pid, threads=()):
    """Kill the traced child `pid` and wait it and its traced `threads` out.
    The kill comes first, so that no thread can run on to make another; then
    each is resumed, since one may sit in a stop already consumed (a reading
    that failed at its exit stop, where the SIGKILL is dropped and only a
    resume lets the exit finish); each may stop again (its exit event)
    before it is gone, and is resumed until it is."""
    try:
        os.kill(pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    for t in (pid, *threads):
        try:
            ptrace(PTRACE_CONT, t)
        except OSError:
            pass
    for t in (*threads, pid):
        while True:
            try:
                _, st = os.waitpid(t, WALL)
            except ChildProcessError:
                break
            if os.WIFEXITED(st) or os.WIFSIGNALED(st):
                break
            try:
                ptrace(PTRACE_CONT, t)
            except OSError:
                pass


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
    # the anonymous peak from runs stopped at each system call, peak RSS from
    # runs stopped only at exit (memory())
    for syscalls in (True, False):
        for _ in range(runs):
            a, h, status, out = memory(cmd, env, syscalls)
            if status != 0:
                raise SystemExit(f"rt-startup: {' '.join(cmd)} exited {status}")
            if want is None:
                want = out
            elif out != want:
                raise SystemExit(f"rt-startup: {' '.join(cmd)} printed differently between runs")
            (anon if syscalls else rss).append(a if syscalls else h)
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
        # (a start main did not measure, or not wholly: nothing to hold it to)
        if not m or any(k not in m for k in ("instr", "anon_kib", "rss_kib")):
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
        if m and any(k not in m for k in ("instr", "anon_kib", "rss_kib")):
            m = None
        if g and any(k not in g for k in ("instr", "anon_kib", "rss_kib")):
            g = None
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
