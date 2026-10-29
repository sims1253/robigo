"""Regression checks for content-based Rdiff ignore blocks."""
import argparse
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
        args = argparse.Namespace(name="fixture", filter=None, all=True, repeat=2,
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


if __name__ == "__main__":
    unittest.main()
