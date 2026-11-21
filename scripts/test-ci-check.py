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

JOBS = ["select", "lint", "miri", "rdb-data", "builtin-diff-data", "rt", "rt-gctorture", "rt-feature-builds", "rt-builtin-diff", "rt-debug-assertions", "rt-release-crate-tests", "base-subset", "build", "perf-base", "release-tests", "test", "bytecode-fidelity", "counters",
        "corpora", "fuzz", "cran", "smoke", "package-gate", "perf", "perf-rt-build", "perf-rt", "rt-corpora"]


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
        self.assertTrue(passes(needs("false", "false", "false", rt="skipped", base_subset="skipped",
                                    perf_rt_build="skipped", perf_rt="skipped", package_gate="skipped",
                                    **SKIP_HEAVY)))

    def test_rt_pull_request_runs_rt_and_skips_heavy_jobs(self):
        self.assertTrue(passes(needs("false", "false", "true", package_gate="skipped", **SKIP_HEAVY)))
        self.assertFalse(passes(needs("false", "false", "true", rt="skipped", package_gate="skipped",
                                     **SKIP_HEAVY)))
        self.assertFalse(passes(needs("false", "false", "true", rt="failure", package_gate="skipped",
                                     **SKIP_HEAVY)))

    def test_perf_rt_runs_with_rt(self):
        # robigo-rt's M1 bars (scripts/perf.py rt-bars): skipped only with rt
        self.assertFalse(passes(needs("false", "false", "true", perf_rt="skipped", package_gate="skipped",
                                     **SKIP_HEAVY)))
        self.assertFalse(passes(needs(perf_rt="failure")))
        # its build job too: a skipped or failed build is a skipped gate
        self.assertFalse(passes(needs("false", "false", "true", perf_rt_build="skipped", package_gate="skipped",
                                     **SKIP_HEAVY)))
        self.assertFalse(passes(needs(perf_rt_build="failure")))
        self.assertTrue(passes(needs("false", "false", "false", rt="skipped", base_subset="skipped",
                                    perf_rt_build="skipped", perf_rt="skipped", package_gate="skipped",
                                    **SKIP_HEAVY)))

    def test_rt_corpora_and_rdb_data_run_with_rt(self):
        # robigo-rt's corpus column and L1's rdb data: skipped only with rt
        for job in ("rt_corpora", "rdb_data", "builtin_diff_data"):
            self.assertFalse(passes(needs("false", "false", "true", package_gate="skipped", **{job: "skipped"},
                                          **SKIP_HEAVY)), job)
            self.assertFalse(passes(needs(**{job: "failure"})), job)
        self.assertTrue(passes(needs("false", "false", "false", rt="skipped", base_subset="skipped",
                                    perf_rt_build="skipped", perf_rt="skipped", rt_corpora="skipped",
                                    rdb_data="skipped", builtin_diff_data="skipped",
                                    package_gate="skipped", **SKIP_HEAVY)))

    def test_base_subset_skips_with_rt_only(self):
        self.assertFalse(passes(needs("false", "false", "true", base_subset="skipped", package_gate="skipped",
                                     **SKIP_HEAVY)))
        self.assertFalse(passes(needs(base_subset="failure")))

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

    def test_perf_base_runs_with_perf_on_pull_requests(self):
        # main measures no base; a light pull request runs no perf
        self.assertTrue(passes(needs(perf_base="skipped"), pull_request=False))
        self.assertTrue(passes(needs("false", "false", perf_base="skipped", package_gate="skipped", **SKIP_HEAVY)))
        # a pull request with perf must have built its base
        self.assertFalse(passes(needs(perf_base="skipped")))
        self.assertFalse(passes(needs(perf_base="failure")))

    def test_rt_shards_follow_rt(self):
        # the rt job's parallel shards (scripts/rt-ci.py --shard): each is
        # required where rt is, may be skipped only where rt may (rt mode
        # false), and fails the check when it fails
        for job in ("rt_gctorture", "rt_feature_builds", "rt_builtin_diff", "rt_debug_assertions"):
            self.assertFalse(passes(needs("false", "false", "true", package_gate="skipped",
                                         **{job: "skipped"}, **SKIP_HEAVY)), job)
            self.assertFalse(passes(needs(**{job: "failure"})), job)
            self.assertFalse(passes(needs(**{job: "cancelled"})), job)
        self.assertTrue(passes(needs("false", "false", "false", rt="skipped", rt_gctorture="skipped",
                                    rt_feature_builds="skipped", rt_builtin_diff="skipped", rt_debug_assertions="skipped",
                                    base_subset="skipped",
                                    perf_rt_build="skipped", perf_rt="skipped", rt_corpora="skipped",
                                    rdb_data="skipped", builtin_diff_data="skipped",
                                    package_gate="skipped", **SKIP_HEAVY)))

    def test_cli(self):
        def run(n, *args):
            return subprocess.run([sys.executable, str(HERE / "ci-check.py"), *args], capture_output=True,
                                  text=True, env={**os.environ, "NEEDS": json.dumps(n)})
        p = run(needs(), "--pull-request")
        self.assertEqual(p.returncode, 0, p.stdout)
        p = run(needs(fuzz="failure"), "--pull-request")
        self.assertEqual(p.returncode, 1)
        self.assertIn("not passed: fuzz", p.stdout)

    def test_stacked_pull_request_skips_all_but_select_lint_and_rt_and_fails(self):
        skip = {**SKIP_HEAVY, **{j.replace("-", "_"): "skipped" for j in chk.STACKED_SKIP}, "package_gate": "skipped"}
        n = needs("false", "false", "true", **skip)
        n["select"]["outputs"]["stacked"] = "true"
        self.assertTrue(passes(n))                  # every skip is explained...
        for job in ("rt", "rt-gctorture", "rt-feature-builds", "rt-builtin-diff", "rt-debug-assertions", "lint", "rdb-data"):      # ...but not these
            m = needs("false", "false", "true", **{**skip, job.replace("-", "_"): "skipped"})
            m["select"]["outputs"]["stacked"] = "true"
            self.assertFalse(passes(m), job)
        p = subprocess.run([sys.executable, str(HERE / "ci-check.py"), "--pull-request"], capture_output=True,
                           text=True, env={**os.environ, "NEEDS": json.dumps(n)})
        self.assertEqual(p.returncode, 1)           # ...and Check still fails: a retarget starts no run
        self.assertIn("stacked pull request", p.stdout)
        # not stacked, the same skips are not explained
        self.assertFalse(passes(needs("false", "false", "true", **skip)))

    def test_the_release_crate_tests_run_on_main_and_may_be_skipped_on_pull_requests(self):
        skipped = needs(rt_release_crate_tests="skipped")
        self.assertTrue(passes(skipped, pull_request=True))
        self.assertFalse(passes(skipped, pull_request=False))       # main: it must run
        self.assertTrue(passes(needs("true", "true", "false", rt_release_crate_tests="skipped"), pull_request=False))
        self.assertFalse(passes(needs(rt_release_crate_tests="failure"), pull_request=False))
        self.assertFalse(passes(needs(rt_release_crate_tests="failure"), pull_request=True))

    def test_a_cancelled_run_fails_check(self):
        # every need cancelled (a run cancelled by hand, or superseded): red
        n = {j: {"result": "cancelled", "outputs": {}} for j in JOBS}
        self.assertFalse(passes(n))
        self.assertFalse(passes(n, pull_request=False))
        self.assertFalse(passes(needs(rt="cancelled")))

    def test_check_runs_always_and_pull_requests_share_no_group(self):
        # Check runs in a cancelled run too, so a hand-cancelled head run is
        # red; then no shared group may make a new run wait behind it
        wf = (HERE.parent / ".github/workflows/ci.yml").read_text()
        self.assertIn("    name: Check\n    if: ${{ always() }}\n", wf)
        # main's push runs share one group that never cancels a started run
        # (a newer push replaces only the pending one); every other run,
        # pull requests' included, has a group of its own
        self.assertIn("  group: ci-${{ github.event_name == 'push' && 'main' || github.run_id }}\n"
                      "  cancel-in-progress: false\n", wf)
        self.assertEqual(wf.split("\non:\n", 1)[1].split("\n  pull_request:", 1)[0],
                         "  push:\n    branches: [main]", "push runs are main's alone")
        # select cancels the superseded runs, as its first step
        steps = wf.split("\n  select:\n", 1)[1].split("\n    steps:\n", 1)[1].splitlines()
        first = next(l.strip() for l in steps if not l.strip().startswith("#"))
        self.assertEqual(first, "- name: Cancel this pull request's superseded runs")

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
