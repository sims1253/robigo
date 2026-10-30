"""Regression checks for the parse corpus ratchet (one marker per source)."""
import contextlib
import importlib.util
import io
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("parse_corpus", Path(__file__).with_name("parse-corpus.py"))
parse = importlib.util.module_from_spec(spec)
spec.loader.exec_module(parse)


class RatchetTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        (self.root / "passing").mkdir()
        self.patches = [patch.object(parse, "ROOT", self.root),
                        patch.object(parse, "RATCHET_DIR", self.root / "passing")]
        for p in self.patches:
            p.start()

    def tearDown(self):
        for p in self.patches:
            p.stop()
        self.tmp.cleanup()

    def mark(self, name, cid):
        p = self.root / "passing" / name / (cid + parse.RATCHET_MARKER)
        p.parent.mkdir(parents=True, exist_ok=True)
        p.touch()

    def test_one_marker_per_source(self):
        self.mark("fastr", "S4/TestR5/abc")
        self.assertEqual(parse.load_ratchet("fastr"), {"S4/TestR5/abc"})
        self.assertEqual(parse.add_ratchet("fastr", {"S4/TestR5/abc", "a/b"}), 1)
        self.assertTrue((self.root / "passing/fastr/a/b.pass").is_file())
        self.assertEqual(parse.add_ratchet("fastr", {"a/b"}), 0)

    def test_dangling_marker_is_reported(self):
        self.mark("fastr", "ghost")
        listed, dangling, lost = parse.ratchet_status("fastr", ["real"], ["real"])
        self.assertEqual(listed, {"ghost"})
        self.assertEqual(dangling, ["ghost"])
        self.assertEqual(lost, [])

    def test_stray_file_is_an_error(self):
        (self.root / "passing/fastr").mkdir(parents=True)
        (self.root / "passing/fastr/notes.txt").write_text("x")
        with self.assertRaises(SystemExit):
            parse.load_ratchet("fastr")

    def test_legacy_list_is_read(self):
        (self.root / "passing/fastr.txt").write_text("a\nb\n")
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(parse.load_ratchet("fastr"), {"a", "b"})

    def test_migrate_converts_and_proves(self):
        (self.root / "passing/fastr.txt").write_text("a/b\nc\n")
        with patch.object(parse, "corpus_ids", return_value={"a/b", "c"}), \
             contextlib.redirect_stdout(io.StringIO()):
            parse.cmd_migrate(["fastr"], None)
        self.assertFalse((self.root / "passing/fastr.txt").exists())
        self.assertEqual(parse.load_ratchet("fastr"), {"a/b", "c"})

    def test_migrate_rejects_entry_with_no_source(self):
        (self.root / "passing/fastr.txt").write_text("ghost\n")
        with patch.object(parse, "corpus_ids", return_value={"real"}), \
             contextlib.redirect_stdout(io.StringIO()), \
             self.assertRaises(SystemExit):
            parse.cmd_migrate(["fastr"], None)


if __name__ == "__main__":
    unittest.main()
