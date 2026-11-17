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

    def test_conformance_runs_alone_in_its_own_steps(self):
        steps = rt.steps(["robigo-rt", "robigo-rtcore"])
        own = [(env, cmd) for _, env, cmd in steps if rt.CONFORMANCE in cmd and "--skip" not in cmd]
        # once plain, once under each observation mode, then only under a
        # collector mode or the byte-code crosscheck (V3: every translated
        # closure also compiled from source and compared, its report read by
        # `main`; kept out of the plain run, which it would not leave plain)
        self.assertEqual(len([e for e, _ in own if not e]), 1)
        modes = [sorted(e) for e, _ in own if e and set(e) & {"ROBIGO_NO_INPLACE", "ROBIGO_MATERIALIZE"}]
        self.assertEqual(modes, [["ROBIGO_NO_INPLACE"], ["ROBIGO_MATERIALIZE"]])
        for env, cmd in own:
            self.assertEqual(cmd[cmd.index("--test") + 1], "conformance")
            self.assertIn("--exact", cmd)
            if env:
                self.assertTrue({"ROBIGO_GCTORTURE", "R_MAX_VSIZE", "ROBIGO_NO_INPLACE", "ROBIGO_MATERIALIZE", "ROBIGO_FRAME_QUARANTINE",
                                 "ROBIGO_BC_CROSSCHECK"} & set(env), env)
        for title, _, cmd in steps:
            if cmd[0] == "cargo" and rt.CONFORMANCE not in cmd[-1:] and "--exact" not in cmd:
                self.assertEqual(cmd[-2:], ["--skip", rt.CONFORMANCE], title)
        # The test exists under that name: after a rename the conformance step
        # would run nothing, and still pass.
        src = (HERE.parent / "crates/robigo-rtapi/tests/conformance.rs").read_text()
        self.assertIn(f"fn {rt.CONFORMANCE}()", src)

    def test_slow_case_timeouts_are_the_moving_collector_steps_only(self):
        # INTERIM(H8): exactly the GC_MOVE + gctorture conformance step gets
        # the longer timeouts, and each case it names exists
        steps = rt.steps(["robigo-rt"])
        with_overrides = [title for title, env, _ in steps if "ROBIGO_CONF_TIMEOUTS" in env]
        self.assertEqual(with_overrides, ["conformance under ROBIGO_GC_MOVE with gctorture (E3)"])
        env = next(e for t, e, _ in steps if t == with_overrides[0])
        self.assertEqual(env["ROBIGO_GC_MOVE"], "1")
        self.assertEqual(env["ROBIGO_GCTORTURE"], "1")
        named = dict(item.rsplit("=", 1) for item in env["ROBIGO_CONF_TIMEOUTS"].split(","))
        self.assertEqual(set(named), set(rt.GC_MOVE_SLOW))
        for case, secs in named.items():
            self.assertTrue((HERE.parent / case).is_file(), case)
            self.assertGreater(int(secs), int(env["ROBIGO_CONF_TIMEOUT"]), case)

    def test_the_pin_report_expects_only_the_probes(self):
        import tempfile
        probe = ("robigo pin report: kind=finalizer key=EXTPTRSXP@0x1 weakref=0x2 word=0x1@0x3 "
                 "dll=capi_test routine=capi_pp_probe pid=1")
        other = probe.replace("capi_pp_probe", "rt_nested")
        with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as f:
            f.write(probe + "\n" + other + "\n" + probe.replace("capi_pp_probe", "capi_eb_probe") + "\n")
        try:
            self.assertEqual(rt.pin_report_hits(f.name), [other])
        finally:
            os.unlink(f.name)
        self.assertEqual(rt.pin_report_hits(f.name + ".missing"), [])

    def test_this_workspace_holds_the_premise(self):
        m = rt.metadata()
        crates = rt.rt_crates(m, PATS)
        self.assertIn("robigo-rt", crates)
        self.assertNotIn("robigo-cli", crates)
        self.assertEqual(rt.leaks(m, crates), [])


if __name__ == "__main__":
    unittest.main()
