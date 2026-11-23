#!/usr/bin/env python3
"""The rt-bars gate's decisions (scripts/perf.py rt_verdict, cmd_rt_bars),
on synthetic numbers: no valgrind, no robigo-rt."""
import importlib.util
import hashlib
import io
import json
import os
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

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


class Guards(unittest.TestCase):
    """A guard row (m1-bars.toml's [[guard]]): no bar, held to main only."""

    SPEC = {**SPEC, "guard": [{"row": "subset"}]}
    INSTR = {**OK, "subset": 100.0}

    def test_a_guard_only_reports_without_main(self):
        failures, notes, table = perf.rt_verdict(self.SPEC, self.INSTR, RSS)
        self.assertEqual((failures, notes), ([], []))
        self.assertEqual(table[-1], ("subset", 100.0, None, "main", None, None, True, "guard"))

    def test_a_guard_slower_than_main_fails(self):
        main = {"instr": {**self.INSTR, "subset": 99.0}, "rss": RSS}
        failures, _, table = perf.rt_verdict(self.SPEC, self.INSTR, RSS, main)
        self.assertEqual(failures, ["`subset`: +1.01% instructions against main (99.0 -> 100.0)"])
        self.assertEqual(table[-1][-1], "guard, SLOWER")
        main = {"instr": {**self.INSTR, "subset": 99.6}, "rss": RSS}
        self.assertEqual(perf.rt_verdict(self.SPEC, self.INSTR, RSS, main)[0], [])

    def test_a_guard_must_be_a_row_once_and_not_a_bars(self):
        spec = {**SPEC, "guard": [{"row": "nope"}, {"row": "plain"}, {"row": "x"}, {"row": "x"}]}
        self.assertEqual(perf.rt_guard_errors(spec, {"plain", "x"}), [
            "guard `nope`: no row of that name in benchmarks/rt/calls-rows.toml",
            "guard `plain`: a bar's row already, which holds it to main",
            "guard `x`: named twice",
        ])

    def test_rt_bars_measures_a_guard_on_both_binaries_and_fails_its_slowdown(self):
        # cmd_rt_bars' wiring, with the measurements mocked: the guard is
        # measured with the bars on robigo-rt and on main, shown as a guard,
        # and 1% slower than main fails the command
        guard = "guard-subset-dbl"
        bars = (f'[gnur]\nversion = "{perf.pins()["rt"]}"\nsource = "x"\n[gnur.instr]\nplain = 1000.0\n'
                '[gnur.rss]\n"p.R" = 50000\n[[bar]]\nrow = "plain"\nagainst = "gnur"\nmax = 0.6\n'
                f'[[guard]]\nrow = "{guard}"\n[[rss]]\nprogram = "p.R"\nagainst = ["gnur"]\n')
        measured = []

        boots = []

        def instr(binary, rows, *_, **kw):
            measured.append((binary, list(rows)))
            boots.append(kw.get("boot"))
            return {"plain": 500.0, guard: 101.0 if binary == "new-bin" else 100.0}

        for slower, rc in ((True, 1), (False, 0)):
            measured.clear()
            with tempfile.TemporaryDirectory() as tmp:
                Path(tmp, "bars.toml").write_text(bars)
                a = SimpleNamespace(bars=str(Path(tmp, "bars.toml")), robigo_rt="new-bin",
                                    main="main-bin" if slower else None, jobs=1, runs=1,
                                    out=os.path.join(tmp, "out.json"), gnur=None, cycles=False, boot="rdb")
                buf = io.StringIO()
                with patch.object(perf, "rt_instr", instr), \
                        patch.object(perf, "rt_rss", lambda *_, **__: {"p.R": 30000}), \
                        patch.object(perf, "binary_identity", lambda *_: {}), \
                        patch.object(perf, "report_identity", lambda *_, **__: None), \
                        patch.object(perf, "cpu_signature", lambda: ("cpu", "0" * 16)), redirect_stdout(buf):
                    self.assertEqual(perf.cmd_rt_bars(a), rc)
            # every row of both binaries under the one boot named (rt-bars' --boot)
            self.assertEqual(set(boots), {"rdb"})
            boots.clear()
            out = buf.getvalue()
            self.assertEqual([r for _, r in measured], [["plain", guard]] * (2 if slower else 1))
            self.assertRegex(out, rf"\| {guard} \| 101\.0 \|.*\| main \| \| guard \|")
            if slower:
                self.assertEqual([b for b, _ in measured], ["new-bin", "main-bin"])
                self.assertIn(f"FAIL `{guard}`: +1.00% instructions against main (100.0 -> 101.0)", out)
            else:
                self.assertNotIn("FAIL", out)

    def test_the_committed_guards(self):
        import tomllib
        with open(perf.RT_ROWS, "rb") as f:
            rows = {r["name"]: r for r in tomllib.load(f)["row"]}
        with open(os.path.join(perf.ROOT, "benchmarks", "rt", "m1-bars.toml"), "rb") as f:
            bars = tomllib.load(f)
        self.assertEqual(perf.rt_guard_errors(bars, set(rows)), [])
        guards = [g["row"] for g in bars["guard"]]
        self.assertEqual(guards, [
            "guard-subset-dbl", "guard-subassign-dbl", "guard-subset-matrix",
            "grow-dbl", "scalars", "d0-attr", "d0-attributes", "d0-names",
            "d0-unlist", "d0-paste", "d0-paste0",
            "guard-vec-arith", "guard-vec-sum", "guard-dots", "guard-dollar-assign", "guard-while", "guard-repeat",
        ])
        # every row of the guard group is one
        self.assertLessEqual({n for n, r in rows.items() if r["group"] == "guard"}, set(guards))


class Rows(unittest.TestCase):
    """A rows file's `setup_ref`, and a bars file that names no program."""

    def test_a_row_takes_its_own_setup_or_the_shared_one(self):
        doc = {"row": [{"name": "a", "setup": "x <- 1"}, {"name": "jit-b", "setup_ref": "calls"}, {"name": "c"}],
               "setups": {"calls": "f <- function() 1"}}
        rows = perf.rows_with_setups(doc)
        self.assertEqual([r.get("setup") for r in rows], ["x <- 1", "f <- function() 1", None])
        # the parsed document is left as it was
        self.assertNotIn("setup", doc["row"][1])

    def test_an_unknown_setup_names_its_row(self):
        with self.assertRaises(KeyError) as e:
            perf.rows_with_setups({"row": [{"name": "jit-c", "setup_ref": "nope"}]})
        self.assertEqual(e.exception.args[0], "row `jit-c`: no setup named `nope` in the [setups] table")

    def test_no_program_means_no_rss_run(self):
        # (rss.py refuses a call without files; /nonexistent would fail if run)
        self.assertEqual(perf.rt_rss(None, [], 1, "/nonexistent", "gnur"), {})


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
                                out=os.path.join(tmp, "out.json"), gnur=None, cycles=False, boot="m1")
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
                                out=os.path.join(tmp, "out.json"), gnur=str(g), cycles=False, boot="m1")
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
                                out=os.path.join(tmp, "out.json"), gnur=None, cycles=False, boot="m1")
            buf = io.StringIO()
            with redirect_stdout(buf):
                self.assertEqual(perf.cmd_rt_bars(a), 1)
            self.assertIn("holds GNU R 9.9.9's numbers, but the rt pin", buf.getvalue())

    def test_the_committed_numbers_are_for_the_rt_pin(self):
        import tomllib
        with open(perf.RT_ROWS, "rb") as f:
            names = {r["name"] for r in tomllib.load(f)["row"]}
        # m1-bars.toml, and V4's JIT-cost bars, not gated until V4 step 3
        jit = os.path.join(perf.ROOT, "benchmarks", "rt", "jit-bars.toml")
        specs = {}
        for path in (perf.RT_BARS, jit):
            with open(path, "rb") as f:
                specs[path] = tomllib.load(f)
        for path, spec in specs.items():
            self.assertEqual(spec["gnur"]["version"], perf.pins()["rt"], path)
            rows = {b["row"] for b in spec["bar"]}
            for b in spec["bar"]:
                self.assertIn(b["row"], spec[b["against"]]["instr"], path)
                self.assertEqual(perf.rt_acceptance_errors(b), [], path)
            for r in spec["rss"]:
                for against in r["against"]:
                    self.assertIn(r["program"], spec[against]["rss"], path)
            self.assertLessEqual(rows, names, path)
        # (robigo-rt is at about 1.9x GNU R's there: none is met yet)
        self.assertTrue(all(b.get("met") is False for b in specs[jit]["bar"]))


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
        instr.update({g["row"]: 1000.0 for g in spec.get("guard", [])})
        rss = {r["program"]: min(spec[a]["rss"][r["program"]] for a in r["against"]) - 1 for r in spec["rss"]}
        failures, _, table = perf.rt_verdict(spec, instr, rss)
        self.assertEqual(failures, [])
        self.assertEqual(dict((r, st) for r, *_, st in table)["s3"], "not met yet")
        main = {"instr": dict(instr), "rss": dict(rss)}
        failures, _, _ = perf.rt_verdict(spec, {**instr, "s3": instr["s3"] * 1.006}, rss, main)
        self.assertEqual(len(failures), 1)
        self.assertIn("`s3`: +0.60% instructions against main", failures[0])


ivr_spec = importlib.util.spec_from_file_location("idiom_vs_r", Path(__file__).with_name("idiom-vs-r.py"))
ivr = importlib.util.module_from_spec(ivr_spec)
ivr_spec.loader.exec_module(ivr)


class AddressShifts(unittest.TestCase):
    """The address-sensitive rows are measured as the median over heap
    shifts (docs/ledger/g3/2026-11-17-address-rows.md): a real slowdown
    still fails, one layout's count does not."""

    SPEC = {"gnur": {"version": "4.6.2", "source": "test", "instr": {}, "rss": {}},
            "today": {"source": "test", "instr": {"e7-match": 114413.0}, "rss": {}},
            "bar": [{"row": "e7-match", "against": "today", "max": 1.0}], "rss": []}
    # main's counts over k = 0, 300, 1000, 3000, 5000 (the foreman's range on main)
    MAIN = [99000.0, 98900.0, 99276.0, 99100.0, 98950.0]

    def verdict(self, new):
        main = {"instr": {"e7-match": ivr.median(self.MAIN)}, "rss": {}}
        return perf.rt_verdict(self.SPEC, {"e7-match": ivr.median(new)}, {}, main)[0]

    def test_a_real_regression_on_every_shift_fails(self):
        failures = self.verdict([v * 1.006 for v in self.MAIN])
        self.assertEqual(len(failures), 1)
        self.assertIn("`e7-match`: +0.60% instructions against main", failures[0])

    def test_one_layout_moving_alone_does_not_fail(self):
        # BW's case: the same code, one shift's keys landing elsewhere (+4.15%)
        new = list(self.MAIN)
        new[0] *= 1.0415
        self.assertEqual(self.verdict(new), [])

    def test_rows_measures_each_shift_of_a_marked_row(self):
        with tempfile.TemporaryDirectory() as tmp:
            rows = Path(tmp, "rows.toml")
            rows.write_text('[[row]]\nname = "hashed"\ngroup = "e7"\naddress_sensitive = true\nsetup = "s <- 1"\n'
                            'body = "x"\nn = 10\n[[row]]\nname = "plain"\ngroup = "calls"\nsetup = "s <- 1"\n'
                            'body = "x"\nn = 10\n')
            seen = []

            class Fake:
                robigo, robigo_identity, robigo_env, r_bin, valgrind = "rt", None, {}, "R", "valgrind"

                def robigo_iter(self, setup, body, n, tag, env=None):
                    seen.append(setup)
                    return 1000.0 + 10 * setup.count("seq_len") * (len(seen) % 3)

                def robigo_collections(self, setup, body, k, env=None):
                    return 0

            out = Path(tmp, "out.json")
            a = SimpleNamespace(rows=str(rows), filter=None, no_r=True, no_robigo=False, cycles=False,
                                shifts=list(perf.ADDRESS_SHIFTS), json=str(out), boot="m1")
            with redirect_stdout(io.StringIO()):
                ivr.cmd_rows(a, Fake())
            doc = json.loads(out.read_text())["rows"]
            # the marked row ran once per shift, k = 0 as written; the other once
            self.assertEqual(seen[0], "s <- 1")
            self.assertEqual(seen[1], 'zz <- paste0("z", seq_len(300L))\ns <- 1')
            self.assertEqual(len(seen), len(perf.ADDRESS_SHIFTS) + 1)
            self.assertEqual(sorted(doc["hashed"]["shifts"]), sorted(str(k) for k in perf.ADDRESS_SHIFTS))
            counts = [v["robigo_instr"] for v in doc["hashed"]["shifts"].values()]
            self.assertEqual(doc["hashed"]["robigo_instr"], ivr.median(counts))
            self.assertNotIn("shifts", doc["plain"])

    def test_rt_instr_passes_the_shifts_and_collects_their_range(self):
        calls = []

        def fake_run(cmd, **kw):
            calls.append(cmd)
            out = cmd[cmd.index("--json") + 1]
            Path(out).write_text(json.dumps({"rows": {"e7-match": {
                "robigo_instr": 99000.0, "robigo_est_cycles": None,
                "shifts": {str(k): {"robigo_instr": v} for k, v in zip(perf.ADDRESS_SHIFTS, self.MAIN)}}}}))
            return SimpleNamespace(returncode=0, stderr="")
        ranges = {}
        with tempfile.TemporaryDirectory() as tmp, patch.object(perf.subprocess, "run", side_effect=fake_run):
            instr = perf.rt_instr("rt", ["e7-match"], 1, tmp, "new", ranges=ranges)
        self.assertEqual(instr, {"e7-match": 99000.0})
        self.assertEqual(ranges, {"e7-match": (98900.0, 99276.0)})
        cmd = calls[0]
        self.assertEqual(cmd[cmd.index("--shifts") + 1], "0,300,1000,3000,5000")

    def test_the_committed_rows_mark_exactly_the_hashing_rows(self):
        import tomllib
        with open(perf.RT_ROWS, "rb") as f:
            marked = sorted(r["name"] for r in tomllib.load(f)["row"] if r.get("address_sensitive"))
        self.assertEqual(marked, ["e7-match"])


class RtStarts(unittest.TestCase):
    """perf.py rt-starts: the starts ([[startup]]) as an invocation of their
    own, with their own JSON, beside rt-bars in the one perf-rt job."""

    BARS = '[[startup]]\nname = "interim"\n[[startup]]\nname = "rdb-jit"\ngnur = "jit"\n'

    def run_starts(self, bars, new_instr, gnur=None):
        su = perf.rt_startup()
        calls = []

        def run(binary, starts, root, runs, trees, valgrind):
            calls.append((binary, list(starts), trees))
            base = {"instr": 1000.0, "anon_kib": 2000.0, "rss_kib": 9000.0}
            return {n: dict(base, instr=new_instr if binary == "new-bin" else 1000.0) for n in starts}

        with tempfile.TemporaryDirectory() as tmp:
            Path(tmp, "bars.toml").write_text(bars)
            a = SimpleNamespace(bars=str(Path(tmp, "bars.toml")), robigo_rt="new-bin", main="main-bin", runs=3,
                                trees="t-new.tar", main_trees="t-main.tar", gnur=gnur,
                                out=os.path.join(tmp, "starts.json"))
            buf = io.StringIO()
            with patch.object(su, "run", run), patch.object(perf, "rt_startup", lambda: su), \
                    patch.object(perf, "binary_identity", lambda *_: {}), \
                    patch.object(perf, "report_identity", lambda *_, **__: None), redirect_stdout(buf):
                rc = perf.cmd_rt_starts(a)
            doc = json.loads(Path(a.out).read_text()) if Path(a.out).exists() else None
        return rc, calls, doc, buf.getvalue()

    def test_both_binaries_are_measured_from_their_own_trees_into_its_own_json(self):
        rc, calls, doc, out = self.run_starts(self.BARS, 1000.0)
        self.assertEqual(rc, 0)
        self.assertEqual(calls, [("new-bin", ["interim", "rdb-jit"], "t-new.tar"),
                                 ("main-bin", ["interim", "rdb-jit"], "t-main.tar")])
        self.assertEqual(set(doc), {"new", "main", "binaries", "lengths"})
        self.assertIn("| rdb-jit (default) |", out)
        self.assertIn("0 failure(s)", out)

    def test_a_start_over_its_limit_fails_the_invocation(self):
        rc, _, _, out = self.run_starts(self.BARS, 1006.0)
        self.assertEqual(rc, 1)
        self.assertIn("FAIL", out)

    def test_a_bad_startup_entry_is_refused_before_measuring(self):
        rc, calls, _, out = self.run_starts('[[startup]]\nname = "nope"\n', 1000.0)
        self.assertEqual((rc, calls), (1, []))
        self.assertIn("'nope'", out)


class Boot(unittest.TestCase):
    def test_rt_instr_hands_the_boot_to_idiom_vs_r_for_robigo_only(self):
        cmds = []

        def fake_run(cmd, **kw):
            cmds.append(cmd)
            Path(cmd[cmd.index("--json") + 1]).write_text(json.dumps({"rows": {"plain": {
                "robigo_instr": 1.0, "gnur_instr": 1.0, "robigo_est_cycles": None, "gnur_est_cycles": None}}}))
            return SimpleNamespace(returncode=0, stderr="")
        with tempfile.TemporaryDirectory() as tmp, patch.object(perf.subprocess, "run", side_effect=fake_run):
            perf.rt_instr("rt", ["plain"], 1, tmp, "new", boot="rdb")
            perf.rt_instr(None, ["plain"], 1, tmp, "gnur", boot="rdb")
        robigo, gnur = cmds
        self.assertEqual(robigo[robigo.index("--robigo-env") + 1], "ROBIGO_RT_BOOT=rdb")
        self.assertNotIn("--robigo-env", gnur)
class RowCollections(unittest.TestCase):
    """A row's window holds no collection (idiom-vs-r.py's check before
    measuring; docs/ledger/g3/2026-11-23-row-collections.md)."""

    ROWS = ('[[row]]\nname = "{name}"\ngroup = "g"\nsetup = "s <- 1"\nbody = "x"\nn = 10\n{extra}')

    class Fake:
        robigo, robigo_identity, r_bin, valgrind = "rt", None, "R", "valgrind"

        def __init__(self, robigo=(0, 0), gnur=(0, 0), shifted=None):
            self.robigo_env, self.gnur_env = {}, {}
            self.counts = {"robigo": robigo, "gnur": gnur}
            self.shifted = shifted
            self.envs = []
            self.measured = []

        def pick(self, side, setup, k):
            c = self.shifted if self.shifted and setup != "s <- 1" else self.counts[side]
            return c[0] if k == 10 else c[1]

        def robigo_collections(self, setup, body, k, env=None):
            self.envs.append(("robigo-count", env))
            return self.pick("robigo", setup, k)

        def gnur_collections(self, setup, body, k, env=None):
            self.envs.append(("gnur-count", env))
            return self.pick("gnur", setup, k)

        def robigo_iter(self, setup, body, n, tag, env=None):
            self.envs.append(("robigo", env))
            self.measured.append("robigo")
            return 1000.0

        def gnur_iter(self, setup, body, n, tag, env=None):
            self.envs.append(("gnur", env))
            self.measured.append("gnur")
            return 2000.0

    def rows(self, fake, extra="", name="r", no_r=True, shifts=None):
        with tempfile.TemporaryDirectory() as tmp:
            rows = Path(tmp, "rows.toml")
            rows.write_text(self.ROWS.format(name=name, extra=extra))
            out = Path(tmp, "out.json")
            a = SimpleNamespace(rows=str(rows), filter=None, no_r=no_r, no_robigo=False, cycles=False,
                                shifts=shifts, json=str(out))
            with redirect_stdout(io.StringIO()) as so:
                ivr.cmd_rows(a, fake)
            return json.loads(out.read_text())["rows"], so.getvalue()

    def refused(self, fake, extra="", shifts=None):
        with self.assertRaises(SystemExit) as cm:
            self.rows(fake, extra, shifts=shifts)
        return str(cm.exception.code)

    def test_a_collection_inside_the_window_refuses_the_row_before_measuring(self):
        fake = self.Fake(robigo=(0, 1))
        with patch.dict(os.environ, {"ROBIGO_RT_BOOT": "rdb"}):
            msg = self.refused(fake)
        self.assertIn("row `r`: robigo-rt collected 0 times at N = 10 and 1 at 2N (boot rdb; the default heap)", msg)
        self.assertIn("`heap` table", msg)
        self.assertIn("`collects = true`", msg)
        self.assertEqual(fake.measured, [])

    def test_equal_counts_that_are_not_zero_refuse_the_row_too(self):
        msg = self.refused(self.Fake(robigo=(2, 2)))
        self.assertIn("collected 2 times at N = 10 and 2 at 2N", msg)

    def test_every_refused_row_is_named_at_once(self):
        fake = self.Fake(robigo=(0, 1))
        with tempfile.TemporaryDirectory() as tmp:
            rows = Path(tmp, "rows.toml")
            rows.write_text(self.ROWS.format(name="a", extra="") + self.ROWS.format(name="b", extra=""))
            a = SimpleNamespace(rows=str(rows), filter=None, no_r=True, no_robigo=False, cycles=False,
                                shifts=None, json=None)
            with self.assertRaises(SystemExit) as cm, redirect_stdout(io.StringIO()):
                ivr.cmd_rows(a, fake)
        msg = str(cm.exception.code)
        self.assertIn("row `a`", msg)
        self.assertIn("row `b`", msg)

    def test_a_row_marked_collects_is_measured_and_its_counts_reported(self):
        doc, table = self.rows(self.Fake(robigo=(5, 9), gnur=(21, 42)), "collects = true\n", no_r=False)
        self.assertEqual(doc["r"]["collections"], {"robigo": [5, 9], "gnur": [21, 42]})
        self.assertTrue(doc["r"]["collects"])
        self.assertIn("robigo 5→9; GNU R 21→42 (collects)", table)

    def test_gnur_collecting_inside_the_window_is_reported_not_refused(self):
        doc, table = self.rows(self.Fake(gnur=(0, 1)), no_r=False)
        self.assertEqual(doc["r"]["collections"], {"robigo": [0, 0], "gnur": [0, 1]})
        self.assertIn("robigo 0→0; GNU R 0→1", table)

    def test_a_rows_heap_sizes_both_sides_counts_and_measurements(self):
        fake = self.Fake()
        doc, _ = self.rows(fake, 'heap = { R_VSIZE = "256M" }\n', no_r=False)
        self.assertEqual({side for side, _ in fake.envs}, {"robigo-count", "gnur-count", "robigo", "gnur"})
        self.assertTrue(all(env == {"R_VSIZE": "256M"} for _, env in fake.envs))
        self.assertEqual(doc["r"]["heap"], {"R_VSIZE": "256M"})

    def test_a_heap_shift_that_moves_a_collection_into_the_window_refuses_the_row(self):
        # (an address-sensitive row, measured over shifts: k = 0 is clean)
        msg = self.refused(self.Fake(shifted=(0, 1)), "address_sensitive = true\n", shifts=[0, 300])
        self.assertIn("collected 0 times at N = 10 and 1 at 2N", msg)
        self.assertIn("(heap shift k = 300)", msg)

    def test_a_binary_without_a_collection_report_is_measured_unchecked(self):
        fake = self.Fake(robigo=(None, None))
        with patch("sys.stderr", new_callable=io.StringIO) as err:
            doc, table = self.rows(fake)
        self.assertIn("reports no collections", err.getvalue())
        self.assertIn("robigo -", table)
        self.assertEqual(fake.measured, ["robigo"])

    def test_bad_collects_and_heap_fields_are_refused(self):
        self.assertEqual(ivr.row_errors({"name": "r", "collects": True, "heap": {"R_NSIZE": "2000000"}}), [])
        self.assertIn("`collects` is true or false", ivr.row_errors({"name": "r", "collects": "yes"})[0])
        self.assertIn("may set only R_VSIZE, R_NSIZE", ivr.row_errors({"name": "r", "heap": {"R_MAX_VSIZE": "1G"}})[0])
        self.assertIn("a size string", ivr.row_errors({"name": "r", "heap": {"R_VSIZE": 256}})[0])
        with self.assertRaises(SystemExit) as cm:
            self.rows(self.Fake(), 'heap = { R_MAX_VSIZE = "1G" }\n')
        self.assertIn("`heap` sets R_MAX_VSIZE", str(cm.exception.code))

    def test_gnur_counts_its_gcinfo_reports(self):
        stderr = ("Garbage collection 1 = 1+0+0 (level 0) ... \n13.3 Mbytes of cons cells used (51%)\n"
                  "Garbage collection 2 = 2+0+0 (level 0) ... \n")
        with tempfile.TemporaryDirectory() as tmp, \
                patch.object(ivr.subprocess, "run", return_value=SimpleNamespace(returncode=0, stderr=stderr)) as run:
            m = SimpleNamespace(tmp=tmp, r_bin="R", gnur_run_env=lambda env: {"R_HOME": "h", **(env or {})})
            self.assertEqual(ivr.Meter.gnur_collections(m, "s <- 1", "x", 10, {"R_VSIZE": "1G"}), 2)
            script = run.call_args[0][0][-1]
            self.assertTrue(Path(script).read_text().startswith("invisible(gcinfo(TRUE))\ns <- 1\n"))
            self.assertEqual(run.call_args[1]["env"]["R_VSIZE"], "1G")

    def test_robigo_counts_its_heaps_report(self):
        stderr = "robigo gc: collections 3 (entry 0, loop 3, return 0)\nrobigo gc: allocated 9 objects, 9 bytes\n"
        with tempfile.TemporaryDirectory() as tmp, \
                patch.object(ivr.subprocess, "run", return_value=SimpleNamespace(returncode=0, stderr=stderr)) as run:
            m = SimpleNamespace(robigo="rt", robigo_env={"ROBIGO_COMPILE": "1"},
                                r_script=lambda setup, body, k: os.path.join(tmp, "s.R"))
            self.assertEqual(ivr.Meter.robigo_collections(m, "s <- 1", "x", 10, {"R_VSIZE": "1G"}), 3)
            env = run.call_args[1]["env"]
            self.assertEqual((env["ROBIGO_GC_STATS"], env["R_VSIZE"]), ("1", "1G"))

    def test_main_and_the_pr_collecting_differently_warns_and_never_fails(self):
        notes = perf.rt_collection_notes(["jit-safe", "jit-tag"], {"jit-safe": [8, 16], "jit-tag": [5, 10]},
                                         {"jit-safe": [8, 15], "jit-tag": [5, 10]})
        self.assertEqual(len(notes), 1)
        self.assertIn("`jit-safe`: robigo-rt collects 8→16 times at N→2N, main 8→15", notes[0])
        self.assertEqual(perf.rt_collection_notes(["jit-safe"], {"jit-safe": [8, 16]}, {}), [])

    def test_rt_instr_collects_each_rows_counts(self):
        def fake_run(cmd, **kw):
            Path(cmd[cmd.index("--json") + 1]).write_text(json.dumps({"rows": {"jit-safe": {
                "robigo_instr": 1.0, "robigo_est_cycles": None,
                "collections": {"robigo": [8, 16]}}}}))
            return SimpleNamespace(returncode=0, stderr="")
        colls = {}
        with tempfile.TemporaryDirectory() as tmp, patch.object(perf.subprocess, "run", side_effect=fake_run):
            perf.rt_instr("rt", ["jit-safe"], 1, tmp, "new", colls=colls)
        self.assertEqual(colls, {"jit-safe": [8, 16]})

    def test_the_committed_rows_mark_the_jit_rows_and_size_guard_vec_arith(self):
        import tomllib
        with open(perf.RT_ROWS, "rb") as f:
            rows = tomllib.load(f)["row"]
        self.assertEqual(sorted(r["name"] for r in rows if r.get("collects")),
                         sorted(r["name"] for r in rows if r.get("group") == "jit"))
        self.assertEqual({r["name"]: r["heap"] for r in rows if "heap" in r}, {"guard-vec-arith": {"R_VSIZE": "256M"}})
        self.assertEqual([e for r in rows for e in ivr.row_errors(r)], [])


class EveryRunNamesItsBoot(unittest.TestCase):
    """No robigo-rt run of rt-bars or rt-starts goes without an explicit
    ROBIGO_RT_BOOT: asserted on what is spawned, not on one function."""

    def test_rt_bars_spawns_every_robigo_rt_run_with_the_boot(self):
        bars = (f'[gnur]\nversion = "{perf.pins()["rt"]}"\nsource = "x"\n[gnur.instr]\nplain = 1000.0\n'
                '[gnur.rss]\n"p.R" = 50000\n[[bar]]\nrow = "plain"\nagainst = "gnur"\nmax = 0.6\n'
                '[[rss]]\nprogram = "p.R"\nagainst = ["gnur"]\n')
        cmds = []

        def fake_run(cmd, **kw):
            cmds.append(cmd)
            out = cmd[cmd.index("--json") + 1]
            if "rss.py" in cmd[1]:
                Path(out).write_text(json.dumps({"results": {"p.R": {"robigo": {"median_kib": 30000}}}}))
            else:
                Path(out).write_text(json.dumps({"rows": {"plain": {
                    "robigo_instr": 500.0, "robigo_est_cycles": None, "collections": {"robigo": [0, 0]}}}}))
            return SimpleNamespace(returncode=0, stderr="", stdout="")
        with tempfile.TemporaryDirectory() as tmp:
            Path(tmp, "bars.toml").write_text(bars)
            a = SimpleNamespace(bars=str(Path(tmp, "bars.toml")), robigo_rt="new-bin", main="main-bin", jobs=1, runs=1,
                                out=os.path.join(tmp, "out.json"), gnur=None, cycles=False, boot="rdb")
            with patch.object(perf.subprocess, "run", side_effect=fake_run), \
                    patch.object(perf, "binary_identity", lambda *_: {}), \
                    patch.object(perf, "report_identity", lambda *_, **__: None), \
                    patch.object(perf, "cpu_signature", lambda: ("cpu", "0" * 16)), redirect_stdout(io.StringIO()):
                perf.cmd_rt_bars(a)
        robigo = [c for c in cmds if "--robigo" in c]
        # both binaries, the rows and the RSS programs
        self.assertEqual(sorted({c[c.index("--robigo") + 1] for c in robigo}), ["main-bin", "new-bin"])
        self.assertEqual({"rss.py" in c[1] for c in robigo}, {True, False})
        for c in robigo:
            self.assertEqual(c[c.index("--robigo-env") + 1], "ROBIGO_RT_BOOT=rdb", c)

    def test_an_unknown_boot_is_refused_before_anything_runs(self):
        a = SimpleNamespace(bars="unused", robigo_rt="x", main=None, jobs=1, runs=1, out=None, gnur=None,
                            cycles=False, boot="m2")
        with patch.object(perf.subprocess, "run", side_effect=AssertionError("ran")), \
                redirect_stdout(io.StringIO()) as out:
            self.assertEqual(perf.cmd_rt_bars(a), 1)
        self.assertIn("--boot 'm2' is not a boot", out.getvalue())

    def test_rt_starts_runs_every_start_with_its_boot(self):
        su = perf.rt_startup()
        envs = []

        def instr(cmd, env, valgrind):
            envs.append(env)
            return 1000

        def memory(cmd, env, syscalls):
            envs.append(env)
            return 100, 5000, 0, b"", 0
        with tempfile.TemporaryDirectory() as home, patch.object(su, "instructions", instr), \
                patch.object(su, "memory", memory):
            su.run("bin", None, home, 1, trees=False)
        self.assertGreater(len(envs), 0)
        self.assertTrue(all("ROBIGO_RT_BOOT" in e for e in envs))


class BinaryIdentity(unittest.TestCase):
    """Which binary a measurement measured (perf.binary_identity), and the
    warning for a binary older than the sources it is built from."""

    def test_the_robigo_rt_binary_is_built_from_its_crate_tree(self):
        names = {os.path.basename(d) for d in perf.crate_dirs("robigo-rtapi")}
        self.assertLessEqual({"robigo-rtapi", "robigo-rt", "robigo-rtcore", "robigo-heap"}, names)
        # dev-dependencies and the old runtime are not part of the binary
        self.assertNotIn("robigo-conformance", names)
        self.assertNotIn("robigo-runtime", names)
        self.assertEqual(perf.crate_of("dist/robigo-rt-main"), "robigo-rtapi")
        self.assertEqual(perf.crate_of(".robigo/bin/robigo"), "robigo-cli")

    def test_identity_names_the_file_and_flags_an_old_build(self):
        with tempfile.TemporaryDirectory() as tmp:
            b = os.path.join(tmp, "robigo-rt")
            Path(b).write_bytes(b"not an ELF file")
            ident = perf.binary_identity(b)
            self.assertEqual(ident["sha256"], hashlib.sha256(b"not an ELF file").hexdigest())
            self.assertIsNone(ident["build_id"])
            self.assertIsNone(ident["stale"])  # no crate given: nothing to compare with
            # the crates' last commit, as git would report it (CI's oracle
            # image has no git, so the history is given here)
            history = {"log": "2000000000 abc1234\n", "status": "", "describe": "v1-2-gabc1234\n"}
            with patch.object(perf, "git", side_effect=lambda args, root=perf.ROOT: history[args[0]]):
                os.utime(b, (1000, 1000))  # older than that commit: stale
                stale = perf.binary_identity(b, "robigo-rtapi")
                self.assertEqual(stale["stale"]["source"], "commit abc1234")
                self.assertEqual(stale["checkout"], "v1-2-gabc1234")
                buf = io.StringIO()
                perf.report_identity("robigo-rt", stale, out=buf)
                self.assertIn("is not a build of this checkout's sources", buf.getvalue())
                os.utime(b, (2000000001, 2000000001))  # newer: a build of this checkout
                self.assertIsNone(perf.binary_identity(b, "robigo-rtapi")["stale"])

    def test_without_git_nothing_is_stale_and_nothing_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            b = os.path.join(tmp, "robigo-rt")
            Path(b).write_bytes(b"x")
            os.utime(b, (1000, 1000))
            with patch.object(perf.subprocess, "run", side_effect=FileNotFoundError("git")):
                ident = perf.binary_identity(b, "robigo-rtapi")
            self.assertIsNone(ident["stale"])
            self.assertIsNone(ident["checkout"])
            self.assertIsNone(ident["build_id"])

    def test_uncommitted_edits_are_read_from_porcelain_z(self):
        # a path with a space, a rename (its source follows and is skipped)
        # and an untracked file, as `git status --porcelain -z` lists them
        with tempfile.TemporaryDirectory() as root:
            for name, t in (("a b.rs", 3000), ("new.rs", 2000), ("dir/x.rs", 4000)):
                os.makedirs(os.path.dirname(os.path.join(root, name)) or root, exist_ok=True)
                Path(root, name).write_text("x")
                os.utime(os.path.join(root, name), (t, t))
            out = " M a b.rs\0R  new.rs\0old.rs\0?? dir/x.rs\0"
            with patch.object(perf, "git", side_effect=lambda args, r=None: "" if args[0] == "log" else out):
                t, what = perf.newest_source([root], root)
            self.assertEqual((t, what), (4000, "dir/x.rs (uncommitted)"))
            out = " M a b.rs\0R  new.rs\0old.rs\0"
            with patch.object(perf, "git", side_effect=lambda args, r=None: "" if args[0] == "log" else out):
                self.assertEqual(perf.newest_source([root], root), (3000, "a b.rs (uncommitted)"))

if __name__ == "__main__":
    unittest.main()
