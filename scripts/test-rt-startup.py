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
"""

    def test_counts_the_allocator_by_compressed_name(self):
        with tempfile.NamedTemporaryFile("w", suffix=".cg", delete=False) as f:
            f.write(self.CG)
        try:
            # malloc 3 + 4 (named once, then by number), free 2
            self.assertEqual(iv.allocator_calls(f.name), 9)
        finally:
            os.unlink(f.name)


if __name__ == "__main__":
    unittest.main()
