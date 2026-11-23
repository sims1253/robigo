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
import unittest.mock
from types import SimpleNamespace

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

    def test_a_start_s_cache_hits_are_read_from_its_stats_line(self):
        self.assertEqual(su.cache_hits("x\ntranslation cache: hit 430, no stub 2, no database 0\n"), 430)
        self.assertEqual(su.cache_hits("translation cache: hit 0, no stub 435\n"), 0)
        self.assertIsNone(su.cache_hits("no such line\n"))

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


class Layout(unittest.TestCase):
    """The layout shifts (docs/ledger/g3/2026-11-23-startup-layout.md): a
    start's instructions and anonymous peak follow the heap's layout, so
    both are taken over the same set of layouts, the same for every binary."""

    def test_the_shifts_are_fixed(self):
        self.assertEqual(su.LAYOUT_LENGTHS, tuple(range(35, 134, 7)))
        self.assertEqual(su.TOP_PADS, tuple(131072 + 4096 * k for k in range(15)))
        self.assertEqual(su.LAYOUT_SHIFTS, tuple(zip(su.LAYOUT_LENGTHS, su.TOP_PADS)))

    def test_every_binary_gets_the_same_lengths_wherever_its_trees_are(self):
        # main's trees and the PR's are unpacked in different directories:
        # the homes the starts boot from have the same absolute lengths, and
        # each leads to its own trees
        with tempfile.TemporaryDirectory() as a, tempfile.TemporaryDirectory(prefix="x" * 40) as b:
            ha, hb = su.layout_homes(a), su.layout_homes(b)
            self.assertEqual(list(ha), list(su.LAYOUT_LENGTHS))
            for n in su.LAYOUT_LENGTHS:
                self.assertEqual((len(ha[n]), len(hb[n])), (n, n))
                self.assertEqual(os.path.realpath(ha[n]), os.path.realpath(a))
                self.assertEqual(os.path.realpath(hb[n]), os.path.realpath(b))

    def test_the_environment_is_fixed(self):
        # nothing of the caller's: L's 104 KiB step flipped between two
        # binaries at a few bytes of environment
        before = su.clean_env()
        with unittest.mock.patch.dict(os.environ, {"PADPADPAD": "x" * 4000, "R_HOME": "/elsewhere",
                                                   "ROBIGO_RT_BOOT": "rdb"}):
            after = su.clean_env()
        self.assertEqual(before, after)
        self.assertEqual(set(after) - set(su.BASE), {"PATH", "HOME", "ROBIGO_RSRC"})
        # the sources' link: a fixed-length path, whatever the sources' own
        self.assertEqual(len(after["ROBIGO_RSRC"]), len("/tmp/rs-") + 12 + len("/rsrc"))

    def test_measure_boots_each_shift_and_takes_its_figures(self):
        seen = {"instr": [], "memory": []}

        def instr(cmd, env, valgrind):
            seen["instr"].append((env["HOME_VAR"], env["GLIBC_TUNABLES"]))
            return 1000 + len(env["HOME_VAR"])

        def memory(cmd, env, syscalls):
            seen["memory"].append((syscalls, env["HOME_VAR"], env.get("GLIBC_TUNABLES")))
            return (100 + len(env["HOME_VAR"]) if syscalls else 0), 5000, 0, b"", 0

        with tempfile.TemporaryDirectory() as home, \
                unittest.mock.patch.object(su, "instructions", instr), unittest.mock.patch.object(su, "memory", memory):
            v = su.measure(["bin"], {}, 3, "valgrind", "HOME_VAR", home)
        pads = dict(su.LAYOUT_SHIFTS)
        self.assertEqual(sorted(len(h) for h, _ in seen["instr"]), list(su.LAYOUT_LENGTHS))
        for h, t in seen["instr"]:
            self.assertEqual(t, f"glibc.malloc.top_pad={pads[len(h)]}")
        self.assertEqual(v["instr"], 1000 + sum(su.LAYOUT_LENGTHS) / 15)
        # the anonymous peak at every shift, peak RSS at the home itself
        anon = [m for m in seen["memory"] if m[0]]
        self.assertEqual(sorted(len(h) for _, h, _ in anon), list(su.LAYOUT_LENGTHS))
        self.assertEqual(v["anon_kib"], su.ANON_STAT([100 + n for n in su.LAYOUT_LENGTHS]))
        self.assertEqual([m for m in seen["memory"] if not m[0]], [(False, os.path.abspath(home), None)] * 3)


class Boots(unittest.TestCase):
    """Every start names its boot for both binaries, so no measurement
    depends on a binary's default (L's default-boot flip changes M1_BOOT,
    here only), and the gate never compares two boots."""

    def test_every_start_names_its_boot(self):
        for n, env in su.STARTS.items():
            if env is not None:
                self.assertIn("ROBIGO_RT_BOOT", env, n)
        self.assertEqual(su.STARTS["interim"]["ROBIGO_RT_BOOT"], su.M1_BOOT)
        self.assertEqual(set(su.BOOTS), {"m1", "rdb"})
        self.assertEqual(su.BOOTS["m1"], su.M1_BOOT)

    def test_the_default_start_is_marked(self):
        self.assertEqual(su.DEFAULT_START, "rdb-jit")
        v = {"instr": 1.0, "anon_kib": 1.0, "rss_kib": 1.0}
        rows = su.table([{"name": "interim"}, {"name": "rdb-jit"}], {"interim": v, "rdb-jit": v})
        self.assertTrue(rows[2].startswith("| interim |"))
        self.assertTrue(rows[3].startswith("| rdb-jit (default) |"))


class Reallocs(unittest.TestCase):
    def test_growth_reallocs_are_charged_to_the_function_that_grew(self):
        # callgrind_annotate's lines for a file made with --separate-callers=3:
        # the allocator's and RawVec's own frames are passed over
        annotate = (
            "   224,458 ( 1.53%)  ???:realloc'<alloc::raw_vec::RawVecInner>::finish_grow'"
            "<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>'"
            "robigo_rt::define_builtins [/bin/robigo-rt]\n"
            "   100,000 ( 0.68%)  ???:__libc_realloc'<alloc::raw_vec::RawVec<usize>>::grow_one'"
            "<robigo_rt::compile::Comp>::emit\n"
            "    50,000 ( 0.34%)  ???:realloc'0x00000000000bdad0'<robigo_rt::compile::Comp>::emit\n"
            "    10,000 ( 0.07%)  ???:malloc'robigo_rt::define_builtins\n")
        with unittest.mock.patch.object(su.subprocess, "run",
                                        return_value=SimpleNamespace(stdout=annotate, returncode=0)):
            got = su.growth_reallocs("cg.out")
        self.assertEqual(got, [("robigo_rt::define_builtins", 224458), ("<robigo_rt::compile::Comp>::emit", 150000)])


class ReallocsRun(unittest.TestCase):
    """rt-startup.py --reallocs: its starts are checked as run()'s, and its
    callgrind runs are bounded and read as instructions()'."""

    def test_an_unknown_or_reserved_start_is_refused(self):
        for name in ("nope", "rdb-pkgs"):
            with self.assertRaises(SystemExit) as cm:
                su.reallocs("bin", [name], ".", False, "valgrind")
            self.assertIn(f"no start '{name}'", str(cm.exception))

    def test_a_run_over_its_time_and_a_file_without_totals_say_so(self):
        import subprocess as sp
        with tempfile.TemporaryDirectory() as home:
            with unittest.mock.patch.object(su.subprocess, "run", side_effect=sp.TimeoutExpired("valgrind", 1)):
                with self.assertRaises(SystemExit) as cm:
                    su.reallocs("bin", ["interim"], home, False, "valgrind")
                self.assertIn("ran over", str(cm.exception))

            def no_totals(cmd, **kw):
                out = next(a.split("=", 1)[1] for a in cmd if a.startswith("--callgrind-out-file="))
                with open(out, "w") as f:
                    f.write("version: 1\n")
                return SimpleNamespace(returncode=0, stderr="")
            with unittest.mock.patch.object(su.subprocess, "run", side_effect=no_totals):
                with self.assertRaises(SystemExit) as cm:
                    su.reallocs("bin", ["interim"], home, False, "valgrind")
                self.assertIn("no total", str(cm.exception))


class Memory(unittest.TestCase):
    """memory() on small programs: its readings, and that it cannot hang."""
    PY = sys.executable

    def test_a_large_output_does_not_block_the_child(self):
        # far more than a pipe's buffer, printed before the exit stop
        a, h, status, out, _ = su.memory([self.PY, "-c", "import sys; sys.stdout.write('x' * 2_000_000)"],
                                      dict(os.environ))
        self.assertEqual(status, 0)
        self.assertEqual(out, b"x" * 2_000_000)

    def test_the_anonymous_peak_survives_a_free_before_exit(self):
        # 64 MB touched, then freed: at the exit stop little is left
        prog = "b = bytearray(64 << 20)\nfor i in range(0, len(b), 4096): b[i] = 1\ndel b\n"
        a, h, status, _, _ = su.memory([self.PY, "-c", prog], dict(os.environ))
        self.assertEqual(status, 0)
        self.assertGreaterEqual(a, 64 << 10)
        self.assertGreaterEqual(h, a)

    def no_child_left(self):
        with self.assertRaises(ChildProcessError):
            os.waitpid(-1, os.WNOHANG)

    def test_an_exit_status_comes_back_and_no_child_is_left(self):
        for syscalls in (True, False):
            _, h, status, _, _ = su.memory([self.PY, "-c", "import sys; sys.exit(3)"], dict(os.environ), syscalls)
            self.assertEqual(status, 3)
            self.assertGreater(h, 0)
            self.no_child_left()

    def test_a_real_sigtrap_reaches_the_child(self):
        # its default action ends the process: not ptrace's to swallow
        _, _, status, _, _ = su.memory([self.PY, "-c", "import os, signal; os.kill(os.getpid(), signal.SIGTRAP)"],
                                    dict(os.environ))
        self.assertEqual(status, -signal.SIGTRAP)
        self.no_child_left()

    def test_a_start_with_threads_is_measured_with_their_memory(self):
        # a thread alive at exit; one that touches 32 MB and frees it before
        # its join (the peak is in the thread's own memory, freed by its own
        # system calls); twenty at once
        alive = "import threading, time, os\nthreading.Thread(target=time.sleep, args=(0.2,)).start()\nos._exit(0)\n"
        touch = ("import threading\ndef f():\n    b = bytearray(32 << 20)\n    for i in range(0, len(b), 4096): b[i] = 1\n"
                 "    del b\nt = threading.Thread(target=f)\nt.start()\nt.join()\n")
        many = ("import threading, time\nts = [threading.Thread(target=time.sleep, args=(0.2,)) for _ in range(20)]\n"
                "[t.start() for t in ts]\n[t.join() for t in ts]\n")
        for prog in (alive, touch, many):
            for syscalls in (True, False):
                a, h, status, _, kids = su.memory([self.PY, "-c", prog], dict(os.environ), syscalls, timeout=30)
                self.assertEqual((status, kids), (0, 0))
                if prog is touch and syscalls:
                    self.assertGreaterEqual(a, 32 << 10)
                self.no_child_left()

    def test_a_clone_that_is_a_process_is_detached_and_killed_at_exit(self):
        # clone() without CLONE_THREAD and with an exit signal other than
        # SIGCHLD: a process of its own, which waits forever; the start's exit
        # kills it
        prog = ("import ctypes, signal\nlibc = ctypes.CDLL(None, use_errno=True)\n"
                "stack = ctypes.create_string_buffer(1 << 16)\n"
                "CB = ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_void_p)\n"
                "f = CB(lambda _: libc.pause())\n"
                "top = ctypes.c_void_p(ctypes.addressof(stack) + len(stack))\n"
                "libc.clone(f, top, signal.SIGUSR1, None)\nimport os\nos._exit(0)\n")
        _, _, status, _, kids = su.memory([self.PY, "-c", prog], dict(os.environ), timeout=20)
        self.assertEqual((status, kids), (0, 1))
        self.no_child_left()
        self.assertEqual(self.group_left(), [])

    def test_a_stop_signal_is_resumed_not_held(self):
        # the child stops itself (SIGSTOP and a terminal's SIGTSTP): delivered,
        # then its group-stop resumed (a restart's signal is ignored there,
        # ptrace(2)), so it runs on to its exit in time
        for sig in ("SIGSTOP", "SIGTSTP"):
            prog = f"import os, signal\nos.kill(os.getpid(), signal.{sig})\nprint('after')\n"
            for syscalls in (True, False):
                _, _, status, out, _ = su.memory([self.PY, "-c", prog], dict(os.environ), syscalls, timeout=20)
                self.assertEqual((status, out), (0, b"after\n"), (sig, syscalls))
                self.no_child_left()

    def group_left(self):
        """Processes still in a measured start's process group: memory()
        gives each start a group of its own, whose members name it."""
        left = []
        for d in os.listdir("/proc"):
            if d.isdigit():
                try:
                    with open(f"/proc/{d}/cmdline", "rb") as f:
                        if b"rt-startup-test-child" in f.read():
                            left.append(int(d))
                except OSError:
                    pass
        return left

    def test_a_start_that_starts_processes_is_measured_and_they_are_killed(self):
        # GNU R's exit: `sh -c 'rm -Rf' R_TempDir` through posix_spawn (a
        # vfork), waited for; fork(); and a child left running at the exit,
        # which is killed with the start's group
        for prog, kids in (("import os\nos.posix_spawn('/bin/sh', ['sh', '-c', 'true'], os.environ)\nos.wait()\n", 1),
                           ("import os\nif os.fork() == 0:\n    os._exit(0)\nos.wait()\n", 1),
                           ("import subprocess\nsubprocess.Popen(['sleep', '60', 'rt-startup-test-child'])\n", 1)):
            for syscalls in (True, False):
                a, h, status, _, n = su.memory([self.PY, "-c", prog], dict(os.environ), syscalls, timeout=20)
                self.assertEqual((status, n), (0, kids), prog)
                self.assertGreater(a if syscalls else h, 0)
                self.no_child_left()
                self.assertEqual(self.group_left(), [], prog)

    def test_runs_side_by_side_do_not_take_each_others_events(self):
        # memory() waits only on its own start's process group: several at
        # once in one process (threads) each finish with their own numbers
        import concurrent.futures
        prog = "b = bytearray({n} << 20)\nfor i in range(0, len(b), 4096): b[i] = 1\n"
        with concurrent.futures.ThreadPoolExecutor(4) as ex:
            got = list(ex.map(lambda n: su.memory([self.PY, "-c", prog.format(n=n)], dict(os.environ), True, 60),
                              (8, 16, 24, 32)))
        for n, (a, _, status, _, _) in zip((8, 16, 24, 32), got):
            self.assertEqual(status, 0)
            self.assertGreaterEqual(a, n << 10)
        self.no_child_left()

    def test_one_layout_gives_one_anonymous_peak(self):
        # the repeated-invocation check: memory() is exact for a layout, so
        # one program under one environment reads the same peak every run
        # (L's 104 KiB flips came from the environment's size between
        # binaries, not from runs; docs/ledger/g3/2026-11-23-startup-layout.md)
        prog = "b = bytearray(24 << 20)\nfor i in range(0, len(b), 4096): b[i] = 1\n"
        env = dict(su.FIXED_ENV, PYTHONHASHSEED="0")
        peaks = {su.memory([self.PY, "-c", prog], env, True)[0] for _ in range(5)}
        self.assertEqual(len(peaks), 1, peaks)

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
