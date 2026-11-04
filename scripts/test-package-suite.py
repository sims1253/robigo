#!/usr/bin/env python3
"""scripts/package-suite.py's scoring and its history record, without R.

The rules that decide what counts as compatible live here: a block nobody
ran must not pass, a skip must not improve the score, and a partial run must
not look like a smaller denominator.
"""
import importlib.util
import json
import os
import sys
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, file))
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


suite = load("package_suite", "package-suite.py")
history = load("history", "history.py")


def block(test="t", n=1, passed=1, fail=0, error=0, skip=0, warn=0, reasons=(), msg=None, open_=False):
    return {"test": test, "n": n, "pass": passed, "fail": fail, "error": error, "skip": skip, "warn": warn,
            "reasons": list(reasons), "msg": msg, "open": open_}


def recorded(file="test-a", unstable=False, **kw):
    b = block(**kw)
    return {suite.block_id(file, b): {"id": suite.block_id(file, b), "file": file, "test": b["test"], "n": b["n"],
                                      "outcome": suite.outcome(b), "unstable": unstable}}


def state(exp, got):
    rows, _ = suite.compare(exp, {}, got)
    return next(iter(rows.values()))


class Compare(unittest.TestCase):
    def test_same_outcome_matches(self):
        r = state(recorded(), {"test-a": ("done", 1, [block()], None)})
        self.assertEqual(r["state"], "match")
        self.assertTrue(r["scored"])

    def test_extra_warning_differs(self):
        self.assertEqual(state(recorded(), {"test-a": ("done", 1, [block(warn=1)], None)})["state"], "differ")

    def test_fewer_expectations_differ(self):
        self.assertEqual(state(recorded(passed=3), {"test-a": ("done", 1, [block(passed=2)], None)})["state"], "differ")

    def test_robigo_skip_of_a_block_gnu_ran_differs(self):
        r = state(recorded(), {"test-a": ("done", 1, [block(passed=0, skip=1, reasons=["x"])], None)})
        self.assertEqual(r["state"], "differ")
        self.assertTrue(r["scored"])

    def test_gnu_failure_is_not_scored(self):
        self.assertFalse(state(recorded(fail=1), {"test-a": ("done", 1, [block(fail=1)], None)})["scored"])

    def test_skipped_by_both_is_not_scored(self):
        b = {"passed": 0, "skip": 1, "reasons": ["x"]}
        self.assertFalse(state(recorded(**b), {"test-a": ("done", 1, [block(**b)], None)})["scored"])

    def test_unstable_is_not_scored(self):
        self.assertFalse(state(recorded(unstable=True), {"test-a": ("done", 1, [block()], None)})["scored"])

    def test_missing_block_is_not_run(self):
        r = state(recorded(), {"test-a": ("timeout", 60, [], None)})
        self.assertEqual(r["state"], "notrun")
        self.assertTrue(r["scored"])

    def test_interrupted_block_does_not_match(self):
        r = state(recorded(), {"test-a": ("crash", 1, [block(open_=True)], None)})
        self.assertEqual(r["state"], "interrupted")

    def test_file_never_started(self):
        self.assertEqual(state(recorded(), {})["state"], "notrun")


class Green(unittest.TestCase):
    ROW = {"version": "1.0"}

    def summary(self, exp, got):
        files = {"test-a": {"file": "test-a", "seconds": 1, "status": "done", "stable": True}}
        return suite.summarize("p", self.ROW, exp, files, got)[0]

    def test_all_matching_is_green(self):
        self.assertTrue(self.summary(recorded(), {"test-a": ("done", 1, [block()], None)})["green"])

    def test_a_crashed_file_is_not_green(self):
        self.assertFalse(self.summary(recorded(), {"test-a": ("crash", 1, [block()], None)})["green"])

    def test_nothing_scored_is_not_green(self):
        exp = recorded(passed=0, skip=1, reasons=["x"])
        self.assertFalse(self.summary(exp, {"test-a": ("done", 1, [block(passed=0, skip=1, reasons=["x"])], None)})["green"])

    def test_diverging_from_a_skip_is_not_green(self):
        exp = {**recorded(), **recorded(test="s", passed=0, skip=1, reasons=["x"])}
        got = {"test-a": ("done", 1, [block(), block(test="s", passed=0, error=1, msg="boom")], None)}
        s = self.summary(exp, got)
        self.assertEqual(s["skip_diverged"], 1)
        self.assertFalse(s["green"])

    def test_an_extra_block_is_not_green(self):
        got = {"test-a": ("done", 1, [block(), block(test="(outside any test)", passed=0, error=1, msg="x")], None)}
        self.assertFalse(self.summary(recorded(), got)["green"])

    def test_mostly_skipped_is_not_green(self):
        exp = recorded()
        for i in range(9):
            exp.update(recorded(test=f"s{i}", passed=0, skip=1, reasons=["x"]))
        got = {"test-a": ("done", 1, [block()] + [block(test=f"s{i}", passed=0, skip=1, reasons=["x"]) for i in range(9)],
                          None)}
        self.assertFalse(self.summary(exp, got)["green"])


class Events(unittest.TestCase):
    def parse(self, lines):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "events")
            with open(p, "w") as f:
                f.write("\n".join(lines))
            return suite.parse_events(p)

    def test_stream_cut_mid_line(self):
        blocks, done, _ = self.parse(['{"e":"begin"}', '{"e":"start","test":"a"}', '{"e":"result","kind":"success"}',
                                      '{"e":"end"}', '{"e":"start","test":"b"}', '{"e":"res'])
        self.assertEqual([(b["test"], b["open"]) for b in blocks], [("a", False), ("b", True)])
        self.assertFalse(done)

    def test_a_repeated_name_gets_its_own_block(self):
        ev = ['{"e":"start","test":"a"}', '{"e":"end"}'] * 2 + ['{"e":"done"}']
        blocks, done, _ = self.parse(ev)
        self.assertEqual([b["n"] for b in blocks], [1, 2])
        self.assertNotEqual(suite.block_id("f", blocks[0]), suite.block_id("f", blocks[1]))
        self.assertTrue(done)

    def test_nested_blocks(self):
        blocks, _, _ = self.parse(['{"e":"start","test":"outer"}', '{"e":"start","test":"inner"}',
                                   '{"e":"result","kind":"success"}', '{"e":"end"}',
                                   '{"e":"result","kind":"failure","msg":"x"}', '{"e":"end"}', '{"e":"done"}'])
        self.assertEqual([(b["test"], b["pass"], b["fail"], b["open"]) for b in blocks],
                         [("outer", 0, 1, False), ("inner", 1, 0, False)])

    def test_a_result_outside_any_test(self):
        blocks, _, _ = self.parse(['{"e":"result","kind":"error","msg":"top"}'])
        self.assertEqual((blocks[0]["test"], blocks[0]["error"], blocks[0]["open"]), ("(outside any test)", 1, False))


class Canary(unittest.TestCase):
    def good(self):
        return [block(test=t, passed=o[0], fail=o[1], error=o[2], skip=o[3], warn=o[4]) for t, o in suite.CANARY.items()]

    def test_all_expected(self):
        n = len(suite.CANARY)
        self.assertEqual(suite.canary_score("done", self.good()), [n, n])

    def test_a_crashed_file_fails(self):
        self.assertEqual(suite.canary_score("crash", self.good())[0], 0)

    def test_an_open_block_fails(self):
        blocks = self.good()
        blocks[0]["open"] = True
        self.assertLess(suite.canary_score("done", blocks)[0], len(suite.CANARY))

    def test_an_unexpected_block_fails(self):
        self.assertEqual(suite.canary_score("done", self.good() + [block(test="other")])[0], 0)


class Supervise(unittest.TestCase):
    def test_timeout_kills_the_group(self):
        with tempfile.TemporaryDirectory() as d:
            start = time.time()
            status, _, _ = suite.supervise(["sh", "-c", "sleep 30 & sleep 30"], dict(os.environ), d,
                                           os.path.join(d, "log"), 1)
        self.assertEqual(status, "timeout")
        self.assertLess(time.time() - start, 10)

    def test_memory_cap(self):
        cap, suite.MEMORY_CAP = suite.MEMORY_CAP, 1 << 20
        try:
            with tempfile.TemporaryDirectory() as d:
                code = "b = bytearray(64 << 20)\nimport time\ntime.sleep(30)"
                status, _, _ = suite.supervise([sys.executable, "-c", code], dict(os.environ), d,
                                               os.path.join(d, "log"), 20)
            self.assertEqual(status, "oom")
        finally:
            suite.MEMORY_CAP = cap


class History(unittest.TestCase):
    def doc(self, shard, m=1, n=2, valid=True, pkg="p"):
        return {"kind": "packages", "snapshot": "2026-11-01", "lock": "l", "selection": 2, "valid": valid,
                "canary": [9, 9], "shard": shard, "regressions": [], "causes": [["why", n - m, [pkg]]],
                "pkgs": {pkg: {"m": m, "n": n, "green": m == n, "v": "1", "s": 1, "gnu_s": 1, "files": {"done": 1}}}}

    def test_shards_add_up(self):
        p = history.packages_from([self.doc([1, 2]), self.doc([2, 2], 3, 4, pkg="q")], None)
        self.assertTrue(p["complete"])
        self.assertEqual(p["score"], [4, 6])
        self.assertEqual(p["causes"][0][:2], ["why", 2])

    def test_a_missing_shard_is_incomplete(self):
        self.assertFalse(history.packages_from([self.doc([1, 2])], None)["complete"])

    def entry(self, docs):
        with tempfile.TemporaryDirectory() as d:
            for i, doc in enumerate(docs):
                with open(os.path.join(d, f"packages.shard{i + 1}.json"), "w") as f:
                    json.dump(doc, f)
            return history.entry_from(d, "a" * 40, "2026-11-03", "t")

    def test_only_a_complete_valid_run_sets_a_score(self):
        self.assertEqual(self.entry([self.doc([1, 1])])["corpora"]["packages"], [1, 2])
        self.assertNotIn("corpora", self.entry([self.doc([1, 1], valid=False)]))
        self.assertNotIn("corpora", self.entry([self.doc([1, 2])]))

    def test_a_package_matching_nothing_renders(self):
        e = self.entry([self.doc([1, 1], m=0, n=5)])
        self.assertIn("Top CRAN packages", history.packages_section([e, e], "https://x"))

    def test_merge_keeps_what_the_other_run_measured(self):
        night = {"sha": "x", "packages": {}, "corpora": {"packages": [1, 2]}}
        day = {"sha": "x", "perf": {}, "corpora": {"fastr": [3, 4]}}
        merged = history.merge_entry(night, day)
        self.assertEqual(merged["corpora"], {"packages": [1, 2], "fastr": [3, 4]})
        self.assertIn("perf", merged)
        self.assertIn("packages", merged)


if __name__ == "__main__":
    unittest.main()
