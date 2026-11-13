#!/usr/bin/env python3
"""Check scripts/rt-ci.py: which crates the `rt` job tests, and its premise check."""
import importlib.util
import os
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("rt_ci", HERE / "rt-ci.py")
rt = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rt)

PATS = rt.rt_patterns(rt.ci_select.load())


def pkg(name, path, deps=()):
    return {"id": name, "name": name, "manifest_path": os.path.join(rt.ROOT, path, "Cargo.toml"),
            "dependencies": [{"name": d} for d in deps]}


def meta(*pkgs):
    return {"packages": list(pkgs), "workspace_members": [p["id"] for p in pkgs]}


class RtCi(unittest.TestCase):
    def test_crates_follow_the_selection_rule(self):
        m = meta(pkg("robigo-rt", "crates/robigo-rt"), pkg("robigo-rtcore", "crates/robigo-rtcore"),
                 pkg("robigo-heap", "crates/robigo-heap"), pkg("robigo-heap-derive", "crates/robigo-heap-derive"),
                 pkg("robigo-core", "crates/robigo-core"), pkg("robigo-runtime", "crates/robigo-runtime"))
        self.assertEqual(rt.rt_crates(m, PATS), ["robigo-heap", "robigo-heap-derive", "robigo-rt", "robigo-rtcore"])

    def test_a_dependency_from_outside_breaks_the_premise(self):
        m = meta(pkg("robigo-rt", "crates/robigo-rt", ["robigo-rtcore"]), pkg("robigo-rtcore", "crates/robigo-rtcore"),
                 pkg("robigo-cli", "crates/robigo-cli", ["robigo-runtime", "robigo-rt"]),
                 pkg("robigo-runtime", "crates/robigo-runtime"))
        crates = rt.rt_crates(m, PATS)
        self.assertEqual(rt.leaks(m, crates), [("robigo-cli", "robigo-rt")])
        m["packages"][2]["dependencies"] = [{"name": "robigo-runtime"}]
        self.assertEqual(rt.leaks(m, crates), [])

    def test_this_workspace_holds_the_premise(self):
        m = rt.metadata()
        crates = rt.rt_crates(m, PATS)
        self.assertIn("robigo-rt", crates)
        self.assertNotIn("robigo-cli", crates)
        self.assertEqual(rt.leaks(m, crates), [])


if __name__ == "__main__":
    unittest.main()
