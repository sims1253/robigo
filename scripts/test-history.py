#!/usr/bin/env python3
"""Check scripts/history.py's `check`: slow perf creep on main."""
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from datetime import datetime, timedelta
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("history", HERE / "history.py")
h = importlib.util.module_from_spec(spec)
spec.loader.exec_module(h)

START = datetime(2026, 11, 1, 12)
BASE = {"a": 200_000_000, "b": 500_000_000, "c": 1_000_000_000, "d": 300_000_000}


def run(k, counts, hours=4, gnur=None, extra=None):
    """The k-th main run, `hours` apart: robigo counts, GNU R a fixed 3x."""
    perf = {n: {"robigo": int(v), "gnur": int((gnur or {}).get(n, 3 * BASE.get(n, v)))} for n, v in counts.items()}
    for n, x in (extra or {}).items():
        perf[n].update(x)
    return {"sha": f"{k:040x}", "date": (START + timedelta(hours=hours * k)).isoformat(), "title": f"#{k}: change {k}",
            "perf": perf}


def history(n, scale=lambda k, name: 1.0, **kw):
    return [run(k, {name: v * scale(k, name) for name, v in BASE.items()}, **kw) for k in range(n)]


def check(runs, i=None):
    i = len(runs) - 1 if i is None else i
    res, _, _ = h.check_at(runs, i)
    return h.regressions(res), res


class Band(unittest.TestCase):
    def test_floor_when_history_is_flat(self):
        self.assertEqual(h.noise_band([100, 100, 100, 100], 1.0), 1.0)

    def test_three_mad_when_history_is_noisy(self):
        # median 100, MAD 2 -> 3 * 1.4826 * 2 = 8.9%
        self.assertAlmostEqual(h.noise_band([96, 98, 100, 102, 104], 1.0), 300 * 1.4826 * 2 / 100)

    def test_doubled_below_three_runs(self):
        self.assertEqual(h.noise_band([100, 100], 1.0), 2.0)


class Check(unittest.TestCase):
    def test_flat_history_regresses_nothing(self):
        found, res = check(history(50))
        self.assertEqual(found, set())
        self.assertEqual(res["median"]["n"], len(BASE))
        self.assertIn("anchor", res)

    def test_step_regression_against_the_median(self):
        runs = history(50, lambda k, n: 1.02 if n == "b" and k == 49 else 1.0)
        found, res = check(runs)
        self.assertIn(("median", "b"), found)
        self.assertIn(("anchor", "b"), found)
        self.assertNotIn(("median", "a"), found)
        row = next(r for r in res["median"]["rows"] if r[0] == "b")
        self.assertAlmostEqual(row[3], 2.0, places=3)

    def test_step_below_the_band_is_within(self):
        found, _ = check(history(40, lambda k, n: 1.008 if n == "b" and k == 39 else 1.0))
        self.assertEqual(found, set())

    def test_creep_below_every_gate_is_caught_by_the_anchor(self):
        # 0.3% (under the pull-request gate's 0.5%) every tenth run, six a
        # day: never past the window median's band, past 1% within the week.
        runs = history(60, lambda k, n: 1.003 ** (k // 10) if n == "c" else 1.0)
        found, res = check(runs)
        self.assertNotIn(("median", "c"), found)
        self.assertIn(("anchor", "c"), found)
        # and the median alone would never have said so
        for i in range(8, 60):
            self.assertNotIn(("median", "c"), check(runs, i)[0])

    def test_geomean_creep(self):
        # every benchmark 0.7% slower than the anchor: none past its own 1%,
        # the geometric mean past its 0.5%
        runs = history(60, lambda k, n: 1.007 if k >= 50 else 1.0)
        found, res = check(runs)
        self.assertEqual(found, {("anchor", "geomean")})
        self.assertAlmostEqual(res["anchor"]["geomean"][0], 0.7, places=3)

    def test_improvement_is_not_a_regression(self):
        found, res = check(history(40, lambda k, n: 0.9 if k == 39 else 1.0))
        self.assertEqual(found, set())
        self.assertEqual(sum(r[5] == "improved" for r in res["median"]["rows"]), len(BASE))

    def test_noisy_benchmark_gets_a_wider_band(self):
        # "d" moves by 1% from run to run: a 2% step stays within its band
        noise = lambda k, n: (0.99, 1.0, 1.01)[k % 3] if n == "d" else 1.0
        runs = history(40, lambda k, n: 1.02 if n == "d" and k == 39 else noise(k, n))
        found, res = check(runs)
        self.assertNotIn(("median", "d"), found)
        band = next(r for r in res["median"]["rows"] if r[0] == "d")[4]
        self.assertGreater(band, 2.0)

    def test_changed_program_is_not_compared(self):
        # the benchmark's input appeared: GNU R did 10x the work as well
        runs = history(40)
        runs[-1]["perf"]["a"] = {"robigo": BASE["a"] * 12, "gnur": BASE["a"] * 30}
        found, res = check(runs)
        self.assertEqual(found, set())
        self.assertIn("a", res["median"]["changed"])

    def test_wrong_or_failed_benchmarks_are_not_compared(self):
        runs = history(40)
        runs[-1]["perf"]["a"].update(robigo=BASE["a"] * 2, wrong=True)
        runs[-1]["perf"]["b"].update(robigo=BASE["b"] * 2, error=True)
        found, res = check(runs)
        self.assertEqual(found, set())
        self.assertEqual(res["median"]["n"], len(BASE) - 2)

    def test_new_benchmark_has_nothing_to_compare(self):
        runs = history(40)
        runs[-1]["perf"]["new"] = {"robigo": 10 ** 9, "gnur": 3 * 10 ** 9}
        found, res = check(runs)
        self.assertEqual(found, set())
        self.assertEqual(res["median"]["n"], len(BASE))

    def test_no_anchor_in_a_young_history(self):
        found, res = check(history(10))  # 36 hours
        self.assertNotIn("anchor", res)

    def test_anchor_is_the_newest_run_a_week_older(self):
        runs = history(60)
        a = h.anchor_of(runs, 59)
        self.assertEqual(a["sha"], runs[59 - 42]["sha"])  # 42 runs of 4 hours

    def test_largest_step(self):
        runs = history(60, lambda k, n: (1.004 if k >= 30 else 1.0) * (1.006 if k >= 45 else 1.0) if n == "c"
                       else 1.0)
        step, pct = h.largest_step(runs, 17, 59, "c")
        self.assertEqual(step["sha"], runs[45]["sha"])
        self.assertAlmostEqual(pct, 0.6, places=3)


class Command(unittest.TestCase):
    """The command as CI runs it: a report file only when there is news."""

    def run_check(self, runs, commit=None):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "history.jsonl")
            h.save(path, runs)
            report = os.path.join(d, "creep.md")
            with open(report, "w") as f:
                f.write("stale")  # from an earlier run: must not survive one without news
            args = [sys.executable, str(HERE / "history.py"), "check", path, "--report", report]
            if commit:
                args += ["--commit", commit]
            p = subprocess.run(args, capture_output=True, text=True)
            self.assertEqual(p.returncode, 0, p.stderr)
            body = open(report).read() if os.path.exists(report) else None
            return p.stdout, body

    def test_new_regression_writes_the_report(self):
        runs = history(50, lambda k, n: 1.02 if n == "b" and k == 49 else 1.0)
        out, body = self.run_check(runs)
        self.assertIn("| b | median |", body)
        self.assertIn("**new**", body)
        self.assertIn("#49: change 49", body)  # the largest step names the pull request
        self.assertEqual(out.strip(), body.strip())

    def test_known_regression_writes_no_report(self):
        # the step landed one run earlier: this run shows it, but it is not news
        runs = history(50, lambda k, n: 1.02 if n == "b" and k >= 48 else 1.0)
        out, body = self.run_check(runs)
        self.assertIn("| b | median |", out)
        self.assertIsNone(body)

    def test_clean_run_writes_no_report(self):
        out, body = self.run_check(history(50))
        self.assertIn("Nothing regressed.", out)
        self.assertIsNone(body)

    def test_older_commit_is_checked_against_the_runs_before_it(self):
        runs = history(50, lambda k, n: 1.02 if n == "b" and k == 30 else 1.0)
        out, body = self.run_check(runs, commit=runs[30]["sha"])
        self.assertIn("**new**", body)

    def test_commit_without_counts_is_not_checked(self):
        runs = history(20)
        runs.append({"sha": "f" * 40, "date": (START + timedelta(days=30)).isoformat(), "title": "x",
                     "corpora": {"fastr": [1, 2]}})
        out, body = self.run_check(runs, commit="f" * 40)
        self.assertIn("Not checked", out)
        self.assertIsNone(body)

    def test_real_history_shape(self):
        # a line as `add` writes it, with the other keys check ignores
        runs = history(30)
        for e in runs:
            e.update(corpora={"fastr": [10, 12]}, builtins=[600, 700], run=1)
        json.dumps(runs)
        out, body = self.run_check(runs)
        self.assertIn("Nothing regressed.", out)


class Entry(unittest.TestCase):
    def test_other_r_version_results_stay_out_of_the_history(self):
        # docs/verification.md "Oracles": a run against GNU R 4.6.2 only
        # reports, so its pass counts must not fold into the gated ones
        with tempfile.TemporaryDirectory() as d:
            for name, extra, passed in (("conformance.json", {}, 600), ("conformance-462.json",
                                        {"r_version": "4.6.2"}, 570)):
                summary = dict({"corpus": "conformance", "cases": 628, "pass": passed}, **extra)
                Path(d, name).write_text(json.dumps({"summary": summary, "results": []}))
            e = h.entry_from(d, "abc", "2026-11-13", "t")
        self.assertEqual(e["corpora"], {"conformance": [600, 628]})


if __name__ == "__main__":
    unittest.main()
