#!/usr/bin/env python3
"""scripts/scratch.py: a tool's temporary directories are gone after every
way out (a normal exit, an exception, sys.exit, SIGTERM with children and a
busy thread pool, SIGTERM during an early removal or the exit's), a
forked child removes none of its parent's, and each tool that makes them (builtin-diff, perf.py rt-bars and
rt-gnur, rast.py, the C API bench) leaves none after a normal or a failing
exit.

    python3 scripts/test-scratch.py
"""

import importlib.util
import os
import signal
import stat
import subprocess
import sys
import tempfile
import textwrap
import time
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
spec = importlib.util.spec_from_file_location("scratch", HERE / "scratch.py")
scratch = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scratch)


def run_py(code, tmp, timeout=60):
    """`code` in a fresh python3 with TMPDIR=tmp and scripts/ importable:
    (exit status, the lines it printed)."""
    env = dict(os.environ, TMPDIR=tmp, PYTHONPATH=str(HERE))
    p = subprocess.run([sys.executable, "-c", textwrap.dedent(code)], env=env, cwd=tmp,
                       capture_output=True, text=True, timeout=timeout)
    return p.returncode, p.stdout.split(), p.stderr


class Module(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="test-scratch-")

    def tearDown(self):
        scratch.rmtree(self.tmp)

    def made(self, tail, rc):
        code, out, err = run_py("from scratch import scratch_dir\n"
                                "d = scratch_dir('t-')\nopen(d + '/f', 'w').write('x')\nprint(d, flush=True)\n" + tail,
                                self.tmp)
        self.assertEqual(code, rc, err)
        self.assertTrue(out and out[0].startswith(self.tmp), (out, err))
        self.assertFalse(os.path.exists(out[0]), "the directory outlived its process")

    def test_gone_after_a_normal_exit(self):
        self.made("", 0)

    def test_gone_after_an_exception(self):
        self.made("raise RuntimeError('boom')", 1)

    def test_gone_after_sys_exit(self):
        self.made("import sys; sys.exit(3)", 3)

    def test_kept_with_keep_and_removed_early_with_remove(self):
        code, out, err = run_py("from scratch import scratch_dir, keep, remove\n"
                                "a, b = scratch_dir('t-'), scratch_dir('t-')\nkeep(a); remove(b)\n"
                                "import os; print(a, os.path.exists(b))", self.tmp)
        self.assertEqual(code, 0, err)
        self.assertTrue(os.path.isdir(out[0]))
        self.assertEqual(out[1], "False")

    def sigterm(self, body, first="d = scratch_dir('t-')\n", delay=0.0):
        """Starts `body` (after `first`, which makes a directory on the main
        thread; `body` prints a directory, a pid and READY), SIGTERMs it:
        (exit status, seconds to exit, the directory, the pid)."""
        env = dict(os.environ, TMPDIR=self.tmp, PYTHONPATH=str(HERE))
        code = "import subprocess, sys, time\nfrom scratch import scratch_dir\n" + first + body
        p = subprocess.Popen([sys.executable, "-c", code], env=env, stdout=subprocess.PIPE, text=True)
        lines = []
        while "READY" not in lines:
            line = p.stdout.readline()
            self.assertTrue(line, "the child exited before READY")
            lines.append(line.strip())
        time.sleep(delay)
        t0 = time.monotonic()
        p.send_signal(signal.SIGTERM)
        rc = p.wait(timeout=60)
        return rc, time.monotonic() - t0, lines[0], lines[1]

    def alive(self, pid):
        try:
            with open(f"/proc/{pid}/stat") as f:
                return f.read().rsplit(")", 1)[1].split()[0] != "Z"
        except OSError:
            return False

    def test_sigterm_ends_the_children_and_removes_the_directory(self):
        # a child with the directory as its working directory: its open
        # files would keep the blocks in use
        rc, secs, d, kid = self.sigterm(
            "c = subprocess.Popen(['sleep', '300'], cwd=d)\nprint(d); print(c.pid); print('READY', flush=True)\n"
            "c.wait()\ntime.sleep(300)\n")
        self.assertEqual(rc, 128 + signal.SIGTERM)
        self.assertLess(secs, 10)
        self.assertFalse(os.path.exists(d))
        self.assertFalse(self.alive(int(kid)), "the child outlived the tool")

    def test_sigterm_with_a_thread_pool_waiting_on_workers_still_exits(self):
        # builtin-diff's shape: workers in subprocess.run, the main thread in
        # the executor's exit, which waits for them
        rc, secs, d, kid = self.sigterm(
            "import concurrent.futures, os\nprint(d); print(os.getpid()); print('READY', flush=True)\n"
            "with concurrent.futures.ThreadPoolExecutor(2) as ex:\n"
            "    list(ex.map(lambda _: subprocess.run(['sleep', '300'], cwd=d), range(4)))\n")
        self.assertEqual(rc, 128 + signal.SIGTERM)
        self.assertLess(secs, 15)
        self.assertFalse(os.path.exists(d))

    def test_sigterm_removes_a_directory_a_worker_thread_made_first(self):
        # builtin-diff's rows: the first scratch_dir runs in a pool worker
        rc, secs, d, kid = self.sigterm(
            "import concurrent.futures, os\n"
            "with concurrent.futures.ThreadPoolExecutor(1) as ex:\n"
            "    w = ex.submit(scratch_dir, 'w-').result()\n"
            "print(w); print(os.getpid()); print('READY', flush=True)\ntime.sleep(300)\n", first="")
        self.assertEqual(rc, 128 + signal.SIGTERM)
        self.assertFalse(os.path.exists(d))

    def test_a_signal_while_the_main_thread_makes_a_directory_waits_for_it(self):
        # the handler only notes it; scratch_dir runs it once the directory
        # is registered, so the exit removes it
        # sent by a thread that already ran (a pool worker's), so it can
        # receive the signal whatever the main thread blocks
        code, out, err = run_py("import os, signal, tempfile, threading, time, scratch\n"
                                "go = threading.Event()\n"
                                "threading.Thread(target=lambda: go.wait() and os.kill(os.getpid(), signal.SIGTERM)).start()\n"
                                "real = os.mkdir\n"
                                "def mk(p, *a):\n"
                                "    real(p, *a); print(p, flush=True)\n"
                                "    go.set(); time.sleep(0.5)\n"
                                "os.mkdir = mk\nscratch.scratch_dir('t-')\nprint('NOT REACHED')", self.tmp)
        self.assertEqual(code, 128 + signal.SIGTERM, err)
        self.assertNotIn("NOT", out)
        self.assertFalse(os.path.exists(out[0]))

    def test_sigterm_during_the_exit_cleanup_waits_for_it(self):
        # S17: the normal exit's remove_all waits on a child that ignores
        # SIGTERM (end_children's grace); a SIGTERM then must not cut the
        # cleanup short
        rc, secs, d, kid = self.sigterm(
            "import os\nc = subprocess.Popen(['sh', '-c', 'trap \"\" TERM; sleep 30'])\n"
            "print(d); print(os.getpid()); print('READY', flush=True)\ntime.sleep(1)\n",
            delay=2.0)
        self.assertEqual(rc, 128 + signal.SIGTERM)
        self.assertFalse(os.path.exists(d))

    def test_a_forked_child_removes_none_of_its_parents_directories(self):
        # S21: the child inherits the registry; it owns none of it
        code, out, err = run_py("import os, sys\nfrom scratch import scratch_dir\nd = scratch_dir('t-')\n"
                                "pid = os.fork()\n"
                                "if pid == 0:\n    sys.exit(0)        # the child's exit handlers run\n"
                                "os.waitpid(pid, 0)\nprint(d, os.path.isdir(d), flush=True)", self.tmp)
        self.assertEqual(code, 0, err)
        self.assertEqual(out[1], "True")         # still there after the child exited
        self.assertFalse(os.path.exists(out[0]))  # and gone with its maker

    def test_sigterm_during_an_early_removal_waits_for_it(self):
        # S20: the signal lands inside remove(); the removal finishes, then
        # the exit removes the rest
        code, out, err = run_py("import os, signal, scratch\n"
                                "a, b = scratch.scratch_dir('t-'), scratch.scratch_dir('t-')\nprint(a, b, flush=True)\n"
                                "real = scratch.rmtree\n"
                                "def rm(p, *a):\n    os.kill(os.getpid(), signal.SIGTERM); real(p, *a)\n"
                                "scratch.rmtree = rm\nscratch.remove(a)\nprint('NOT REACHED')", self.tmp)
        self.assertEqual(code, 128 + signal.SIGTERM, err)
        self.assertNotIn("NOT", out)
        self.assertFalse(os.path.exists(out[0]) or os.path.exists(out[1]))

    def test_a_directory_that_cannot_be_removed_does_not_stop_the_others(self):
        # S22
        code, out, err = run_py("import scratch\n"
                                "a, b = scratch.scratch_dir('t-'), scratch.scratch_dir('t-')\nprint(a, b, flush=True)\n"
                                "real, failed = scratch.rmtree, []\n"
                                "def rm(p, *rest):\n    if p == min(a, b) and not failed:\n        failed.append(p); raise PermissionError(p)\n"
                                "    real(p, *rest)\nscratch.rmtree = rm", self.tmp)
        self.assertEqual(code, 0, err)
        stuck = min(out)
        self.assertTrue(os.path.isdir(stuck))       # its one removal failed
        self.assertFalse(os.path.exists(max(out)))  # the other went
        scratch.rmtree(stuck)

    def test_ctrl_c_right_after_the_directory_is_made_leaves_nothing(self):
        # S1: a real Ctrl-C at once after mkdir waits for the registration,
        # so the exit finds the directory
        code, out, err = run_py("import os, signal, scratch\nreal = os.mkdir\n"
                                "def mk(p, *a):\n    real(p, *a); print(p, flush=True); os.kill(os.getpid(), signal.SIGINT)\n"
                                "os.mkdir = mk\nscratch.scratch_dir('t-')\nprint('NOT REACHED')", self.tmp)
        self.assertEqual(code, 128 + signal.SIGINT, err)
        self.assertNotIn("NOT", out)
        self.assertTrue(out, err)
        self.assertFalse(os.path.exists(out[0]))

    def test_sigterm_ends_a_child_started_after_the_first_was_ended(self):
        # S2: a worker that reaps its child and starts the next one; each
        # child notes its pid outside the directory
        pids = os.path.join(self.tmp, "pids")
        rc, secs, d, kid = self.sigterm(
            "import threading, os\n"
            "def work():\n    for _ in range(50):\n"
            f"        subprocess.run(['sh', '-c', 'echo $$ >> {pids}; exec sleep 300'])\n"
            "threading.Thread(target=work, daemon=True).start()\n"
            "print(d); print(os.getpid()); print('READY', flush=True)\ntime.sleep(300)\n", delay=1.0)
        self.assertEqual(rc, 128 + signal.SIGTERM)
        time.sleep(0.5)
        started = open(pids).read().split()
        self.assertTrue(started)
        self.assertEqual([p for p in started if self.alive(int(p))], [], "a later child outlived the tool")
        self.assertFalse(os.path.exists(d))

    def test_a_signal_after_the_exit_removal_still_ends_the_process(self):
        # S4: an exit handler registered before scratch's runs after it; a
        # SIGTERM there comes after the removal's last look at signals
        code, out, err = run_py("import atexit, os, signal, time\n"
                                "atexit.register(lambda: (os.kill(os.getpid(), signal.SIGTERM), time.sleep(2)))\n"
                                "import scratch\nscratch.scratch_dir('t-')", self.tmp)
        self.assertEqual(code, 128 + signal.SIGTERM, err)

    def test_a_directory_moved_to_a_removed_path_is_not_this_processs(self):
        # F4: after remove(d), another process puts its own directory at d
        outside = Path(self.tmp, "theirs")
        outside.mkdir()
        (outside / "sentinel").write_text("x")
        code, out, err = run_py("import os, scratch\nd = scratch.scratch_dir('t-')\nscratch.remove(d)\n"
                                f"os.rename({str(outside)!r}, d)\nprint(d, flush=True)", self.tmp)
        self.assertEqual(code, 0, err)
        self.assertTrue(Path(out[0], "sentinel").exists())

    def test_a_failed_mkdir_registers_nothing(self):
        # F3: mkdir fails; the name is later made by someone else
        code, out, err = run_py("import os, scratch\nreal = os.mkdir\nseen = []\n"
                                "def mk(p, *a):\n    seen.append(p); raise FileNotFoundError(2, 'gone', p)\n"
                                "os.mkdir = mk\n"
                                "try:\n    scratch.scratch_dir('t-')\nexcept FileNotFoundError:\n    pass\n"
                                "os.mkdir = real\nreal(seen[0]); open(seen[0] + '/sentinel', 'w').write('x')\n"
                                "print(seen[0], flush=True)", self.tmp)
        self.assertEqual(code, 0, err)
        self.assertTrue(Path(out[0], "sentinel").exists())

    def test_the_permission_repair_never_follows_a_name(self):
        # F5: a directory under ours swapped for a link to an outside one
        # just before the repair changes a mode
        outside = Path(self.tmp, "outside")
        outside.mkdir()
        os.chmod(outside, 0o755)
        code, out, err = run_py("import os, sys, scratch\nd = scratch.scratch_dir('t-')\n"
                                "sub = os.path.join(d, 'sub'); os.mkdir(sub); open(sub + '/f', 'w').write('x')\n"
                                "os.chmod(sub, 0o500)\ndone = []\n"
                                "def hook(event, args):\n"
                                "    if event == 'os.chmod' and args[0] == sub and not done:\n"
                                "        done.append(1); os.chmod(sub, 0o700); os.unlink(sub + '/f'); os.rmdir(sub)\n"
                                f"        os.symlink({str(outside)!r}, sub)\n"
                                "sys.addaudithook(hook)\nprint(d, flush=True)", self.tmp)
        self.assertEqual(code, 0, err)
        self.assertEqual(stat.S_IMODE(os.lstat(outside).st_mode), 0o755)

    def test_a_child_signalled_while_the_parent_was_busy_keeps_the_status(self):
        # F1: forked from a worker while the main thread is in scratch_dir;
        # an earlier fork hook sends SIGHUP to the child
        code, out, err = run_py("import os, signal, sys, threading, time\n"
                                "os.register_at_fork(after_in_child=lambda: os.kill(os.getpid(), signal.SIGHUP))\n"
                                "import scratch\nreal, st = os.mkdir, {}\n"
                                "def fork():\n    pid = os.fork()\n"
                                "    if pid == 0:\n        os._exit(0) if False else sys.exit(0)\n"
                                "    st['code'] = os.waitstatus_to_exitcode(os.waitpid(pid, 0)[1])\n"
                                "def mk(p, *a):\n    real(p, *a); w = threading.Thread(target=fork); w.start(); w.join()\n"
                                "os.mkdir = mk\nd = scratch.scratch_dir('t-')\nprint(d, st['code'], flush=True)", self.tmp)
        self.assertEqual(code, 0, err)
        self.assertEqual(out[1], str(128 + signal.SIGHUP))

    def test_a_child_calling_remove_before_the_fork_hook_removes_nothing(self):
        # F6
        code, out, err = run_py("import os, sys\nholder = {}\n"
                                "os.register_at_fork(after_in_child=lambda: holder['s'].remove(holder['d']))\n"
                                "import scratch\nholder['s'] = scratch\nholder['d'] = d = scratch.scratch_dir('t-')\n"
                                "pid = os.fork()\n"
                                "if pid == 0:\n    sys.exit(0)\n"
                                "os.waitpid(pid, 0)\nprint(d, os.path.isdir(d), flush=True)", self.tmp)
        self.assertEqual(code, 0, err)
        self.assertEqual(out[1], "True")
        self.assertFalse(os.path.exists(out[0]))

    def test_sigterm_exits_even_when_a_worker_holds_the_lock_forever(self):
        # F2: the deadline is armed before the removal waits for the lock
        env = dict(os.environ, TMPDIR=self.tmp, PYTHONPATH=str(HERE))
        code = ("import threading, time, scratch\nscratch.STOP_SECONDS = 1\nscratch.scratch_dir('t-')\n"
                "threading.Thread(target=lambda: (scratch._lock.acquire(), time.sleep(300)), daemon=True).start()\n"
                "time.sleep(0.3)\nprint('READY', flush=True)\ntime.sleep(300)\n")
        p = subprocess.Popen([sys.executable, "-c", code], env=env, stdout=subprocess.PIPE, text=True)
        try:
            self.assertEqual(p.stdout.readline().strip(), "READY")
            p.send_signal(signal.SIGTERM)
            self.assertEqual(p.wait(timeout=20), 128 + signal.SIGTERM)
        finally:
            if p.poll() is None:
                p.kill()
                p.wait()

    def test_ctrl_c_with_a_worker_that_never_returns_removes_first_then_exits(self):
        # E2: the removal does not wait for the pool; the exit comes after
        # STOP_SECONDS (shortened here)
        env = dict(os.environ, TMPDIR=self.tmp, PYTHONPATH=str(HERE))
        code = ("import threading, time, scratch\nscratch.STOP_SECONDS = 2\nd = scratch.scratch_dir('t-')\n"
                "threading.Thread(target=threading.Event().wait).start()\n"
                "print(d, flush=True)\ntime.sleep(300)\n")
        p = subprocess.Popen([sys.executable, "-c", code], env=env, stdout=subprocess.PIPE, text=True)
        try:
            d = p.stdout.readline().strip()
            p.send_signal(signal.SIGINT)
            time.sleep(1)
            self.assertFalse(os.path.exists(d), "not removed before the pool's wait")
            self.assertEqual(p.wait(timeout=30), 128 + signal.SIGINT)
        finally:
            if p.poll() is None:
                p.kill()        # (a failing run never leaves the process)
                p.wait()

    def test_a_signal_raised_where_cpython_drops_it_keeps_its_status(self):
        # E4: SIGTERM during an exit handler that runs before scratch's
        code, out, err = run_py("import atexit, os, signal, time, scratch\nd = scratch.scratch_dir('t-')\n"
                                "print(d, flush=True)\n"
                                "atexit.register(lambda: (os.kill(os.getpid(), signal.SIGTERM), time.sleep(1)))", self.tmp)
        self.assertEqual(code, 128 + signal.SIGTERM, err)
        self.assertFalse(os.path.exists(out[0]))

    def test_a_child_signalled_before_the_fork_hook_removes_nothing(self):
        # E5: an earlier fork hook signals the child while it still holds
        # the parent's registry
        code, out, err = run_py("import os, signal, sys\n"
                                "os.register_at_fork(after_in_child=lambda: os.kill(os.getpid(), signal.SIGHUP))\n"
                                "import scratch\nd = scratch.scratch_dir('t-')\npid = os.fork()\n"
                                "if pid == 0:\n    sys.exit(0)\n"
                                "_, status = os.waitpid(pid, 0)\n"
                                "print(d, os.path.isdir(d), os.waitstatus_to_exitcode(status), flush=True)", self.tmp)
        self.assertEqual(code, 0, err)
        self.assertEqual(out[1:], ["True", str(128 + signal.SIGHUP)])
        self.assertFalse(os.path.exists(out[0]))

    def test_signals_are_installed_by_the_main_thread_after_a_worker_first_import(self):
        # E6: the module first imported (and its first directory made) in a
        # worker thread; the main thread's first call installs the handlers
        env = dict(os.environ, TMPDIR=self.tmp, PYTHONPATH=str(HERE))
        code = ("import threading, time, sys\n"
                "t = threading.Thread(target=lambda: __import__('scratch').scratch_dir('w-'))\nt.start(); t.join()\n"
                "import scratch\nscratch.scratch_dir('t-')\nprint('READY', flush=True)\ntime.sleep(300)\n")
        p = subprocess.Popen([sys.executable, "-c", code], env=env, stdout=subprocess.PIPE, text=True)
        try:
            self.assertEqual(p.stdout.readline().strip(), "READY")
            p.send_signal(signal.SIGTERM)
            self.assertEqual(p.wait(timeout=30), 128 + signal.SIGTERM)
        finally:
            if p.poll() is None:
                p.kill()
                p.wait()
        self.assertEqual([n for n in os.listdir(self.tmp) if n.startswith(("t-", "w-"))], [])

    def test_no_directory_is_made_after_the_exit_removal(self):
        # E7: an exit handler that runs after scratch's
        code, out, err = run_py("import atexit, os\n"
                                "def late():\n    import scratch\n"
                                "    try:\n        print(scratch.scratch_dir('t-'), flush=True)\n"
                                "    except SystemExit:\n        print('refused', flush=True)\n"
                                "atexit.register(late)\nimport scratch\nscratch.scratch_dir('t-')", self.tmp)
        self.assertEqual(out, ["refused"], err)
        self.assertEqual([n for n in os.listdir(self.tmp) if n.startswith("t-")], [])

    def test_a_relative_parent_is_registered_as_returned(self):
        # E8
        code, out, err = run_py("import os, scratch\nos.mkdir('rel')\n"
                                "a, b, c = (scratch.scratch_dir('t-', dir='rel') for _ in range(3))\n"
                                "scratch.remove(a); scratch.keep(b); os.chdir('rel')\n"
                                "print(a, b, c, os.path.isabs(c), os.path.exists(a), flush=True)", self.tmp)
        self.assertEqual(code, 0, err)
        a, b, c, absolute, a_there = out
        self.assertEqual((absolute, a_there), ("True", "False"))
        self.assertTrue(os.path.isdir(b))          # kept
        self.assertFalse(os.path.exists(c))        # removed at exit, after the chdir

    def test_a_name_collision_does_not_unregister_a_live_directory(self):
        # E9
        code, out, err = run_py("import secrets, scratch\nnames = iter(['same', 'same', 'other'])\n"
                                "secrets.token_hex = lambda n: next(names)\n"
                                "print(scratch.scratch_dir('t-'), scratch.scratch_dir('t-'), flush=True)", self.tmp)
        self.assertEqual(code, 0, err)
        self.assertEqual(len(set(out)), 2)
        self.assertFalse(any(os.path.exists(p) for p in out))

    def test_a_tools_own_sigterm_handler_stays(self):
        code, out, err = run_py("import signal\nsignal.signal(signal.SIGTERM, lambda *a: None)\nh = signal.getsignal(signal.SIGTERM)\n"
                                "from scratch import scratch_dir\nscratch_dir('t-')\nprint(signal.getsignal(signal.SIGTERM) is h)",
                                self.tmp)
        self.assertEqual(out, ["True"], err)

    def test_a_directory_whose_case_took_its_permissions_is_removed(self):
        for mode in (0o500, 0o000):
            d = Path(self.tmp, "locked")
            (d / "sub" / "deeper").mkdir(parents=True)
            (d / "sub" / "deeper" / "f").write_text("x")
            os.chmod(d / "sub" / "deeper", mode)
            os.chmod(d / "sub", mode)
            os.chmod(d, mode)
            scratch.rmtree(str(d))
            self.assertFalse(d.exists(), oct(mode))


class Tools(unittest.TestCase):
    """Each tool's directory, made through its real code path with the
    expensive parts mocked, is gone after a normal and after a failing exit.
    Each snippet prints the directory the tool made as DIR=<path>."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="test-scratch-tools-")

    def tearDown(self):
        scratch.rmtree(self.tmp)

    def both(self, code):
        for fail, rc in ((False, 0), (True, 1)):
            with self.subTest(fail=fail):
                got, out, err = run_py(f"FAIL = {fail}\n" + textwrap.dedent(code), self.tmp)
                self.assertEqual(got, rc, err)
                dirs = [w[4:] for w in out if w.startswith("DIR=")]
                self.assertTrue(dirs, err)
                for d in dirs:
                    self.assertFalse(os.path.exists(d), f"{d} outlived the tool")

    def test_builtin_diff_robigo_rts_records(self):
        self.both(f"""
            import importlib.util, json, os, sys
            from types import SimpleNamespace
            from unittest import mock
            s = importlib.util.spec_from_file_location("bd", {str(HERE / 'builtin-diff.py')!r})
            bd = importlib.util.module_from_spec(s); s.loader.exec_module(bd)
            d = os.path.join(os.environ["TMPDIR"], "oracle"); os.makedirs(d, exist_ok=True)
            json.dump({{"lost": {{}}}}, open(os.path.join(d, "done"), "w"))
            def batch(cmd, env, b, out, n, work):
                print("DIR=" + os.path.dirname(out), flush=True)
                if FAIL:
                    raise RuntimeError("robigo-rt died")
                return {{}}
            with mock.patch.object(bd, "oracle_dir", lambda *a: d), mock.patch.object(bd, "read_cases", lambda d: []), \\
                    mock.patch.object(bd, "run_batch", batch):
                bd.run_rt(SimpleNamespace(data=d, seed=0, rsrc="x", boot="interim"), SimpleNamespace(isolated=False), "off", "rt", [])
            """)

    def perf(self, call):
        return f"""
            import importlib.util, os
            from types import SimpleNamespace
            from unittest import mock
            s = importlib.util.spec_from_file_location("perf", {str(HERE / 'perf.py')!r})
            perf = importlib.util.module_from_spec(s); s.loader.exec_module(perf)
            tmp = os.environ["TMPDIR"]
            def instr(binary, rows, jobs, work, *a, **k):
                print("DIR=" + work, flush=True)
                if FAIL:
                    raise RuntimeError("callgrind died")
                return ({{"plain": 500.0}}, {{"plain": 400.0}}) if k.get("cycles") else {{"plain": 500.0}}
            with mock.patch.object(perf, "rt_instr", instr), mock.patch.object(perf, "rt_rss", lambda *a: {{"p.R": 1}}), \\
                    mock.patch.object(perf, "binary_identity", lambda *_: {{}}), \\
                    mock.patch.object(perf, "report_identity", lambda *_, **__: None), \\
                    mock.patch.object(perf, "gnur_version", lambda: "4.6.2"), \\
                    mock.patch.object(perf, "valgrind_path", lambda: "true"), \\
                    mock.patch.object(perf, "cpu_signature", lambda: ("cpu", "0" * 16)):
                {call}
            """

    BARS = ('[gnur]\nversion = "%s"\nsource = "x"\n[gnur.instr]\nplain = 1000.0\n[gnur.rss]\n"p.R" = 50000\n'
            '[[bar]]\nrow = "plain"\nagainst = "gnur"\nmax = 0.6\n[[rss]]\nprogram = "p.R"\nagainst = ["gnur"]\n')

    def test_perf_rt_bars(self):
        self.both(self.perf(
            f'b = os.path.join(tmp, "bars.toml"); open(b, "w").write({self.BARS!r} % perf.pins()["rt"]); '
            'perf.cmd_rt_bars(SimpleNamespace(bars=b, robigo_rt="rt", main=None, jobs=1, runs=1, '
            'out=os.path.join(tmp, "o.json"), gnur=None, cycles=False))'))

    def test_perf_rt_gnur(self):
        self.both(self.perf(
            f'b = os.path.join(tmp, "bars.toml"); open(b, "w").write({self.BARS!r} % perf.pins()["rt"]); '
            'perf.cmd_rt_gnur(SimpleNamespace(bars=b, jobs=1, runs=1, out=os.path.join(tmp, "g.json")))'))

    def test_rast_parser(self):
        # `true` stands in for Rscript: the helper process exits at once
        self.both(f"""
            import importlib.util
            s = importlib.util.spec_from_file_location("rast", {str(HERE / 'rast.py')!r})
            rast = importlib.util.module_from_spec(s); s.loader.exec_module(rast)
            p = rast.Parser(rscript="true")
            print("DIR=" + p.dir, flush=True)
            if FAIL:
                raise RuntimeError("the parse helper died")   # no close(): the exit handler removes it
            p.close()
            """)

    def test_capi_bench(self):
        self.both(f"""
            import importlib.util, sys, subprocess
            from unittest import mock
            s = importlib.util.spec_from_file_location("bench", {str(ROOT / 'crates/robigo-capi/bench/bench.py')!r})
            bench = importlib.util.module_from_spec(s); s.loader.exec_module(bench)
            def docker(args, cwd):
                print("DIR=" + cwd, flush=True)
                return subprocess.CompletedProcess(args, 1 if FAIL else 0, "", "SHLIB failed")
            sys.argv = ["bench.py"]
            with mock.patch.object(bench, "docker", docker), mock.patch.object(bench, "run", lambda *a, **k: {{}}):
                try:
                    bench.main()
                except SystemExit as e:
                    sys.exit(1 if e.code else 0)
            """)


if __name__ == "__main__":
    unittest.main()
