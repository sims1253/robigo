#!/usr/bin/env python3
"""Tests of scripts/builtin-diff.py's harness, with no R: the rows' sources,
the case classes, the batch runner's crash, hang and start handling, and the
known/ ratchet's key check."""
import importlib.util
import os
import sys
import tempfile
import textwrap
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("bd", os.path.join(HERE, "builtin-diff.py"))
bd = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bd)


class Sources(unittest.TestCase):
    def test_names_c_rows(self):
        with tempfile.TemporaryDirectory() as d:
            os.makedirs(os.path.join(d, "src", "main"))
            with open(os.path.join(d, bd.NAMES_C), "w") as f:
                f.write('{"nchar",\tdo_nchar,\t1,\t11,\t4,\t{PP_FUNCALL, PREC_FN,\t0}},\n'
                        '{"seq.int",\tdo_seq,\t\t0,\t1,\t-1,\t{PP_FUNCALL, PREC_FN,\t0}},\n'
                        '{"if",\t\tdo_if,\t\t0,\t200,\t-1,\t{PP_IF,\t     PREC_FN,\t  1}},\n')
            self.assertEqual(bd.names_c(d), {"nchar": (11, 4), "seq.int": (1, -1), "if": (200, -1)})

    def test_registry_effects(self):
        reg = bd.registry()
        self.assertEqual(reg["nchar"], "Pure")
        self.assertEqual(reg["set.seed"], "Rng")
        self.assertEqual(reg["lapply"], "Calls")

    def test_rows_refuse_what_cannot_run(self):
        nc = {"if": (200, -1), "lapply": (11, 2), "nchar": (11, 4), "x": (11, 1)}
        reg = {"lapply": "Calls", "nchar": "Pure"}
        for name, spec, msg in [("if", {}, "special"), ("lapply", {}, "only Pure and Rng"),
                                ("nchar", {"effect": "Rng"}, "the registry Pure"), ("x", {}, "needs `effect`"),
                                ("nope", {}, "not in GNU R's names.c")]:
            with self.assertRaises(SystemExit) as e:
                bd.Row(name, spec, nc, reg)
            self.assertIn(msg, str(e.exception))
        r = bd.Row("nchar", {}, nc, reg)
        self.assertTrue(r.internal)
        self.assertEqual((r.arity, r.effect, r.jits), (4, "Pure", ["off"]))
        self.assertEqual(bd.Row("nchar", {"jit": "both"}, nc, reg).jits, ["off", "on"])

    def test_keys_follow_the_row_and_seed(self):
        nc, reg = {"nchar": (11, 4)}, {"nchar": "Pure"}
        a = bd.Row("nchar", {}, nc, reg)
        b = bd.Row("nchar", {"extra": ["nchar('a')"]}, nc, reg)
        self.assertEqual(a.key(0, "off"), bd.Row("nchar", {}, nc, reg).key(0, "off"))
        self.assertNotEqual(a.key(0, "off"), b.key(0, "off"))
        self.assertNotEqual(a.key(0, "off"), a.key(1, "off"))
        self.assertNotEqual(a.key(0, "off"), a.key(0, "on"))

    def test_every_registry_builtin_has_a_row_or_a_reason(self):
        self.assertEqual(bd.cmd_check_rows(None), 0)


class Classes(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.mkdtemp()

    def f(self, name, data):
        p = os.path.join(self.d, name)
        with open(p, "wb") as f:
            f.write(data)
        return p

    def test_each_class(self):
        o = self.f("o", b"value 1")
        same = self.f("r1", b"value 1")
        other = self.f("r2", b"value 2")
        ny = self.f("r3", b"robigo: x is not implemented yet")
        res = self.f("o2", b"error: cannot allocate vector of size 16.0 Gb")
        gone = os.path.join(self.d, "missing")
        self.assertEqual(bd.classify(o, same, {}, {}, 1), "pass")
        self.assertEqual(bd.classify(o, other, {}, {}, 1), "mismatch")
        self.assertEqual(bd.classify(o, ny, {}, {}, 1), "not-implemented")
        self.assertEqual(bd.classify(res, other, {}, {}, 1), "resource")
        self.assertEqual(bd.classify(res, gone, {}, {1: "crash"}, 1), "resource")
        self.assertEqual(bd.classify(gone, same, {1: "crash"}, {}, 1), "oracle-crash")
        self.assertEqual(bd.classify(o, gone, {}, {1: "timeout"}, 1), "timeout")
        self.assertEqual(bd.classify(o, gone, {}, {1: "crash"}, 1), "crash")

    def test_score_line(self):
        c = {"pass": 5, "mismatch": 1, "not-implemented": 2, "crash": 1, "timeout": 0, "resource": 3,
             "oracle-crash": 4}
        self.assertEqual(bd.score_line("x", c), "x: 5 / 9 (not-implemented 2, crash 1, timeout 0, "
                                                "resource 3 excluded, oracle-crash 4 excluded)")


# a stand-in driver: writes each case's record from A4_FROM on, exits at
# case CRASH_AT and sleeps at HANG_AT
FAKE = textwrap.dedent("""\
    import os, sys, time
    out, start = os.environ["A4_OUT"], int(os.environ["A4_FROM"])
    if os.environ.get("UNUSABLE"):
        sys.stderr.write("Error: driver unusable on fake: withVisible\\n"); sys.exit(1)
    if os.environ.get("NOSTART"):
        sys.stderr.write("cannot boot\\n"); sys.exit(1)
    open(os.path.join(out, "started"), "w").close()
    for i in range(start, int(os.environ["N"]) + 1):
        if i == int(os.environ.get("CRASH_AT", "0")):
            sys.stderr.write("thread 'main' panicked\\n"); os._exit(101)
        if i == int(os.environ.get("HANG_AT", "0")):
            time.sleep(30)
        with open(os.path.join(out, "%06d.rds" % i), "w") as f:
            f.write(str(i))
    """)


class Batches(unittest.TestCase):
    def run_fake(self, n, **env):
        d = tempfile.mkdtemp()
        drv = os.path.join(d, "fake.py")
        with open(drv, "w") as f:
            f.write(FAKE)
        out, scratch = os.path.join(d, "out"), os.path.join(d, "scratch")
        os.makedirs(scratch)
        e = {**os.environ, "N": str(n), **{k: str(v) for k, v in env.items()}}
        lost = bd.run_batch([sys.executable, drv], e, "batch", out, n, scratch, timeout=1.0)
        return lost, sorted(f for f in os.listdir(out) if f.endswith(".rds")), out

    def test_a_crash_and_a_hang_cost_their_own_cases(self):
        lost, recs, out = self.run_fake(8, CRASH_AT=3, HANG_AT=6)
        self.assertEqual(lost, {3: "crash", 6: "timeout"})
        self.assertEqual(recs, [f"{i:06d}.rds" for i in (1, 2, 4, 5, 7, 8)])
        with open(os.path.join(out, "000003.lost")) as f:
            self.assertIn("panicked", f.read())

    def test_no_losses(self):
        lost, recs, _ = self.run_fake(4)
        self.assertEqual((lost, len(recs)), ({}, 4))

    def test_an_unusable_driver_stops_the_run(self):
        with self.assertRaises(bd.Unusable) as e:
            self.run_fake(4, UNUSABLE=1)
        self.assertEqual(str(e.exception), "driver unusable on fake: withVisible")

    def test_a_runtime_that_does_not_start_stops_the_run(self):
        with self.assertRaises(bd.Unusable) as e:
            self.run_fake(4, NOSTART=1)
        self.assertIn("did not start", str(e.exception))
        self.assertIn("cannot boot", str(e.exception))


class Known(unittest.TestCase):
    def test_a_list_for_another_key_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            old = bd.KNOWN
            bd.KNOWN = d
            try:
                r = {"res": {1: "pass", 2: "mismatch", 3: "crash"}}
                bd.write_known("x", "abc", r, [2, 3])
                self.assertEqual(bd.read_known("x", "abc"), {2, 3})
                with self.assertRaises(SystemExit) as e:
                    bd.read_known("x", "def")
                self.assertIn("builtin-diff:", str(e.exception))
                self.assertIsNone(bd.read_known("y", "abc"))
            finally:
                bd.KNOWN = old


if __name__ == "__main__":
    unittest.main()
