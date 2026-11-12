#!/usr/bin/env python3
"""Check the counter gate's rules (scripts/counters.py) without a robigo:
fixture headers, the known list's shrink-only semantics, per-iteration
arithmetic, and that the checked-in fixtures and known list read."""
import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("counters", HERE / "counters.py")
c = importlib.util.module_from_spec(spec)
spec.loader.exec_module(c)


def fixture(header, name="f"):
    with tempfile.TemporaryDirectory() as tmp:
        p = Path(tmp) / f"{name}.R"
        p.write_text(header + "for (i in seq_len(n)) NULL\n")
        return c.read_fixture(p)


F = fixture("#| vector_copies <= 0\n#| jit vm_ops <= 0\n#| ast,vm allocs <= 2\n")


def failing(tier, per_iter, known=()):
    return {j.counter for j in c.judge(F, tier, per_iter, list(known)) if j.verdict != "pass"}


class Header(unittest.TestCase):
    def test_tiers(self):
        self.assertEqual(F.bounds, {("ast", "vector_copies"): 0, ("vm", "vector_copies"): 0,
                                    ("jit", "vector_copies"): 0, ("jit", "vm_ops"): 0,
                                    ("ast", "allocs"): 2, ("vm", "allocs"): 2})

    def test_op_counter(self):
        self.assertEqual(fixture("#| vm op:Call <= 1\n").bounds, {("vm", "op:Call"): 1})
        self.assertEqual(fixture("#| vm op:SetIndex2 <= 0\n").bounds, {("vm", "op:SetIndex2"): 0})

    def test_refused(self):
        for bad in ("#| allocs < 1\n", "#| x86 allocs <= 1\n", "#| allocs <= 1\n#| vm allocs <= 2\n",
                    "# no bound\n"):
            with self.assertRaises(ValueError, msg=bad):
                fixture(bad)


class Judge(unittest.TestCase):
    def test_bounds(self):
        self.assertEqual(failing("jit", {"vector_copies": 0, "vm_ops": 0}), set())
        self.assertEqual(failing("jit", {"vector_copies": 1, "vm_ops": 0.5}), {"vector_copies", "vm_ops"})
        self.assertEqual(failing("vm", {"vector_copies": 0, "allocs": 2, "vm_ops": 99}), set(),
                         "a counter bounded in another tier only is not held here")

    def test_unmeasured_fails(self):
        self.assertEqual(failing("vm", {"allocs": 0}), {"vector_copies"})

    def test_op_kind_absent_is_zero(self):
        g = fixture("#| vm op:Call <= 0\n")
        self.assertEqual([j.verdict for j in c.judge(g, "vm", {"allocs": 0}, [])], ["pass"])

    def test_known_list_only_shrinks(self):
        known = [c.Known("f", "vm", "allocs", 34, "the VM's nested replacement allocates")]
        self.assertEqual(failing("vm", {"vector_copies": 0, "allocs": 34}, known), set(),
                         "a known failure at its number passes")
        self.assertEqual(failing("vm", {"vector_copies": 0, "allocs": 20}, known), set(),
                         "and below it")
        self.assertEqual(failing("vm", {"vector_copies": 0, "allocs": 35}, known), {"allocs"},
                         "a known failure that got worse fails")
        self.assertEqual(failing("vm", {"vector_copies": 0, "allocs": 2}, known), {"allocs"},
                         "a known failure that passes fails until it is taken out")
        self.assertEqual(failing("ast", {"vector_copies": 0, "allocs": 34}, known), {"allocs"},
                         "the entry is of one tier")

    def test_record_keeps_causes(self):
        known = [c.Known("f", "vm", "allocs", 34, "why")]
        judged = c.judge(F, "vm", {"vector_copies": 1, "allocs": 30}, known)
        again = c.recorded(judged, known)
        self.assertEqual(again, [c.Known("f", "vm", "vector_copies", 1, "not yet said"),
                                 c.Known("f", "vm", "allocs", 30, "why")])
        self.assertEqual(c.read_known(c.render_known(again), [F]), sorted(again, key=lambda k: k.key))

    def test_record_never_raises_a_number(self):
        known = [c.Known("f", "vm", "allocs", 34, "why")]
        judged = c.judge(F, "vm", {"vector_copies": 0, "allocs": 40}, known)
        self.assertEqual(c.recorded(judged, known), known)


class KnownFile(unittest.TestCase):
    def test_refused(self):
        ok = 'fixture = "f"\ntier = "vm"\ncounter = "allocs"\nmeasured = 3\ncause = "x"\n'
        self.assertEqual(len(c.read_known("[[known]]\n" + ok, [F])), 1)
        for bad in (ok.replace('"allocs"', '"envs"'), ok.replace('"vm"', '"jit"'),
                    ok.replace('cause = "x"', 'cause = ""'), ok.replace("measured = 3", 'measured = "3"'),
                    ok.replace('cause = "x"\n', "")):
            with self.assertRaises(ValueError, msg=bad):
                c.read_known("[[known]]\n" + bad, [F])
        with self.assertRaises(ValueError):
            c.read_known("[[known]]\n" + ok + "[[known]]\n" + ok, [F])


class Counts(unittest.TestCase):
    def test_parse(self):
        err = 'Error x\nrobigo-counters: {"allocs": 5, "vm_ops": 2, "ops": {"Call": 2}}\n'
        self.assertEqual(c.parse_counts(err), {"allocs": 5, "vm_ops": 2, "op:Call": 2})
        self.assertIsNone(c.parse_counts("no counts\n"))

    def test_per_iteration(self):
        short, long = {"allocs": 100, "op:Call": 3}, {"allocs": 4100, "envs": 7}
        self.assertEqual(c.per_iteration(short, long, (1000, 3000)), {"allocs": 2.0, "op:Call": -0.0015,
                                                                    "envs": 0.0035})


class CheckedIn(unittest.TestCase):
    def test_fixtures_and_known_list_read(self):
        every = c.fixtures()
        self.assertTrue(every)
        known = c.read_known(c.KNOWN.read_text(), every)
        keys = [k.key for k in known]
        self.assertEqual(keys, sorted(keys), "known.toml is sorted as render_known writes it")


if __name__ == "__main__":
    unittest.main(argv=sys.argv[:1] + sys.argv[1:])
