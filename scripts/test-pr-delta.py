#!/usr/bin/env python3
"""Check scripts/pr-delta.py (size labels and the test-delta comment)."""
import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("pr_delta", HERE / "pr-delta.py")
pd = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pd)


class Measure(unittest.TestCase):
    def test_size_counts_code_not_tests(self):
        d = pd.measure("40\t20\tcrates/a/src/x.rs\n500\t0\ttests/r/x.R\n3\t0\tscripts/test-x.py\n9\t9\tCargo.lock\n",
                       "", "")
        self.assertEqual((d["code_lines"], d["test_lines"], d["label"]), (60, 503, "size:M"))

    def test_tests_only_pull_request_is_sized_by_its_tests(self):
        d = pd.measure("700\t0\ttests/r/x.R\n0\t0\ttests/r/passing/x.R.pass\n", "", "")
        self.assertEqual(d["label"], "size:XL")

    def test_label_bounds(self):
        self.assertEqual([pd.size_label(n) for n in (0, 9, 10, 29, 30, 99, 100, 499, 500, 999, 1000)],
                         ["size:XS", "size:XS", "size:S", "size:S", "size:M", "size:M", "size:L", "size:L",
                          "size:XL", "size:XL", "size:XXL"])

    def test_binary_files_count_zero(self):
        self.assertEqual(pd.measure("-\t-\ttests/corpora/x/expected.jsonl.gz\n", "", "")["test_lines"], 0)

    def test_markers_by_suite_and_renames_ignored(self):
        ns = "\n".join([
            "A\ttests/corpora/fastr/passing/a/b.pass",
            "A\ttests/corpora/fastr/passing/a/c.pass",
            "D\ttests/corpora/gnur-tests/passing/d.pass",
            "A\ttests/parse/passing/fastr/e.pass",
            "A\ttests/r/passing/strings/x.R.pass",
            "A\ttests/packages/passing/rlang/a/b.pass",
            "R100\ttests/r/passing/old.R.pass\ttests/r/passing/new.R.pass",
            "A\ttests/r/strings/x.R",
            "D\ttests/r/strings/y.R",
            "M\ttests/r/strings/z.R",
        ])
        d = pd.measure("", ns, "")
        self.assertEqual(d["markers"], {
            "conformance": {"added": 1, "removed": 0},
            "fastr": {"added": 2, "removed": 0},
            "gnur-tests": {"added": 0, "removed": 1},
            "packages": {"added": 1, "removed": 0},
            "parse fastr": {"added": 1, "removed": 0},
        })
        self.assertEqual(d["scripts"], {"added": 1, "removed": 1})

    def test_rust_and_python_tests(self):
        patch = "\n".join([
            "+++ b/crates/a/src/x.rs", "+    #[test]", "+    #[test]", "-    #[test]", "+ // #[test] in a comment",
            "+++ b/scripts/test-x.py", "+    def test_one(self):", "-    def test_two(self):",
            "+++ b/scripts/x.py", "+def test_not_a_test_file():",
            # deleted files: their tests are removed
            "--- a/crates/a/tests/gone.rs", "+++ /dev/null", "-#[test]",
            "--- a/scripts/test-gone.py", "+++ /dev/null", "-    def test_gone(self):",
        ])
        d = pd.measure("", "", patch)
        self.assertEqual(d["rust_tests"], {"added": 2, "removed": 2})
        self.assertEqual(d["python_tests"], {"added": 1, "removed": 2})


class Render(unittest.TestCase):
    def test_nothing_to_say(self):
        self.assertEqual(pd.render(pd.measure("3\t1\tcrates/a/src/x.rs\n", "M\tcrates/a/src/x.rs", "")), "")

    def test_removed_marker_is_called_out(self):
        body = pd.render(pd.measure("", "D\ttests/corpora/fastr/passing/x.pass", ""))
        self.assertTrue(body.startswith(pd.MARKER))
        self.assertIn("| ratchet markers: fastr | +0 | -1 | -1 |", body)
        self.assertIn("1 ratchet marker(s) removed", body)


class Git(unittest.TestCase):
    def test_against_a_real_repository(self):
        with tempfile.TemporaryDirectory() as t:
            def g(*a):
                subprocess.run(["git", "-C", t, "-c", "user.name=t", "-c", "user.email=t@t", *a], check=True,
                               capture_output=True)
            g("init", "-q")
            Path(t, "a.rs").write_text("fn a() {}\n")
            g("add", "-A"); g("commit", "-qm", "base")
            Path(t, "a.rs").write_text("fn a() {}\n#[test]\nfn t() {}\n")
            Path(t, "tests/r/passing").mkdir(parents=True)
            Path(t, "tests/r/passing/x.R.pass").write_text("")
            Path(t, "tests/r/x.R").write_text("1\n")
            g("add", "-A"); g("commit", "-qm", "head")
            cwd = Path.cwd()
            try:
                import os
                os.chdir(t)
                d = pd.delta("HEAD~1", "HEAD")
            finally:
                os.chdir(cwd)
        self.assertEqual((d["code_lines"], d["test_lines"], d["label"]), (2, 1, "size:XS"))
        self.assertEqual(d["markers"], {"conformance": {"added": 1, "removed": 0}})
        self.assertEqual(d["rust_tests"], {"added": 1, "removed": 0})


if __name__ == "__main__":
    unittest.main()
