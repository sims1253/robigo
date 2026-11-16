#!/usr/bin/env python3
"""The rt-bars gate's decisions (scripts/perf.py rt_verdict, cmd_rt_bars),
on synthetic numbers: no valgrind, no robigo-rt."""
import importlib.util
import io
import os
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from types import SimpleNamespace

spec = importlib.util.spec_from_file_location("perf", Path(__file__).with_name("perf.py"))
perf = importlib.util.module_from_spec(spec)
spec.loader.exec_module(perf)

SPEC = {
    "gnur": {"version": "4.6.2", "source": "test", "instr": {"plain": 1000.0, "s3": 1000.0},
             "rss": {"p.R": 50000}},
    "today": {"source": "test", "instr": {"fib": 1000.0, "paste": 1000.0}, "rss": {"p.R": 40000}},
    "bar": [
        {"row": "plain", "against": "gnur", "max": 0.6},
        {"row": "s3", "against": "gnur", "max": 1.0, "met": False},
        {"row": "fib", "against": "today", "max": 1.0, "met": False},
        {"row": "paste", "against": "today", "max": 1.0, "met": False, "accepted": "record.md", "ceiling": 2.0},
    ],
    "rss": [{"program": "p.R", "against": ["today", "gnur"]}],
}
OK = {"plain": 500.0, "s3": 1100.0, "fib": 1400.0, "paste": 1930.0}
RSS = {"p.R": 30000}


def verdict(instr=None, rss=None, main=None):
    return perf.rt_verdict(SPEC, instr or OK, rss or RSS, main)


class Bars(unittest.TestCase):
    def test_met_bars_hold_and_unmet_bars_only_report(self):
        failures, notes, table = verdict()
        self.assertEqual(failures, [])
        self.assertEqual(notes, [])
        status = {row: st for row, *_, st in table}
        self.assertEqual(status, {"plain": "ok", "s3": "not met yet", "fib": "not met yet",
                                  "paste": "accepted up to 2.0x"})

    def test_a_met_bar_that_breaks_fails(self):
        failures, _, _ = verdict(instr={**OK, "plain": 601.0})
        self.assertEqual(len(failures), 1)
        self.assertIn("`plain`: 0.601x gnur breaks its bar of 0.6x", failures[0])

    def test_an_unmet_bar_gets_worse_without_failing_and_says_when_it_is_met(self):
        failures, notes, _ = verdict(instr={**OK, "s3": 1500.0, "fib": 900.0})
        self.assertEqual(failures, [])
        self.assertEqual(len(notes), 1)
        self.assertIn("`fib`: 0.900x today meets its bar", notes[0])

    def test_an_accepted_regression_holds_up_to_its_ceiling(self):
        failures, _, _ = verdict(instr={**OK, "paste": 2000.0})  # 2.000x: at the ceiling
        self.assertEqual(failures, [])
        failures, _, table = verdict(instr={**OK, "paste": 2001.0})
        self.assertEqual(failures, ["`paste`: 2.001x today is above the ceiling of 2.0x that its acceptance "
                                    "holds to (record.md)"])
        self.assertEqual(dict((r, st) for r, *_, st in table)["paste"], "ABOVE CEILING")

    def test_an_accepted_regression_that_meets_its_bar_is_retired(self):
        failures, notes, _ = verdict(instr={**OK, "paste": 990.0})
        self.assertEqual(failures, [])
        self.assertEqual(notes, ["`paste`: 0.990x today meets its bar of 1.0x, so the accepted regression is no "
                                 "longer needed: remove `accepted` and `ceiling` and set it to met in "
                                 "benchmarks/rt/m1-bars.toml"])

    def test_rss_above_any_reference_fails(self):
        failures, _, _ = verdict(rss={"p.R": 41000})
        self.assertEqual(failures, ["`p.R`: peak RSS 41,000 KiB, above today's 40,000"])


class AgainstMain(unittest.TestCase):
    # main's binary, measured on the same machine
    MAIN = {"instr": {**OK}, "rss": {"p.R": 30000}}

    def test_within_the_threshold_passes(self):
        instr = {r: v * 1.004 for r, v in OK.items()}  # +0.4% each
        failures, _, _ = verdict(instr=instr, main=self.MAIN)
        self.assertEqual(failures, [])

    def test_a_slowdown_of_an_unmet_row_fails(self):
        failures, _, table = verdict(instr={**OK, "s3": 1110.0}, main=self.MAIN)  # +0.91%
        self.assertEqual(failures, ["`s3`: +0.91% instructions against main (1,100.0 -> 1,110.0)"])
        self.assertIn("SLOWER", dict((r, st) for r, *_, st in table)["s3"])

    def test_peak_rss_uses_its_own_threshold(self):
        small = verdict(rss={"p.R": 30500}, main=self.MAIN)[0]  # +1.7%
        big = verdict(rss={"p.R": 31000}, main=self.MAIN)[0]
        self.assertEqual(small, [])
        self.assertEqual(big, ["`p.R`: peak RSS +3.3% against main (30,000 -> 31,000 KiB)"])


class Acceptance(unittest.TestCase):
    BAR = {"row": "paste", "against": "today", "max": 1.0, "met": False}

    def test_a_row_with_only_one_key_is_rejected(self):
        self.assertEqual(perf.rt_acceptance_errors({**self.BAR, "ceiling": 2.0}),
                         ["`paste`: `accepted` and `ceiling` go together, but it has only `ceiling`"])
        self.assertEqual(perf.rt_acceptance_errors({**self.BAR, "accepted": "README.md"}),
                         ["`paste`: `accepted` and `ceiling` go together, but it has only `accepted`"])

    def test_a_met_row_cannot_be_accepted(self):
        bar = {**self.BAR, "met": True, "accepted": "README.md", "ceiling": 2.0}
        self.assertEqual(perf.rt_acceptance_errors(bar),
                         ["`paste`: an accepted regression is unmet by definition, so it needs `met = false`"])

    def test_a_missing_record_or_a_ceiling_at_or_below_the_bar_is_rejected(self):
        self.assertEqual(perf.rt_acceptance_errors({**self.BAR, "accepted": "no-such-record.md", "ceiling": 2.0}),
                         ["`paste`: its acceptance record no-such-record.md does not exist"])
        self.assertEqual(perf.rt_acceptance_errors({**self.BAR, "accepted": "README.md", "ceiling": 1.0}),
                         ["`paste`: its ceiling 1.0x is not above its bar of 1.0x"])

    def test_a_bars_file_with_a_broken_acceptance_refuses_to_gate(self):
        with tempfile.TemporaryDirectory() as tmp:
            bars = Path(tmp, "bars.toml")
            bars.write_text(f'[gnur]\nversion = "{perf.pins()["rt"]}"\nsource = "x"\n[gnur.instr]\n[today]\n'
                            '[[bar]]\nrow = "paste"\nagainst = "today"\nmax = 1.0\nmet = false\nceiling = 2.0\n'
                            '[[rss]]\nprogram = "p.R"\nagainst = []\n')
            a = SimpleNamespace(bars=str(bars), robigo_rt="/nonexistent", main=None, jobs=1, runs=1,
                                out=os.path.join(tmp, "out.json"))
            buf = io.StringIO()
            with redirect_stdout(buf):
                self.assertEqual(perf.cmd_rt_bars(a), 1)
            self.assertIn("`paste`: `accepted` and `ceiling` go together, but it has only `ceiling`", buf.getvalue())

    def test_a_complete_acceptance_passes(self):
        self.assertEqual(perf.rt_acceptance_errors({**self.BAR, "accepted": "README.md", "ceiling": 2.0}), [])


class Pin(unittest.TestCase):
    def test_numbers_for_another_pin_refuse_to_gate(self):
        with tempfile.TemporaryDirectory() as tmp:
            bars = Path(tmp, "bars.toml")
            bars.write_text('[gnur]\nversion = "9.9.9"\nsource = "x"\n[gnur.instr]\n[today]\n[[bar]]\n'
                            'row = "plain"\nagainst = "gnur"\nmax = 1.0\n[[rss]]\nprogram = "p.R"\nagainst = []\n')
            a = SimpleNamespace(bars=str(bars), robigo_rt="/nonexistent", main=None, jobs=1, runs=1,
                                out=os.path.join(tmp, "out.json"))
            buf = io.StringIO()
            with redirect_stdout(buf):
                self.assertEqual(perf.cmd_rt_bars(a), 1)
            self.assertIn("holds GNU R 9.9.9's numbers, but the rt pin", buf.getvalue())

    def test_the_committed_numbers_are_for_the_rt_pin(self):
        import tomllib
        with open(perf.RT_BARS, "rb") as f:
            spec = tomllib.load(f)
        self.assertEqual(spec["gnur"]["version"], perf.pins()["rt"])
        rows = {b["row"] for b in spec["bar"]}
        for b in spec["bar"]:
            self.assertIn(b["row"], spec[b["against"]]["instr"])
            self.assertEqual(perf.rt_acceptance_errors(b), [])
        for r in spec["rss"]:
            for against in r["against"]:
                self.assertIn(r["program"], spec[against]["rss"])
        with open(perf.RT_ROWS, "rb") as f:
            names = {r["name"] for r in tomllib.load(f)["row"]}
        self.assertLessEqual(rows, names)


if __name__ == "__main__":
    unittest.main()
