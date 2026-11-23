"""The machine-wide pool of case slots, for the Python tools.

robigo-conformance's runner takes a slot of the pool in ROBIGO_CONF_SLOTS_DIR
before it starts a case, and a case's time limit counts from then
(crates/robigo-conformance/src/lib.rs, take_slot). This module is the same
pool for the tools that run robigo-rt themselves under a time limit
(builtin-diff's driver processes) or that run many test threads at once
(rt-ci.py's crate-test steps): one exclusive flock on `<dir>/slot-<i>` per
slot held, the same files and the same lock as the runner's (Rust's
File::lock is flock(2) on Linux), so every user of the pool shares one count.

    default_dir(env)   the directory a local run of this user uses (None: no
                       pool, or the caller already chose)
    from_env(env)      the pool ROBIGO_CONF_SLOTS_DIR names (None: no pool)
    Pool.take(k)       wait for one slot, then add up to k - 1 more that are
                       free; a context manager that releases them all
"""

import fcntl
import os
import stat
import sys
import threading
import time


def default_dir(env=None):
    """The machine-wide pool a local run shares with every other local run
    of this user: several checkouts' gates at once took the box to load
    45-66 on 24 CPUs, and cases ran into their limits (2026-11-23). The
    caller's own ROBIGO_CONF_SLOTS_DIR wins (empty: no pool); CI has none
    (one run per runner). None: leave the environment as it is."""
    env = os.environ if env is None else env
    if "ROBIGO_CONF_SLOTS_DIR" in env or env.get("GITHUB_ACTIONS") == "true":
        return None
    base = env.get("XDG_RUNTIME_DIR")
    if base and os.path.isdir(base):
        d = os.path.join(base, "robigo-conf-slots")
    else:
        # fixed, not TMPDIR's: every run of this user must find the same pool
        d = f"/tmp/robigo-conf-slots-{os.getuid()}"
    problem = private_dir(d)
    if problem:
        print(f"::warning::no conformance slot pool ({problem})")
        return None
    return d


def private_dir(d):
    """Make `d` (0700) if missing; a problem if it is not a private
    directory of this user's (another user able to write there could
    replace a slot file, and two runs would lock two files for one slot),
    else None."""
    try:
        try:
            os.makedirs(d, mode=0o700, exist_ok=True)
        except FileExistsError:
            pass            # (a file or a link there: lstat and the check below say what)
        st = os.lstat(d)
    except OSError as e:
        return f"{d}: {e}"
    if not stat.S_ISDIR(st.st_mode) or st.st_uid != os.getuid() or st.st_mode & 0o077:
        return f"{d} is not a private directory of this user's: mode {oct(stat.S_IMODE(st.st_mode))}"
    return None


class PoolError(Exception):
    pass


def from_env(env=None):
    """The pool ROBIGO_CONF_SLOTS_DIR names, of ROBIGO_CONF_SLOTS slots
    (default: one per CPU), as the conformance runner reads them; None when
    it is unset or empty. A pool that cannot be used is a PoolError: a tool
    told to share the machine must not run without it."""
    env = os.environ if env is None else env
    d = env.get("ROBIGO_CONF_SLOTS_DIR")
    if not d:
        return None
    s = env.get("ROBIGO_CONF_SLOTS")
    if s is None:
        n = os.cpu_count() or 2
    else:
        try:
            n = int(s.strip())
        except ValueError:
            n = 0
        if n <= 0:
            raise PoolError(f"ROBIGO_CONF_SLOTS={s!r} is not a positive number")
    problem = private_dir(d)
    if problem:
        raise PoolError(f"ROBIGO_CONF_SLOTS_DIR {problem}")
    return Pool(d, n)


class Pool:
    def __init__(self, d, n, first_note=10.0, every_note=60.0, step=0.02):
        self.dir, self.n = d, n
        self.first_note, self.every_note, self.step = first_note, every_note, step

    def take(self, k=1, who="a case"):
        """Slots of the pool: one, waiting as long as it takes (a line on
        stderr after 10 s of waiting, then every minute, so a stuck pool is
        told apart from a hung case), then up to k - 1 more that are free at
        that moment, without waiting (so no holder of some ever waits for
        more: two takers cannot deadlock)."""
        start = (os.getpid() * 31 + threading.get_ident()) % self.n
        since, said = time.monotonic(), None
        held = Held()
        try:
            while not held.files:
                held.grab(self, start, 1)
                if held.files:
                    break
                waited = time.monotonic() - since
                if waited >= (self.first_note if said is None else said + self.every_note):
                    said = waited
                    print(f"waiting {waited:.0f} s for a conformance slot for {who} ({self.n} in {self.dir})",
                          file=sys.stderr, flush=True)
                time.sleep(self.step)
            if k > 1:
                held.grab(self, start, k - 1)
        except BaseException:
            held.release()
            raise
        return held


class Held:
    """Slots taken; released by release() or at the end of a `with`."""

    def __init__(self):
        self.files = []

    @property
    def count(self):
        return len(self.files)

    def grab(self, pool, start, want):
        mine = {f.name for f in self.files}
        for j in range(pool.n):
            if want <= 0:
                return
            path = os.path.join(pool.dir, f"slot-{(start + j) % pool.n}")
            if path in mine:
                continue
            # (not inherited: a child that outlived its parent's release
            # would otherwise keep the slot)
            f = open(path, "a")
            try:
                fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                f.close()
                continue
            except OSError as e:
                f.close()
                # a filesystem that does not lock would make every taker
                # wait forever
                raise PoolError(f"lock {path}: {e}") from e
            self.files.append(f)
            want -= 1

    def release(self):
        for f in self.files:
            f.close()
        self.files = []

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.release()
