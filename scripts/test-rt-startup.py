#!/usr/bin/env python3
"""Tests of the startup gate (scripts/rt-startup.py's verdict) and of
idiom-vs-r.py's count of a loop's allocator calls, which decides the heap
shifts. Run by the perf-rt job before the bars."""
import importlib.util
import os
import signal
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, file))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


su = load("rt_startup", "rt-startup.py")
iv = load("idiom_vs_r", "idiom-vs-r.py")


def start(instr, anon, rss):
    return {"instr": instr, "anon_kib": anon, "rss_kib": rss}


class Verdict(unittest.TestCase):
    MAIN = {"interim": start(15_000_000, 1_500, 9_800), "rdb-jit": start(95_000_000, 6_800, 15_600)}

    def fails(self, new):
        return su.verdict(new, self.MAIN)

    def test_equal_passes(self):
        self.assertEqual(self.fails(dict(self.MAIN)), [])

    def test_instructions_at_and_over_the_limit(self):
        at = 95_000_000 * 1.005
        self.assertEqual(self.fails({"rdb-jit": start(at, 6_800, 15_600)}), [])
        f = self.fails({"rdb-jit": start(at + 1, 6_800, 15_600)})
        self.assertEqual(len(f), 1)
        self.assertIn("instructions", f[0])

    def test_anonymous_memory_has_a_floor(self):
        # 1% of 1,500 KiB is 15 KiB: the 64 KiB floor holds
        self.assertEqual(self.fails({"interim": start(15_000_000, 1_564, 9_800)}), [])
        f = self.fails({"interim": start(15_000_000, 1_565, 9_800)})
        self.assertEqual(len(f), 1)
        self.assertIn("anonymous", f[0])
        # above the floor, 1%: 6,800 KiB allows 68 KiB
        self.assertEqual(self.fails({"rdb-jit": start(95_000_000, 6_868, 15_600)}), [])
        self.assertEqual(len(self.fails({"rdb-jit": start(95_000_000, 6_869, 15_600)})), 1)

    def test_peak_rss_is_loose(self):
        self.assertEqual(self.fails({"interim": start(15_000_000, 1_500, 9_800 * 1.05)}), [])
        f = self.fails({"interim": start(15_000_000, 1_500, 9_800 * 1.05 + 1)})
        self.assertEqual(len(f), 1)
        self.assertIn("peak RSS", f[0])

    def test_a_start_main_lacks_is_not_judged(self):
        self.assertEqual(su.verdict({"rdb-nojit": start(1, 1, 1)}, self.MAIN), [])
        self.assertEqual(su.verdict({"interim": start(10**12, 10**9, 10**9)}, {}), [])
        # a record main has only in part
        self.assertEqual(su.verdict({"interim": start(10**12, 10**9, 10**9)}, {"interim": {"instr": 1}}), [])
        self.assertEqual(len(su.table([{"name": "interim"}], {"interim": start(1, 2, 3)}, {"interim": {"instr": 1}})), 3)

    def test_a_bars_files_starts_are_checked_before_measuring(self):
        ok = [{"name": "interim", "gnur": "jit"}, {"name": "rdb-nojit", "gnur": "nojit"}]
        self.assertEqual(su.spec_errors(ok), [])
        bad = su.spec_errors([{"name": "rdb-jti", "gnur": "jit"}, {"name": "rdb-pkgs"},
                              {"name": "interim", "gnur": "noJIT"}, {"name": "interim"}, {"gnur": "jit"}])
        self.assertEqual(len(bad), 5, bad)
        self.assertIn("no such start", bad[0])
        self.assertIn("reserved", bad[1])
        self.assertIn("no GNU R start", bad[2])
        self.assertIn("twice", bad[3])
        self.assertIn("no name", bad[4])

    def test_only_defined_starts_run(self):
        self.assertNotIn("rdb-pkgs", su.defined())
        with self.assertRaises(SystemExit):
            su.run("/nonexistent", ["rdb-pkgs"])

    def test_table_has_a_row_per_entry(self):
        lines = su.table([{"name": "interim", "gnur": "jit"}], {"interim": start(1, 2, 3)},
                         {"interim": start(1, 2, 3)}, {"jit": start(4, 5, 6)})
        self.assertEqual(len(lines), 3)
        self.assertIn("(jit)", lines[2])


class Memory(unittest.TestCase):
    """memory() on small programs: its readings, and that it cannot hang."""
    PY = sys.executable

    def test_a_large_output_does_not_block_the_child(self):
        # far more than a pipe's buffer, printed before the exit stop
        a, h, status, out = su.memory([self.PY, "-c", "import sys; sys.stdout.write('x' * 2_000_000)"],
                                      dict(os.environ))
        self.assertEqual(status, 0)
        self.assertEqual(out, b"x" * 2_000_000)

    def test_the_anonymous_peak_survives_a_free_before_exit(self):
        # 64 MB touched, then freed: at the exit stop little is left
        prog = "b = bytearray(64 << 20)\nfor i in range(0, len(b), 4096): b[i] = 1\ndel b\n"
        a, h, status, _ = su.memory([self.PY, "-c", prog], dict(os.environ))
        self.assertEqual(status, 0)
        self.assertGreaterEqual(a, 64 << 10)
        self.assertGreaterEqual(h, a)

    def no_child_left(self):
        with self.assertRaises(ChildProcessError):
            os.waitpid(-1, os.WNOHANG)

    def test_an_exit_status_comes_back_and_no_child_is_left(self):
        for syscalls in (True, False):
            _, h, status, _ = su.memory([self.PY, "-c", "import sys; sys.exit(3)"], dict(os.environ), syscalls)
            self.assertEqual(status, 3)
            self.assertGreater(h, 0)
            self.no_child_left()

    def test_a_real_sigtrap_reaches_the_child(self):
        # its default action ends the process: not ptrace's to swallow
        _, _, status, _ = su.memory([self.PY, "-c", "import os, signal; os.kill(os.getpid(), signal.SIGTRAP)"],
                                    dict(os.environ))
        self.assertEqual(status, -signal.SIGTRAP)
        self.no_child_left()

    def test_a_child_that_starts_a_thread_is_refused(self):
        # alive at exit, or started and joined long before it
        for prog in ("import threading, time, os\nthreading.Thread(target=time.sleep, args=(0.2,)).start()\n"
                     "os._exit(0)\n",
                     "import threading\nt = threading.Thread(target=lambda: bytearray(8 << 20))\nt.start()\n"
                     "t.join()\n"):
            with self.assertRaises(SystemExit) as e:
                su.memory([self.PY, "-c", prog], dict(os.environ))
            self.assertIn("thread", str(e.exception))
            self.no_child_left()

    def test_consecutive_threads_do_not_outrun_the_cleanup(self):
        # the refusal kills the child before it is resumed: no thread made
        # after the first is left stopped (it would hold the exit up)
        prog = "import threading, time\nts = [threading.Thread(target=time.sleep, args=(0.2,)) for _ in range(20)]\n" \
               "[t.start() for t in ts]\n[t.join() for t in ts]\n"
        for _ in range(5):
            with self.assertRaises(SystemExit):
                su.memory([self.PY, "-c", prog], dict(os.environ))
            self.no_child_left()

    def test_a_clone_that_is_a_process_is_killed_too(self):
        # clone() without CLONE_THREAD and with an exit signal other than
        # SIGCHLD: a traced process of its own, which waits forever
        prog = ("import ctypes, signal\nlibc = ctypes.CDLL(None, use_errno=True)\n"
                "stack = ctypes.create_string_buffer(1 << 16)\n"
                "CB = ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_void_p)\n"
                "f = CB(lambda _: libc.pause())\n"
                "top = ctypes.c_void_p(ctypes.addressof(stack) + len(stack))\n"
                "libc.clone(f, top, signal.SIGUSR1, None)\nlibc.pause()\n")
        with self.assertRaises(SystemExit):
            su.memory([self.PY, "-c", prog], dict(os.environ), timeout=20)
        self.no_child_left()

    def test_a_stop_signal_is_resumed_not_held(self):
        # the child stops itself (SIGSTOP and a terminal's SIGTSTP): delivered,
        # then its group-stop resumed (a restart's signal is ignored there,
        # ptrace(2)), so it runs on to its exit in time
        for sig in ("SIGSTOP", "SIGTSTP"):
            prog = f"import os, signal\nos.kill(os.getpid(), signal.{sig})\nprint('after')\n"
            for syscalls in (True, False):
                _, _, status, out = su.memory([self.PY, "-c", prog], dict(os.environ), syscalls, timeout=20)
                self.assertEqual((status, out), (0, b"after\n"), (sig, syscalls))
                self.no_child_left()

    def test_a_child_that_forks_is_refused_and_nothing_is_left(self):
        # fork(), and vfork() through posix_spawn: refused as the new process
        # is made, which waits forever, and is killed with the child
        for prog in ("import os, time\nif os.fork() == 0:\n    time.sleep(60)\nos._exit(0)\n",
                     "import os, subprocess\nsubprocess.Popen(['sleep', '60'])\nos._exit(0)\n"):
            with self.assertRaises(SystemExit) as e:
                su.memory([self.PY, "-c", prog], dict(os.environ), timeout=20)
            self.assertIn("another process", str(e.exception))
            self.no_child_left()

    def test_a_run_over_its_time_is_killed(self):
        with self.assertRaises(SystemExit) as e:
            su.memory([self.PY, "-c", "import time; time.sleep(30)"], dict(os.environ), timeout=1)
        self.assertIn("ran over", str(e.exception))
        self.no_child_left()

    def test_an_exception_while_traced_reaps_the_child(self):
        # a reading that fails at a system-call stop, and one that fails at
        # the exit stop (where a SIGKILL is dropped): no child is left
        fail = lambda pid: (_ for _ in ()).throw(OSError("no /proc"))  # noqa: E731
        for name in ("anon_kib", "status_kib"):
            saved = getattr(su, name)
            setattr(su, name, fail)
            try:
                with self.assertRaises(OSError):
                    su.memory([self.PY, "-c", "print(1)"], dict(os.environ))
            finally:
                setattr(su, name, saved)
            self.no_child_left()


class Trees(unittest.TestCase):
    """The starts' R_HOME trees: given as perf-rt-build's tarball, missing,
    or to be built (refused in CI)."""

    def setUp(self):
        self.d = tempfile.mkdtemp(prefix="rt-trees-")
        self.env = {k: os.environ.get(k) for k in ("GITHUB_ACTIONS", "CI")}

    def tearDown(self):
        import shutil
        shutil.rmtree(self.d, ignore_errors=True)
        for k, v in self.env.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v

    def tarball(self, members):
        import io
        import tarfile
        path = os.path.join(self.d, "trees.tar")
        with tarfile.open(path, "w") as t:
            for name in members:
                data = b"x"
                info = tarfile.TarInfo(name)
                info.size = len(data)
                t.addfile(info, io.BytesIO(data))
        return path

    def test_given_trees_replace_the_checkouts(self):
        co = os.path.join(self.d, "co")
        os.makedirs(os.path.join(co, ".robigo", "rt-home-db", "old"))
        tar = self.tarball([".robigo/rt-home/library/base/R/base", ".robigo/rt-home-db/library/base/R/base.rdb"])
        su.unpack_trees(tar, co)
        self.assertEqual(su.tree_dirs(co), [".robigo/rt-home", ".robigo/rt-home-db"])
        self.assertTrue(os.path.isfile(os.path.join(co, ".robigo/rt-home-db/library/base/R/base.rdb")))
        self.assertFalse(os.path.exists(os.path.join(co, ".robigo/rt-home-db/old")))

    def test_missing_trees_are_refused(self):
        with self.assertRaises(SystemExit) as e:
            su.unpack_trees(os.path.join(self.d, "none.tar"), self.d)
        self.assertIn("no trees", str(e.exception))

    def test_a_tarball_with_more_than_trees_is_refused(self):
        for bad in ("etc/passwd", ".robigo/other/x", "../escape"):
            tar = self.tarball([".robigo/rt-home/x", bad])
            with self.assertRaises(SystemExit):
                su.unpack_trees(tar, os.path.join(self.d, "co"))

    def test_a_tarball_with_a_link_is_refused(self):
        # a tree's root a symlink to ../scripts: a later member would land there
        import tarfile
        path = os.path.join(self.d, "link.tar")
        with tarfile.open(path, "w") as t:
            info = tarfile.TarInfo(".robigo/rt-home")
            info.type = tarfile.SYMTYPE
            info.linkname = "../scripts"
            t.addfile(info)
        co = os.path.join(self.d, "co")
        with self.assertRaises(SystemExit) as e:
            su.unpack_trees(path, co)
        self.assertIn("links", str(e.exception))
        self.assertFalse(os.path.lexists(os.path.join(co, ".robigo", "rt-home")))

    def test_packing_is_allowed_in_ci(self):
        # perf-rt-build packs under GITHUB_ACTIONS: the refusal is a measurement's
        os.environ["GITHUB_ACTIONS"] = "true"
        co = os.path.join(self.d, "co")
        made = []

        def fake(binary, checkout, home):
            made.append(binary)
            os.makedirs(os.path.join(home, ".robigo", "rt-home", "library"))
        saved = su.make_trees
        su.make_trees = fake
        try:
            out = os.path.join(self.d, "out.tar")
            self.assertEqual(su.pack_trees("/bin/true", co, out), [".robigo/rt-home"])
            self.assertEqual(made, ["/bin/true"])
            self.assertTrue(os.path.isfile(out))
        finally:
            su.make_trees = saved

    def test_a_local_run_leaves_the_checkouts_trees_alone(self):
        # built (or unpacked) in a scratch home: the checkout's .robigo is
        # not written, so a developer's tree or an rt-ci beside it is safe
        os.environ.pop("GITHUB_ACTIONS", None)
        os.environ.pop("CI", None)
        co = os.path.join(self.d, "co")
        os.makedirs(os.path.join(co, ".robigo", "rt-home-db", "mine"))
        seen = []

        def fake(binary, checkout, home):
            seen.append((checkout, home))
            os.makedirs(os.path.join(home, ".robigo", "rt-home-db", "library"))
        saved = su.make_trees
        su.make_trees = fake
        try:
            home = su.build_trees("/bin/true", co)
        finally:
            su.make_trees = saved
        self.assertEqual(seen, [(co, home)])
        self.assertNotEqual(os.path.realpath(home), os.path.realpath(co))
        self.assertEqual(os.listdir(os.path.join(co, ".robigo", "rt-home-db")), ["mine"])
        tar = self.tarball([".robigo/rt-home/x"])
        home2 = su.new_home()
        su.unpack_trees(tar, home2)
        self.assertEqual(os.listdir(os.path.join(co, ".robigo")), ["rt-home-db"])

    def test_building_is_refused_in_ci(self):
        os.environ["GITHUB_ACTIONS"] = "true"
        with self.assertRaises(SystemExit) as e:
            su.build_trees("/nonexistent", self.d)
        self.assertIn("perf-rt-build", str(e.exception))
        with self.assertRaises(SystemExit):
            su.run("/nonexistent", ["interim"], self.d, trees=True)


class AllocatorCalls(unittest.TestCase):
    CG = """events: Ir
fn=(1) main
1 10
cfn=(2) malloc
calls=3 0
1 30
cfn=(3) free
calls=2 0
1 20
cfn=(4) strlen
calls=7 0
1 7
fn=(5) other
cfn=(2)
calls=4 0
1 40
cfn=(6) freelist_push
calls=9 0
1 9
cfn=(7) free'2
calls=5 0
1 5
cfn=(8) __libc_malloc
calls=1 0
1 1
cfn=(9) (3)'3
calls=2 0
1 2
cfn=(9)
calls=4 0
1 4
cfn=(10) (2)'(5)
calls=3 0
1 3
cfn=(11) malloc'worker'(1)
calls=6 0
1 6
cfn=(12) (99)'2
calls=7 0
1 7
frfn=(13) realloc
jfn=(14) calloc
cfn=(13)
calls=2 0
1 2
cfn=(14)
calls=1 0
1 1
"""

    def test_which_rows_are_measured_over_the_shifts(self):
        self.assertTrue(iv.follows_heap({"address_sensitive": True}, {}))
        # #974's s3: three malloc/free pairs per iteration
        self.assertTrue(iv.follows_heap({}, {"robigo": 6.0}))
        # either side's loop is enough
        self.assertTrue(iv.follows_heap({}, {"robigo": 0.0, "gnur": 2.0}))
        # s3 after #975, one call per 2,000 iterations: still shifted (a
        # rare realloc can copy in another layout), and plain's none: not
        self.assertTrue(iv.follows_heap({}, {"robigo": 0.0005}))
        self.assertFalse(iv.follows_heap({}, {"robigo": 0.0}))
        # a side not measured is not in `allocs`
        self.assertFalse(iv.follows_heap({}, {}))

    def test_counts_the_allocator_by_compressed_name(self):
        with tempfile.NamedTemporaryFile("w", suffix=".cg", delete=False) as f:
            f.write(self.CG)
        try:
            # malloc 3 + 4 (named once, then by number), free 2, a recursive
            # free'2 5, __libc_malloc 1, free'3 written as a reference with a
            # suffix (--compress-mangled=yes) 2 + 4, malloc under a caller
            # (--separate-callers), referenced 3 and named 6; not
            # freelist_push or strlen, nor a reference to a name never given;
            # realloc and calloc first named on frfn= and jfn= lines, 2 + 1
            self.assertEqual(iv.allocator_calls(f.name), 33)
        finally:
            os.unlink(f.name)


if __name__ == "__main__":
    unittest.main()
