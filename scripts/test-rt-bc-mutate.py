#!/usr/bin/env python3
"""Tests of scripts/rt-bc-mutate.py's own logic (#976), with fake children in
place of robigo-rt: how a start's end is judged (after SIGINT as on the
normal path), the address-space cap's wrapper, and the failures file that
`--failures-out` writes and the corpus reader reads back."""
import importlib.util
import os
import stat
import tempfile
import textwrap
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("rt_bc_mutate", os.path.join(HERE, "rt-bc-mutate.py"))
bcm = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bcm)


class RtBcMutate(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory(prefix="test-rt-bc-mutate-")
        self.addCleanup(self.dir.cleanup)

    def fake(self, body):
        """A fake robigo-rt: a Python script that runs `body` (its
        arguments are the harness's, ignored)."""
        p = os.path.join(self.dir.name, "fake-rt")
        with open(p, "w") as f:
            f.write("#!/usr/bin/env python3\nimport signal, sys, time\n" + textwrap.dedent(body))
        os.chmod(p, os.stat(p).st_mode | stat.S_IXUSR)
        return p

    def start(self, body, limit=1):
        return bcm.start(self.fake(body), HERE, self.dir.name, limit)

    def test_an_interrupt_ended_as_r_code_ends_passes(self):
        # R's error at the interrupt: "Execution halted", exit 1
        out = self.start("""
            def stop(*_):
                sys.stderr.write("mutation 3\\n\\nExecution halted\\n"); sys.exit(1)
            signal.signal(signal.SIGINT, stop)
            time.sleep(30)
        """)
        self.assertEqual(out, ("interrupted", ""))

    def test_a_panic_after_the_interrupt_fails(self):
        out = self.start("""
            def stop(*_):
                sys.stderr.write("mutation 7\\nthread 'main' panicked at crates/robigo-rt/src/vm.rs:1:1:\\n")
                sys.exit(1)  # (exit 1: the panic is told by stderr alone)
            signal.signal(signal.SIGINT, stop)
            time.sleep(30)
        """)
        self.assertEqual(out[0], "panic")
        self.assertTrue(out[1].startswith("mutation 7"), out)

    def test_a_signal_after_the_interrupt_fails(self):
        out = self.start("""
            import os
            def stop(*_):
                signal.signal(signal.SIGSEGV, signal.SIG_DFL); os.kill(os.getpid(), signal.SIGSEGV)
            signal.signal(signal.SIGINT, stop)
            time.sleep(30)
        """)
        self.assertEqual(out[0], f"signal {int(__import__('signal').SIGSEGV)}")

    def test_another_exit_status_after_the_interrupt_fails(self):
        out = self.start("""
            def stop(*_):
                sys.exit(3)
            signal.signal(signal.SIGINT, stop)
            time.sleep(30)
        """)
        self.assertEqual(out[0], "exit 3")

    def test_a_start_the_interrupt_does_not_end_fails(self):
        out = self.start("""
            signal.signal(signal.SIGINT, signal.SIG_IGN)
            time.sleep(30)
        """)
        self.assertEqual(out[0], "not interruptible")

    def test_the_normal_path_judges_as_before(self):
        self.assertEqual(self.start("print('done')"), ("done", ""))
        self.assertEqual(self.start("sys.exit(1)"), ("R error", ""))
        self.assertEqual(self.start("sys.stderr.write(\"thread 'main' panicked at x\\n\"); sys.exit(101)")[0], "panic")

    def test_the_cap_is_set_by_the_wrapper(self):
        import subprocess
        p = subprocess.run(bcm.capped(["/bin/sh", "-c", "ulimit -v"]), capture_output=True, text=True)
        self.assertEqual(p.stdout.strip(), str(bcm.MEM_CAP // 1024))
        # and no preexec_fn passed anywhere in the harness
        with open(os.path.join(HERE, "rt-bc-mutate.py")) as f:
            self.assertNotIn("preexec_fn=", f.read())

    def test_the_failures_file_replays_as_corpus_lines(self):
        bad = [
            ("random 700001 x100", "panic", "mutation 47: thread 'main' panicked at vm.rs"),
            ("random 700002 x100", "signal 11", "before any mutation: Segmentation fault"),
            ("random 700003 x100 pool", "not interruptible", "mutation 12"),
            ("corpus 100027 12 (panic)", "panic", "mutation 12: x"),
        ]
        lines = bcm.failure_lines(bad)
        self.assertEqual(lines, ["700001 47 panic", "700003 12 not-interruptible pool"])
        path = os.path.join(self.dir.name, "corpus.txt")
        with open(path, "w") as f:
            f.write("# a comment\n" + "\n".join(lines) + "\n")
        entries = bcm.corpus(path)
        self.assertEqual(entries, [(700001, 47, "panic", False), (700003, 12, "not-interruptible", True)])
        # and each replays as the start that failed: its seed, mutations 1..k,
        # BCM_POOL as it ran
        self.assertEqual([bcm.gen_env(s, k, p) for s, k, _, p in entries],
                         [{"BCM_SEED": "700001", "BCM_BATCH": "47", "BCM_POOL": "0"},
                          {"BCM_SEED": "700003", "BCM_BATCH": "12", "BCM_POOL": "1"}])

    def test_the_corpus_reads(self):
        # every line of the checked-in corpus parses, the old lines without pool
        entries = bcm.corpus()
        self.assertGreaterEqual(len(entries), 60)
        self.assertIn((700001, 47, "panic", False), entries)

if __name__ == "__main__":
    unittest.main()
