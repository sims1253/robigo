#!/usr/bin/env python3
"""scripts/package-suite.py's scoring and its history record, without R.

The rules that decide what counts as compatible live here: a block nobody
ran must not pass, a skip must not improve the score, and a partial run must
not look like a smaller denominator.
"""
import argparse
import importlib.util
import json
import os
import shutil
import sys
import tempfile
import time
import unittest
from unittest import mock

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


class OwnSkips(unittest.TestCase):
    """Skips the package's own code always takes leave the floor's denominator."""
    ROW = {"version": "1.0"}

    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.path, suite.OWN_SKIPS = suite.OWN_SKIPS, os.path.join(self.dir.name, "own-skips.tsv")
        with open(suite.OWN_SKIPS, "w") as f:
            f.write("# a comment\npackage\tversion\treason\n# why\np\t1.0\tReason: never\nq\t1.0\tReason: other\n"
                    "p\t0.9\tReason: old\n")

    def tearDown(self):
        suite.OWN_SKIPS = self.path
        self.dir.cleanup()

    def mostly_skipped(self, reasons, got_skip=True, unstable=False):
        exp, got = recorded(), [block()]
        for i in range(9):
            exp.update(recorded(test=f"s{i}", passed=0, skip=1, reasons=reasons, unstable=unstable))
            got.append(block(test=f"s{i}", passed=0, skip=1, reasons=reasons) if got_skip else
                       block(test=f"s{i}", passed=0, error=1, msg="boom"))
        files = {"test-a": {"file": "test-a", "seconds": 1, "status": "done", "stable": True}}
        return suite.summarize("p", self.ROW, exp, files, {"test-a": ("done", 1, got, None)})[0]

    def test_reasons_of_this_package_and_version(self):
        self.assertEqual(suite.own_skips("p", "1.0"), {"Reason: never"})
        self.assertEqual(suite.own_skips("p", "0.9"), {"Reason: old"})
        self.assertEqual(suite.own_skips("r", "1.0"), set())

    def test_an_own_skip_leaves_the_floor(self):
        s = self.mostly_skipped(["Reason: never"])
        self.assertEqual((s["own_skips"], s["blocks"], s["n"]), (9, 10, 1))
        self.assertTrue(s["green"])

    def test_another_packages_or_versions_reason_does_not(self):
        self.assertFalse(self.mostly_skipped(["Reason: other"])["green"])
        self.assertFalse(self.mostly_skipped(["Reason: old"])["green"])
        self.assertEqual(self.mostly_skipped(["Reason: other"])["own_skips"], 0)

    def test_an_unlisted_reason_beside_a_listed_one_does_not(self):
        self.assertFalse(self.mostly_skipped(["Reason: never", "Reason: offline"])["green"])

    def test_an_unstable_skip_does_not(self):
        self.assertFalse(self.mostly_skipped(["Reason: never"], unstable=True)["green"])

    def test_robigo_must_still_skip_it(self):
        s = self.mostly_skipped(["Reason: never"], got_skip=False)
        self.assertEqual(s["skip_diverged"], 9)
        self.assertFalse(s["green"])

    def test_a_block_that_ran_before_skipping_is_not_an_own_skip(self):
        self.assertFalse(suite.own_skip({"outcome": [1, 0, 0, 1, 0, ["Reason: never"]], "unstable": False},
                                        {"Reason: never"}))


class OwnSkipsFile(unittest.TestCase):
    """tests/packages/own-skips.tsv itself."""

    def entries(self):
        out, comment = [], False
        with open(suite.OWN_SKIPS) as f:
            for line in f:
                if line.startswith("#"):
                    comment = True
                elif line.strip() and not line.startswith("package\t"):
                    out.append((line.rstrip("\n").split("\t"), comment))
                    comment = False
        return out

    def test_every_entry_cites_its_source(self):
        for fields, comment in self.entries():
            self.assertEqual(len(fields), 3, fields)
            self.assertTrue(comment, f"{fields} has no comment citing the package's source")

    def test_every_entry_is_of_the_selection(self):
        sel = suite.selection()
        for (pkg, version, reason), _ in self.entries():
            self.assertIn(pkg, sel)
            self.assertEqual(version, sel[pkg]["version"], f"{pkg}: an entry of another version")
            self.assertTrue(reason.startswith("Reason: "), reason)

    def test_every_entry_is_recorded(self):
        for (pkg, version, reason), _ in self.entries():
            exp, _ = suite.read_expected(pkg)
            self.assertTrue(any(reason in e["outcome"][5] for e in exp.values()), f"{pkg}: no block skips for {reason!r}")


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
    def test_spawn_failure_closes_logs(self):
        def fd_count():
            return len(os.listdir("/proc/self/fd"))

        before = fd_count()
        with tempfile.TemporaryDirectory() as d:
            try:
                suite.supervise(["/definitely/missing-command"], dict(os.environ), d,
                                os.path.join(d, "log"), 1, errlog=os.path.join(d, "err"))
                self.fail("expected FileNotFoundError")
            except FileNotFoundError:
                # While the exception is live, its traceback retains the
                # supervise frame in which the pre-fix code leaked handles.
                self.assertEqual(fd_count(), before)

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


def read(path):
    with open(path, errors="replace") as f:
        return f.read()


def free_port():
    import socket
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def userns():
    """Can this user make a network namespace (as the oracle's root can)?"""
    import subprocess
    try:
        return subprocess.run(["unshare", "-rn", "true"], capture_output=True).returncode == 0
    except OSError:  # no unshare here
        return False


def alive(pid):
    """Is `pid` a process that has not exited (a zombie has)?"""
    try:
        with open(f"/proc/{pid}/stat") as f:
            return f.read().rsplit(")", 1)[1].split()[0] != "Z"
    except OSError:
        return False


class Server(unittest.TestCase):
    """A package's test server runs in the worker's own network namespace."""

    def run_worker(self, worker, server_cmd, port, d):
        server = {"port": port, "cmd": server_cmd, "log": os.path.join(d, "server.log"), "limit": 20}
        if userns():
            cmd = suite.offline(worker, server)
            self.assertEqual(cmd[:2], ["unshare", "-n"])
            cmd[1] = "-rn"  # the oracle's `unshare -n`, without root
        else:
            cmd = suite.in_namespace(worker, server)
        return suite.supervise(cmd, dict(os.environ), d, os.path.join(d, "log"), 60)

    def http_server(self, port):
        return [sys.executable, "-m", "http.server", "-b", "127.0.0.1", str(port)]

    def fetch(self, url):
        return [sys.executable, "-c", f"import urllib.request; print(urllib.request.urlopen({url!r}).status)"]

    def test_the_worker_reaches_its_server(self):
        port = free_port()
        with tempfile.TemporaryDirectory() as d:
            status, rc, _ = self.run_worker(self.fetch(f"http://127.0.0.1:{port}/"),
                                            self.http_server(port), port, d)
            log = read(os.path.join(d, "log"))
            self.assertEqual((status, rc), (None, 0), log)
            self.assertEqual(log.strip(), "200")
            self.assertIn("GET / ", read(os.path.join(d, "server.log")))

    def test_the_server_goes_with_the_file(self):
        # the witness is the server's pid: in a namespace of its own, its port
        # is not visible from here whether it still runs or not
        port = free_port()
        with tempfile.TemporaryDirectory() as d:
            pidfile = os.path.join(d, "pid")
            server = [sys.executable, "-c", "import http.server, os, sys\n"
                      "open(sys.argv[1], 'w').write(str(os.getpid()))\n"
                      "http.server.HTTPServer(('127.0.0.1', int(sys.argv[2])), "
                      "http.server.SimpleHTTPRequestHandler).serve_forever()", pidfile, str(port)]
            _, rc, _ = self.run_worker(["true"], server, port, d)
            self.assertEqual(rc, 0, read(os.path.join(d, "log")))  # the server did start
            pid = int(read(pidfile))
        deadline = time.time() + 10
        while alive(pid) and time.time() < deadline:
            time.sleep(0.1)
        self.assertFalse(alive(pid), f"the server {pid} outlived its file")

    def test_a_server_that_does_not_start_stops_the_file(self):
        with tempfile.TemporaryDirectory() as d:
            _, rc, _ = self.run_worker(["true"], [sys.executable, "-c", "print('no such port'); exit(3)"],
                                       free_port(), d)
            log = read(os.path.join(d, "log"))
        self.assertNotEqual(rc, 0)
        self.assertIn("did not start", log)
        self.assertIn("no such port", log)

    @unittest.skipUnless(userns(), "needs a network namespace")
    def test_still_no_network(self):
        port = free_port()
        with tempfile.TemporaryDirectory() as d:
            _, rc, _ = self.run_worker(self.fetch("http://192.0.2.1/"), self.http_server(port), port, d)
            log = read(os.path.join(d, "log"))
        self.assertNotEqual(rc, 0)
        self.assertIn("unreachable", log)

    def test_the_servers_names_must_lead_to_the_loopback(self):
        servers = dict(suite.SERVERS)
        try:
            suite.SERVERS["loop"] = dict(suite.SERVERS["httpbin"], hosts=["localhost"])
            suite.SERVERS["nowhere"] = dict(suite.SERVERS["httpbin"], hosts=["no-such-host.invalid"])
            suite.check_hosts({"a": {"server": "loop"}, "b": {"server": ""}, "c": {}})
            with self.assertRaises(SystemExit) as e:
                suite.check_hosts({"a": {"server": "loop"}, "b": {"server": "nowhere"}})
            self.assertIn("--add-host no-such-host.invalid:127.0.0.1", str(e.exception))
            with self.assertRaises(SystemExit) as e:
                suite.check_hosts({"a": {"server": "httbin"}})  # a typo in selection.tsv
            self.assertIn("unknown test server 'httbin'", str(e.exception))
        finally:
            suite.SERVERS.clear()
            suite.SERVERS.update(servers)

    def test_httpbin_is_what_curls_helper_probes(self):
        # curl's helper probes http://httpbin.org/get whatever HTTPBIN_TEST_SERVER says
        server, env = suite.server_of({"server": "httpbin"}, "/x/server.log")
        self.assertEqual((server["port"], server["log"]), (80, "/x/server.log"))
        self.assertIn("httpbin.core", server["cmd"])
        self.assertEqual(suite.SERVERS["httpbin"]["hosts"], ["httpbin.org"])
        self.assertEqual(env, {"HTTPBIN_TEST_SERVER": "http://httpbin.org"})

    def test_no_server_by_default(self):
        self.assertEqual(suite.server_of({"server": ""}, "log"), (None, {}))
        self.assertEqual(suite.server_of({}, "log"), (None, {}))  # the canary's row
        with self.assertRaises(SystemExit):
            suite.server_of({"server": "ftp"}, "log")

    def test_the_selection_names_known_servers(self):
        rows = suite.selection()
        self.assertEqual(rows["curl"]["server"], "httpbin")
        for name, row in rows.items():
            self.assertIn(row["server"], {""} | set(suite.SERVERS), name)


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

    def test_a_broken_nightly_keeps_the_good_one(self):
        good = {"sha": "x", "packages": {"valid": True, "complete": True, "score": [5, 9]}, "corpora": {"packages": [5, 9]}}
        bad = {"sha": "x", "packages": {"valid": False, "complete": True, "score": [0, 9]}}
        merged = history.merge_entry(good, bad)
        self.assertEqual(merged["packages"]["score"], [5, 9])
        self.assertEqual(merged["corpora"]["packages"], [5, 9])

    def test_the_canary_adds_up_over_shards(self):
        docs = [self.doc([1, 2]), self.doc([2, 2], pkg="q")]
        docs[1]["canary"] = [8, 9]
        docs[1]["valid"] = False
        self.assertEqual(history.packages_from(docs, None)["canary"], [17, 18])

    def test_merge_keeps_what_the_other_run_measured(self):
        night = {"sha": "x", "packages": {}, "corpora": {"packages": [1, 2]}}
        day = {"sha": "x", "perf": {}, "corpora": {"fastr": [3, 4]}}
        merged = history.merge_entry(night, day)
        self.assertEqual(merged["corpora"], {"packages": [1, 2], "fastr": [3, 4]})
        self.assertIn("perf", merged)
        self.assertIn("packages", merged)


class Smoke(unittest.TestCase):
    EXP = ("out\n", "err\n", 0)

    def test_identical_transcript_passes(self):
        self.assertEqual(suite.smoke_state(self.EXP, ("out\n", "err\n"), None, 0), "pass")

    def test_refusal_passes_only_by_refusing_identically(self):
        # A script whose point is that it errors: GNU R's exit 1 and both
        # streams are the expected transcript, so the same refusal is a
        # pass and anything else (a clean exit, a different message) is not.
        exp = ("", "Error: boom\n", 1)
        self.assertEqual(suite.smoke_state(exp, ("", "Error: boom\n"), None, 1), "pass")
        self.assertEqual(suite.smoke_state(exp, ("", "Error: boom\n"), None, 0), "diff")
        self.assertEqual(suite.smoke_state(exp, ("done\n", ""), None, 0), "diff")

    def test_hang_where_gnu_exits_is_a_timeout_not_a_pass(self):
        exp = ("", "Error: boom\n", 1)
        self.assertEqual(suite.smoke_state(exp, ("", ""), "timeout", -9), "timeout")

    def test_stdout_stderr_and_status_all_must_match(self):
        self.assertEqual(suite.smoke_state(self.EXP, ("out\n", "other\n"), None, 0), "diff")
        self.assertEqual(suite.smoke_state(self.EXP, ("out\n", "err\n"), None, 2), "diff")

    def test_signal_death_is_a_crash(self):
        self.assertEqual(suite.smoke_state(self.EXP, ("out\n", "err\n"), None, -11), "crash")

    def test_mask_covers_addresses_and_temp_paths_only(self):
        got = suite.smoke_mask(b"<env 0x55ab12> /tmp/pkgsmoke-x/f\n0x keep\n", "/tmp/pkgsmoke-x")
        self.assertEqual(got, "<env 0x> <tmp>/f\n0x keep\n")

    def test_cases_parse_needs_and_timeout(self):
        with tempfile.TemporaryDirectory() as d:
            cases = os.path.join(d, "cases")
            os.mkdir(cases)
            open(os.path.join(cases, "x.R"), "w").write("# needs: dplyr pillar\n# timeout: 7\n1\n")
            old = suite.SMOKE_CASES
            suite.SMOKE_CASES = cases
            try:
                parsed = suite.smoke_cases()
            finally:
                suite.SMOKE_CASES = old
        self.assertEqual(parsed["x"][1:], (["dplyr", "pillar"], 7))

    def test_history_records_smoke_and_renders_it(self):
        with tempfile.TemporaryDirectory() as d:
            with open(os.path.join(d, "smoke.json"), "w") as f:
                json.dump({"kind": "smoke", "pass": 1, "scored": 2,
                           "cases": {"a": {"state": "pass"}, "b": {"state": "timeout"}},
                           "pending": ["b"]}, f)
            e = history.entry_from(d, "x" * 40, "2026-11-07", "t")
        self.assertEqual(e["smoke"]["cases"], {"a": "pass", "b": "timeout"})
        page = history.packages_section([e], "https://x")
        self.assertIn("Package scripts as users run them", page)
        self.assertIn("timeout", page)

    def test_every_case_needs_only_installed_shape_names(self):
        for name, (_path, needs, _limit) in suite.smoke_cases().items():
            for pkg in needs:
                self.assertRegex(pkg, r"^[A-Za-z][A-Za-z0-9.]*$", f"{name}: {pkg}")




class OwnSelection(unittest.TestCase):
    def test_own_selection_parses_and_excludes(self):
        sel = suite.own_selection()
        all_sel = suite.own_selection(all_rows=True)
        # at least the 5 required packages are present (runnable or listed)
        for p in ("dagriculture", "bayesfam", "bayesim", "huerd", "bayesgrove"):
            self.assertIn(p, all_sel, p)
        # runnable packages have commit and repo
        for name, row in sel.items():
            self.assertTrue(row.get("commit"), name)
            self.assertFalse(row.get("reason_excluded"), name)
        # excluded packages have a reason
        for name, row in all_sel.items():
            if name not in sel:
                self.assertTrue(row.get("reason_excluded"), name)

    def test_own_pins_give_versions(self):
        # every runnable package has a pin with a version, and nothing else
        # (not the header row) is a pin
        pins = suite.own_pins()
        self.assertEqual(set(pins), set(suite.own_selection()))
        for name, pin in pins.items():
            self.assertEqual(pin["commit"], suite.own_selection()[name]["commit"], name)
            self.assertTrue(pin["version"], name)


class OwnHistory(unittest.TestCase):
    def doc(self, shard, m=1, n=2, valid=True, pkg="dagriculture"):
        return {"kind": "own-packages", "snapshot": "own:abc", "lock": "l", "selection": 1, "valid": valid,
                "canary": [9, 9], "shard": shard, "regressions": [], "causes": [["why", n - m, [pkg]]],
                "excluded": {"bayeshear": "needs Stan"},
                "pkgs": {pkg: {"m": m, "n": n, "green": m == n, "v": "0.3.0", "s": 1, "gnu_s": 1,
                               "files": {"done": 1}}}}

    def entry(self, docs):
        with tempfile.TemporaryDirectory() as d:
            for i, doc in enumerate(docs):
                with open(os.path.join(d, f"own-packages.shard{i + 1}.json"), "w") as f:
                    json.dump(doc, f)
            return history.entry_from(d, "a" * 40, "2026-11-08", "t")

    def test_own_packages_recorded_separately(self):
        e = self.entry([self.doc([1, 1])])
        self.assertIn("own_packages", e)
        self.assertNotIn("packages", e)
        self.assertNotIn("packages", e.get("corpora", {}))
        self.assertEqual(e["own_packages"]["score"], [1, 2])
        self.assertEqual(e["own_packages"]["excluded"], {"bayeshear": "needs Stan"})

    def test_own_section_renders(self):
        e = self.entry([self.doc([1, 1])])
        page = history.own_packages_section([e], "https://x")
        self.assertIn("Own R packages", page)
        self.assertIn("dagriculture", page)
        self.assertIn("bayeshear", page)

    def test_own_section_from_a_real_result(self):
        # scripts/testdata/own-packages.json: `run --own` of 2026-11-09 on
        # main's binary (37f9b847), less its per-block `matching` list
        with tempfile.TemporaryDirectory() as d:
            shutil.copy(os.path.join(HERE, "testdata", "own-packages.json"), d)
            e = history.entry_from(d, "37f9b847" + "0" * 32, "2026-11-09", "t")
        own = e["own_packages"]
        self.assertTrue(history.trusted(own))
        self.assertEqual(own["score"], [2563, 2566])
        self.assertEqual((own["green"], own["selection"]), (4, 6))
        page = history.own_packages_section([e], "https://x")
        self.assertIn("<strong>99.9%</strong> of 2,566 test blocks", page)
        self.assertIn("<strong>4 of 6</strong> packages are fully green", page)
        self.assertIn("only 3 of 13 blocks scorable", page)
        self.assertIn("bayesim", page)
        self.assertIn("<code>bayeshear</code>", page)
        self.assertNotIn("no score", page)
        # not green with every block scorable (extra output): no scorable note
        own["pkgs"]["bayesim"].update(m=663, n=663, blocks=663, green=False)
        page = history.own_packages_section([e], "https://x")
        self.assertNotIn("of 663 blocks scorable", page)

    def test_lock_own_needs_the_cran_project(self):
        with tempfile.TemporaryDirectory() as d, \
                mock.patch.object(suite, "PROJECT", os.path.join(d, "rproject.toml")), \
                mock.patch.object(suite, "own_selection", return_value={"x": {"commit": "c" * 40}}), \
                mock.patch.object(suite, "own_description", return_value={"Version": "1"}), \
                mock.patch.object(suite, "write_own_pins"):
            with self.assertRaises(SystemExit):
                suite.cmd_lock_own(None)

    def test_merge_keeps_good_own_packages(self):
        good = {"sha": "x", "own_packages": {"valid": True, "complete": True, "score": [5, 9]}}
        bad = {"sha": "x", "own_packages": {"valid": False, "complete": True, "score": [0, 9]}}
        self.assertEqual(history.merge_entry(good, bad)["own_packages"]["score"], [5, 9])


class OwnHistoryPanel(unittest.TestCase):
    """The own panel's headline counts against the declared selection and
    only trusted runs (canary held, every shard reported) set a score or a
    trend point."""

    def doc(self, m, valid=True, shard=(1, 1)):
        return {"kind": "own-packages", "snapshot": "own:abc", "lock": "l", "selection": 6, "valid": valid,
                "canary": [9, 9] if valid else [8, 9], "shard": list(shard), "regressions": [], "causes": [],
                "excluded": {}, "pkgs": {"dagriculture": {"m": m, "n": 4, "green": m == 4, "v": "0.3.0", "s": 1,
                                                           "gnu_s": 1, "files": {"done": 1}}}}

    def entry(self, doc):
        with tempfile.TemporaryDirectory() as d:
            with open(os.path.join(d, "own-packages.json"), "w") as f:
                json.dump(doc, f)
            return history.entry_from(d, "a" * 40, "2026-11-09", "t")

    def test_green_is_out_of_the_selection(self):
        page = history.own_packages_section([self.entry(self.doc(4))], "https://x")
        self.assertIn("<strong>1 of 6</strong> packages", page)
        self.assertIn("100.0%", page)

    def test_an_invalid_run_sets_no_score(self):
        page = history.own_packages_section([self.entry(self.doc(4, valid=False))], "https://x")
        self.assertIn("<strong>no score</strong>", page)
        self.assertIn("canary failed", page)

    def test_an_incomplete_run_sets_no_score(self):
        page = history.own_packages_section([self.entry(self.doc(4, shard=(1, 2)))], "https://x")
        self.assertIn("<strong>no score</strong>", page)
        self.assertIn("incomplete", page)

    def test_trends_skip_untrusted_runs(self):
        entries = [self.entry(self.doc(1)), self.entry(self.doc(0, valid=False)), self.entry(self.doc(3))]
        seen = []
        real = history.sparkline
        history.sparkline = lambda pts, n, log=True: seen.append(pts) or ""
        try:
            history.own_packages_section(entries, "https://x")
        finally:
            history.sparkline = real
        self.assertEqual(seen, [[(0, 0.25, ""), (2, 0.75, "")]])


class OwnClosure(unittest.TestCase):
    def test_framework_and_suggests_minus_what_cannot_install(self):
        sel = {"a": {"framework": "testthat"}, "b": {"framework": "tinytest"}}
        descs = {"a": {"Suggests": "testthat (>= 3.0.0), logitnorm, cmdstanr, b, roxygen2"},
                 "b": {"Suggests": "stats, posterior (>= 1.3.1),\n  withr"}}
        deps = suite.own_test_deps(sel, desc=lambda name, row: descs[name])
        self.assertEqual(deps, ["logitnorm", "posterior", "testthat", "tinytest", "withr"])

    def test_the_lock_has_every_framework_and_suggest(self):
        # what lock --own wrote: the committed closure carries each one
        names = {p["name"] for p in suite.locked("own")["packages"]}
        with open(suite.OWN_PROJECT, "rb") as f:
            wanted = {d["name"] for d in suite.tomllib.load(f)["project"]["dependencies"]}
        self.assertIn("testthat", wanted)
        self.assertLessEqual(wanted, names)
        self.assertLessEqual(set(suite.own_selection()), names)


class OwnRun(unittest.TestCase):
    def test_groups_write_their_own_results(self):
        self.assertNotEqual(suite.result_path("own"), suite.result_path("cran"))
        self.assertTrue(suite.result_path("own").endswith("own-packages.json"))

    def test_an_own_run_after_a_cran_run_keeps_both_results(self):
        s = {"m": 1, "n": 1, "green": True, "files": {"done": 1}, "s": 1, "gnu_s": 1}
        stubs = {"in_oracle": lambda: None, "sync": lambda: None, "sync_own": lambda: None,
                 "library": lambda group="cran": f"/{group}", "check_hosts": lambda names: None,
                 "check_canary": lambda lib: [9, 9], "read_expected": lambda pkg, group="cran": ({}, {}),
                 "run_package": lambda *a, **kw: {}, "summarize": lambda *a: (dict(s), set(), {}, []),
                 "markers": lambda pkg, group="cran": set(), "hand_back": lambda *a: None}
        with tempfile.TemporaryDirectory() as d, mock.patch.multiple(suite, CACHE=d, **stubs):
            cran = next(iter(suite.selection()))
            for own, pkg in ((False, cran), (True, "ciecam16")):
                a = argparse.Namespace(own=own, packages=[pkg], shard=None, out=None, update=False)
                with self.assertRaises(SystemExit) as done:
                    suite.cmd_run(a)
                self.assertEqual(done.exception.code, 0)
            with open(os.path.join(d, "results", "packages.json")) as f:
                self.assertEqual((json.load(f)["kind"]), "packages")
            with open(os.path.join(d, "results", "own-packages.json")) as f:
                self.assertEqual(list(json.load(f)["pkgs"]), ["ciecam16"])

    def test_the_canary_runs_against_the_groups_library(self):
        calls = []
        real = (suite.run_file, suite.canary_library)
        suite.run_file = lambda engine, pkg, row, path, lib, limit: calls.append(lib) or ("done", 1, [], None)
        suite.canary_library = lambda: "/c"
        try:
            suite.check_canary("/own-lib")
        finally:
            suite.run_file, suite.canary_library = real
        self.assertEqual(calls, ["/c:/own-lib"])

    def test_the_canary_installs_into_an_empty_cache(self):
        ran = []
        real = (suite.CACHE, suite.subprocess.run)
        with tempfile.TemporaryDirectory() as d:
            suite.CACHE = d
            suite.subprocess.run = lambda cmd, **kw: ran.append(cmd)
            try:
                lib = suite.canary_library()
            finally:
                suite.CACHE, suite.subprocess.run = real
            self.assertTrue(lib.startswith(d))
        self.assertEqual(ran[0][:3], ["R", "CMD", "INSTALL"])

    def test_a_failed_own_install_fails_sync_own(self):
        def fail(cmd, **kw):
            raise suite.subprocess.CalledProcessError(1, cmd)
        real = (suite.CACHE, suite.subprocess.run)
        with tempfile.TemporaryDirectory() as d:
            suite.CACHE = d
            suite.subprocess.run = fail
            try:
                with self.assertRaises(suite.subprocess.CalledProcessError):
                    suite.sync_own()
            finally:
                suite.CACHE, suite.subprocess.run = real

    def test_a_package_rv_did_not_install_fails_sync_own(self):
        real = (suite.CACHE, suite.subprocess.run)
        with tempfile.TemporaryDirectory() as d:
            suite.CACHE = d
            suite.subprocess.run = lambda cmd, **kw: None  # "succeeds", installs nothing
            try:
                with self.assertRaises(SystemExit):
                    suite.sync_own()
            finally:
                suite.CACHE, suite.subprocess.run = real

    def test_a_failed_own_install_fails_the_smoke_run(self):
        real = (suite.sync, suite.sync_own)
        suite.sync = lambda: None

        def fail():
            raise suite.subprocess.CalledProcessError(1, ["rv", "sync"])
        suite.sync_own = fail
        try:
            with self.assertRaises(suite.subprocess.CalledProcessError):
                suite.smoke_library()
        finally:
            suite.sync, suite.sync_own = real

    def test_only_an_excluded_own_package_may_be_absent(self):
        self.assertEqual(suite.smoke_absent(["bayeshear"], set()), ["bayeshear"])
        with self.assertRaises(SystemExit):
            suite.smoke_absent(["dagriculture"], set())
        self.assertEqual(suite.smoke_absent(["dagriculture", "stats"], {"dagriculture"}), [])

    def test_sync_own_without_a_lock_says_how_to_make_one(self):
        real = suite.OWN_LOCK
        suite.OWN_LOCK = "/nonexistent/own-rv.lock"
        try:
            with self.assertRaises(SystemExit):
                suite.sync_own()
        finally:
            suite.OWN_LOCK = real

    def test_absorb_refuses_a_result_with_no_lock_to_check(self):
        real = suite.OWN_LOCK
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "own-packages.json")
            with open(path, "w") as f:
                json.dump({"kind": "own-packages", "valid": True, "canary": [1, 1], "lock": "x",
                           "matching": {"ciecam16": ["a"]}}, f)
            suite.OWN_LOCK = os.path.join(d, "missing.lock")
            added = []
            real_add = suite.add_markers
            suite.add_markers = lambda *a: added.append(a)
            try:
                with self.assertRaises(SystemExit):
                    suite.cmd_absorb(argparse.Namespace(results=[path]))
            finally:
                suite.OWN_LOCK, suite.add_markers = real, real_add
            self.assertEqual(added, [])


if __name__ == "__main__":
    unittest.main()
