"""Regression checks for content-based Rdiff ignore blocks."""
import argparse
import contextlib
import io
import json
import os
import socket
import subprocess
import tempfile
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("corpus", Path(__file__).with_name("corpus.py"))
corpus = importlib.util.module_from_spec(spec)
spec.loader.exec_module(corpus)


class RecordingTests(unittest.TestCase):
    def test_record_omits_masks_inside_ignore_block(self):
        before = b"stable\n> ## IGNORE_RDIFF_BEGIN\nold host\n> ## IGNORE_RDIFF_END\nold outside\n"
        after = before.replace(b"old", b"new")
        previous = {"id": "case", "out": corpus.enc(before), "err": corpus.enc(b""), "status": 0}
        args = argparse.Namespace(name="fixture", profile=None, filter=None, all=True, repeat=2,
                                  mask_against_previous=True, jobs=1)
        with patch.object(corpus, "corpus_conf", return_value={}), \
             patch.object(corpus, "build_corpus"), \
             patch.object(corpus, "load_cases", return_value=[("case", b"1")]), \
             patch.object(corpus, "load_expected", return_value={"case": previous}), \
             patch.object(corpus.subprocess, "run") as version, \
             patch.object(corpus, "run_case", return_value=(after, b"", 0)), \
             patch.object(corpus, "write_expected") as write:
            version.return_value.stdout = "R version 4.5.0\n"
            corpus.cmd_record(args)
        record = write.call_args.args[1]["case"]
        self.assertEqual(record["mask_out"], [4])
        self.assertEqual(corpus.dec(record["out"]), after)

    def test_ignore_block_compares_across_environment_sizes(self):
        for n in (0, 1, 40, 160):
            output = b"prefix\n" + corpus.IGNORE_BEGIN + b"\n" + b"env\n" * n + corpus.IGNORE_END + b"\nsuffix\n"
            self.assertEqual(corpus.masked(output, []), b"prefix\n" + corpus.IGNORE_END + b"\nsuffix\n")


class ProfileTests(unittest.TestCase):
    def test_profile_uses_separate_oracle_and_results_with_same_ratchet(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "fixture").mkdir()
            (root / "corpora.toml").write_text(
                '[fixture]\ndefault_packages = "NULL"\n'
                '[fixture.profiles.methods]\ndefault_packages = "methods"\n'
                'expected_file = "expected-methods.jsonl.gz"\n')
            (root / "fixture/passing").mkdir()
            (root / "fixture/passing/case.pass").write_text("")
            base = {"case": {"id": "case", "sha": corpus.sha(b"1"),
                              "out": corpus.enc(b"base"), "err": corpus.enc(b""), "status": 0}}
            candidate = {"case": dict(base["case"], out=corpus.enc(b"methods"))}
            with patch.object(corpus, "CORPORA", tmp), patch.object(corpus, "STATE", str(root / "state")), \
                 patch.object(corpus, "load_cases", return_value=[("case", b"1")]), \
                 patch.object(corpus, "run_case", return_value=(b"methods", b"", 0)), \
                 contextlib.redirect_stdout(io.StringIO()):
                corpus.write_expected("fixture", base)
                corpus.write_expected("fixture", candidate, "expected-methods.jsonl.gz")
                args = argparse.Namespace(name="fixture", profile=None, update=False,
                    robigo=__file__, filter=None, shard=None, tier=None, jobs=1, keep_output=False, quiet=True)
                self.assertEqual(corpus.cmd_run(args), 1)
                args.profile = "methods"
                self.assertEqual(corpus.cmd_run(args), 0)
                default_result = root / "state/results/fixture.json"
                profile_result = root / "state/results/fixture.methods.json"
                self.assertEqual(json.loads(default_result.read_text())["summary"]["regressions"], ["case"])
                self.assertEqual(json.loads(profile_result.read_text())["summary"]["regressions"], [])
                self.assertEqual(corpus.load_expected("fixture"), base)
                with self.assertRaises(SystemExit):
                    corpus.cmd_absorb(argparse.Namespace(files=[str(profile_result)]))
                args.update = True
                with self.assertRaises(SystemExit):
                    corpus.cmd_run(args)
                self.assertTrue((root / "fixture/passing/case.pass").is_file())
                self.assertFalse((root / "fixture/passing.txt").exists())


def fixture_corpus(root):
    """A corpus directory and manifest, as corpus.py reads them."""
    (root / "fixture").mkdir()
    (root / "corpora.toml").write_text('[fixture]\ndefault_packages = "NULL"\n')
    (root / "fixture/passing").mkdir()
    return {"case": {"id": "case", "sha": corpus.sha(b"1"),
                     "out": corpus.enc(b"out"), "err": corpus.enc(b""), "status": 0}}


class VersionTests(unittest.TestCase):
    def test_versions_keep_case_ids_in_their_own_files(self):
        self.assertEqual(corpus.expected_file({}, "4.5.0"), "expected.jsonl.gz")
        self.assertEqual(corpus.expected_file({}, "4.6.2"), "R-4.6.2/expected.jsonl.gz")
        self.assertEqual(corpus.expected_file({"expected_file": "expected"}, "4.6.2"), "R-4.6.2/expected")
        self.assertIn("4.6.2", corpus.oracle_versions())
        with patch.dict(corpus.os.environ, {"ROBIGO_R_VERSION": ""}):
            self.assertEqual(corpus.r_version(), "4.5.0")
        with self.assertRaises(SystemExit), contextlib.redirect_stderr(io.StringIO()):
            corpus.r_version("4.6.1")  # the host's R, never an oracle

    def test_other_version_run_reports_without_the_ratchet(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "fixture/passing").mkdir(parents=True)
            (root / "corpora.toml").write_text('[fixture]\ndefault_packages = "NULL"\n')
            (root / "fixture/passing/case.pass").write_text("")
            rec = {"id": "case", "sha": corpus.sha(b"1"), "out": corpus.enc(b"4.5.0"), "err": corpus.enc(b""),
                   "status": 0}
            with patch.object(corpus, "CORPORA", tmp), patch.object(corpus, "STATE", str(root / "state")), \
                 patch.object(corpus, "load_cases", return_value=[("case", b"1"), ("new", b"2")]), \
                 patch.object(corpus, "run_case", return_value=(b"4.6.2", b"", 0)), \
                 contextlib.redirect_stdout(io.StringIO()):
                corpus.write_expected("fixture", {"case": rec})
                corpus.write_expected("fixture", {"case": dict(rec, out=corpus.enc(b"4.6.2")),
                                                  "new": dict(rec, id="new", sha=corpus.sha(b"2"))},
                                      corpus.expected_file({}, "4.6.2"))
                args = argparse.Namespace(name="fixture", profile=None, update=False, r_version="4.6.2",
                    robigo=__file__, filter=None, shard=None, tier=None, jobs=1, keep_output=False, quiet=True)
                self.assertEqual(corpus.cmd_run(args), 0)
                result = root / "state/results/fixture.R-4.6.2.json"
                summary = json.loads(result.read_text())["summary"]
                self.assertEqual((summary["r_version"], summary["pass"], summary["new"]), ("4.6.2", 1, []))
                with self.assertRaises(SystemExit):
                    corpus.cmd_absorb(argparse.Namespace(files=[str(result)]))
                args.update = True
                with self.assertRaises(SystemExit):
                    corpus.cmd_run(args)
                self.assertEqual(sorted(p.name for p in (root / "fixture/passing").iterdir()), ["case.pass"])
                # the 4.5.0 run still holds the ratcheted case to 4.5.0's output
                args.update, args.r_version = False, None
                self.assertEqual(corpus.cmd_run(args), 1)


class PinTests(unittest.TestCase):
    """scripts/oracle-versions is the one list of pinned versions: each has
    a tarball pin, and CI's references to the oracles name them."""

    def test_every_pin_is_an_oracle_version(self):
        pins = corpus.pins()
        self.assertEqual(set(pins), {"gated", "rt"})
        self.assertEqual(corpus.GATED_R_VERSION, pins["gated"])
        for v in pins.values():
            self.assertIn(v, corpus.oracle_versions())

    def test_pins_refuse_what_the_format_does_not_allow(self):
        # corpus.pins() and scripts/oracle-pins.sh (the shell scripts' reader)
        good = "# rt=9.9.9 in a comment\n\ngated=4.5.0\nrt=4.6.2\n"
        bad = ["rt=4.6.2\r\n", "rt = 4.6.2\n", " rt=4.6.2\n", "rt=4.6\n", "rt=4.6.2.1\n",
               "rt=4.6.o\n", "rt=v4.6.2\n", "rt=\n", "RT=4.6.2\n", "rt\n", "rt=4.6.2\nrt=4.6.3\n"]
        checker = os.path.join(corpus.ROOT, "scripts", "oracle-pins.sh")
        with tempfile.TemporaryDirectory() as tmp:
            f = Path(tmp, "scripts", "oracle-versions")
            f.parent.mkdir()
            shell = lambda: subprocess.run(["sh", "-c", '. "$1" && printf "%s %s" "$gated" "$rt"', "-", checker],
                                           cwd=tmp, capture_output=True, text=True)
            with patch.object(corpus, "PINS", str(f)):
                f.write_bytes(good.encode())
                self.assertEqual(corpus.pins(), {"gated": "4.5.0", "rt": "4.6.2"})
                self.assertEqual(shell().stdout, "4.5.0 4.6.2")
                for text in bad:
                    f.write_bytes(text.encode())
                    with self.assertRaises(SystemExit, msg=repr(text)):
                        corpus.pins()
                    self.assertNotEqual(shell().returncode, 0, repr(text))
                # the shell's reader also needs both pins
                for text in ["gated=4.5.0\n", "rt=4.6.2\n"]:
                    f.write_bytes(text.encode())
                    r = shell()
                    self.assertNotEqual(r.returncode, 0, repr(text))
                    self.assertIn("= line", r.stderr)

    def test_the_shell_reads_the_same_pins(self):
        # regen-expected.sh, ci-image.sh, base-subset.sh and the rt builtins'
        # record.sh read them through scripts/oracle-pins.sh
        out = subprocess.run(["sh", "-c", ". scripts/oracle-pins.sh && printf '%s %s' \"$gated\" \"$rt\""],
                             cwd=corpus.ROOT, capture_output=True, text=True, check=True).stdout
        pins = corpus.pins()
        self.assertEqual(out, f"{pins['gated']} {pins['rt']}")

    def test_ci_names_the_pinned_oracles(self):
        import re
        ci = Path(corpus.ROOT, ".github", "workflows", "ci.yml").read_text()
        pins = corpus.pins()
        rt = pins["rt"]
        # the rt oracle image (scripts/ci-image.sh's output) and its sources
        self.assertEqual(set(re.findall(r"image_oracle_(\w+)", ci)), {rt.replace(".", "_")})
        self.assertEqual(set(re.findall(r"r-(\d+\.\d+\.\d+)-tarball", ci)), {rt})
        # the gated version's sources (the base-package natives)
        self.assertEqual(set(re.findall(r"r-(\d+\.\d+\.\d+)-sources", ci)), {pins["gated"]})


class RatchetTests(unittest.TestCase):
    """The ratchet is one marker file per case: additions never touch a
    shared file, and a marker that names no case fails loudly."""

    def test_one_marker_per_case(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "fixture/passing").mkdir(parents=True)
            with patch.object(corpus, "CORPORA", tmp), \
                 patch.object(corpus, "load_cases", return_value=[("a", b"1"), ("S4/x/y", b"2")]):
                self.assertEqual(corpus.add_passing("fixture", {"a", "S4/x/y"}), ["S4/x/y", "a"])
                self.assertTrue((root / "fixture/passing/a.pass").is_file())
                self.assertTrue((root / "fixture/passing/S4/x/y.pass").is_file())
                self.assertEqual(corpus.add_passing("fixture", {"a"}), [])
                self.assertEqual(corpus.load_passing("fixture"), {"a", "S4/x/y"})

    def test_dangling_marker_fails_loudly(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "fixture/passing").mkdir(parents=True)
            (root / "fixture/passing/ghost.pass").write_text("")
            with patch.object(corpus, "CORPORA", tmp), \
                 patch.object(corpus, "load_cases", return_value=[("case", b"1")]), \
                 self.assertRaises(SystemExit) as e:
                corpus.load_passing("fixture")
            self.assertIn("ghost", str(e.exception))

    def test_stray_file_fails_loudly(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "fixture/passing").mkdir(parents=True)
            (root / "fixture/passing/notes.txt").write_text("x")
            with patch.object(corpus, "CORPORA", tmp), \
                 patch.object(corpus, "load_cases", return_value=[("case", b"1")]), \
                 self.assertRaises(SystemExit) as e:
                corpus.load_passing("fixture")
            self.assertIn("not a ratchet marker", str(e.exception))

    def test_legacy_list_is_still_read(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "fixture").mkdir()
            (root / "fixture/passing.txt").write_text("# comment\ncase\n")
            with patch.object(corpus, "CORPORA", tmp), \
                 patch.object(corpus, "load_cases", return_value=[("case", b"1")]), \
                 contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(corpus.load_passing("fixture"), {"case"})

    def test_run_update_creates_a_marker(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            exp = fixture_corpus(root)
            with patch.object(corpus, "CORPORA", tmp), patch.object(corpus, "STATE", str(root / "state")), \
                 patch.object(corpus, "load_cases", return_value=[("case", b"1")]), \
                 patch.object(corpus, "run_case", return_value=(b"out", b"", 0)), \
                 contextlib.redirect_stdout(io.StringIO()):
                corpus.write_expected("fixture", exp)
                args = argparse.Namespace(name="fixture", profile=None, update=True,
                    robigo=__file__, filter=None, shard=None, tier=None, jobs=1, keep_output=False, quiet=True)
                self.assertEqual(corpus.cmd_run(args), 0)
            self.assertTrue((root / "fixture/passing/case.pass").is_file())

    def test_absorb_creates_a_marker(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            exp = fixture_corpus(root)
            result = root / "result.json"
            result.write_text(json.dumps({"summary": {"corpus": "fixture", "profile": None},
                                          "results": [{"id": "case", "pass": True}]}))
            with patch.object(corpus, "CORPORA", tmp), patch.object(corpus, "STATE", str(root / "state")), \
                 patch.object(corpus, "load_cases", return_value=[("case", b"1")]), \
                 contextlib.redirect_stdout(io.StringIO()) as out:
                corpus.write_expected("fixture", exp)
                corpus.cmd_absorb(argparse.Namespace(files=[str(result)]))
            self.assertTrue((root / "fixture/passing/case.pass").is_file())
            self.assertIn("1 new pass", out.getvalue())


class TierTests(unittest.TestCase):
    """`run --tier` sets the tier's environment, writes its own result file
    against the shared ratchet, and never feeds the ratchet."""

    def test_tier_run_is_gated_and_kept_apart(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            exp = fixture_corpus(root)
            (root / "fixture/passing/case.pass").write_text("")
            seen = []

            def fake_run_case(name, conf, cid, src, runtime, robigo=None, tier=None):
                seen.append(tier)
                return (b"out" if tier is None else b"tier differs", b"", 0)

            with patch.object(corpus, "CORPORA", tmp), patch.object(corpus, "STATE", str(root / "state")), \
                 patch.object(corpus, "load_cases", return_value=[("case", b"1")]), \
                 patch.object(corpus, "run_case", side_effect=fake_run_case), \
                 contextlib.redirect_stdout(io.StringIO()):
                corpus.write_expected("fixture", exp)
                args = argparse.Namespace(name="fixture", profile=None, update=False, robigo=__file__,
                    filter=None, shard=None, tier=None, jobs=1, keep_output=False, quiet=True)
                self.assertEqual(corpus.cmd_run(args), 0)
                args.tier = "vm"
                self.assertEqual(corpus.cmd_run(args), 1)
                result = root / "state/results/fixture.vm.json"
                doc = json.loads(result.read_text())
                self.assertEqual(doc["summary"]["tier"], "vm")
                self.assertEqual(doc["summary"]["regressions"], ["case"])
                self.assertTrue((root / "state/results/fixture.json").is_file())
                with self.assertRaises(SystemExit):
                    corpus.cmd_absorb(argparse.Namespace(files=[str(result)]))
                args.update = True
                with self.assertRaises(SystemExit):
                    corpus.cmd_run(args)
            self.assertEqual(seen, [None, "vm"])

    def test_tier_environment(self):
        captured = {}

        def fake(cmd, cwd, env, capture_output, timeout):
            captured.update(env)
            return argparse.Namespace(stdout=b"", stderr=b"", returncode=0)

        with patch.object(corpus.subprocess, "run", side_effect=fake):
            corpus.run_case("fixture", {}, "case", b"1", "robigo", "robigo", "vm")
        self.assertEqual((captured["ROBIGO_COMPILE"], captured["ROBIGO_JIT"]), ("1", "0"))
        with patch.object(corpus.subprocess, "run", side_effect=fake):
            corpus.run_case("fixture", {}, "case", b"1", "robigo", "robigo", "interp")
        self.assertEqual((captured["ROBIGO_COMPILE"], captured["ROBIGO_JIT"]), ("0", "0"))


class GcNormalization(unittest.TestCase):
    def test_allocator_counts_and_na_compare_equal(self):
        oracle = (b'[1] "finalizer: 456"\n'
                  b'         used (Mb) gc trigger (Mb) max used (Mb)\n'
                  b'Ncells 340001 18.2 707788 37.8 615097 32.9\n'
                  b'Vcells 610351 4.7 8388608 64.0 1973852 15.1\n')
        runtime = (b'[1] "finalizer: 456"\n'
                   b'       used (Mb) gc trigger (Mb) max used (Mb)\n'
                   b'Ncells NA NA NA NA NA NA\nVcells NA NA NA NA NA NA\n')
        self.assertEqual(corpus.normalize(oracle), corpus.normalize(runtime))
        self.assertNotEqual(corpus.normalize(oracle), corpus.normalize(runtime.replace(b'456', b'123')))
        self.assertNotEqual(corpus.normalize(oracle), corpus.normalize(runtime.replace(b'[1] "finalizer: 456"\n', b'')))
        self.assertEqual(corpus.normalize(oracle).count(b'\n'), oracle.count(b'\n'))
        self.assertEqual(corpus.normalize(corpus.normalize(oracle)), corpus.normalize(oracle))

    def test_limit_columns(self):
        self.assertEqual(corpus.normalize(b'Vcells 1 2.0 3 4.0 NA 6.0 7 8.0\n'),
                         b'Vcells <gc masked>\n')

    def test_other_output_is_preserved(self):
        for output in (b'Ncells are missing\n', b'Vcells 1 2 3\n',
                       b'Ncells 1 2 3 4 5 6 7\n', b'foo Ncells 1 2 3 4 5 6\n',
                       b'[1] 340001 18.2 707788 37.8 615097 32.9\n'):
            self.assertEqual(corpus.normalize(output), output)


class PerCaseExpectedTests(unittest.TestCase):
    def test_per_case_directory_round_trips_and_drops_removed_cases(self):
        with tempfile.TemporaryDirectory() as tmp:
            (Path(tmp) / "fixture").mkdir()
            rec = lambda i, out: {"id": i, "sha": "x", "out": corpus.enc(out), "err": corpus.enc(b""), "status": 0}
            exp = {"a": rec("a", b"1"), "b": rec("b", b"\xff")}
            with patch.object(corpus, "CORPORA", tmp):
                corpus.write_expected("fixture", exp, "expected")
                files = sorted(p.name for p in (Path(tmp) / "fixture/expected").iterdir())
                self.assertEqual(files, ["a.json", "b.json"])
                self.assertEqual(corpus.load_expected("fixture", "expected"), exp)
                a = Path(tmp) / "fixture/expected/a.json"
                before = a.stat().st_mtime_ns
                corpus.write_expected("fixture", {"a": exp["a"]}, "expected")
                self.assertEqual(a.stat().st_mtime_ns, before)  # unchanged record: not rewritten
                self.assertEqual(corpus.load_expected("fixture", "expected"), {"a": exp["a"]})



class NetFixtureTests(unittest.TestCase):
    """gnur-tests' net_fixture: what sockets.R fetches, served on loopback."""
    WWW = os.path.join(corpus.cdir("gnur-tests"), "www")

    @staticmethod
    def request(method):
        # sockets.R's requests, byte for byte
        return (f"{method} /inet-tests/ch11b.dat HTTP/1.0\r\nHost: developer.R-project.org\r\n"
                "Connection: Keep-Alive\r\nAccept: text/plain\r\n\r\n").encode()

    def test_head_then_get_on_one_connection(self):
        body = Path(self.WWW, "inet-tests", "ch11b.dat").read_bytes()
        srv = corpus.start_fixture_server(self.WWW, port=0)
        try:
            with socket.create_connection(srv.server_address, timeout=10) as s:
                buf = b""

                def until(n=None):
                    # the headers (n None) or n bytes of body
                    nonlocal buf
                    while (b"\r\n\r\n" not in buf) if n is None else (len(buf) < n):
                        chunk = s.recv(65536)
                        self.assertTrue(chunk, "the server closed the connection")
                        buf += chunk
                    k = buf.index(b"\r\n\r\n") + 4 if n is None else n
                    out, buf = buf[:k], buf[k:]
                    return out

                s.sendall(self.request("HEAD"))
                head = until()
                s.sendall(self.request("GET"))
                get, data = until(), until(len(body))
        finally:
            srv.shutdown()
            srv.server_close()
        self.assertIn(b"200 OK", head.split(b"\n")[0])
        self.assertIn(f"Content-Length: {len(body)}\r\n".encode(), head)
        # the case's first read of the GET response is as long as HEAD's
        self.assertEqual(len(get), len(head))
        self.assertEqual(data, body)

    def fixture_run(self, body):
        """`body()` inside net_fixture over a scratch hosts file (the server
        on a free port); the file's text after the block, and what the block
        raised (or None)."""
        fx = corpus.corpus_conf("gnur-tests")["net_fixture"]
        real = corpus.start_fixture_server
        with tempfile.TemporaryDirectory() as tmp:
            hosts = Path(tmp, "hosts")
            hosts.write_text("127.0.0.1 localhost\n")
            raised = None
            with patch.object(corpus, "start_fixture_server", lambda root: real(root, port=0)):
                try:
                    with corpus.net_fixture("gnur-tests", fx, hosts_file=str(hosts)):
                        self.assertIn("127.0.0.1 developer.R-project.org cran.r-project.org\n", hosts.read_text())
                        body()
                except BaseException as e:  # (SystemExit and KeyboardInterrupt too)
                    raised = e
            return hosts.read_text(), raised

    def test_hosts_are_mapped_for_the_run_and_restored(self):
        self.assertEqual(self.fixture_run(lambda: None), ("127.0.0.1 localhost\n", None))

    def test_hosts_are_restored_after_an_exception_mid_run(self):
        def fail():
            raise RuntimeError("a case runner failed")
        text, raised = self.fixture_run(fail)
        self.assertEqual(text, "127.0.0.1 localhost\n")
        self.assertIsInstance(raised, RuntimeError)

    def test_hosts_are_restored_on_sigterm(self):
        text, raised = self.fixture_run(lambda: os.kill(os.getpid(), corpus.signal.SIGTERM))
        self.assertEqual(text, "127.0.0.1 localhost\n")
        self.assertIsInstance(raised, SystemExit)
        self.assertEqual(raised.code, 128 + corpus.signal.SIGTERM)

    def test_preflight_runs_only_as_root_in_a_container(self):
        conf = corpus.corpus_conf("gnur-tests")
        exists = os.path.exists
        outside = lambda p: False if p == "/.dockerenv" else exists(p)
        env = {k: v for k, v in os.environ.items() if k != "CI"}
        for uid, where, ci in ((1000, exists, "true"), (0, outside, None)):
            with patch.object(corpus.os, "geteuid", return_value=uid), \
                 patch.object(corpus.os.path, "exists", where), \
                 patch.dict(corpus.os.environ, env if ci is None else {**env, "CI": ci}, clear=True):
                with self.assertRaises(SystemExit) as e:
                    corpus.fixture_preflight("gnur-tests", conf)
                self.assertIn("needs root inside a container", str(e.exception.code))
        with patch.object(corpus.os, "geteuid", return_value=0), patch.dict(corpus.os.environ, {"CI": "true"}):
            self.assertEqual(corpus.fixture_preflight("gnur-tests", conf), conf["net_fixture"])

    def test_preflight_names_a_missing_file(self):
        conf = {"net_fixture": {**corpus.corpus_conf("gnur-tests")["net_fixture"], "files": ["inet-tests/gone.dat"]}}
        with patch.object(corpus.os, "geteuid", return_value=0), patch.dict(corpus.os.environ, {"CI": "true"}):
            with self.assertRaises(SystemExit) as e:
                corpus.fixture_preflight("gnur-tests", conf)
        self.assertIn("inet-tests/gone.dat", str(e.exception.code))

    def test_the_file_is_the_one_gnu_r_fetched(self):
        lines = Path(self.WWW, "inet-tests", "ch11b.dat").read_text().split("\n")
        self.assertEqual(lines[-1], "")
        shown = f'chr [1:{len(lines) - 1}] "{lines[0]}" "{lines[1]}" ...'
        for version in (corpus.GATED_R_VERSION, "4.6.2"):
            exp = corpus.load_expected("gnur-tests", corpus.expected_file({}, version))["sockets"]
            self.assertIn(shown, corpus.dec(exp["out"]).decode(), version)


if __name__ == "__main__":
    unittest.main()
