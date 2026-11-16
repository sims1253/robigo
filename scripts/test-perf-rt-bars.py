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
    "today": {"source": "test", "instr": {"fib": 1000.0}, "rss": {"p.R": 40000}},
    "bar": [
        {"row": "plain", "against": "gnur", "max": 0.6},
        {"row": "s3", "against": "gnur", "max": 1.0, "met": False},
        {"row": "fib", "against": "today", "max": 1.0, "met": False},
    ],
    "rss": [{"program": "p.R", "against": ["today", "gnur"]}],
}
OK = {"plain": 500.0, "s3": 1100.0, "fib": 1400.0}
RSS = {"p.R": 30000}


def verdict(instr=None, rss=None, main=None):
    return perf.rt_verdict(SPEC, instr or OK, rss or RSS, main)


class Bars(unittest.TestCase):
    def test_met_bars_hold_and_unmet_bars_only_report(self):
        failures, notes, table = verdict()
        self.assertEqual(failures, [])
        self.assertEqual(notes, [])
        status = {row: st for row, *_, st in table}
        self.assertEqual(status, {"plain": "ok", "s3": "not met yet", "fib": "not met yet"})

    def test_a_met_bar_that_breaks_fails(self):
        failures, _, _ = verdict(instr={**OK, "plain": 601.0})
        self.assertEqual(len(failures), 1)
        self.assertIn("`plain`: 0.601x gnur breaks its bar of 0.6x", failures[0])

    def test_an_unmet_bar_gets_worse_without_failing_and_says_when_it_is_met(self):
        failures, notes, _ = verdict(instr={**OK, "s3": 1500.0, "fib": 900.0})
        self.assertEqual(failures, [])
        self.assertEqual(len(notes), 1)
        self.assertIn("`fib`: 0.900x today meets its bar", notes[0])

    def test_rss_above_any_reference_fails(self):
        failures, _, _ = verdict(rss={"p.R": 41000})
        self.assertEqual(failures, ["`p.R`: peak RSS 41,000 KiB, above today's 40,000"])


class AgainstMain(unittest.TestCase):
    # main's binary, measured on the same machine
    MAIN = {"instr": {"plain": 500.0, "s3": 1100.0, "fib": 1400.0}, "rss": {"p.R": 30000}}

    def test_within_the_threshold_passes(self):
        instr = {"plain": 502.0, "s3": 1105.0, "fib": 1406.0}  # +0.4% each
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
        for r in spec["rss"]:
            for against in r["against"]:
                self.assertIn(r["program"], spec[against]["rss"])
        with open(perf.RT_ROWS, "rb") as f:
            names = {r["name"] for r in tomllib.load(f)["row"]}
        self.assertLessEqual(rows, names)


if __name__ == "__main__":
    unittest.main()
