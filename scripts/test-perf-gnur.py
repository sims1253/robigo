#!/usr/bin/env python3
"""Check scripts/perf-gnur.py: the GNU R records perf's cache keeps, and
how a pull request completes them."""
import importlib.util
import json
import os
import sys
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

    def test_extract_caches_successes_and_known_failures_only(self):
        # a failure that is not a known one (a timeout on one main run) must
        # not stay in the immutable cache: pull requests measure it again
        run = self.write("perf.json", {"a/x": {"robigo": {"instr": 5}, "gnur": OK}, "a/f": {"gnur": FAIL},
                                       "a/t": {"gnur": {"error": "timeout"}}, "a/r": {"robigo": {"instr": 5}}})
        out = os.path.join(self.d, "gnur.json")
        pg.extract(run, out, known={"a/f"})
        self.assertEqual(pg.load(out), {"a/x": {"gnur": OK}, "a/f": {"gnur": FAIL}})
        # ...and a PR then measures a/t itself, failing closed if GNU R fails twice
        self.progs = ["a/x", "a/f", "a/t"]
        self.assertEqual(pg.fill(out, None, lambda names: {n: {"gnur": {"error": "timeout"}} for n in names},
                                 known={"a/f"}), 1)

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


class Inputs(unittest.TestCase):
    """The perf job's GNU R records are cached by a hash of the files they
    depend on (perf.py gnur-inputs), derived from the suites, not listed by
    hand."""

    def setUp(self):
        spec_ = importlib.util.spec_from_file_location("perf", HERE / "perf.py")
        self.perf = importlib.util.module_from_spec(spec_)
        spec_.loader.exec_module(self.perf)

    def test_every_program_the_job_runs_is_in_the_key_and_rt_rows_are_not(self):
        perf = self.perf
        inputs = set(perf.gnur_inputs())
        c = perf.conf()
        for sname, s in c["suites"].items():
            if "dir" in s:
                for path in perf.suite_files(sname, s):
                    self.assertIn(os.path.relpath(path, perf.ROOT), inputs)
        # (the known failures are optional: test_a_program_added_later_is_in_the_key)
        for f in ("benchmarks/benchmarks.toml", "scripts/perf.py", "scripts/perf-gnur.py"):
            self.assertIn(f, inputs)
        self.assertFalse([p for p in inputs if p.startswith("benchmarks/rt/")], "robigo-rt's rows are not this job's")

    def test_a_program_added_later_is_in_the_key(self):
        perf = self.perf
        d = tempfile.mkdtemp()
        self.addCleanup(__import__("shutil").rmtree, d, ignore_errors=True)
        os.makedirs(os.path.join(d, "benchmarks", "mine"))
        for f in ("a.R", "b.R", "notes.txt"):
            open(os.path.join(d, "benchmarks", "mine", f), "w").close()
        c = {"suites": {"mine": {"dir": "benchmarks/mine"}, "ext": {"repo": "x", "commit": "y", "benchmarks": []}}}
        with mock.patch.object(sys.modules["perf_suite"], "ROOT", d):
            got = perf.gnur_inputs(c)
        self.assertIn("benchmarks/mine/a.R", got)
        self.assertIn("benchmarks/mine/b.R", got)
        self.assertNotIn("benchmarks/mine/notes.txt", got)
        self.assertEqual(got, sorted(got))
        # the known failures are optional: absent, not listed (the key's loop
        # fails on a listed file it cannot read)
        self.assertNotIn("benchmarks/gnur-known-failures.txt", got)
        open(os.path.join(d, "benchmarks", "gnur-known-failures.txt"), "w").close()
        with mock.patch.object(sys.modules["perf_suite"], "ROOT", d):
            self.assertIn("benchmarks/gnur-known-failures.txt", perf.gnur_inputs(c))

    def test_the_job_keys_its_cache_by_those_files(self):
        import re
        yml = (HERE.parent / ".github" / "workflows" / "ci.yml").read_text()
        job = re.search(r"\n  perf:\n(.*?)\n  [a-z0-9-]+:\n", yml, re.S).group(1)
        self.assertIn("python3 scripts/perf.py gnur-inputs", job)
        self.assertIn("perf-gnur-2-", job)
        self.assertNotIn("hashFiles('benchmarks/**'", job)
        # the restore and the save use the one key, which holds the inputs' hash
        keys = re.findall(r"key: (\$\{\{[^}]*\}\})", job)
        self.assertEqual([k for k in keys if "gnur" in k], ["${{ steps.gnur.outputs.key }}"] * 2)


if __name__ == "__main__":
    unittest.main()
