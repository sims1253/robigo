#!/usr/bin/env python3
"""The rt-bars gate's decisions (scripts/perf.py rt_verdict, cmd_rt_bars),
on synthetic numbers: no valgrind, no robigo-rt."""
import importlib.util
import io
import json
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
                                out=os.path.join(tmp, "out.json"), gnur=None, cycles=False)
            buf = io.StringIO()
            with redirect_stdout(buf):
                self.assertEqual(perf.cmd_rt_bars(a), 1)
            self.assertIn("`paste`: `accepted` and `ceiling` go together, but it has only `ceiling`", buf.getvalue())

    def test_a_complete_acceptance_passes(self):
        self.assertEqual(perf.rt_acceptance_errors({**self.BAR, "accepted": "README.md", "ceiling": 2.0}), [])


class Machine(unittest.TestCase):
    """GNU R's side measured on this machine (`rt-gnur`), and the CPU
    signature that keeps a bar from mixing CPUs."""

    def cpu(self, tmp, flags="sse2 avx2 fma", l3="32768K", model="AMD EPYC 9V74 80-Core Processor"):
        info = Path(tmp, f"cpuinfo-{flags}-{l3}-{model}".replace(" ", "_"))
        info.write_text(f"processor\t: 0\nvendor_id\t: AuthenticAMD\ncpu family\t: 25\nmodel\t\t: 17\n"
                        f"model name\t: {model}\nstepping\t: 1\nflags\t\t: {flags}\n\nprocessor\t: 1\n"
                        "flags\t\t: other\n")
        cache = Path(tmp, f"cache-{l3}")
        for i, (level, typ, size) in enumerate([("1", "Data", "32K"), ("3", "Unified", l3)]):
            d = cache / f"index{i}"
            d.mkdir(parents=True, exist_ok=True)
            for k, v in (("level", level), ("type", typ), ("size", size), ("ways_of_associativity", "8"),
                         ("coherency_line_size", "64")):
                (d / k).write_text(v + "\n")
        return perf.cpu_signature(str(info), str(cache))

    def test_the_signature_reads_the_first_cpu_its_flags_and_caches(self):
        with tempfile.TemporaryDirectory() as tmp:
            model, sig = self.cpu(tmp)
            self.assertEqual(model, "AMD EPYC 9V74 80-Core Processor")
            self.assertRegex(sig, r"^[0-9a-f]{16}$")
            self.assertEqual(self.cpu(tmp, flags="fma avx2 sse2")[1], sig)  # the flags' order does not matter
            self.assertNotEqual(self.cpu(tmp, flags="sse2 avx2")[1], sig)
            self.assertNotEqual(self.cpu(tmp, l3="16384K")[1], sig)
            self.assertNotEqual(self.cpu(tmp, model="AMD Ryzen 9 5900X 12-Core Processor")[1], sig)

    def test_a_gnur_file_for_another_pin_cpu_or_cache_model_cannot_gate(self):
        doc = {"version": "4.6.2", "cpu_signature": "abc", "cpu": "x", "cache_model": perf.PINNED_CACHE}
        self.assertEqual(perf.rt_gnur_errors(doc, "4.6.2", "abc"), [])
        self.assertIn("rt pin is 4.6.3", perf.rt_gnur_errors(doc, "4.6.3", "abc")[0])
        self.assertIn("this machine's is def", perf.rt_gnur_errors(doc, "4.6.2", "def")[0])
        self.assertIn("counted under", perf.rt_gnur_errors({**doc, "cache_model": "--LL=1,1,64"}, "4.6.2", "abc")[0])

    def test_the_bars_use_gnur_as_measured_here(self):
        doc = {"instr": {"plain": 800.0, "s3": 1000.0}, "rss": {"p.R": 45000}}
        spec = perf.rt_with_gnur(SPEC, doc)
        self.assertEqual(spec["gnur"]["instr"], doc["instr"])
        self.assertEqual(spec["gnur"]["rss"], {"p.R": 45000})
        self.assertEqual(SPEC["gnur"]["instr"]["plain"], 1000.0)  # the bars file's own numbers are untouched
        failures, _, _ = perf.rt_verdict(spec, {**OK, "plain": 500.0}, RSS)
        self.assertIn("`plain`: 0.625x gnur breaks its bar of 0.6x", failures[0])

    def test_rt_bars_refuses_gnur_measured_on_another_cpu(self):
        with tempfile.TemporaryDirectory() as tmp:
            g = Path(tmp, "gnur.json")
            g.write_text(json.dumps({"version": perf.pins()["rt"], "cpu_signature": "0000000000000000",
                                     "cpu": "elsewhere", "cache_model": perf.PINNED_CACHE, "instr": {}, "rss": {}}))
            a = SimpleNamespace(bars=perf.RT_BARS, robigo_rt="/nonexistent", main=None, jobs=1, runs=1,
                                out=os.path.join(tmp, "out.json"), gnur=str(g), cycles=False)
            buf = io.StringIO()
            with redirect_stdout(buf):
                self.assertEqual(perf.cmd_rt_bars(a), 1)
            self.assertIn("measured on CPU signature 0000000000000000 (elsewhere)", buf.getvalue())


class Pin(unittest.TestCase):
    def test_numbers_for_another_pin_refuse_to_gate(self):
        with tempfile.TemporaryDirectory() as tmp:
            bars = Path(tmp, "bars.toml")
            bars.write_text('[gnur]\nversion = "9.9.9"\nsource = "x"\n[gnur.instr]\n[today]\n[[bar]]\n'
                            'row = "plain"\nagainst = "gnur"\nmax = 1.0\n[[rss]]\nprogram = "p.R"\nagainst = []\n')
            a = SimpleNamespace(bars=str(bars), robigo_rt="/nonexistent", main=None, jobs=1, runs=1,
                                out=os.path.join(tmp, "out.json"), gnur=None, cycles=False)
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


    def test_committed_s3_is_unmet_and_still_held_to_main(self):
        # F2 revised (docs/ledger/g3/2026-11-17-a0b-stock-rival.md): s3 is
        # owed by P3, so 1.096x the stock rival does not fail the gate, but a
        # slowdown against main still does, as for fib
        import tomllib
        with open(perf.RT_BARS, "rb") as f:
            spec = tomllib.load(f)
        s3 = next(b for b in spec["bar"] if b["row"] == "s3")
        self.assertFalse(s3.get("met", True))
        instr = {b["row"]: 0.9 * b["max"] * spec[b["against"]]["instr"][b["row"]] for b in spec["bar"]}
        instr["s3"] = 1.096 * spec["gnur"]["instr"]["s3"]
        rss = {r["program"]: min(spec[a]["rss"][r["program"]] for a in r["against"]) - 1 for r in spec["rss"]}
        failures, _, table = perf.rt_verdict(spec, instr, rss)
        self.assertEqual(failures, [])
        self.assertEqual(dict((r, st) for r, *_, st in table)["s3"], "not met yet")
        main = {"instr": dict(instr), "rss": dict(rss)}
        failures, _, _ = perf.rt_verdict(spec, {**instr, "s3": instr["s3"] * 1.006}, rss, main)
        self.assertEqual(len(failures), 1)
        self.assertIn("`s3`: +0.60% instructions against main", failures[0])

if __name__ == "__main__":
    unittest.main()
