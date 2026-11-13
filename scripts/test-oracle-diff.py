#!/usr/bin/env python3
"""scripts/oracle-diff.py on a fixture: what counts as changed between two
GNU R versions' recordings, per suite."""
import contextlib
import importlib.util
import io
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

HERE = Path(__file__).resolve().parent


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, HERE / file)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


od = load("oracle_diff", "oracle-diff.py")
corpus = od.corpus


def write(p, data):
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_bytes(data)


class ConformanceTests(unittest.TestCase):
    def test_changed_missing_and_stale(self):
        with tempfile.TemporaryDirectory() as tmp:
            r = Path(tmp)
            src = b"print(1)\n"
            sha = corpus.hashlib.sha256(src).hexdigest().encode()
            for name in ("same", "changed", "stale", "unrecorded"):
                write(r / f"a/{name}.R", src)
                write(r / f"a/{name}.Rout", b"[1] 1\n")
                write(r / f"a/{name}.Rstatus", b"0\n")
            for name, out, rsha in (("same", b"[1] 1\n", sha), ("changed", b"[1] 2\n", sha),
                                    ("stale", b"[1] 1\n", b"0" * 64)):
                write(r / f"R-4.6.2/a/{name}.Rout", out)
                write(r / f"R-4.6.2/a/{name}.Rstatus", b"0\n")
                write(r / f"R-4.6.2/a/{name}.Rsha", rsha + b"\n")
            with patch.object(od, "TESTS_R", str(r)):
                res = od.diff_conformance("4.5.0", "4.6.2")
        self.assertEqual(res["compared"], 3)
        self.assertEqual(res["changed"], {"a/changed.R": ["stdout"]})
        self.assertEqual(res["missing_to"], ["a/unrecorded.R"])
        self.assertEqual(res["stale_to"], ["a/stale.R"])
        self.assertEqual(res["missing_from"] + res["stale_from"], [])


class CorpusTests(unittest.TestCase):
    def test_masks_unstable_and_changed(self):
        def rec(cid, out, **kw):
            return dict({"id": cid, "sha": corpus.sha(b"x"), "out": corpus.enc(out), "err": corpus.enc(b""),
                         "status": 0}, **kw)

        old = {"addr": rec("addr", b"<environment: 0x1>\n"), "time": rec("time", b"a\nt1\n"),
               "flaky": rec("flaky", b"x\n"), "changed": rec("changed", b"1\n")}
        new = {"addr": rec("addr", b"<environment: 0x2>\n"), "time": rec("time", b"a\nt2\n", mask_out=[1]),
               "flaky": rec("flaky", b"y\n", unstable=True), "changed": rec("changed", b"1\n", status=1)}
        with tempfile.TemporaryDirectory() as tmp:
            Path(tmp, "fixture").mkdir()
            Path(tmp, "corpora.toml").write_text('[fixture]\n')
            with patch.object(corpus, "CORPORA", tmp), \
                 patch.object(corpus, "load_cases", return_value=[(k, b"x") for k in sorted(old)]):
                corpus.write_expected("fixture", old)
                corpus.write_expected("fixture", new, corpus.expected_file({}, "4.6.2"))
                res = od.diff_corpus("fixture", "4.5.0", "4.6.2")
                with patch.object(corpus, "manifest", return_value={"fixture": {}}), \
                     patch.object(od, "diff_conformance", return_value=od.new_result()), \
                     patch("sys.argv", ["oracle-diff.py", "--ids", "fixture"]), \
                     contextlib.redirect_stdout(io.StringIO()) as out:
                    self.assertEqual(od.main(), 0)
        self.assertEqual(res["compared"], 3)
        self.assertEqual(res["changed"], {"changed": ["status"]})
        self.assertEqual(res["unstable"], ["flaky"])
        self.assertIn("fixture\tchanged\tstatus", out.getvalue())


if __name__ == "__main__":
    os.environ.pop("ROBIGO_R_VERSION", None)
    unittest.main()
