#!/usr/bin/env python3
"""The cost of a start: `--vanilla -e 1` with R_DEFAULT_PACKAGES=NULL, of
robigo-rt and of GNU R.

    scripts/rt-startup.py --robigo-rt BIN [--checkout DIR] [--trees | --trees-from TAR] [--starts A,B] [--runs K] [--json OUT]
    scripts/rt-startup.py --robigo-rt BIN [--checkout DIR] --pack-trees OUT
    scripts/rt-startup.py --gnur [--runs K] [--json OUT]

Each binary boots from the R_HOME trees its own build writes, as it ships
(.robigo/rt-home and rt-home-db, and a translation cache beside the
latter: every .robigo/rt-home* directory). They are built from the
checkout's scripts and sources but never in its .robigo: each run's go in a
scratch directory of their own (scratch.py, removed on exit), so neither a
developer's tree nor an rt-ci in the same checkout sees one rebuilt.
- --trees builds them first with BIN (scripts/rt-home.sh,
  scripts/rt-home-db.sh --robigo-rt BIN), which needs cargo and GNU R's
  sources: locally only. In CI, where the measuring job (perf-rt) runs in
  the oracle image without a toolchain, building is refused.
- --pack-trees OUT builds them with BIN and packs them into the tarball
  OUT, and measures nothing: what CI's perf-rt-build does for the PR's
  binary and for main's, in their own checkouts.
- --trees-from TAR unpacks such a tarball (into a scratch directory) first.
- with neither, the starts boot from the checkout's own .robigo trees.

perf.py rt-starts gates robigo-rt's starts against main's binary (the
[[startup]] entries of benchmarks/rt/m1-bars.toml), and perf.py rt-gnur
records GNU R's beside them, report-only.

Each start has these numbers, the first three judged against main's:
- `instr`: callgrind's total instructions of the whole process, the mean
  over the layout shifts (below);
- `heap_bytes`: the peak of malloc'd bytes, DHAT's t-gmax (robigo-heap's
  chunks included), one run at the first shift: exact and repeatable, no
  page or placement in it (malloc_peak());
- `rss_kib`: total peak RSS, `VmHWM`, in KiB, the median of `runs` runs at
  the home itself;
- reported only: `anon_kib`, peak anonymous memory in KiB (see below), the
  mean over the shifts: physical pages, which glibc's placement of the
  runtime's allocations moves by tens of KiB between binaries doing the
  same work; and `gc_peak`, robigo-heap's own peak in use, where the
  binary prints it (gc_peak()).

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

**Layout shifts.** A start's instruction count follows the heap's layout,
which the length of its home's path moves: the path's strings are allocated
early, and every table grown by realloc later either extends in place or
is copied, by what lies above it (H, 2026-11-23: steps of ~80k on the
interim start, at lengths that differ per binary). One start's count spans
0.7-1.15% of the start over path lengths 35-139, more than the gate's
limit, so a single path decided the verdict by placement. So each start
boots from LAYOUT_LENGTHS homes: symlinks to its trees whose absolute paths
are exactly those lengths (layout_homes()), the same for both binaries
wherever their trees are unpacked, and its count is the mean of the 15
(docs/ledger/g3/2026-11-23-startup-layout.md).
"""
import argparse
import concurrent.futures
import ctypes
import errno
import json
import os
import re
import signal
import statistics
import subprocess
import sys
import shutil
import tarfile
import tempfile
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, HERE)
from scratch import scratch_dir  # noqa: E402

# ROBIGO_RT_BOOT's value for M1's interim loader. Today it is the binary's
# default (unset or empty); L's default-boot flip makes rdb the default and
# this "m1", here only. Every start and every row names its boot, so no
# measurement depends on a binary's default, and the gate never compares two
# boots (BOOTS: rt-bars' --boot)
M1_BOOT = ""
BOOTS = {"m1": M1_BOOT, "rdb": "rdb"}
# the start bar 5 is judged on, and the default boot's
DEFAULT_START = "rdb-jit"
# a start by name: the environment it runs with, over BASE
STARTS = {
    "interim": {"ROBIGO_RT_BOOT": M1_BOOT},
    "rdb-jit": {"ROBIGO_RT_BOOT": "rdb"},
    "rdb-nojit": {"ROBIGO_RT_BOOT": "rdb", "R_ENABLE_JIT": "0"},
    # (the rdb starts above are the defaults, with the translation cache
    # beside rt-home-db; this one guards the uncached path, the
    # translator's own cost, like for like with main: a binary without the
    # cache ignores the variable, and the row is its rdb-jit start)
    "rdb-nocache": {"ROBIGO_RT_BOOT": "rdb", "ROBIGO_TRANSLATION_CACHE": "0"},
    # reserved for L5: default packages loaded (defined when it can run)
    "rdb-pkgs": None,
}
GNUR_STARTS = {"jit": {}, "nojit": {"R_ENABLE_JIT": "0"}}
BASE = {"R_DEFAULT_PACKAGES": "NULL", "LANGUAGE": "en", "LC_ALL": "C.UTF-8", "TZ": "UTC",
        "R_ENVIRON_USER": "", "R_PROFILE_USER": ""}
# the layout shifts: the absolute lengths of the home a start boots from
# (ROBIGO_RT_CHECKOUT for robigo-rt, R_HOME for GNU R), 15 lengths 7 apart.
# Spread over a span this wide, the mean of a no-op change against main
# stayed within ±0.2% (docs/ledger/g3/2026-11-23-startup-layout.md)
LAYOUT_LENGTHS = tuple(range(35, 35 + 7 * 15, 7))
# with each length, glibc malloc's top_pad (the extra each heap extension
# takes), from its default, 128 KiB, by a page: the home's length moves
# where the early allocations fall, top_pad where the heap's top lies, and a
# binary's own size moves its heap's start, which the length alone does not
# cancel (a no-op build, 4 KiB more data: interim -0.67% over the lengths
# alone, -0.07% over the pairs)
TOP_PADS = tuple(131072 + 4096 * k for k in range(15))
LAYOUT_SHIFTS = tuple(zip(LAYOUT_LENGTHS, TOP_PADS))

# the anonymous peak's figure over the layout shifts (measure()). A layout's
# peak is exact (the same run after run), but layouts differ by pages: over
# the 15 shifts a start's readings spread over 64-230 KiB in 4 KiB steps.
# Between a binary and a no-op build of it, the minimum moved by up to
# 72 KiB and the median by up to 60, the mean by at most 14
# (docs/ledger/g3/2026-11-23-startup-layout.md)
ANON_STAT = statistics.fmean

# the gate against main's binary (perf.py rt-starts), in percent: instructions
# as the rows' THRESHOLD; memory as the peak of malloc'd bytes, DHAT's
# t-gmax (bytes requested, robigo-heap's chunks among them: Rust's
# allocator takes them from malloc), exact and repeatable for a given
# allocation sequence, +max(HEAP_LIMIT%, HEAP_FLOOR_BYTES); total peak RSS
# loosely, since its file-backed part follows code layout. The anonymous
# peak (physical pages, which glibc's placement moves by tens of KiB between
# binaries doing the same work) is reported, not judged
# (docs/ledger/g3/2026-11-23-startup-layout.md)
INSTR_LIMIT = 0.5
HEAP_LIMIT = 0.5
HEAP_FLOOR_BYTES = 16 << 10
RSS_LIMIT = 5.0

PTRACE_TRACEME, PTRACE_CONT, PTRACE_DETACH, PTRACE_SYSCALL = 0, 7, 17, 24
PTRACE_SETOPTIONS, PTRACE_GETEVENTMSG = 0x4200, 0x4201
PTRACE_O_TRACESYSGOOD, PTRACE_O_TRACEFORK, PTRACE_O_TRACEVFORK = 0x1, 0x2, 0x4
PTRACE_O_TRACECLONE, PTRACE_O_TRACEEXEC, PTRACE_O_TRACEEXIT = 0x8, 0x10, 0x40
PTRACE_EVENT_FORK, PTRACE_EVENT_VFORK, PTRACE_EVENT_CLONE, PTRACE_EVENT_EXEC, PTRACE_EVENT_EXIT = 1, 2, 3, 4, 6
# waitpid's flag for a traced thread as well as a process
WALL = 0x40000000
# a run's limit in seconds: a start that loops is killed, not waited on.
# Under callgrind, which runs a start some 50 times slower, a limit of its
# own: an rdb start takes tens of seconds there, so 1,800 s is a hang, not a
# slow runner
TIMEOUT = 300
CALLGRIND_TIMEOUT = 1800
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


def tgid(tid):
    """The thread group (process) that thread `tid` belongs to."""
    with open(f"/proc/{tid}/status") as f:
        for ln in f:
            if ln.startswith("Tgid:"):
                return int(ln.split()[1])
    raise SystemExit(f"rt-startup: no Tgid for {tid}")


def ptrace(req, pid, data=0):
    if libc.ptrace(req, pid, None, ctypes.c_void_p(data)) == -1:
        e = ctypes.get_errno()
        raise OSError(e, f"ptrace({req}): {os.strerror(e)}")


def resume(req, tid, sig=0):
    """ptrace(req, tid, sig) restarting a stop memory()'s loop gathered.
    ESRCH there is a tracee killed while its stop waited in the round: a
    SIGKILL ends any ptrace stop (another thread's exit_group, or the start's
    process group killed at its exit stop), and its exit is reported next."""
    if libc.ptrace(req, tid, None, ctypes.c_void_p(sig)) == -1:
        e = ctypes.get_errno()
        if e != errno.ESRCH:
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


def thread_anon_kib(tid):
    """anon_kib() read through thread `tid`, which is stopped: 0 when its
    memory is already gone (a thread past its exit, the process's last)."""
    try:
        return anon_kib(tid)
    except (ProcessLookupError, FileNotFoundError):
        return 0


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
    """(peak anonymous KiB, peak RSS KiB, exit status, stdout, children) of
    one run of `cmd` under ptrace; the exit status is negative for a fatal
    signal, as subprocess's, and `children` the processes it started.

    Anonymous memory grows only by page faults and shrinks only by system
    calls (munmap, madvise, brk, mremap, exit). So with `syscalls` the child
    is stopped at every system call (PTRACE_SYSCALL) and its anonymous
    memory read there from the page tables (anon_kib()): the largest reading
    is the peak. That holds with no swap: memory the kernel takes back by
    itself between two stops (swapping out, splitting an underused huge
    page) makes the reading low, by what it missed; perf-rt's runners have
    no swap.
    - **Threads** share the process's memory, which smaps_rollup counts for
      all of them, but each frees through its own system calls: so every
      thread is traced (PTRACE_O_TRACECLONE attaches it) and stopped at its
      system calls too, and the reading is taken at any thread's stop.
    - **Processes it starts** (fork, vfork, posix_spawn, or a clone without
      CLONE_THREAD; GNU R's exit runs `sh -c 'rm -Rf' R_TempDir` through
      posix_spawn) have memory of their own, not the start's: each is
      detached as it is made and counted. They run in the start's process
      group, which is killed at the start's exit stop, so none outlives the
      measurement.

    Peak RSS is VmHWM, read at the exit stop (PTRACE_EVENT_EXIT, before the
    kernel tears the memory down). Take it from a run without `syscalls`,
    whose stops do not change which file pages are faulted in (measure()).
    Signals other than ptrace's own go on to the child; a stop signal then
    group-stops it, and the group-stop is resumed (the restart's signal is
    ignored there, ptrace(2)): a start has no job control. A run longer than `timeout`
    seconds is killed, through a pidfd, and reported."""
    out = tempfile.TemporaryFile()
    pid = None
    pidfd = None
    timer = None
    threads = []
    # the threads whose first stop has been resumed
    started = set()
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
                # a process group of its own: its children are killed with it
                os.setpgid(0, 0)
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
               PTRACE_O_TRACEEXIT | PTRACE_O_TRACEEXEC | PTRACE_O_TRACECLONE | PTRACE_O_TRACEFORK
               | PTRACE_O_TRACEVFORK | PTRACE_O_TRACESYSGOOD)
        timer = threading.Timer(timeout, expire)
        timer.start()
        anon, hwm = 0, None
        go = PTRACE_SYSCALL if syscalls else PTRACE_CONT
        # the traced threads (the leader's own pid apart) and the processes
        # the start made, as their events name them; a stop of a thread or
        # process no event has named yet waits in `early` for its event
        children, early = [], {}
        # the start's own process group (setpgid in the child): events of
        # its threads and the processes it makes, never another measurement's
        # (measure() and probes run several at once in one process)
        group = pid
        ptrace(go, pid)
        # every stop pending at one moment, gathered before any of them is
        # resumed: waitpid returns the first waitable tracee in the kernel's
        # list, so threads that stop again at once (system calls in a loop,
        # a lock's timed waits) would be served forever while a thread later
        # in the list starves, and if it holds what they wait for, the start
        # never ends (strace gathers its stops the same way). Each round
        # serves every thread stopped at its start.
        pending = []
        while True:
            if not pending:
                pending.append(os.waitpid(-group, WALL))
                while True:
                    try:
                        w = os.waitpid(-group, WALL | os.WNOHANG)
                    except ChildProcessError:
                        break
                    if w[0] == 0:
                        break
                    pending.append(w)
            t, st = pending.pop(0)
            if t == pid and (os.WIFEXITED(st) or os.WIFSIGNALED(st)):
                pid = None
                break
            if os.WIFEXITED(st) or os.WIFSIGNALED(st):
                # a thread that ended (or a detached child we still saw)
                if t in threads:
                    threads.remove(t)
                continue
            sig, event = os.WSTOPSIG(st), st >> 16
            if t != pid and t not in threads:
                # a new thread's or process's first stop before its event
                early[t] = sig
                continue
            if sig == signal.SIGTRAP | 0x80:
                # a system call's entry or exit, of any thread: read through
                # that thread, which shares the memory (the leader may have
                # ended, a zombie whose /proc no longer maps it)
                anon = max(anon, thread_anon_kib(t))
                resume(go, t)
            elif sig == signal.SIGTRAP and event in (PTRACE_EVENT_CLONE, PTRACE_EVENT_FORK, PTRACE_EVENT_VFORK):
                new = event_msg(t)
                if event == PTRACE_EVENT_CLONE and tgid(new) == pid:
                    threads.append(new)
                    # its first stop (a SIGSTOP) resumed, not delivered
                    if early.pop(new, None) is not None:
                        started.add(new)
                        resume(go, new)
                else:
                    # a process of its own: detached at its first stop,
                    # which an earlier round may have kept (`early`), this
                    # round may hold (`pending`: waiting for it again would
                    # block for good), or is still to come
                    children.append(new)
                    mine = [w for w in pending if w[0] == new]
                    if early.pop(new, None) is not None:
                        ptrace(PTRACE_DETACH, new)
                    elif mine:
                        pending.remove(mine[0])
                        if os.WIFSTOPPED(mine[0][1]):
                            ptrace(PTRACE_DETACH, new)
                    else:
                        _, cst = os.waitpid(new, WALL)
                        if os.WIFSTOPPED(cst):
                            ptrace(PTRACE_DETACH, new)
                resume(go, t)
            elif sig == signal.SIGTRAP and event == PTRACE_EVENT_EXIT:
                if t == pid:
                    hwm = status_kib(pid)["VmHWM"]
                    # the processes it started, which outlive it otherwise
                    kill_group(pid)
                if syscalls:
                    anon = max(anon, thread_anon_kib(t))
                resume(PTRACE_CONT, t)
            elif sig == signal.SIGTRAP and event == PTRACE_EVENT_EXEC:
                # a later exec: ptrace's stop, not a signal (an exec by
                # another thread makes it the leader, keeping the pid)
                resume(go, t)
            elif sig == signal.SIGSTOP and t in threads and event == 0 and t not in started:
                # a new thread's first stop: resumed, not delivered
                started.add(t)
                resume(go, t)
            else:
                # a signal for the child (a real SIGTRAP among them): it goes
                # on. A stop signal delivered so group-stops the child, which
                # reports as this same kind of stop; restarting it with the
                # signal again resumes it, because a restart's signal is
                # ignored in any stop but a signal's delivery (ptrace(2),
                # "Signal injection and suppression")
                resume(go, t, sig)
        with clock["lock"]:
            clock["over"] = True
            fired = clock["fired"]
        if fired:
            raise SystemExit(f"rt-startup: {' '.join(cmd)} ran over {timeout} s")
        if hwm is None:
            raise SystemExit(f"rt-startup: no exit stop for {cmd[0]}")
        out.seek(0)
        return anon, hwm, os.waitstatus_to_exitcode(st), out.read(), len(children)
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


def kill_group(pid):
    """SIGKILL to every process in `pid`'s process group but `pid` itself:
    the processes the start made (memory() gave it a group of its own). Read
    while `pid` is not reaped, so the group's id is still its own."""
    killed = []
    for d in os.listdir("/proc"):
        if not d.isdigit() or int(d) == pid:
            continue
        try:
            with open(f"/proc/{d}/stat") as f:
                # pgrp is the 5th field, after the parenthesized name
                pgrp = int(f.read().rsplit(")", 1)[1].split()[2])
        except (OSError, IndexError, ValueError):
            continue
        if pgrp == pid:
            try:
                os.kill(int(d), signal.SIGKILL)
                killed.append(int(d))
            except ProcessLookupError:
                pass
    # (each one dies when next scheduled: waited for, briefly, so that none
    # is still running when the measurement returns)
    deadline = time.monotonic() + 2
    for k in killed:
        while time.monotonic() < deadline:
            try:
                with open(f"/proc/{k}/stat") as f:
                    if f.read().rsplit(")", 1)[1].split()[0] in ("Z", "X"):
                        break
            except OSError:
                break
            time.sleep(0.005)


def reap(pid, threads=()):
    """Kill the traced child `pid` and wait it and its traced `threads` out.
    The kills come first, to every one (a clone without CLONE_THREAD is a
    process of its own, which the leader's kill does not reach), so that
    none can run on to make another; then each is resumed, since one may sit in a stop already consumed (a reading
    that failed at its exit stop, where the SIGKILL is dropped and only a
    resume lets the exit finish); each may stop again (its exit event)
    before it is gone, and is resumed until it is."""
    kill_group(pid)
    for t in (pid, *threads):
        try:
            os.kill(t, signal.SIGKILL)
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
        try:
            p = subprocess.run([valgrind, "--tool=callgrind", f"--callgrind-out-file={out}", *cmd], env=env,
                               stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True,
                               timeout=CALLGRIND_TIMEOUT)
        except subprocess.TimeoutExpired:
            raise SystemExit(f"rt-startup: {' '.join(cmd)} ran over {CALLGRIND_TIMEOUT} s under callgrind")
        if p.returncode:
            raise SystemExit(f"rt-startup: {' '.join(cmd)} under callgrind exited {p.returncode}:\n{p.stderr[-2000:]}")
        with open(out, errors="replace") as f:
            for ln in f:
                if ln.startswith("summary:") or ln.startswith("totals:"):
                    return int(ln.split()[1])
    raise SystemExit(f"rt-startup: no total in callgrind's output of {' '.join(cmd)}")


def layout_homes(home):
    """{length: path}: a symlink to `home` for each of LAYOUT_LENGTHS, whose
    absolute path is exactly that long, in a scratch directory of its own
    under /tmp (a fixed-length name: `/tmp/rs-` and 12 hex digits), so every
    binary gets the same lengths whatever directory its trees are in."""
    base = scratch_dir("rs-", dir="/tmp")
    out = {}
    for n in LAYOUT_LENGTHS:
        pad = n - len(base) - 1
        if pad < 1:
            raise SystemExit(f"rt-startup: {base} is too long for a home of {n} characters")
        link = os.path.join(base, "p" * pad)
        os.symlink(os.path.abspath(home), link)
        out[n] = link
    return out


def measure(cmd, env, runs, valgrind, var, home):
    """One start's numbers: {"instr", "instr_shifts", "anon_kib",
    "anon_shifts", "rss_kib", "rss_runs", "children", "heap_bytes",
    "gc_peak"}, booting from `home` through the environment variable `var`
    (`children`: the processes one run started, GNU R's temporary
    directory's `rm` among them; `heap_bytes`: malloc_peak(), DHAT's t-gmax;
    `gc_peak`: gc_peak(), robigo-heap's peak line, None for a binary that
    does not print it)."""
    homes = layout_homes(home)

    def shifted(n):
        """`env` under layout shift `n` (a home length): that home, and its
        top_pad."""
        pad = dict(LAYOUT_SHIFTS)[n]
        return dict(env, **{var: homes[n]}, GLIBC_TUNABLES=f"glibc.malloc.top_pad={pad}")

    def one(n):
        return str(n), instructions(cmd, shifted(n), valgrind)

    with concurrent.futures.ThreadPoolExecutor(max_workers=min(len(homes), os.cpu_count() or 1)) as ex:
        shifts = dict(ex.map(one, homes))
    want = None
    children = 0

    def run(e, syscalls):
        nonlocal want, children
        a, h, status, out, children = memory(cmd, e, syscalls)
        if status != 0:
            raise SystemExit(f"rt-startup: {' '.join(cmd)} exited {status}")
        if want is None:
            want = out
        elif out != want:
            raise SystemExit(f"rt-startup: {' '.join(cmd)} printed differently between runs")
        return a if syscalls else h

    # the anonymous peak from runs stopped at each system call, one at each
    # layout shift, as the instructions: a start's peak follows its layout
    # (L: a 104 KiB step between two binaries at a few bytes of difference),
    # and is the same at one layout run after run. Peak RSS from runs
    # stopped only at exit, at the home itself (memory())
    anon_shifts = {str(n): run(shifted(n), True) for n in homes}
    rss = [run(dict(env, **{var: os.path.abspath(home)}), False) for _ in range(runs)]
    # memory as allocation: one run at the first shift (the figure moves
    # across shifts only by the home path's own strings, so both binaries
    # are held to the same one), and robigo-heap's own peak where the
    # binary reports it
    first = shifted(LAYOUT_SHIFTS[0][0])
    heap = malloc_peak(cmd, first, valgrind)
    gc = gc_peak(cmd, first)
    return {"instr": statistics.fmean(shifts.values()), "instr_shifts": shifts,
            "anon_kib": ANON_STAT(anon_shifts.values()), "anon_shifts": anon_shifts,
            "rss_kib": statistics.median(rss), "rss_runs": rss, "children": children,
            "heap_bytes": heap, "gc_peak": gc}


def malloc_peak(cmd, env, valgrind):
    """The most malloc'd bytes live at once in one run of `cmd`, as
    requested: DHAT's t-gmax, which valgrind tracks exactly at every
    allocation and free (no snapshot thinning, unlike massif, whose peak
    also weighs the allocator's overhead). No page, alignment or placement
    enters it. A realloc counts at its new size."""
    with tempfile.TemporaryDirectory(prefix="rt-dhat-") as d:
        out = os.path.join(d, "dhat.json")
        full = [valgrind, "--tool=dhat", f"--dhat-out-file={out}"] + cmd
        try:
            p = subprocess.run(full, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True,
                               timeout=CALLGRIND_TIMEOUT)
        except subprocess.TimeoutExpired:
            raise SystemExit(f"rt-startup: {' '.join(cmd)} ran over {CALLGRIND_TIMEOUT} s under DHAT")
        if p.returncode:
            raise SystemExit(f"rt-startup: {' '.join(cmd)} under DHAT exited {p.returncode}:\n{p.stderr[-2000:]}")
        with open(out, errors="replace") as f:
            return malloc_peak_of(f.read())


def malloc_peak_of(text):
    """t-gmax's bytes in a DHAT output file's text: the sum over its program
    points of the bytes each had live at t-gmax ("gb"). Only DHAT's file
    version 2 in heap mode, every point with a non-negative integer "gb", is
    read: anything else is refused, never read as zero (a figure of 0 would
    pass any limit)."""
    try:
        doc = json.loads(text)
    except ValueError:
        raise SystemExit("rt-startup: DHAT's output is not JSON")
    if not isinstance(doc, dict) or doc.get("dhatFileVersion") != 2 or doc.get("mode") != "heap":
        raise SystemExit("rt-startup: DHAT's output is not a version-2 heap profile "
                         f"(dhatFileVersion {doc.get('dhatFileVersion') if isinstance(doc, dict) else None!r}, "
                         f"mode {doc.get('mode') if isinstance(doc, dict) else None!r})")
    pps = doc.get("pps")
    if not isinstance(pps, list) or not pps:
        raise SystemExit("rt-startup: DHAT's output has no program points")
    total = 0
    for pp in pps:
        gb = pp.get("gb") if isinstance(pp, dict) else None
        if type(gb) is not int or gb < 0:
            raise SystemExit(f"rt-startup: a DHAT program point without a non-negative integer \"gb\": {pp!r:.200}")
        total += gb
    return total


# robigo-heap's own peak (ROBIGO_GC_STATS=1): bytes in use (objects, block
# overhead, large) and chunk bytes mapped, logical and exact
GC_PEAK = re.compile(r"^robigo gc: peak bytes in use (\d+) \(objects (\d+), block overhead (\d+), large (\d+)\); "
                     r"peak chunk bytes mapped (\d+)$", re.M)


def gc_peak_of(stderr):
    """{"in_use", "objects", "overhead", "large", "chunks"} from robigo-heap's
    peak line, or None when the binary does not print it."""
    m = GC_PEAK.search(stderr)
    if not m:
        return None
    return dict(zip(("in_use", "objects", "overhead", "large", "chunks"), map(int, m.groups())))


def gc_peak(cmd, env):
    """gc_peak_of() of one plain run of `cmd` under ROBIGO_GC_STATS=1 (GNU R
    ignores the variable, and prints no such line). A run that fails is
    refused, as malloc_peak()'s is: read as None it would print "-", the
    same as a binary that does not report the line."""
    try:
        p = subprocess.run(cmd, env=dict(env, ROBIGO_GC_STATS="1"), stdout=subprocess.DEVNULL,
                           stderr=subprocess.PIPE, text=True, timeout=TIMEOUT)
    except subprocess.TimeoutExpired:
        raise SystemExit(f"rt-startup: {' '.join(cmd)} ran over {TIMEOUT} s")
    if p.returncode:
        raise SystemExit(f"rt-startup: {' '.join(cmd)} under ROBIGO_GC_STATS=1 exited {p.returncode}:\n"
                         f"{p.stderr[-2000:]}")
    return gc_peak_of(p.stderr)


def cache_hits(stderr):
    """The hits of a start's translation-cache stats line
    (ROBIGO_TRANSLATION_CACHE_STATS=1), or None if it printed none."""
    for line in reversed(stderr.splitlines()):
        if line.startswith("translation cache: hit "):
            return int(line[len("translation cache: hit "):].split(",", 1)[0])
    return None


def require_cache_used(name, cmd, env):
    """A start whose tree has a translation cache beside it (and does not
    turn it off) must use it: a cache lost or never rebuilt leaves the start
    correct but uncached, which no comparison with main's (uncached) binary
    would show."""
    p = subprocess.run(cmd, env=dict(env, ROBIGO_TRANSLATION_CACHE_STATS="1"), stdout=subprocess.DEVNULL,
                       stderr=subprocess.PIPE, text=True, timeout=TIMEOUT)
    hits = cache_hits(p.stderr)
    if not hits:
        raise SystemExit(f"rt-startup: start {name} has a translation cache beside its tree and used none of it "
                         f"({'no stats line' if hits is None else 'hit 0'}): {p.stderr[-300:]}")


def defined():
    return [n for n, e in STARTS.items() if e is not None]


def spec_errors(entries):
    """What is wrong with a bars file's [[startup]] entries (`entries`), as
    messages: a name that is not a defined start (a reserved one included),
    a GNU R start that is not one of GNUR_STARTS, a name given twice. perf.py
    rt-bars checks them before it measures anything."""
    errs, seen = [], set()
    for i, e in enumerate(entries):
        n, g = e.get("name"), e.get("gnur")
        if n is None:
            errs.append(f"[[startup]] #{i + 1} has no name")
        elif STARTS.get(n) is None:
            what = "reserved, not defined yet" if n in STARTS else "no such start"
            errs.append(f"[[startup]] {n!r}: {what} (defined: {', '.join(defined())})")
        elif n in seen:
            errs.append(f"[[startup]] {n!r} is given twice")
        else:
            seen.add(n)
        if g is not None and g not in GNUR_STARTS:
            errs.append(f"[[startup]] {n!r}: gnur {g!r} is no GNU R start ({', '.join(GNUR_STARTS)})")
    return errs


def in_ci():
    return os.environ.get("GITHUB_ACTIONS") == "true" or os.environ.get("CI") == "true"


def tree_dirs(checkout):
    """The checkout's R_HOME trees: every .robigo/rt-home* directory."""
    d = os.path.join(checkout, ".robigo")
    if not os.path.isdir(d):
        return []
    return sorted(os.path.join(".robigo", n) for n in os.listdir(d)
                  if n.startswith("rt-home") and os.path.isdir(os.path.join(d, n)))


def clear_trees(home):
    for t in tree_dirs(home):
        shutil.rmtree(os.path.join(home, t))


def new_home():
    """A scratch directory to hold one binary's trees (as a checkout's
    .robigo holds them), removed when this process exits (scratch.py): the
    checkout's own .robigo is never written, so neither a developer's tree
    nor an rt-ci running in the same checkout sees it rebuilt."""
    return scratch_dir("rt-startup-trees-")


def pack_trees(binary, checkout, out):
    """Build `binary`'s trees afresh, from the checkout's sources, in a
    scratch home, and pack them into the tarball `out` (paths relative to
    the home: .robigo/rt-home*)."""
    home = new_home()
    # (packing is what CI's build job does: not refused there, as
    # build_trees is for a measurement)
    make_trees(binary, checkout, home)
    trees = tree_dirs(home)
    if not trees:
        raise SystemExit(f"rt-startup: no .robigo/rt-home* tree in {home} after building")
    with tarfile.open(out, "w") as t:
        for d in trees:
            t.add(os.path.join(home, d), arcname=d)
    return trees


def unpack_trees(tar, home):
    """The trees of the tarball `tar` (pack_trees') into the home `home`
    (a scratch directory, new_home())."""
    if not os.path.isfile(tar):
        raise SystemExit(f"rt-startup: no trees at {tar} (perf-rt-build packs them: --pack-trees)")
    clear_trees(home)
    with tarfile.open(tar) as t:
        names = t.getnames()
        bad = [n for n in names if not (n == ".robigo" or n.startswith(".robigo/rt-home"))
               or os.path.isabs(n) or ".." in n.split("/")]
        if bad:
            raise SystemExit(f"rt-startup: {tar} holds more than .robigo/rt-home* trees: {bad[:3]}")
        # regular files and directories only: a link (a tree's root a
        # symlink to ../scripts, say) would let a later member land outside
        links = [m.name for m in t.getmembers() if not (m.isfile() or m.isdir())]
        if links:
            raise SystemExit(f"rt-startup: {tar} holds links or special files: {links[:3]}")
        t.extractall(home, filter="tar")
    if not tree_dirs(home):
        raise SystemExit(f"rt-startup: {tar} holds no .robigo/rt-home* tree")


def build_trees(binary, checkout):
    """`binary`'s R_HOME trees for a measurement, built from the checkout's
    sources in a scratch home (new_home()), which it returns. Refused in CI
    (in_ci()): there the measuring job has no toolchain, and the trees come
    built (--trees-from)."""
    if in_ci():
        raise SystemExit("rt-startup: not building R_HOME trees in CI: perf-rt-build packs them "
                         "(--pack-trees), and the measurement takes them (--trees-from)")
    home = new_home()
    make_trees(binary, checkout, home)
    return home


def make_trees(binary, checkout, home):
    """build_trees' work, wherever it runs (pack_trees' too): the checkout's
    scripts and sources, the trees written under `home`/.robigo."""
    src = os.path.join(home, ".robigo", "rt-home")
    db = os.path.join(home, ".robigo", "rt-home-db")
    for cmd, extra in ((["scripts/rt-home.sh", src], {}),
                       (["scripts/rt-home-db.sh", "--robigo-rt", os.path.abspath(binary), db],
                        {"RT_HOME_TREE": src})):
        p = subprocess.run(cmd, cwd=checkout, capture_output=True, text=True, env=dict(os.environ, **extra))
        if p.returncode:
            raise SystemExit(f"rt-startup: {' '.join(cmd)} failed:\n{(p.stdout + p.stderr)[-3000:]}")


def run(binary, starts=None, checkout=ROOT, runs=9, trees=False, valgrind=None):
    """robigo-rt's starts (`starts`, default every defined one) of `binary`:
    {start: measure()'s numbers}. `trees` True builds `binary`'s trees from
    the checkout's sources (build_trees), a path unpacks them from that
    tarball (unpack_trees), both into a scratch home the starts boot from;
    False boots from the checkout's own .robigo trees as they are."""
    names = starts or defined()
    for n in names:
        if STARTS.get(n) is None:
            raise SystemExit(f"rt-startup: no start '{n}' (defined: {', '.join(defined())})")
    if isinstance(trees, str):
        home = new_home()
        unpack_trees(trees, home)
    elif trees:
        home = build_trees(binary, checkout)
    else:
        home = checkout
    env0 = clean_env()
    cmd = [os.path.abspath(binary), "--vanilla", "-e", "1"]
    cached = os.path.isdir(os.path.join(home, ".robigo", "rt-home-db-cache"))
    out = {}
    for n in names:
        env = dict(env0, **STARTS[n])
        if cached and env.get("ROBIGO_RT_BOOT") == "rdb" and env.get("ROBIGO_TRANSLATION_CACHE") != "0":
            require_cache_used(n, cmd, dict(env, ROBIGO_RT_CHECKOUT=os.path.abspath(home)))
        out[n] = measure(cmd, env, runs, valgrind or os.environ.get("VALGRIND", "valgrind"), "ROBIGO_RT_CHECKOUT",
                         home)
    return out


def run_gnur(runs=9, valgrind=None):
    """GNU R's starts (GNUR_STARTS) of RSCRIPT_BIN in R_HOME_DIR, with the
    defaults the repo's other GNU R entry points use (perf.py, idiom-vs-r.py)."""
    exe = os.environ.get("RSCRIPT_BIN", "/usr/lib/R/bin/exec/R")
    home = os.environ.get("R_HOME_DIR", "/usr/lib/R")
    if not os.path.exists(exe):
        raise SystemExit(f"rt-startup: no GNU R at {exe} (set RSCRIPT_BIN to its exec/R and R_HOME_DIR)")
    res = {}
    for n, extra in GNUR_STARTS.items():
        env = dict(clean_env(), **extra)
        lib = os.path.join(home, "lib")
        env["LD_LIBRARY_PATH"] = lib + (":" + env["LD_LIBRARY_PATH"] if env.get("LD_LIBRARY_PATH") else "")
        res[n] = measure([exe, "--vanilla", "--no-echo", "-e", "1"], env, runs,
                         valgrind or os.environ.get("VALGRIND", "valgrind"), "R_HOME", home)
    return res


# the fixed part of every measured run's environment: nothing of the
# caller's. The environment's size moves the initial stack and the heap's
# layout, and with them the anonymous peak (L: a 104 KiB step flipping
# between two binaries at a few bytes of environment), so a delegate's shell
# and the runner must hand a start the same one
FIXED_ENV = {"PATH": "/usr/local/bin:/usr/bin:/bin", "HOME": "/tmp"}


_fixed = {}


def clean_env():
    """A measured run's environment: FIXED_ENV and BASE, nothing else but
    ROBIGO_RSRC, the pinned GNU R sources the interim boot loads base's R
    code from (scripts/r-src.sh's tree). That is reached through a symlink
    whose path has a fixed length (a scratch directory under /tmp), so
    wherever the sources are, the environment is the same size."""
    if "rsrc" not in _fixed:
        real = os.environ.get("ROBIGO_RSRC") or subprocess.run(
            [os.path.join(ROOT, "scripts", "r-src.sh")], cwd=ROOT, capture_output=True, text=True,
            check=True).stdout.strip()
        link = os.path.join(scratch_dir("rs-", dir="/tmp"), "rsrc")
        os.symlink(os.path.abspath(real), link)
        _fixed["rsrc"] = link
    return {**FIXED_ENV, **BASE, "ROBIGO_RSRC": _fixed["rsrc"]}


def heap_limit_bytes(main_bytes):
    return max(main_bytes * HEAP_LIMIT / 100, HEAP_FLOOR_BYTES)


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
        if v.get("heap_bytes") is not None and m.get("heap_bytes") is not None:
            lim = heap_limit_bytes(m["heap_bytes"])
            if v["heap_bytes"] > m["heap_bytes"] + lim:
                fails.append(f"start {n}: peak malloc'd bytes {v['heap_bytes']:,}, "
                             f"{v['heap_bytes'] - m['heap_bytes']:+,} against main's {m['heap_bytes']:,} "
                             f"(limit +{lim:,.0f}: {HEAP_LIMIT}%, at least {HEAP_FLOOR_BYTES:,} bytes)")
        if v["rss_kib"] > m["rss_kib"] * (1 + RSS_LIMIT / 100):
            fails.append(f"start {n}: peak RSS {v['rss_kib']:,.0f} KiB, {pct(v['rss_kib'], m['rss_kib']):+.2f}% "
                         f"against main's {m['rss_kib']:,.0f} (limit +{RSS_LIMIT}%)")
    return fails


def pct(a, b):
    return (a - b) / b * 100


def table(entries, new, main=None, gnur=None):
    """The startup table, markdown lines: `entries` the bars file's
    [[startup]] (name, and the GNU R start it is shown beside). Judged:
    instructions, the malloc peak, peak RSS. Reported: the anonymous peak,
    robigo-heap's own peak in use ("-" where a binary does not report it)."""
    lines = ["| start | instructions | main | change | malloc peak KiB | main | change (B) | anon KiB (reported) | main "
             "| peak RSS KiB | main | change | heap in use KiB (reported) | main | GNU R instructions | malloc KiB "
             "| anon KiB | peak RSS KiB |",
             "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|"]
    kib = lambda b: "-" if b is None else f"{b / 1024:,.1f}"  # noqa: E731
    in_use = lambda v: kib((v.get("gc_peak") or {}).get("in_use"))  # noqa: E731
    for e in entries:
        n = e["name"]
        v = new[n]
        m = (main or {}).get(n)
        g = (gnur or {}).get(e.get("gnur", ""))
        if m and any(k not in m for k in ("instr", "anon_kib", "rss_kib")):
            m = None
        if g and any(k not in g for k in ("instr", "anon_kib", "rss_kib")):
            g = None
        # (the default start: the default boot's, the row bar 5 is judged on)
        cells = [f"{n} (default)" if n == DEFAULT_START else n, f"{v['instr']:,.0f}"]
        cells += [f"{m['instr']:,.0f}", f"{pct(v['instr'], m['instr']):+.2f}%"] if m else ["", ""]
        hv, hm = v.get("heap_bytes"), (m or {}).get("heap_bytes")
        cells += [kib(hv), kib(hm) if m else "", f"{hv - hm:+,} B" if hv is not None and hm is not None else ""]
        cells += [f"{v['anon_kib']:,.0f}", f"{m['anon_kib']:,.0f}" if m else ""]
        cells += [f"{v['rss_kib']:,.0f}"] + ([f"{m['rss_kib']:,.0f}", f"{pct(v['rss_kib'], m['rss_kib']):+.2f}%"]
                                             if m else ["", ""])
        cells += [in_use(v), in_use(m) if m else ""]
        cells += [f"{g['instr']:,.0f} ({e['gnur']})", kib(g.get("heap_bytes")), f"{g['anon_kib']:,.0f}",
                  f"{g['rss_kib']:,.0f}"] if g else ["", "", "", ""]
        lines.append("| " + " | ".join(cells) + " |")
    return lines


def growth_reallocs(cg, top=5):
    """[(callee, instructions)]: realloc's inclusive cost in the callgrind
    file `cg` (made with --separate-callers=3) by the function that grew
    the buffer, the first caller outside the allocator and RawVec's growth,
    heaviest first. Each realloc either extends in place or copies, by what
    lies above it: a start's layout-dependent cost
    (docs/ledger/g3/2026-11-23-startup-layout.md)."""
    p = subprocess.run(["callgrind_annotate", "--inclusive=yes", "--threshold=100", "--auto=no", cg],
                       capture_output=True, text=True, check=True)
    grew = {}
    for ln in p.stdout.splitlines():
        m = re.match(r"\s*([\d,]+) \([ \d.]+%\)\s+(?:\S*:)?(?:__libc_)?realloc'(.*)$", ln)
        if not m:
            continue
        frames = [f for f in m.group(2).split(" [")[0].split("'") if f]
        # (the frames past the allocator's own and RawVec's growth path)
        who = next((f for f in frames if not re.search(r"RawVec|finish_grow|do_reserve|grow_one|^0x|^\?\?\?", f)),
                   frames[-1] if frames else "?")
        grew[who] = grew.get(who, 0) + int(m.group(1).replace(",", ""))
    return sorted(grew.items(), key=lambda kv: -kv[1])[:top]


def reallocs(binary, starts, checkout, trees, valgrind, top=5):
    """Print each start's top `top` growth reallocs (growth_reallocs()), as
    shares of the start's instructions, at the first layout shift."""
    names = starts or defined()
    for n in names:
        if STARTS.get(n) is None:
            raise SystemExit(f"rt-startup: no start '{n}' (defined: {', '.join(defined())})")
    home = new_home() if isinstance(trees, str) else (build_trees(binary, checkout) if trees else checkout)
    if isinstance(trees, str):
        unpack_trees(trees, home)
    homes = layout_homes(home)
    n0, pad = LAYOUT_SHIFTS[0]
    for n in names:
        env = dict(clean_env(), **STARTS[n], ROBIGO_RT_CHECKOUT=homes[n0],
                   GLIBC_TUNABLES=f"glibc.malloc.top_pad={pad}")
        with tempfile.TemporaryDirectory(prefix="rt-reallocs-") as d:
            cg = os.path.join(d, "cg.out")
            cmd = [valgrind, "--tool=callgrind", "--separate-callers=3", f"--callgrind-out-file={cg}",
                   os.path.abspath(binary), "--vanilla", "-e", "1"]
            try:
                p = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=CALLGRIND_TIMEOUT)
            except subprocess.TimeoutExpired:
                raise SystemExit(f"rt-startup: {' '.join(cmd)} ran over {CALLGRIND_TIMEOUT} s under callgrind")
            if p.returncode:
                raise SystemExit(f"rt-startup: the {n} start under callgrind exited {p.returncode}:\n{p.stderr[-2000:]}")
            total = None
            with open(cg, errors="replace") as f:
                for ln in f:
                    if ln.startswith(("summary:", "totals:")):
                        total = int(ln.split()[1])
                        break
            if total is None:
                raise SystemExit(f"rt-startup: no total in callgrind's output of the {n} start")
            print(f"{n}: {total:,} instructions; growth reallocs (realloc's inclusive cost, by the function that grew):")
            for who, k in growth_reallocs(cg, top):
                print(f"  {k:>12,} ({k / total:.2%})  {who}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    side = ap.add_mutually_exclusive_group(required=True)
    side.add_argument("--robigo-rt", help="robigo-rt's binary")
    side.add_argument("--gnur", action="store_true",
                      help="GNU R's starts: RSCRIPT_BIN (exec/R) and R_HOME_DIR, as idiom-vs-r.py "
                           "(default /usr/lib/R's)")
    ap.add_argument("--checkout", default=ROOT,
                    help="ROBIGO_RT_CHECKOUT for robigo-rt: its .robigo/rt-home and rt-home-db trees")
    t = ap.add_mutually_exclusive_group()
    t.add_argument("--trees", action="store_true",
                   help="build the checkout's R_HOME trees with BIN first (the database one is BIN's; not in CI)")
    t.add_argument("--trees-from", metavar="TAR", help="unpack the trees from this tarball (--pack-trees') first")
    t.add_argument("--pack-trees", metavar="OUT",
                   help="build the trees with BIN, pack them into this tarball, and measure nothing")
    ap.add_argument("--starts", help="robigo-rt's starts to measure, comma-separated (default: all defined)")
    ap.add_argument("--runs", type=int, default=9, help="memory runs per start (default 9)")
    ap.add_argument("--valgrind", default=os.environ.get("VALGRIND", "valgrind"))
    ap.add_argument("--json", help="write the numbers here")
    ap.add_argument("--reallocs", action="store_true",
                    help="with --robigo-rt: no measurement; each start's top growth reallocs under callgrind "
                         "(the functions to pre-size), as shares of the start")
    a = ap.parse_args()
    if a.reallocs:
        if not a.robigo_rt:
            raise SystemExit("rt-startup: --reallocs needs --robigo-rt")
        reallocs(a.robigo_rt, a.starts.split(",") if a.starts else None, a.checkout, a.trees_from or a.trees,
                 a.valgrind)
        return 0
    if a.pack_trees:
        if not a.robigo_rt:
            raise SystemExit("rt-startup: --pack-trees needs --robigo-rt")
        trees = pack_trees(a.robigo_rt, a.checkout, os.path.abspath(a.pack_trees))
        print(f"packed {', '.join(trees)} of {a.checkout} into {a.pack_trees}")
        return 0
    if a.robigo_rt:
        res = run(a.robigo_rt, a.starts.split(",") if a.starts else None, a.checkout, a.runs,
                  a.trees_from or a.trees, a.valgrind)
    else:
        res = run_gnur(a.runs, a.valgrind)
    for n, v in res.items():
        print(f"{n}: {v['instr']:,.0f} instructions (shifts {', '.join(f'{x:,}' for x in v['instr_shifts'].values())}), "
              f"malloc peak {v['heap_bytes']:,} B, anon {v['anon_kib']:,.0f} KiB, peak RSS {v['rss_kib']:,.0f} KiB"
              + (f", {v['children']} process(es) started" if v.get("children") else ""))
    if a.json:
        with open(a.json, "w") as f:
            json.dump(res, f, indent=1, sort_keys=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
