#!/usr/bin/env python3
"""Tests of the startup gate (scripts/rt-startup.py's verdict) and of
idiom-vs-r.py's count of a loop's allocator calls, which decides the heap
shifts. Run by the perf-rt job before the bars."""
import importlib.util
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
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


def start(instr, anon, rss, heap=None):
    return {"instr": instr, "anon_kib": anon, "rss_kib": rss, "heap_bytes": heap}


class Verdict(unittest.TestCase):
    MAIN = {"interim": start(15_000_000, 1_500, 9_800, 3_456_959),
            "rdb-jit": start(95_000_000, 6_800, 15_600, 9_947_143)}

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

    def test_the_malloc_peak_is_judged_and_the_anonymous_peak_only_reported(self):
        # 0.5% of 3,456,959 bytes is 17,285: above the 16 KiB floor
        self.assertEqual(self.fails({"interim": start(15_000_000, 1_500, 9_800, 3_456_959 + 17_284)}), [])
        f = self.fails({"interim": start(15_000_000, 1_500, 9_800, 3_456_959 + 17_285)})
        self.assertEqual(len(f), 1)
        self.assertIn("peak malloc'd bytes", f[0])
        # a small start: the 16 KiB floor holds
        self.assertEqual(su.heap_limit_bytes(1_000_000), 16 << 10)
        # #991's pair differed by 2 bytes: far inside
        self.assertEqual(self.fails({"rdb-jit": start(95_000_000, 6_800, 15_600, 9_947_145)}), [])
        # the anonymous peak fails nothing, even 1 MiB over
        self.assertEqual(self.fails({"rdb-jit": start(95_000_000, 6_800 + 1024, 15_600, 9_947_143)}), [])
        # a side without the figure (an older binary) is not judged on it
        self.assertEqual(self.fails({"rdb-jit": start(95_000_000, 6_800, 15_600, None)}), [])

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

        def dhat(cmd, env, valgrind):
            seen["dhat"] = env
            return 4242

        def gc(cmd, env):
            seen["gc"] = env
            return None
        with tempfile.TemporaryDirectory() as home, \
                unittest.mock.patch.object(su, "instructions", instr), unittest.mock.patch.object(su, "memory", memory), \
                unittest.mock.patch.object(su, "malloc_peak", dhat), unittest.mock.patch.object(su, "gc_peak", gc):
            v = su.measure(["bin"], {}, 3, "valgrind", "HOME_VAR", home)
        # memory as allocation: one DHAT run and one stats run at the first shift
        n0, pad0 = su.LAYOUT_SHIFTS[0]
        for k in ("dhat", "gc"):
            self.assertEqual((len(seen[k]["HOME_VAR"]), seen[k]["GLIBC_TUNABLES"]),
                             (n0, f"glibc.malloc.top_pad={pad0}"))
        self.assertEqual((v["heap_bytes"], v["gc_peak"]), (4242, None))
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


@unittest.skipUnless(os.environ.get("RT_STARTUP_TEST_BIN") and su.M1_BOOT == "",
                     "a binary in RT_STARTUP_TEST_BIN (perf-rt's Starts step gives it), whose unset boot is M1's")
class M1Name(unittest.TestCase):
    """`ROBIGO_RT_BOOT=m1` names the boot an unset value is until L5's flip
    (main.rs), so that the flip's one-line change of M1_BOOT to "m1" moves
    nothing: the interim start under each, on the binary under test, over
    the same 15 layouts, within #995's measured floor (0.073% instructions,
    14 KiB anonymous peak). Skipped once M1_BOOT is "m1" (the flip): unset
    is then the rdb boot."""

    def tearDown(self):
        # the measurement's pools and timers end with it: a thread left
        # alive here would make the next test's fork a fork of a
        # multi-threaded process
        others = [t.name for t in threading.enumerate() if t is not threading.main_thread()]
        self.assertEqual(others, [], "threads left alive after M1Name")

    def test_the_interim_start_is_the_same_under_m1(self):
        bin_ = os.environ["RT_STARTUP_TEST_BIN"]
        trees = os.environ.get("RT_STARTUP_TEST_TREES") or False
        starts = dict(su.STARTS, **{"interim-m1": {"ROBIGO_RT_BOOT": "m1"}})
        with unittest.mock.patch.object(su, "STARTS", starts):
            got = su.run(bin_, starts=["interim", "interim-m1"], trees=trees, runs=1)
        a, b = got["interim"], got["interim-m1"]
        self.assertLessEqual(abs(b["instr"] / a["instr"] - 1) * 100, 0.073, (a["instr"], b["instr"]))
        self.assertLessEqual(abs(b["anon_kib"] - a["anon_kib"]), 14, (a["anon_kib"], b["anon_kib"]))


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


class LogicalPeak(unittest.TestCase):
    """The memory clause's figures: DHAT's t-gmax, and robigo-heap's own
    peak line where a binary prints it."""

    def test_a_stats_run_that_fails_is_refused_not_read_as_no_line(self):
        # a binary failing only under ROBIGO_GC_STATS=1 must not print "-",
        # the same as one that does not report the line (#1009's bot)
        crash = [sys.executable, "-c", "import os, sys\nsys.exit(3 if os.environ.get('ROBIGO_GC_STATS') else 0)"]
        with self.assertRaises(SystemExit) as cm:
            su.gc_peak(crash, dict(os.environ))
        self.assertIn("under ROBIGO_GC_STATS=1 exited 3", str(cm.exception))
        quiet = [sys.executable, "-c", "pass"]
        self.assertIsNone(su.gc_peak(quiet, dict(os.environ)))

    def test_the_malloc_peak_sums_the_program_points_at_t_gmax(self):
        doc = {"dhatFileVersion": 2, "mode": "heap", "pps": [{"gb": 5000, "tb": 9}, {"gb": 1200}, {"gb": 0}]}
        self.assertEqual(su.malloc_peak_of(json.dumps(doc)), 6200)

    def test_malformed_dhat_output_is_refused_never_read_as_zero(self):
        # Sol: each of these passed the clause as 0 before
        good = {"dhatFileVersion": 2, "mode": "heap", "pps": [{"gb": 5000}]}
        for bad in ("not json", "[]", "{}",
                    json.dumps(dict(good, pps=[{}])),
                    json.dumps(dict(good, pps=[{"gb": -1}])),
                    json.dumps(dict(good, pps=[{"gb": 1.5}])),
                    json.dumps(dict(good, pps=[{"gb": True}])),
                    json.dumps(dict(good, pps=[])),
                    json.dumps(dict(good, mode="copy")),
                    json.dumps(dict(good, dhatFileVersion=1))):
            with self.assertRaises(SystemExit, msg=bad):
                su.malloc_peak_of(bad)

    def test_robigo_heaps_peak_line_is_read_where_it_is_printed(self):
        line = ("robigo gc: collections 0 (entry 0)\n"
                "robigo gc: peak bytes in use 123456 (objects 100000, block overhead 20000, large 3456); "
                "peak chunk bytes mapped 2097152\n")
        self.assertEqual(su.gc_peak_of(line), {"in_use": 123456, "objects": 100000, "overhead": 20000,
                                               "large": 3456, "chunks": 2097152})
        # main's binary before H's line: not reported, never None-valued counts
        self.assertIsNone(su.gc_peak_of("robigo gc: collections 0 (entry 0)\n"))
        v = {"instr": 1.0, "anon_kib": 1.0, "rss_kib": 1.0, "heap_bytes": 2048, "gc_peak": None}
        row = su.table([{"name": "interim"}], {"interim": v}, {"interim": dict(v, gc_peak=su.gc_peak_of(line))})[2]
        self.assertIn("| - | 120.6 |", row)

    def peak(self, prog):
        return su.malloc_peak([sys.executable, "-c", prog], dict(su.FIXED_ENV, PYTHONHASHSEED="0",
                                                                 PYTHONMALLOC="malloc"), "valgrind")

    @unittest.skipUnless(shutil.which("valgrind") and shutil.which("cc"), "no valgrind or cc")
    def test_the_peak_is_the_exact_maximum_of_live_requested_bytes(self):
        # Sol's case against massif's peak snapshot (which weighs the
        # allocator's overhead): 512 KiB freed, then 40,000 one-byte blocks
        # and 2 MiB live at once. Exactly 2 MiB + 40,000 bytes
        src = ("#include <stdlib.h>\n#include <string.h>\nint main(void) {\n"
               "char *a = malloc(512 << 10); memset(a, 1, 512 << 10); free(a);\n"
               "static char *p[40000]; for (int i = 0; i < 40000; i++) p[i] = malloc(1);\n"
               "char *b = malloc(2 << 20); memset(b, 1, 2 << 20); return p[0] != 0 && b != 0 ? 0 : 1; }\n")
        with tempfile.TemporaryDirectory() as d:
            c, exe = os.path.join(d, "t.c"), os.path.join(d, "t")
            with open(c, "w") as f:
                f.write(src)
            import subprocess
            subprocess.run(["cc", "-O0", "-o", exe, c], check=True)
            self.assertEqual(su.malloc_peak([exe], dict(su.FIXED_ENV), "valgrind"), (2 << 20) + 40000)

    @unittest.skipUnless(shutil.which("valgrind"), "no valgrind")
    def test_one_binary_reads_the_same_bytes_every_run(self):
        prog = "b = bytearray(3 << 20)\nc = [bytes(100) for _ in range(1000)]\n"
        self.assertEqual(len({self.peak(prog) for _ in range(3)}), 1)

    @unittest.skipUnless(shutil.which("valgrind"), "no valgrind")
    def test_a_retained_allocation_fails_the_clause_by_its_size(self):
        base = "b = bytearray(4 << 20)\n"
        more = base + "keep = bytearray(256 << 10)\n"
        a, b = self.peak(base), self.peak(more)
        self.assertGreaterEqual(b - a, 256 << 10)
        self.assertLess(b - a, (256 << 10) + 4096)
        main = {"interim": start(1, 1, 1, a)}
        self.assertEqual(su.verdict({"interim": start(1, 1, 1, a)}, main), [])
        f = su.verdict({"interim": start(1, 1, 1, b)}, main)
        self.assertEqual(len(f), 1)
        self.assertIn(f"{b - a:+,}", f[0])


class Memory(unittest.TestCase):
    """memory() on small programs: its readings, and that it cannot hang."""
    PY = sys.executable

    @unittest.skipUnless(shutil.which("cc"), "no cc")
    def test_no_thread_starves_behind_threads_that_stop_again_at_once(self):
        # sixteen threads in a loop of system calls, each stopping again as
        # soon as it is resumed, and an older thread that must run once to
        # end them. Served one waitpid at a time the older one starved: the
        # run never ended (perf-rt on #1006, 20 sleeping Python threads, once;
        # here it timed out 5 runs of 5). Every stop of a round is served now
        c = ("#include <pthread.h>\n#include <stdatomic.h>\n#include <unistd.h>\n#include <sys/syscall.h>\n"
             "static atomic_int done;\n"
             "static void *older(void *_) { usleep(50000); atomic_store_explicit(&done, 1, memory_order_relaxed);"
             " return 0; }\n"
             "static void *newer(void *_) {\n"
             "    while (!atomic_load_explicit(&done, memory_order_relaxed)) syscall(SYS_getppid);\n"
             "    return 0;\n}\n"
             "int main(void) {\n    pthread_t a, b[16];\n    pthread_create(&a, 0, older, 0);\n"
             "    for (int i = 0; i < 16; i++) pthread_create(&b[i], 0, newer, 0);\n"
             "    pthread_join(a, 0);\n    for (int i = 0; i < 16; i++) pthread_join(b[i], 0);\n    return 0;\n}\n")
        with tempfile.TemporaryDirectory() as d:
            src, exe = os.path.join(d, "starve.c"), os.path.join(d, "starve")
            with open(src, "w") as f:
                f.write(c)
            subprocess.run(["cc", "-O2", "-pthread", "-o", exe, src], check=True)
            _, _, status, _, kids = su.memory([exe], dict(os.environ), True, timeout=10)
        self.assertEqual((status, kids), (0, 0))

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

    def bounded(self, f, limit):
        """f()'s outcome (its value, or the exception it raised), from a
        thread given `limit` seconds: a memory() that hangs fails the test
        instead of the suite."""
        box = {}

        def run():
            try:
                box["value"] = f()
            except BaseException as e:  # noqa: BLE001 - SystemExit included
                box["error"] = e
        th = threading.Thread(target=run, daemon=True)
        t0 = time.monotonic()
        th.start()
        th.join(limit)
        self.assertFalse(th.is_alive(), f"memory() still running after {limit} s")
        return box, time.monotonic() - t0

    def test_a_start_that_never_ends_with_processes_and_threads_ends_at_its_timeout(self):
        # the timeout ends every wait memory() makes: a process the start
        # made (waited for through the group, never alone), threads, and the
        # leader parked at its exit stop once killed
        # (each child names the marker in its argv, which group_left() reads;
        # sleep(1) refuses it as an interval, so a Python sleeper)
        sleeper = "[sys.executable, '-c', 'import time; time.sleep(60)', 'rt-startup-test-child']"
        prog = ("import subprocess, threading, time, os, sys\n"
                f"subprocess.Popen({sleeper})\n"
                f"if os.fork() == 0:\n    os.execv(sys.executable, {sleeper})\n"
                "[threading.Thread(target=time.sleep, args=(60,)).start() for _ in range(4)]\n"
                "time.sleep(60)\n")
        for syscalls in (True, False):
            box, secs = self.bounded(lambda: su.memory([self.PY, "-c", prog], dict(os.environ), syscalls, timeout=2),
                                     30)
            self.assertIsInstance(box.get("error"), SystemExit, box)
            self.assertIn("ran over 2 s", str(box["error"]))
            self.assertLess(secs, 30)
            self.no_child_left()
            self.assertEqual(self.group_left(), [])

    def test_every_blocking_wait_is_on_the_group(self):
        # the only wait the timeout cannot end is one on a single tracee: past
        # the exec stop's (before the timer starts), every blocking wait is on
        # the start's process group
        calls = []
        real = os.waitpid

        def spy(pid, options):
            calls.append((pid, options))
            return real(pid, options)
        clone = ("import ctypes, signal\nlibc = ctypes.CDLL(None, use_errno=True)\n"
                 "stack = ctypes.create_string_buffer(1 << 16)\n"
                 "CB = ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_void_p)\n"
                 "f = CB(lambda _: libc.pause())\n"
                 "top = ctypes.c_void_p(ctypes.addressof(stack) + len(stack))\n"
                 "libc.clone(f, top, signal.SIGUSR1, None)\nimport os\nos._exit(0)\n")
        threads = "import threading, time\n[threading.Thread(target=time.sleep, args=(0.05,)).start() for _ in range(4)]\n"
        forks = "import os\nif os.fork() == 0:\n    os._exit(0)\nos.wait()\n"
        for prog in (clone, threads, forks):
            calls.clear()
            with unittest.mock.patch.object(su.os, "waitpid", spy):
                su.memory([self.PY, "-c", prog], dict(os.environ), timeout=20)
            blocking = [pid for pid, options in calls if not options & os.WNOHANG]
            self.assertGreater(blocking[0], 0)
            self.assertTrue(all(pid < 0 for pid in blocking[1:]), blocking)
            self.no_child_left()

    def test_an_exception_at_a_clone_event_reaps_every_tracee(self):
        # Sol's case: the new thread never registered (its clone event's
        # handling failed), so only a reap that waits on the whole group
        # finds it, parked at its exit stop
        prog = "import threading, time\n[threading.Thread(target=time.sleep, args=(0.2,)).start() for _ in range(8)]\n"
        with unittest.mock.patch.object(su, "event_msg", side_effect=RuntimeError("no event message")):
            box, secs = self.bounded(lambda: su.memory([self.PY, "-c", prog], dict(os.environ), timeout=5), 30)
        self.assertIsInstance(box.get("error"), RuntimeError, box)
        self.no_child_left()

    @unittest.skipUnless(shutil.which("cc"), "no cc")
    def test_a_thread_made_as_the_leader_exits_does_not_hold_the_exit(self):
        # Sol's case: workers create threads while main exits. The SIGKILL
        # of exit_group can suppress a clone event, so a new thread's exit
        # stop comes from a tracee no event named: kept for its event it was
        # never resumed, and the leader's exit waited past any timeout. And
        # a clone event can name a task already gone (its message 0), which
        # failed reading /proc/0. Four makers hit both within a few runs
        c = ("#include <pthread.h>\n#include <unistd.h>\n"
             "static void *idle(void *_) { return 0; }\n"
             "static void *maker(void *_) { for (;;) { pthread_t t; if (!pthread_create(&t, 0, idle, 0))"
             " pthread_detach(t); } }\n"
             "int main(void) { for (int i = 0; i < 4; i++) { pthread_t m; pthread_create(&m, 0, maker, 0); }"
             " usleep(200); _exit(0); }\n")
        with tempfile.TemporaryDirectory() as d:
            src, exe = os.path.join(d, "maker.c"), os.path.join(d, "maker")
            with open(src, "w") as f:
                f.write(c)
            subprocess.run(["cc", "-O2", "-pthread", "-o", exe, src], check=True)
            # (each race is rare per run: 100 runs, 50 in each mode, make
            # a regression all but certain to show)
            for i in range(100):
                # with stops at system calls and without (Sol's r3: the
                # event's own thread killed before its message is read)
                syscalls = i % 2 == 0
                box, secs = self.bounded(lambda: su.memory([exe], dict(os.environ), syscalls, timeout=5), 30)
                self.assertNotIn("error", box, f"run {i} (syscalls={syscalls}): {box.get('error')!r}")
                self.assertEqual(box["value"][2], 0)
                self.no_child_left()

    def test_once_the_timer_fires_the_stops_kept_for_an_event_are_resumed(self):
        # a stop consumed before the timer fired and kept for an event that
        # never comes (`early`): the run's end resumes it. The kept stop is
        # scripted (a tracee id nothing uses; its resume meets ESRCH), the
        # rest of the run is real
        ghost = 4_000_000
        script = [(ghost, (signal.SIGSTOP << 8) | 0x7F)]
        real = os.waitpid
        resumed = []
        real_resume = su.resume

        def wait(pid, options):
            if pid < 0 and not options & os.WNOHANG and script:
                return script.pop(0)
            return real(pid, options)

        def spy(req, tid, sig=0):
            resumed.append(tid)
            return real_resume(req, tid, sig)
        with unittest.mock.patch.object(su.os, "waitpid", wait), unittest.mock.patch.object(su, "resume", spy):
            box, _ = self.bounded(lambda: su.memory([self.PY, "-c", "import time; time.sleep(60)"],
                                                    dict(os.environ), False, timeout=1), 30)
        self.assertIsInstance(box.get("error"), SystemExit, box)
        self.assertIn("ran over 1 s", str(box["error"]))
        self.assertIn(ghost, resumed)
        self.no_child_left()

    def test_a_gone_task_has_no_thread_group(self):
        # a clone event can name a task an exit_group already took (its
        # message 0, or a tid whose /proc entry is reaped): no group, so
        # memory() registers nothing for it (the race above hits it rarely)
        self.assertIsNone(su.tgid(0))
        self.assertIsNone(su.tgid(4_000_002))
        self.assertEqual(su.tgid(os.getpid()), os.getpid())

    def test_a_resume_ignores_esrch_only(self):
        # ESRCH on restarting a gathered stop: the tracee was SIGKILLed while
        # its stop waited in the round. Any other errno, and ESRCH from any
        # request that is not such a restart, still raises
        import errno
        for err, swallowed in ((errno.ESRCH, True), (errno.EPERM, False), (errno.EIO, False), (errno.EINVAL, False)):
            with unittest.mock.patch.object(su, "libc", SimpleNamespace(ptrace=lambda *a: -1)), \
                    unittest.mock.patch.object(su.ctypes, "get_errno", return_value=err):
                for req in (su.PTRACE_SYSCALL, su.PTRACE_CONT, su.PTRACE_DETACH):
                    if swallowed:
                        self.assertIsNone(su.resume(req, 12345))
                    else:
                        with self.assertRaises(OSError) as cm:
                            su.resume(req, 12345)
                        self.assertEqual(cm.exception.errno, err)
                with self.assertRaises(OSError) as cm:
                    su.ptrace(su.PTRACE_SETOPTIONS, 12345)
                self.assertEqual(cm.exception.errno, err)

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
