#!/usr/bin/env python3
"""Tests of scripts/wall13.py's verdicts and report (no runtime runs)."""
import contextlib
import importlib.util
import io
import os
import tempfile
import types
import unittest
import unittest.mock

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("wall13", os.path.join(HERE, "wall13.py"))
w = importlib.util.module_from_spec(spec)
spec.loader.exec_module(w)


def doc(load):
    rec = {"median": {"robigo-rt": 0.5, "gnur": 1.0}, "samples": {}, "batch": {},
           "robigo_rt_vs": {"gnur": {"ratio": 0.5, "verdict": "faster"}}}
    return {"impls": ["robigo-rt", "gnur"], "rounds": 11,
            "machine": {"cpu": "X", "cpus": 8, "host": "h"},
            "robigo_rt": {"checkout": "c", "sha256": "0" * 64},
            "programs": {"p/a": rec, "p/b": {"error": "not GNU R's answer: robigo-rt"}},
            "load": {"start": (load, 0, 0), "end": (load, 0, 0)}}


class Wall13(unittest.TestCase):
    def test_a_tie_is_within_two_percent_and_never_faster(self):
        self.assertEqual(w.verdict(0.981), "tie")
        self.assertEqual(w.verdict(1.019), "tie")
        self.assertEqual(w.verdict(0.979), "faster")
        self.assertEqual(w.verdict(1.021), "slower")

    def test_the_report_counts_verdicts_and_names_errors(self):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            w.report(doc(0.5))
        text = out.getvalue()
        self.assertIn("robigo-rt vs gnur: faster 1, tie 0, slower 0 (of 1)", text)
        self.assertIn("not GNU R's answer: robigo-rt", text)
        self.assertNotIn("WARNING", text)

    def test_a_busy_machine_is_called_out(self):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            w.report(doc(3.0))
        self.assertIn("WARNING: the machine was busy", out.getvalue())

    def test_each_sample_is_a_batch_of_at_least_50_ms(self):
        # the first run (the answer) takes 100 ms, every later one 1 ms: a
        # sample is then 50 processes, never one
        times = {"robigo-rt": [0.1], "gnur": [0.1]}

        def run(impl):
            t = times[impl].pop(0) if times[impl] else 0.001
            return t, "same answer\n", 0

        class Runner:
            def run(self, impl, file, pk, timeout, robigo):
                return run(impl)

        a = types.SimpleNamespace(rounds=10, timeout=1.0, robigo_rt="x")
        with tempfile.TemporaryDirectory() as d, \
             unittest.mock.patch.object(w, "run_rt", lambda *x: run("robigo-rt")):
            rec = w.measure(("p/a", "1", "", "utils"), ["robigo-rt", "gnur"], Runner(), a, d, io.StringIO())
        self.assertEqual(rec["runs"]["gnur"], [50] * 10)
        self.assertAlmostEqual(rec["median"]["robigo-rt"], 0.001)

    def test_gnur_failing_on_base_alone_is_reported_not_timed(self):
        class Runner:
            def run(self, impl, file, pk, timeout, robigo):
                self.pk = pk
                return 0.1, "Error: could not find function median", 1

        a = types.SimpleNamespace(rounds=10, timeout=1.0, robigo_rt="x")
        r = Runner()
        with tempfile.TemporaryDirectory() as d, \
             unittest.mock.patch.object(w, "run_rt", lambda *x: (0.1, "ok\n", 0)):
            rec = w.measure(("p/a", "1", "", "utils,stats"), ["robigo-rt", "gnur"], r, a, d, io.StringIO())
        self.assertEqual(r.pk, "NULL")
        self.assertIn("needs packages", rec["error"])

    def test_fewer_than_ten_rounds_are_refused(self):
        with self.assertRaises(SystemExit):
            w.main(["--robigo-rt", "x", "--out", "y", "--rounds", "9"])


if __name__ == "__main__":
    unittest.main()
