#!/usr/bin/env python3
"""Check scripts/ci-check.py, the rule behind ci.yml's required `Check` job."""
import importlib.util
import json
import os
import subprocess
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("ci_check", HERE / "ci-check.py")
chk = importlib.util.module_from_spec(spec)
spec.loader.exec_module(chk)

JOBS = ["select", "lint", "miri", "rt", "build", "release-tests", "test", "bytecode-fidelity", "counters",
        "corpora", "fuzz", "cran", "smoke", "package-gate", "perf"]


def needs(heavy="true", runtime="true", rt_sel="true", **results):
    d = {j: {"result": "success", "outputs": {}} for j in JOBS}
    d["select"]["outputs"] = {"heavy": heavy, "runtime": runtime, "rt": rt_sel}
    for k, v in results.items():
        d[k.replace("_", "-")]["result"] = v
    return d


def passes(n, pull_request=True):
    return all(ok for _, _, ok in chk.verdicts(n, pull_request))


SKIP_HEAVY = {j.replace("-", "_"): "skipped" for j in chk.HEAVY}


class Rule(unittest.TestCase):
    def test_all_green(self):
        self.assertTrue(passes(needs()))

    def test_light_pull_request_skips_heavy_jobs_and_the_gate(self):
        self.assertTrue(passes(needs("false", "false", "false", rt="skipped", package_gate="skipped",
                                    **SKIP_HEAVY)))

    def test_rt_pull_request_runs_rt_and_skips_heavy_jobs(self):
        self.assertTrue(passes(needs("false", "false", "true", package_gate="skipped", **SKIP_HEAVY)))
        self.assertFalse(passes(needs("false", "false", "true", rt="skipped", package_gate="skipped",
                                     **SKIP_HEAVY)))
        self.assertFalse(passes(needs("false", "false", "true", rt="failure", package_gate="skipped",
                                     **SKIP_HEAVY)))

    def test_rt_skipped_without_reason_fails(self):
        self.assertFalse(passes(needs(rt="skipped")))
        n = needs("false", "false", rt="skipped", package_gate="skipped", **SKIP_HEAVY)
        del n["select"]["outputs"]["rt"]
        self.assertFalse(passes(n))

    def test_heavy_job_skipped_without_reason_fails(self):
        self.assertFalse(passes(needs(corpora="skipped")))
        self.assertFalse(passes(needs("true", "false", perf="skipped")))

    def test_always_on_jobs_may_never_skip(self):
        for j in ("lint", "miri", "bytecode-fidelity", "select"):
            self.assertFalse(passes(needs("false", "false", **{j.replace("-", "_"): "skipped"})), j)

    def test_failure_and_cancel_fail(self):
        self.assertFalse(passes(needs(lint="failure")))
        self.assertFalse(passes(needs(perf="cancelled")))

    def test_failed_build_fails_even_though_dependents_skip(self):
        self.assertFalse(passes(needs(package_gate="skipped", **{**SKIP_HEAVY, "build": "failure"})))

    def test_failed_select_fails(self):
        n = needs(**SKIP_HEAVY, package_gate="skipped", select="failure")
        n["select"]["outputs"] = {}
        self.assertFalse(passes(n))

    def test_package_gate_skips(self):
        self.assertTrue(passes(needs(package_gate="skipped"), pull_request=False))
        self.assertTrue(passes(needs("true", "false", package_gate="skipped")))
        self.assertFalse(passes(needs(package_gate="skipped")))
        self.assertFalse(passes(needs(package_gate="failure"), pull_request=False))
        # a missing select output explains no skip: fail closed
        n = needs(package_gate="skipped")
        n["select"]["outputs"] = {}
        self.assertFalse(passes(n))

    def test_cli(self):
        def run(n, *args):
            return subprocess.run([sys.executable, str(HERE / "ci-check.py"), *args], capture_output=True,
                                  text=True, env={**os.environ, "NEEDS": json.dumps(n)})
        p = run(needs(), "--pull-request")
        self.assertEqual(p.returncode, 0, p.stdout)
        p = run(needs(fuzz="failure"), "--pull-request")
        self.assertEqual(p.returncode, 1)
        self.assertIn("not passed: fuzz", p.stdout)

    def test_workflow_needs_every_gating_job(self):
        import re
        wf = (HERE.parent / ".github/workflows/ci.yml").read_text()
        m = re.search(r"(?m)^  check:\n(?:    .*\n)*?    needs: \[([^\]]*)\]", wf)
        self.assertIsNotNone(m, "ci.yml has no check job with needs: [...]")
        listed = {j.strip() for j in m.group(1).split(",")}
        jobs = set(re.findall(r"(?m)^  ([a-z][a-z-]*):\n", wf.split("\njobs:\n", 1)[1])) - {"check", "progress"}
        self.assertEqual(listed, jobs)
        self.assertEqual(set(JOBS), jobs)


if __name__ == "__main__":
    unittest.main()
