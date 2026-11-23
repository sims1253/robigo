#!/usr/bin/env python3
"""Check scripts/perf-gnur.py: the GNU R records perf's cache keeps, and
how a pull request completes them."""
import importlib.util
import json
import contextlib
import io
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
CODE = "c1"


def digest(n):
    """The fake program digest of benchmark `n` (Gnur's setUp)."""
    return "p-" + n


def rec(g, n):
    """A record of benchmark `n` as fill and extract keep it: GNU R's
    result, stamped with its program's and the code's digest."""
    return {"gnur": g, "program": digest(n), "code": CODE}


class Gnur(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.mkdtemp()
        self.progs = ["a/x", "a/y", "a/z"]
        p = mock.patch.object(pg, "selected", lambda rx=None: [n for n in self.progs if not rx or rx in n])
        p.start()
        self.addCleanup(p.stop)
        # every benchmark's program digest, and the code's
        for name, fn in (("program_digests", lambda: {n: digest(n) for n in "a/x a/y a/z a/f a/t a/r".split()}),
                         ("gnur_code_digest", lambda: CODE)):
            q = mock.patch.object(pg.perf, name, fn)
            q.start()
            self.addCleanup(q.stop)

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
        self.assertEqual(pg.load(out), {"a/x": rec(OK, "a/x"), "a/f": rec(FAIL, "a/f")})
        # ...and a PR then measures a/t itself, failing closed if GNU R fails twice
        self.progs = ["a/x", "a/f", "a/t"]
        self.assertEqual(pg.fill(out, None, lambda names: {n: {"gnur": {"error": "timeout"}} for n in names},
                                 known={"a/f"}), 1)

    def test_fill_keeps_the_cache_and_measures_only_what_is_missing(self):
        g = self.write("gnur.json", {"a/x": rec(OK, "a/x"), "a/y": rec(FAIL, "a/y")})
        asked = []

        def measure(names):
            asked.append(list(names))
            return {n: {"gnur": OK} for n in names}
        self.assertEqual(pg.fill(g, None, measure, known={"a/y"}), 0)
        self.assertEqual(asked, [["a/z"]])                      # a/y's cached failure is main's, a known one: kept
        self.assertEqual(pg.load(g)["a/y"], rec(FAIL, "a/y"))

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
        self.assertEqual(pg.load(g)["a/y"], rec(OK, "a/y"))

    def test_a_failure_twice_fails_unless_it_is_a_known_one(self):
        # two failures on one runner do not show that main's record failed:
        # the answer would go unjudged
        fail_all = lambda names: {n: {"gnur": FAIL} for n in names}  # noqa: E731
        g = self.write("gnur.json", {})
        self.assertEqual(pg.fill(g, None, fail_all, known=set()), 1)
        g = self.write("gnur2.json", {})
        self.assertEqual(pg.fill(g, None, fail_all, known={"a/x", "a/y", "a/z"}), 0)
        self.assertEqual(pg.load(g)["a/x"], rec(FAIL, "a/x"))
        # a timeout is no answer either
        g = self.write("gnur3.json", {})
        self.assertEqual(pg.fill(g, None, lambda names: {n: {"gnur": {"error": "timeout"}} for n in names},
                                 known=set()), 1)

    def test_the_known_failures_are_main_s(self):
        # the committed list names what GNU R fails on main today
        self.assertEqual(pg.known_failures(), {"rbench/real/flexclust", "rbench/real/flexclust_no_s4",
                                               "rbench/real/volcano"})

    def test_a_benchmark_without_any_record_fails(self):
        g = self.write("gnur.json", {"a/x": rec(OK, "a/x")})
        self.assertEqual(pg.fill(g, None, lambda names: {}), 1)  # GNU R gave nothing for a/y, a/z
        self.assertEqual(pg.fill(g, "a/x", lambda names: {}), 0)  # ...unless they are not selected


class Digests(unittest.TestCase):
    """A record is reused only if its program's and the code's digest still
    match: a restored set (an older cache entry, restore-keys) can hold a
    record of the same name measured from other input."""

    def setUp(self):
        self.d = tempfile.mkdtemp()
        self.progs = ["a/x", "a/y", "a/z"]
        self.digests = {n: digest(n) for n in self.progs}
        p = mock.patch.object(pg, "selected", lambda rx=None: list(self.progs))
        p.start()
        self.addCleanup(p.stop)
        self.asked = []

    def measure(self, names):
        self.asked.append(sorted(names))
        return {n: {"gnur": OK} for n in names}

    def fill(self, have, digests=None, code=CODE):
        path = os.path.join(self.d, "gnur.json")
        pg.save(path, have)
        out = io.StringIO()
        with contextlib.redirect_stdout(out), mock.patch.dict(os.environ, {"GITHUB_STEP_SUMMARY": ""}):
            status = pg.fill(path, None, self.measure, known=set(), digests=digests or self.digests, code=code)
        return status, out.getvalue(), pg.load(path)

    def test_a_changed_program_is_measured_again_and_the_others_are_not(self):
        have = {n: rec(OK, n) for n in self.progs}
        changed = dict(self.digests, **{"a/y": "p-new"})
        status, text, got = self.fill(have, changed)
        self.assertEqual((status, self.asked), (0, [["a/y"]]))
        self.assertIn("1 of 3 selected benchmark(s) re-measured (changed program 1, changed measurement code 0, "
                      "failure no longer known 0, no record 0)", text)
        self.assertEqual(got["a/y"]["program"], "p-new")

    def test_changed_measuring_code_measures_all_again(self):
        status, text, got = self.fill({n: rec(OK, n) for n in self.progs}, code="c2")
        self.assertEqual(self.asked, [["a/x", "a/y", "a/z"]])
        self.assertIn("changed measurement code 3", text)
        self.assertTrue(all(r["code"] == "c2" for r in got.values()))

    def test_a_record_of_the_same_name_from_other_input_is_never_reused(self):
        # what makes restore-keys safe: an older set's record for a/x, of
        # an older program, is measured again; its name alone is not enough
        old = {"a/x": {"gnur": {"instr": 999, "out": "old\n", "status": 0}, "program": "p-old", "code": CODE},
               "a/y": rec(OK, "a/y"), "a/z": {"gnur": OK}}           # a/z: from before the digests
        status, text, got = self.fill(old)
        self.assertEqual(self.asked, [["a/x", "a/z"]])
        self.assertEqual(got["a/x"]["gnur"], OK)
        self.assertIn("changed program 1, changed measurement code 1, failure no longer known 0, no record 0", text)

    def test_a_failure_no_longer_known_is_measured_again(self):
        # a restored set (restore-keys) keeps a failure main recorded while
        # benchmarks/gnur-known-failures.txt named it; once the list drops
        # it, its answer must be judged: measured again, not reused
        have = {"a/x": rec(FAIL, "a/x"), "a/y": rec(OK, "a/y"), "a/z": rec(FAIL, "a/z")}
        path = os.path.join(self.d, "gnur.json")
        pg.save(path, have)
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            status = pg.fill(path, None, self.measure, known={"a/z"}, digests=self.digests, code=CODE)
        self.assertEqual((status, self.asked), (0, [["a/x"]]))
        self.assertIn("failure no longer known 1", out.getvalue())
        self.assertEqual(pg.load(path)["a/x"]["gnur"], OK)

    def test_the_measurement_s_environment_and_orchestration_are_in_the_code_digest(self):
        # run_one passes its caller's environment on: R_ENABLE_JIT=0 and 3
        # give one loop 3.6 and 1.2 billion instructions
        spec_ = importlib.util.spec_from_file_location("perf", HERE / "perf.py")
        perf = importlib.util.module_from_spec(spec_)
        spec_.loader.exec_module(perf)
        base = {"PATH": "/bin", "HOME": "/h", "GITHUB_RUN_ID": "1"}
        d = perf.gnur_code_digest(base)
        self.assertEqual(d, perf.gnur_code_digest(dict(base, GITHUB_RUN_ID="2", HOME="/other")))
        for k, v in (("R_ENABLE_JIT", "0"), ("OMP_NUM_THREADS", "1"), ("GLIBC_TUNABLES", "x"), ("LD_PRELOAD", "y")):
            self.assertNotEqual(d, perf.gnur_code_digest(dict(base, **{k: v})), k)
        # BASE_ENV wins over the caller's for what it sets
        self.assertEqual(d, perf.gnur_code_digest(dict(base, LC_ALL="de_DE.UTF-8")))
        # and cmd_run's source is part of it
        import inspect
        self.assertNotEqual(perf.code_digest([inspect.getsource(perf.cmd_run)], base), perf.code_digest([], base))

    def test_nothing_changed_nothing_measured_and_the_line_says_so(self):
        status, text, _ = self.fill({n: rec(OK, n) for n in self.progs})
        self.assertEqual((status, self.asked), (0, []))
        self.assertIn("0 of 3 selected benchmark(s) re-measured", text)

    def test_the_digests_do_not_depend_on_perf_py(self):
        # perf.py's robigo-rt side changes often and measures none of these
        spec_ = importlib.util.spec_from_file_location("perf", HERE / "perf.py")
        perf = importlib.util.module_from_spec(spec_)
        spec_.loader.exec_module(perf)
        self.assertNotIn("scripts/perf.py", perf.CODE_FILES)
        self.assertNotIn("scripts/perf.py", perf.gnur_inputs())
        self.assertIn("scripts/perf_suite.py", perf.CODE_FILES)
        # every program has a digest, and an external one changes with its commit
        c = perf.conf()
        d = perf.program_digests(c)
        self.assertEqual(sorted(d), sorted(p[0] for p in perf.programs(c)))
        ext = next(n for n, s in c["suites"].items() if "repo" in s)
        c2 = {"suites": {**c["suites"], ext: {**c["suites"][ext], "commit": "0" * 40}}}
        d2 = perf.program_digests(c2)
        self.assertTrue(all(d[n] != d2[n] for n in d if n.startswith(ext + "/")))
        self.assertTrue(all(d[n] == d2[n] for n in d if not n.startswith(ext + "/")))


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
        for f in ("benchmarks/benchmarks.toml", "scripts/perf_suite.py", "scripts/perf-gnur.py"):
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
        self.assertIn("perf-gnur-3-", job)
        # the fallback is the prefix without the inputs (the digests make it safe)
        self.assertIn("restore-keys: ${{ steps.gnur.outputs.prefix }}", job)
        self.assertIn('echo "key=$prefix$inputs"', job)
        self.assertNotIn("hashFiles('benchmarks/**'", job)
        # the restore and the save use the one key, which holds the inputs' hash
        keys = re.findall(r"key: (\$\{\{[^}]*\}\})", job)
        self.assertEqual([k for k in keys if "gnur" in k], ["${{ steps.gnur.outputs.key }}"] * 2)


class ReExports(unittest.TestCase):
    """perf.py re-exports what moved to perf_suite.py: the scripts that call
    `perf.<name>` keep working, and a later cleanup cannot drop one of
    those names silently."""

    def test_every_moved_name_resolves_on_perf_as_the_same_object(self):
        import re
        spec_ = importlib.util.spec_from_file_location("perf", HERE / "perf.py")
        perf = importlib.util.module_from_spec(spec_)
        spec_.loader.exec_module(perf)
        suite = sys.modules["perf_suite"]
        src = (HERE / "perf_suite.py").read_text()
        # (ROOT is each module's own: the same path, not a re-export)
        public = [n for n in re.findall(r"^(?:def ([a-z]\w*)|([A-Z][A-Z_]*) =)", src, re.M) for n in n
                  if n and n != "ROOT"]
        self.assertGreaterEqual(len(public), 15)
        for n in public:
            self.assertIs(getattr(perf, n, None), getattr(suite, n), n)

    def test_every_perf_call_in_the_scripts_resolves(self):
        import re
        spec_ = importlib.util.spec_from_file_location("perf", HERE / "perf.py")
        perf = importlib.util.module_from_spec(spec_)
        spec_.loader.exec_module(perf)
        missing = []
        for f in sorted(HERE.glob("*.py")):
            text = f.read_text(errors="replace")
            # (the module itself: a local variable of that name is not it)
            if not re.search(r"^\s*import perf\b|spec_from_file_location\(\"perf\"", text, re.M):
                continue
            for n in set(re.findall(r"\bperf\.([A-Za-z_]\w*)\(", text)):
                if not hasattr(perf, n):
                    missing.append(f"{f.name}: perf.{n}")
        self.assertEqual(missing, [])


if __name__ == "__main__":
    unittest.main()
