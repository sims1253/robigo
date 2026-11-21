"""Temporary directories that a tool removes on every way out.

    from scratch import scratch_dir
    work = scratch_dir("rt-bars-")      # removed when the process exits

Two rules, and nothing here decides whether another run is alive:

1. A tool removes its own directories on every exit path: a normal return,
   an exception, sys.exit, Ctrl-C, and SIGTERM or SIGHUP (a CI job's cancel,
   `timeout`, a killed delegate). Those two signals, while at their default
   action, end this process's children (robigo-rt, GNU R, docker), remove
   the directories and raise SystemExit, so the tool's own `finally`s and
   exit handlers run; a process still there STOP_SECONDS later (a thread
   pool waiting on its workers) exits then. The signals wait while a
   directory is being made or any removal runs, a directory leaves the
   registry only once it is gone, and one that cannot be removed does not
   stop the others. A child made by fork owns none of its parent's
   directories, so a process removes only what it made. keep(path) exempts
   a directory (a tool's --keep), remove(path) removes one early.
2. A sweep by age alone, for what a SIGKILL or an OOM kill leaves: the first
   scratch_dir call for a prefix removes that prefix's directories in the
   same parent that this user owns and whose own modification time is more
   than SWEEP_HOURS old. No gate runs for a day, so a live run is never a
   candidate, whatever happens meanwhile. The sweep removes through a
   descriptor of the directory it judged (an entry renamed or replaced after
   the listing is not followed), and checks that the name still holds that
   directory before removing the entry itself.

Before this, /tmp collected 28 GB of these on the shared workstation
(2026-11-21): the tools removed their directories on success only, and none
on a signal. What rule 2 gives up: a killed run's directories stay for up to
a day.
"""

import atexit
import os
import secrets
import shutil
import signal
import stat
import tempfile
import threading
import time

# rule 2: a directory this much older than now is a killed run's
SWEEP_HOURS = 24

# How long the process may take to exit after SIGTERM or SIGHUP before it
# is ended without its remaining exit handlers (its directories are gone by
# then)
STOP_SECONDS = 30

# reentrant: the signal handler runs on the main thread, which may hold it
_lock = threading.RLock()
_live = set()       # the directories this process removes at exit
_swept = set()      # (parent, prefix) pairs already swept by this process
_installed = False
_closing = False
_busy = 0           # a directory is being made or removed: signals wait
_pending = None     # a signal that arrived meanwhile
_exiting = False    # the exit handler runs: a signal waits for its end
_exit_done = False  # the exit handler's removal is done: a signal ends the process now
_SIGNALS = {signal.SIGTERM, signal.SIGHUP}


class _Deferring:
    """A creation or removal: it holds the lock, so the handler's removal
    waits for a worker thread's to finish; and while the main thread is in
    one, where Python runs the handler, SIGTERM and SIGHUP only note
    themselves, and are handled when the outermost one ends."""

    def __enter__(self):
        global _busy
        _lock.acquire()
        if threading.current_thread() is threading.main_thread():
            _busy += 1

    def __exit__(self, *exc):
        global _busy, _pending
        signum = None
        if threading.current_thread() is threading.main_thread():
            _busy -= 1
            if _busy == 0 and not _exiting:
                signum, _pending = _pending, None
        _lock.release()
        if signum:
            _on_signal(signum, None)
        return False


_deferring = _Deferring()


def scratch_dir(prefix, dir=None):
    """A new directory `<dir>/<prefix>XXXXXXXX` (dir: the temporary
    directory), removed when this process exits."""
    parent = dir or tempfile.gettempdir()
    with _deferring:
        if _closing:
            # a worker thread starting new work after a SIGTERM
            raise SystemExit(1)
        _install()
        if (parent, prefix) not in _swept:
            _swept.add((parent, prefix))
            sweep(prefix, parent)
        # the name is registered before the directory exists, so no moment
        # has a directory this process made and would not remove (a Ctrl-C
        # included); a name that is not made leaves the registry at exit
        while True:
            path = os.path.join(parent, prefix + secrets.token_hex(6))
            _live.add(path)
            try:
                os.mkdir(path, 0o700)
                return os.path.abspath(path)
            except FileExistsError:
                _live.discard(path)


def keep(path):
    """Leave `path` in place at exit (a sweep removes it once it is
    SWEEP_HOURS old)."""
    with _lock:
        _live.discard(path)


def remove(path):
    """Remove `path` now (one that cannot be removed stays registered and
    is tried again at exit)."""
    with _deferring:
        if path in _live:
            _remove_registered(path)


def remove_all():
    """Remove every directory this process still holds (the exit handler,
    and SIGTERM's)."""
    with _deferring:
        if _live:
            end_children()
        for path in sorted(_live):
            _remove_registered(path)


def _remove_registered(path):
    try:
        rmtree(path)
    except Exception:
        pass                # this one stays; the others go on
    if not os.path.lexists(path):
        _live.discard(path)


def rmtree(path):
    """rm -rf `path`, also when what ran in it took its own write
    permission away (a builtin-diff case's Sys.chmod): every directory is
    given back to its owner and the removal is tried again."""
    shutil.rmtree(path, ignore_errors=True)
    if not os.path.lexists(path) or os.path.islink(path):
        return
    try:
        os.chmod(path, 0o700)
    except OSError:
        pass
    # top-down: each directory is opened before os.walk lists it
    for root, dirs, _ in os.walk(path, onerror=lambda e: None):
        for x in dirs:
            d = os.path.join(root, x)
            try:
                if not os.path.islink(d):
                    os.chmod(d, 0o700)
            except OSError:
                pass
    shutil.rmtree(path, ignore_errors=True)


def descendants():
    """The pids of this process's children, theirs, and so on (none known
    without /proc): a tool may start a command through another process
    (perf.py's valgrind under a Python of its own)."""
    parent = {}
    try:
        pids = [p for p in os.listdir("/proc") if p.isdigit()]
    except OSError:
        return []
    for pid in pids:
        try:
            with open(f"/proc/{pid}/stat") as f:
                # the command is in parentheses and may hold spaces
                fields = f.read().rsplit(")", 1)[1].split()
            if fields[0] != "Z":
                parent[int(pid)] = int(fields[1])
        except (OSError, IndexError, ValueError):
            continue
    out, frontier = [], {os.getpid()}
    while frontier:
        frontier = {p for p, pp in parent.items() if pp in frontier and p not in out}
        out.extend(sorted(frontier))
    return out


def end_children(grace=5.0):
    """SIGTERM this process's descendants, again for any that appear (a
    worker thread starting the next command), and SIGKILL the ones still
    there `grace` seconds later."""
    deadline = time.monotonic() + grace
    signalled = set()
    while True:
        kids = descendants()
        if not kids:
            return
        if time.monotonic() >= deadline:
            for pid in kids:
                try:
                    os.kill(pid, signal.SIGKILL)
                except OSError:
                    pass
            return
        for pid in kids:
            if pid not in signalled:
                signalled.add(pid)
                try:
                    os.kill(pid, signal.SIGTERM)
                except OSError:
                    pass
        time.sleep(0.05)


def _on_signal(signum, frame):
    global _closing, _pending
    code = 128 + signum      # the shell's convention for a death by signal
    if _exit_done:
        os._exit(code)       # after the exit handler's removal: nothing is left to remove
    if _busy or _exiting:
        _pending = signum    # handled once the creation or removal is done
        return
    if signum == signal.SIGINT:
        raise KeyboardInterrupt
    _closing = True
    remove_all()
    # the directories are gone; a process still there this much later (a
    # thread pool's exit waiting on its workers) is ended
    t = threading.Timer(STOP_SECONDS, os._exit, (code,))
    t.daemon = True
    t.start()
    # unwinds the main thread through its `finally`s to the exit handlers
    raise SystemExit(code)


def _at_exit():
    global _exiting, _exit_done
    # (an exception raised in an exit handler is dropped, so a signal that
    # comes during this removal is honoured here, not by raising)
    _exiting = True
    remove_all()
    _exit_done = True
    if _pending and not _closing:
        # a signal during the normal exit's removal: honoured once it is done
        os._exit(128 + _pending)


def _forked_child():
    # a child made by fork owns none of its parent's directories (rule 1)
    global _lock, _busy, _pending, _exiting, _exit_done
    _live.clear()
    _lock = threading.RLock()
    _busy, _pending, _exiting, _exit_done = 0, None, False, False


def _install():
    global _installed
    if _installed:
        return
    _installed = True
    atexit.register(_at_exit)
    os.register_at_fork(after_in_child=_forked_child)
    if threading.current_thread() is not threading.main_thread():
        return
    for s in _SIGNALS:
        # a tool's own handler stays; only the default (die without the
        # exit handlers) is replaced
        if signal.getsignal(s) is signal.SIG_DFL:
            signal.signal(s, _on_signal)
    # Ctrl-C: still KeyboardInterrupt, but not inside a creation or a removal
    if signal.getsignal(signal.SIGINT) is signal.default_int_handler:
        signal.signal(signal.SIGINT, _on_signal)


def sweep(prefix, parent=None, now=None):
    """Remove `parent`'s `<prefix>*` directories that this user owns and
    whose own modification time is more than SWEEP_HOURS old (rule 2). The
    directories removed."""
    parent = parent or tempfile.gettempdir()
    cutoff = (now if now is not None else time.time()) - SWEEP_HOURS * 3600
    try:
        pfd = os.open(parent, os.O_RDONLY | os.O_DIRECTORY)
    except OSError:
        return []
    gone = []
    try:
        for name in sorted(os.listdir(pfd)):
            if name.startswith(prefix) and _sweep_one(pfd, name, cutoff):
                gone.append(os.path.join(parent, name))
    finally:
        os.close(pfd)
    return gone


def _old_mine(st, cutoff):
    return stat.S_ISDIR(st.st_mode) and st.st_uid == os.getuid() and st.st_mtime < cutoff


def _sweep_one(pfd, name, cutoff):
    try:
        st = os.lstat(name, dir_fd=pfd)
    except OSError:
        return False
    if not _old_mine(st, cutoff):
        return False        # a link, a file, another user's, or young
    dfd = _open_dir(name, pfd)
    if dfd is None:
        return False
    try:
        # the directory opened is the one judged: a name replaced since the
        # listing is left alone
        now = os.fstat(dfd)
        if (now.st_dev, now.st_ino) != (st.st_dev, st.st_ino) or not _old_mine(now, cutoff):
            return False
        _empty(dfd)
    finally:
        os.close(dfd)
    try:
        # the entry, if the name still holds that directory (rmdir removes
        # only an empty one, so a directory made at the name in between and
        # written to is never removed)
        last = os.lstat(name, dir_fd=pfd)
        if (last.st_dev, last.st_ino) != (st.st_dev, st.st_ino):
            return False
        os.rmdir(name, dir_fd=pfd)
    except OSError:
        return False
    return True


def _open_dir(name, pfd):
    """A descriptor of directory `name` in `pfd`, not following a link;
    given back to its owner first when a case took its permissions."""
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
    try:
        return os.open(name, flags, dir_fd=pfd)
    except PermissionError:
        try:
            os.chmod(name, 0o700, dir_fd=pfd, follow_symlinks=False)
            return os.open(name, flags, dir_fd=pfd)
        except (OSError, NotImplementedError):
            return None
    except OSError:
        return None


def _empty(dfd):
    """Remove everything in the directory `dfd` names, through descriptors
    only (nothing is reached by a path from outside it)."""
    for name in os.listdir(dfd):
        try:
            st = os.lstat(name, dir_fd=dfd)
            if stat.S_ISDIR(st.st_mode):
                sub = _open_dir(name, dfd)
                if sub is None:
                    continue
                try:
                    _empty(sub)
                finally:
                    os.close(sub)
                os.rmdir(name, dir_fd=dfd)
            else:
                os.unlink(name, dir_fd=dfd)
        except OSError:
            continue


# at import, on the main thread: the first directory is often made by a
# worker thread (builtin-diff's rows), which cannot install signal handlers
if threading.current_thread() is threading.main_thread():
    _install()
