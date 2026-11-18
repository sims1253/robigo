#!/usr/bin/env python3
"""Check scripts/perf-gnur.py: the GNU R records perf's cache keeps, and
how a pull request completes them."""
import importlib.util
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("perf_gnur", HERE / "perf-gnur.py")
pg = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pg)

OK = {"instr": 10, "out": "1\n", "status": 0}
FAIL = {"instr": 1, "out": "", "status": 1}


class Gnur(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.mkdtemp()
        self.progs = ["a/x", "a/y", "a/z"]
        p = mock.patch.object(pg, "selected", lambda rx=None: [n for n in self.progs if not rx or rx in n])
        p.start()
        self.addCleanup(p.stop)

    def write(self, name, results):
        path = os.path.join(self.d, name)
        pg.save(path, results)
        return path

    def test_extract_keeps_every_gnur_record_failures_too(self):
        run = self.write("perf.json", {"a/x": {"robigo": {"instr": 5}, "gnur": OK}, "a/f": {"gnur": FAIL},
                                       "a/r": {"robigo": {"instr": 5}}})
        out = os.path.join(self.d, "gnur.json")
        pg.extract(run, out)
        self.assertEqual(pg.load(out), {"a/x": {"gnur": OK}, "a/f": {"gnur": FAIL}})

    def test_fill_keeps_the_cache_and_measures_only_what_is_missing(self):
        g = self.write("gnur.json", {"a/x": {"gnur": OK}, "a/y": {"gnur": FAIL}})
        asked = []

        def measure(names):
            asked.append(list(names))
            return {n: {"gnur": OK} for n in names}
        self.assertEqual(pg.fill(g, None, measure, known=set()), 0)
        self.assertEqual(asked, [["a/z"]])                      # a/y's cached failure is main's: kept
        self.assertEqual(pg.load(g)["a/y"], {"gnur": FAIL})

    def test_a_fresh_failure_is_measured_once_more(self):
        g = self.write("gnur.json", {})
        calls = []

        def measure(names):
            calls.append(list(names))
            if len(calls) == 1:
                return {n: {"gnur": FAIL if n == "a/y" else OK} for n in names}
            return {"a/y": {"gnur": OK}}                        # a timeout the first time
        self.assertEqual(pg.fill(g, None, measure, known=set()), 0)
        self.assertEqual(calls, [["a/x", "a/y", "a/z"], ["a/y"]])
        self.assertEqual(pg.load(g)["a/y"], {"gnur": OK})

    def test_a_failure_twice_fails_unless_it_is_a_known_one(self):
        # two failures on one runner do not show that main's record failed:
        # the answer would go unjudged
        fail_all = lambda names: {n: {"gnur": FAIL} for n in names}  # noqa: E731
        g = self.write("gnur.json", {})
        self.assertEqual(pg.fill(g, None, fail_all, known=set()), 1)
        g = self.write("gnur2.json", {})
        self.assertEqual(pg.fill(g, None, fail_all, known={"a/x", "a/y", "a/z"}), 0)
        self.assertEqual(pg.load(g)["a/x"], {"gnur": FAIL})
        # a timeout is no answer either
        g = self.write("gnur3.json", {})
        self.assertEqual(pg.fill(g, None, lambda names: {n: {"gnur": {"error": "timeout"}} for n in names},
                                 known=set()), 1)

    def test_the_known_failures_are_main_s(self):
        # the committed list names what GNU R fails on main today
        self.assertEqual(pg.known_failures(), {"rbench/real/flexclust", "rbench/real/flexclust_no_s4",
                                               "rbench/real/volcano"})

    def test_a_benchmark_without_any_record_fails(self):
        g = self.write("gnur.json", {"a/x": {"gnur": OK}})
        self.assertEqual(pg.fill(g, None, lambda names: {}), 1)  # GNU R gave nothing for a/y, a/z
        self.assertEqual(pg.fill(g, "a/x", lambda names: {}), 0)  # ...unless they are not selected


if __name__ == "__main__":
    unittest.main()
