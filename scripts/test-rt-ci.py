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

    def test_conformance_runs_once_in_its_own_step(self):
        steps = rt.steps(["robigo-rt", "robigo-rtcore"])
        own = [cmd for _, env, cmd in steps if rt.CONFORMANCE in cmd and "--skip" not in cmd and not env]
        self.assertEqual(len(own), 1)
        self.assertEqual(own[0][own[0].index("--test") + 1], "conformance")
        # a verification mode may run the conformance again, in its own step
        # (D5's quarantine mode); every other step skips it
        for title, env, cmd in steps:
            if cmd is own[0]:
                continue
            if "--skip" not in cmd:
                self.assertEqual(cmd[cmd.index("--test") + 1], "conformance", title)
                self.assertTrue(env, title)
            else:
                self.assertEqual(cmd[-2:], ["--skip", rt.CONFORMANCE], title)
        # The test exists under that name: after a rename the conformance step
        # would run nothing, and still pass.
        src = (HERE.parent / "crates/robigo-rt/tests/conformance.rs").read_text()
        self.assertIn(f"fn {rt.CONFORMANCE}()", src)

    def test_this_workspace_holds_the_premise(self):
        m = rt.metadata()
        crates = rt.rt_crates(m, PATS)
        self.assertIn("robigo-rt", crates)
        self.assertNotIn("robigo-cli", crates)
        self.assertEqual(rt.leaks(m, crates), [])


if __name__ == "__main__":
    unittest.main()
