#!/usr/bin/env python3
"""Tests of scripts/builtin-diff.py's harness, with no R: the rows' sources,
the case classes, the batch runner's crash, hang and start handling, and the
known/ ratchet's key check."""
import fcntl
import importlib.util
import json
import os
import shutil
import subprocess
import unittest.mock
import sys
import tempfile
import textwrap
import threading
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("bd", os.path.join(HERE, "builtin-diff.py"))
bd = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bd)
conf_slots = bd.conf_slots


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
        for name, spec, msg in [("if", {}, "`special = true` runs it"), ("lapply", {"effect": "Nope"}, "the registry Calls"),
                                ("nchar", {"effect": "Rng"}, "the registry Pure"), ("x", {}, "needs `effect`"),
                                ("nope", {}, "not in GNU R's names.c")]:
            with self.assertRaises(SystemExit) as e:
                bd.Row(name, spec, nc, reg)
            self.assertIn(msg, str(e.exception))
        r = bd.Row("nchar", {}, nc, reg)
        self.assertTrue(r.internal)
        self.assertEqual((r.arity, r.effect, r.jits), (4, "Pure", ["off"]))
        self.assertEqual(bd.Row("nchar", {"jit": "both"}, nc, reg).jits, ["off", "on"])
        self.assertTrue(bd.Row("if", {"special": True, "effect": "Pure"}, nc, reg).internal is False)
        self.assertFalse(bd.Row("lapply", {}, nc, reg).isolated)
        self.assertTrue(bd.Row("x", {"effect": "External"}, nc, reg).isolated)

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

    def test_an_unlisted_builtin_warns_until_unlisted_fails(self):
        import contextlib, io
        from unittest import mock
        reg = {**bd.registry(), "a4.unlisted": "Pure"}
        for strict, want in ((False, 0), (True, 1)):
            out = io.StringIO()
            with mock.patch.object(bd, "registry", return_value=reg), \
                 mock.patch.object(bd, "UNLISTED_FAILS", strict), contextlib.redirect_stdout(out):
                self.assertEqual(bd.cmd_check_rows(None), want)
            self.assertIn("'a4.unlisted' has no row", out.getvalue())
            self.assertEqual("warning: registry builtin" in out.getvalue(), not strict)


class Classes(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.mkdtemp()

    def f(self, name, data):
        p = os.path.join(self.d, name)
        with open(p, "wb") as f:
            f.write(data)
        return p

    def test_each_class(self):
        o = self.f("o.rds", b"value 1")
        same = self.f("r1.rds", b"value 1")
        printed = self.f("r4.rds", b"value 1")
        self.f("r4.out", b"extra\n")
        other = self.f("r2", b"value 2")
        ny = self.f("r3", b"robigo: x is not implemented yet")
        res = self.f("o2", b"error: cannot allocate vector of size 16.0 Gb")
        gone = os.path.join(self.d, "missing")
        self.assertEqual(bd.classify(o, same, {}, {}, 1), "pass")
        self.assertEqual(bd.classify(o, printed, {}, {}, 1), "mismatch")
        self.assertEqual(bd.classify(o, other, {}, {}, 1), "mismatch")
        self.assertEqual(bd.classify(o, ny, {}, {}, 1), "not-implemented")
        self.assertEqual(bd.classify(res, other, {}, {}, 1), "resource")
        self.assertEqual(bd.classify(res, gone, {}, {1: "crash"}, 1), "resource")
        self.assertEqual(bd.classify(gone, same, {1: "crash"}, {}, 1), "oracle-crash")
        self.assertEqual(bd.classify(o, gone, {}, {1: "timeout"}, 1), "timeout")
        self.assertEqual(bd.classify(o, gone, {}, {1: "crash"}, 1), "crash")

    def test_files_left_behind_are_compared(self):
        o = self.f("o.rds", b"r")
        r = self.f("r.rds", b"r")
        self.f("o.files", b"[]")
        self.f("r.files", b'[["a4plant", "file", 420, 0, "e3b0"]]')
        self.assertEqual(bd.classify(o, r, {}, {}, 1), "mismatch")

    def test_a_big_value_is_compared_by_its_digest(self):
        # driver.R records a value over A4_BIG_BYTES as its summary, writes
        # the value to <id>.big<k>.rds and marks the summary <id>.big<k>.want;
        # seal_big replaces the value by its sha256, and the comparison
        # needs the same marks on both sides, each with its digest
        go, rt = (os.path.join(self.d, x) for x in ("o", "r"))
        for d in (go, rt):
            os.makedirs(d)
            with open(os.path.join(d, "000951.rds"), "wb") as f:
                f.write(b"summary: character, 100000, and an ordinary string a4_big")
            with open(os.path.join(d, "000951.big1.rds"), "wb") as f:
                f.write(b"one gigabyte")
            open(os.path.join(d, "000951.big1.want"), "w").close()
        bd.seal_big(go)
        bd.seal_big(rt)
        self.assertEqual(sorted(os.listdir(go)), ["000951.big1.sha256", "000951.big1.want", "000951.rds"])
        o, r = os.path.join(go, "000951.rds"), os.path.join(rt, "000951.rds")

        def cls():
            bd._INDEX.clear()           # (the files change between the checks)
            return bd.classify(o, r, {}, {}, 951)
        self.assertEqual(cls(), "pass")
        # the same summary, another value past its first and last elements
        with open(os.path.join(rt, "000951.big1.sha256"), "w") as f:
            f.write("0" * 64 + "\n")
        self.assertEqual(cls(), "mismatch")
        bd._INDEX.clear()
        with unittest.mock.patch.object(bd, "render", return_value={}):
            field = bd.diff_fields([(951, o, r)])[951][0]
        self.assertEqual(field, "digest")
        # the digest missing on one side, then on both
        os.remove(os.path.join(rt, "000951.big1.sha256"))
        self.assertEqual(cls(), "mismatch")
        os.remove(os.path.join(go, "000951.big1.sha256"))
        self.assertEqual(cls(), "mismatch")
        # a summary for k = 1 with only a k = 2 digest on both sides
        for d in (go, rt):
            with open(os.path.join(d, "000951.big2.sha256"), "w") as f:
                f.write("1" * 64 + "\n")
        self.assertEqual(cls(), "mismatch")
        bd._INDEX.clear()
        with unittest.mock.patch.object(bd, "render", return_value={}):
            field, gnur, rt_ = bd.diff_fields([(951, o, r)])[951]
        self.assertEqual(field, "digest")
        self.assertIn("missing [1], unexpected [2]", gnur)
        # and a case's marks and digests never mix with another case's
        with open(os.path.join(go, "000095.rds"), "wb") as f:
            f.write(b"x")
        bd._INDEX.clear()
        self.assertEqual(bd.big_digests(os.path.join(go, "000095.rds")), (frozenset(), {}))
        # a record that merely holds the string "a4_big" needs no digest
        for d in (go, rt):
            with open(os.path.join(d, "000096.rds"), "wb") as f:
                f.write(b"a4_big")
        bd._INDEX.clear()
        self.assertEqual(bd.classify(os.path.join(go, "000096.rds"), os.path.join(rt, "000096.rds"), {}, {}, 96),
                         "pass")

    def test_tree_lists_files_dirs_and_links(self):
        with tempfile.TemporaryDirectory() as d:
            os.makedirs(os.path.join(d, "sub"))
            with open(os.path.join(d, "sub", "f"), "w") as f:
                f.write("x")
            os.symlink("sub/f", os.path.join(d, "l"))
            t = bd.tree(d)
        self.assertEqual([x[:2] for x in t], [[".", "dir"], ["l", "link"], ["sub", "dir"], ["sub/f", "file"]])
        self.assertEqual(t[3][3], 1)

    def test_tree_sees_the_directory_own_mode(self):
        # Sys.chmod(".") must show even when the directory is empty
        with tempfile.TemporaryDirectory() as d:
            os.chmod(d, 0o755)
            a = bd.tree(d)
            os.chmod(d, 0o700)
            self.assertNotEqual(a, bd.tree(d))

    def test_runtimes_run_under_a_fixed_umask(self):
        r = subprocess.run(["sh", "-c", "umask"], preexec_fn=bd.cap, capture_output=True, text=True)
        self.assertEqual(int(r.stdout, 8), bd.UMASK)

    def test_data_directories_are_artifact_paths(self):
        # CI's builtin-diff-data job uploads the records as an artifact,
        # which refuses " : < > | * ? CR LF in paths: row `[<-` failed it
        doc, _ = bd.load_rows()
        for name in doc:
            for v in (name, name + "@jit"):
                self.assertFalse(set(bd.data_name(v)) & bd.ARTIFACT_REFUSES, v)
        self.assertEqual(bd.data_name("[<-"), "[%3C-")
        self.assertEqual(bd.data_name("nchar"), "nchar")
        self.assertNotEqual(bd.data_name("a%3C"), bd.data_name("a<"))   # % is escaped too

    def test_score_line(self):
        c = {"pass": 5, "mismatch": 1, "not-implemented": 2, "crash": 1, "timeout": 0, "resource": 3,
             "oracle-crash": 4}
        self.assertEqual(bd.score_line("x", c), "x: 5 / 9 (not-implemented 2, crash 1, timeout 0, "
                                                "resource 3 excluded, oracle-crash 4 excluded)")


# a stand-in driver: from A4_FROM on, each case's marker line, output, record
# and done marker; it exits at case CRASH_AT, exits halfway through writing
# the record at TORN_AT, and sleeps at HANG_AT
FAKE = textwrap.dedent("""\
    import os, sys, time
    out, start = os.environ["A4_OUT"], int(os.environ["A4_FROM"])
    if os.environ.get("UNUSABLE"):
        sys.stderr.write("Error: driver unusable on fake: withVisible\\n"); sys.exit(1)
    if os.environ.get("NOSTART"):
        sys.stderr.write("cannot boot\\n"); sys.exit(1)
    # (a start that takes half the case limit, as robigo-rt's boot and the
    # batch's readRDS can)
    time.sleep(float(os.environ.get("START_DELAY", "0")))
    open(os.path.join(out, "started"), "w").close()
    if os.environ.get("SLOT_FILE"):
        # whether the slot is held while this process runs
        import fcntl
        with open(os.environ["SLOT_FILE"], "a") as f:
            try:
                fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
                held = "free"
            except BlockingIOError:
                held = "held"
        open(os.path.join(out, "slot"), "w").write(held)
    for i in range(start, min(int(os.environ["N"]), int(os.environ.get("A4_TO", "999999"))) + 1):
        if i == int(os.environ.get("CRASH_AT", "0")):
            sys.stderr.write("thread 'main' panicked\\n"); os._exit(101)
        if i == int(os.environ.get("HANG_AT", "0")):
            time.sleep(30)
        sys.stdout.write("\\x01A4 %d\\x01\\n" % i)
        if i % 2:
            sys.stdout.write("printed by %d\\n" % i)
        sys.stdout.flush()
        with open(os.path.join(out, "%06d.rds" % i), "w") as f:
            f.write(str(i))
            if i == int(os.environ.get("TORN_AT", "0")):
                f.flush(); os._exit(101)
        open(os.path.join(out, "%06d.ok" % i), "w").close()
        if os.environ.get("MAKE_FILES"):
            open("made-%d" % i, "w").write(os.environ.get("A4_LEAK", "clean"))
        if os.environ.get("CHECK_HOME"):
            # the case's tree is still its own after a pause
            time.sleep(0.2)
            link = os.path.join(os.environ["HOME"], ".cache", "robigo-research", "rsrc")
            if not (os.path.lexists(link) and os.path.exists("made-%d" % i)):
                sys.stderr.write("the case's directory went away\\n"); os._exit(101)
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

    def test_a_crash_while_writing_a_record_costs_that_case(self):
        lost, recs, _ = self.run_fake(5, TORN_AT=3)
        self.assertEqual(lost, {3: "crash"})
        self.assertIn("000004.rds", recs)

    def test_each_case_keeps_what_it_printed(self):
        _, _, out = self.run_fake(4, CRASH_AT=2)
        got = {f: open(os.path.join(out, f)).read() for f in sorted(os.listdir(out)) if f.endswith(".out")}
        self.assertEqual(got, {"000001.out": "printed by 1\n", "000003.out": "printed by 3\n"})
        self.assertFalse([f for f in os.listdir(out) if f.startswith("stdout.")])

    def test_an_external_row_runs_each_case_alone_and_records_its_files(self):
        d = tempfile.mkdtemp()
        drv = os.path.join(d, "fake.py")
        with open(drv, "w") as f:
            f.write(FAKE)
        out = os.path.join(d, "out")
        env = {**os.environ, "N": "3", "MAKE_FILES": "1", "A4_LEAK": "leaked", "A4_LABEL": "x"}
        # (the fake driver reads N and MAKE_FILES: pass them as ENV-like keys)
        with unittest.mock.patch.object(bd, "ENV", {**bd.ENV, "N": "3", "MAKE_FILES": "1"}):
            lost = bd.run_isolated([sys.executable, drv], env, "batch", out, 3, "test-key")
        self.assertEqual(lost, {})
        for i in (1, 2, 3):
            with open(os.path.join(out, f"{i:06d}.files")) as f:
                files = json.load(f)
            # its own directory: only this case's file, and no leaked variable
            self.assertEqual([x[0] for x in files], [".", f"made-{i}"])
            self.assertEqual(files[1][3], len("clean"))

    def test_two_runs_of_one_external_row_at_once_both_pass(self):
        # as two checkouts' `ci` runs on one machine: the row's case
        # directory is one path for both (case_root), and neither run may
        # remove the other's tree mid-case (the driver checks its HOME's
        # link to GNU R's sources and its own file after a pause)
        d = tempfile.mkdtemp()
        # a key of this invocation's own, shared by its two runs: another
        # copy of this suite running at once has its own row
        key = f"test-concurrent-{os.path.basename(d)}"
        drv = os.path.join(d, "fake.py")
        with open(drv, "w") as f:
            f.write(FAKE)
        run = os.path.join(d, "run.py")
        with open(run, "w") as f:
            f.write(textwrap.dedent(f"""\
                import importlib.util, json, os, sys, unittest.mock
                spec = importlib.util.spec_from_file_location("bd", {os.path.join(HERE, "builtin-diff.py")!r})
                bd = importlib.util.module_from_spec(spec)
                spec.loader.exec_module(bd)
                keep = {{"N": "4", "MAKE_FILES": "1", "CHECK_HOME": "1"}}
                with unittest.mock.patch.object(bd, "ENV", {{**bd.ENV, **keep}}):
                    lost = bd.run_isolated([sys.executable, {drv!r}], {{**os.environ, **keep}}, "batch", sys.argv[1], 4,
                                           {key!r}, rsrc={os.path.join(d, "rsrc", "R")!r})
                print(json.dumps({{str(k): v for k, v in lost.items()}}))
                """))
        outs = [os.path.join(d, f"out{k}") for k in (1, 2)]
        ps = [subprocess.Popen([sys.executable, run, o], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
              for o in outs]
        try:
            results = [p.communicate(timeout=120) for p in ps]
        finally:
            # (a run that hangs must not keep holding the row's lock)
            for p in ps:
                if p.poll() is None:
                    p.kill()
                    p.wait()
        for p, o, (sout, serr) in zip(ps, outs, results):
            self.assertEqual(p.returncode, 0, serr)
            self.assertEqual(json.loads(sout), {}, f"{o}: cases lost")
            for i in range(1, 5):
                with open(os.path.join(o, f"{i:06d}.files")) as f:
                    self.assertEqual([x[0] for x in json.load(f)], [".", f"made-{i}"])
        self.assertFalse(os.path.lexists(bd.case_root(key)))
        os.remove(bd.case_root(key) + ".lock")

    def test_a_run_waiting_for_the_rows_lock_names_its_holder(self):
        import io
        d = tempfile.mkdtemp()
        path = os.path.join(d, "row.lock")
        with open(path, "a+") as held:
            bd.lock_row(held, path)
            with open(path, "a+") as waiting, unittest.mock.patch("sys.stderr", new_callable=io.StringIO) as err:
                slept = []

                def sleep(s):
                    # the holder lets go after the waiter's third wait
                    slept.append(s)
                    if len(slept) == 3:
                        fcntl.flock(held, fcntl.LOCK_UN)
                with unittest.mock.patch.object(bd.time, "sleep", sleep):
                    bd.lock_row(waiting, path, first=0.0, every=60.0, step=1.0)
                self.assertEqual(len(slept), 3)
                self.assertIn(f"waiting 0 s for {path}, held by another run of this row "
                              f"(last recorded holder: pid {os.getpid()})", err.getvalue())
                self.assertEqual(err.getvalue().count("waiting"), 1, "it repeats sparsely")
            with open(path) as f:
                self.assertEqual(f.read(), str(os.getpid()))

    def test_the_case_directory_is_removed_when_a_run_fails(self):
        d = tempfile.mkdtemp()
        drv = os.path.join(d, "fake.py")
        with open(drv, "w") as f:
            f.write(FAKE)
        key = f"test-unusable-{os.path.basename(d)}"
        with unittest.mock.patch.object(bd, "ENV", {**bd.ENV, "N": "2", "UNUSABLE": "1"}):
            with self.assertRaises(bd.Unusable):
                bd.run_isolated([sys.executable, drv], {**os.environ, "N": "2", "UNUSABLE": "1"}, "batch",
                                os.path.join(d, "out"), 2, key)
        self.assertFalse(os.path.lexists(bd.case_root(key)))
        os.remove(bd.case_root(key) + ".lock")

    def test_the_case_directory_mode_does_not_follow_the_harness_umask(self):
        d = tempfile.mkdtemp()
        drv = os.path.join(d, "fake.py")
        with open(drv, "w") as f:
            f.write(FAKE)
        modes = []
        for mask in (0o022, 0o077):
            out = os.path.join(d, f"out{mask}")
            old = os.umask(mask)
            try:
                with unittest.mock.patch.object(bd, "ENV", {**bd.ENV, "N": "1"}):
                    bd.run_isolated([sys.executable, drv], {**os.environ, "N": "1"}, "batch", out, 1, "test-umask")
            finally:
                os.umask(old)
            with open(os.path.join(out, "000001.files")) as f:
                modes.append(json.load(f)[0])
        self.assertEqual(modes[0], modes[1])
        self.assertEqual(modes[0][2], 0o755)

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


class SlotPool(unittest.TestCase):
    """builtin-diff's driver processes share the conformance runner's
    machine-wide slot pool (scripts/conf_slots.py)."""

    def pool(self, n=1):
        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d, ignore_errors=True)
        pool = conf_slots.Pool(d, n)
        patch = unittest.mock.patch.object(bd, "POOL", pool)
        patch.start()
        self.addCleanup(patch.stop)
        return pool

    def test_a_driver_holds_a_slot_while_it_runs(self):
        pool = self.pool()
        slot = os.path.join(pool.dir, "slot-0")
        lost, recs, out = Batches.run_fake(self, 2, SLOT_FILE=slot)
        self.assertEqual((lost, len(recs)), ({}, 2))
        with open(os.path.join(out, "slot")) as f:
            self.assertEqual(f.read(), "held")
        # and gives it back when it has ended (a probe that does not wait:
        # a leaked slot fails here at once)
        with open(slot, "a") as f:
            try:
                fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                self.fail("the driver's slot is still held after it ended")

    def test_a_cases_clock_starts_when_its_driver_takes_its_slot(self):
        # the only slot is someone else's for 2.5 s, past the 1 s limit
        # run_fake gives each case, and the driver's start takes half of
        # it: the wait is no case's time
        pool = self.pool()
        other = pool.take()
        timer = threading.Timer(2.5, other.release)
        timer.start()
        self.addCleanup(timer.cancel)
        t0 = time.monotonic()
        lost, recs, _ = Batches.run_fake(self, 2, START_DELAY=0.5)
        self.assertGreaterEqual(time.monotonic() - t0, 2.4)
        self.assertEqual((lost, len(recs)), ({}, 2))

    def test_a_hang_still_times_out_once_its_slot_is_taken(self):
        self.pool()
        lost, recs, _ = Batches.run_fake(self, 3, HANG_AT=2)
        self.assertEqual((lost, len(recs)), ({2: "timeout"}, 2))

    def test_a_pool_directory_removed_mid_run_is_made_again(self):
        # (/tmp/robigo-conf-slots-<uid> is in a place that gets cleaned)
        pool = self.pool()
        with pool.take() as held:
            self.assertEqual(held.count, 1)
        shutil.rmtree(pool.dir)
        with pool.take() as held:
            self.assertEqual(held.count, 1)
        self.assertEqual(os.stat(pool.dir).st_mode & 0o777, 0o700)
        # one that cannot be made again is the pool's error, naming it
        shutil.rmtree(pool.dir)
        open(pool.dir, "w").close()
        self.addCleanup(lambda: os.path.isfile(pool.dir) and os.remove(pool.dir))
        with self.assertRaises(conf_slots.PoolError) as e:
            pool.take()
        self.assertIn(pool.dir, str(e.exception))

    def test_a_slot_held_by_flock_1_is_held_for_the_pool(self):
        # the other side of the pool: the Rust runner's File::try_lock is
        # flock(2) (std's sys/fs/unix.rs on Linux), as flock(1) is, and as
        # the foreman's gate-slot.sh is; a slot one holds, the other waits for
        if not shutil.which("flock"):
            self.skipTest("no flock(1)")
        pool = self.pool()
        slot = os.path.join(pool.dir, "slot-0")
        holder = subprocess.Popen(["flock", "-x", slot, "sh", "-c", "echo held; sleep 2"], stdout=subprocess.PIPE,
                                  text=True)
        self.addCleanup(holder.wait)
        self.assertEqual(holder.stdout.readline().strip(), "held")
        t0 = time.monotonic()
        with pool.take() as held:
            waited = time.monotonic() - t0
            self.assertEqual(held.count, 1)
        self.assertGreater(waited, 1.0, "Pool.take did not wait for flock(1)'s slot")

    def test_a_take_that_fails_mid_run_ends_the_run_cleanly(self):
        # a pool that stops working while builtin-diff runs is its PoolError,
        # caught where a driver's refusal is (no traceback)
        pool = self.pool()
        shutil.rmtree(pool.dir)
        open(pool.dir, "w").close()
        self.addCleanup(lambda: os.path.isfile(pool.dir) and os.remove(pool.dir))
        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d, ignore_errors=True)
        out = os.path.join(d, "out")
        os.makedirs(out)
        with self.assertRaises(conf_slots.PoolError):
            bd.one_run([sys.executable, "-c", "pass"], dict(os.environ), d, out, 1, 1, 1.0)
        # nothing was opened for the run that could not start
        self.assertEqual(os.listdir(out), [])

    def test_the_pool_follows_rt_cis_rule(self):
        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d, ignore_errors=True)
        # the caller's directory wins, empty means none, CI has none
        self.assertIsNone(bd.slot_pool({"ROBIGO_CONF_SLOTS_DIR": ""}))
        self.assertIsNone(bd.slot_pool({"GITHUB_ACTIONS": "true"}))
        mine = os.path.join(d, "mine")
        pool = bd.slot_pool({"ROBIGO_CONF_SLOTS_DIR": mine, "ROBIGO_CONF_SLOTS": "3"})
        self.assertEqual((pool.dir, pool.n), (mine, 3))
        # otherwise the user's machine-wide one, as rt-ci.py's
        pool = bd.slot_pool({"XDG_RUNTIME_DIR": d})
        self.assertEqual((pool.dir, pool.n), (os.path.join(d, "robigo-conf-slots"), len(os.sched_getaffinity(0))))
        # a pool that cannot be used stops the run
        os.chmod(mine, 0o755)
        with self.assertRaises(SystemExit) as e:
            bd.slot_pool({"ROBIGO_CONF_SLOTS_DIR": mine})
        self.assertIn("not a private directory", str(e.exception))
        with self.assertRaises(SystemExit) as e:
            bd.slot_pool({"ROBIGO_CONF_SLOTS_DIR": mine + "2", "ROBIGO_CONF_SLOTS": "0"})
        self.assertIn("not a positive number", str(e.exception))


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
