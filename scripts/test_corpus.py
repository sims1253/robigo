"""Regression checks for content-based Rdiff ignore blocks."""
import argparse
import contextlib
import io
import json
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
            (root / "fixture/passing.txt").write_text("case\n")
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
                    robigo=__file__, filter=None, shard=None, jobs=1, keep_output=False, quiet=True)
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
                self.assertEqual((root / "fixture/passing.txt").read_text(), "case\n")


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


if __name__ == "__main__":
    unittest.main()
