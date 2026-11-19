#!/usr/bin/env python3
"""Check scripts/base-subset-stale.py, the report of base-subset.sh --check."""
import importlib.util
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("stale", HERE / "base-subset-stale.py")
st = importlib.util.module_from_spec(spec)
spec.loader.exec_module(st)

HEAD = ["# robigo-rt's base subset", "# 2 closures, 1 primitives, 0 internals."]


class Report(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.d, ignore_errors=True)

    def f(self, name, lines):
        p = os.path.join(self.d, name)
        with open(p, "w") as f:
            f.write("\n".join(lines) + "\n")
        return p

    def test_same_files_report_nothing(self):
        a = self.f("a", HEAD + ["closure f a.R"])
        self.assertIsNone(st.report(a, self.f("b", HEAD + ["closure f a.R"])))

    def test_names_gained_and_lost_with_the_inputs_that_reach_them(self):
        a = self.f("a", HEAD + ["closure f a.R", "internal old"])
        b = self.f("b", HEAD + ["closure f a.R", "closure chartr chartr.R", "internal chartr"])
        by = self.f("by", ["# header", "closure chartr chartr.R env:probe", "internal chartr env:probe,sort:x"])
        r = st.report(a, b, by)
        self.assertIn("now reach 2 name(s) it does not list, and no longer reach 1", r)
        self.assertIn("  + closure chartr chartr.R    (reached by env:probe)", r)
        self.assertIn("  + internal chartr    (reached by env:probe,sort:x)", r)
        self.assertIn("  - internal old", r)
        self.assertIn("scripts/base-subset.sh", r)
        self.assertIn("The generator ran; its output differs", r)

    def test_a_missing_reached_by_still_reports(self):
        a = self.f("a", HEAD + ["closure f a.R"])
        b = self.f("b", HEAD + ["closure f a.R", "primitive g"])
        r = st.report(a, b, os.path.join(self.d, "absent"))
        self.assertIn("  + primitive g\n", r + "\n")

    def test_header_only_and_order_only(self):
        a = self.f("a", HEAD + ["closure f a.R", "closure g b.R"])
        self.assertIn("Only its header differs", st.report(a, self.f("b", ["# other"] + HEAD[1:] + ["closure f a.R", "closure g b.R"])))
        self.assertIn("another order", st.report(a, self.f("c", HEAD + ["closure g b.R", "closure f a.R"])))

    def test_long_lists_are_cut(self):
        a = self.f("a", HEAD)
        b = self.f("b", HEAD + [f"primitive p{i}" for i in range(st.MAX + 5)])
        self.assertIn("+ ... 5 more", st.report(a, b))

    def test_cli_exit_status(self):
        a = self.f("a", HEAD + ["closure f a.R"])
        same = subprocess.run([sys.executable, str(HERE / "base-subset-stale.py"), a, a], capture_output=True)
        self.assertEqual(same.returncode, 0)
        b = self.f("b", HEAD)
        diff = subprocess.run([sys.executable, str(HERE / "base-subset-stale.py"), a, b], capture_output=True,
                              text=True)
        self.assertEqual(diff.returncode, 1)
        self.assertIn("is not what scripts/base-subset.sh generates", diff.stderr)


if __name__ == "__main__":
    unittest.main()
