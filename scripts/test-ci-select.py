#!/usr/bin/env python3
"""Check the CI job selection (scripts/ci-select.py, scripts/ci-selection.toml)."""
import importlib.util
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent

spec = importlib.util.spec_from_file_location("ci_select", HERE / "ci-select.py")
sel = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sel)

RULES = sel.load()


def pick(*paths):
    mode, runtime, rows = sel.decide(list(paths), RULES)
    return mode, runtime, rows


class Selection(unittest.TestCase):
    def test_docs_and_repo_metadata_only_are_light(self):
        for p in ("docs/verification.md", "docs/ledger/test-infra.md", "docs/foo.md",
                  "README.md", "AGENTS.md", "LICENSE", ".gitignore"):
            mode, runtime, rows = pick(p)
            self.assertEqual((mode, runtime, rows[0][1]), ("light", False, "no-binary-effect"), p)
        self.assertEqual(pick(".githooks/pre-push")[:2], ("light", False))

    def test_markdown_under_crates_and_rlib_is_runtime(self):
        # a markdown file under crates/ or rlib/ can be build/runtime input
        # (e.g. include_str!), so the runtime rule must win over a light *.md rule
        for p in ("crates/x/README.md", "crates/robigo-runtime/src/note.md", "rlib/base/README.md"):
            mode, runtime, rows = pick(p)
            self.assertEqual((mode, runtime, rows[0][1]), ("full", True, "runtime"), p)

    def test_tests_run_heavy_jobs(self):
        # tests/** feeds test/corpora/parse/cran/smoke: a wrong marker or case
        # that only adds files must not pass CI
        cases = [
            ("tests/r/strings/escape.R", "tests"),
            ("tests/r/passing/strings/escape.R.pass", "tests"),
            ("tests/corpora/fastr/passing/x.pass", "tests"),
            ("tests/parse/fastr/case.R", "tests"),
            ("tests/cran/expected.json", "tests"),
            ("tests/cran/packages.tsv", "tests"),
        ]
        for p, rule in cases:
            mode, runtime, rows = pick(p)
            self.assertEqual((mode, runtime, rows[0][1]), ("full", False, rule), p)

    def test_package_inputs_run_the_package_gate(self):
        cases = [
            "tests/packages/passing/rlang/a/b.pass",
            "tests/packages/markers/rlang/a/b.pass",
            "tests/packages/expected/rlang.jsonl.gz",
            "tests/packages/canary/DESCRIPTION",
            "tests/packages/canary/R/canary.R",
            "tests/packages/canary/tests/testthat/test-canary.R",
            "tests/packages/selection.tsv",
            "tests/packages/rv.lock",
        ]
        for p in cases:
            mode, runtime, rows = pick(p)
            self.assertEqual((mode, runtime, rows[0][1]), ("full", True, "package-gate-inputs"), p)

    def test_runtime_changes_run_everything_and_mark_runtime(self):
        for p in ("crates/robigo-runtime/src/interp.rs", "crates/robigo-syntax/src/lib.rs",
                  "rlib/base/R/apply.R", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml"):
            mode, runtime, rows = pick(p)
            self.assertEqual((mode, runtime, rows[0][1]), ("full", True, "runtime"), p)

    def test_runtime_producing_scripts_run_the_package_gate(self):
        # produce the runtime/natives/R_HOME/rdbflat the package gate runs in
        for p in ("scripts/build-natives.sh", "scripts/natives-layout.h",
                  "scripts/robigo-home.sh", "scripts/build.sh",
                  "scripts/build-rdbflat.sh", "scripts/build-rdbflat.py",
                  "scripts/build-rdbflat-force.R", "scripts/build-rdbflat-index.R"):
            mode, runtime, rows = pick(p)
            self.assertEqual((mode, runtime, rows[0][1]), ("full", True, "package-gate-inputs"), p)

    def test_gate_definition_runs_the_package_gate(self):
        # a change to the gate or its workflow/action must run the gate
        for p in ("scripts/package-suite.py", "scripts/test-package-suite.py",
                  ".github/actions/package-suite/action.yml",
                  ".github/actions/setup/action.yml",
                  ".github/actions/package-cache/action.yml",
                  ".github/ci-image/Dockerfile", ".github/ci-image/apt.txt", "scripts/ci-image.sh",
                  ".github/workflows/ci.yml", ".github/workflows/packages.yml",
                  "scripts/ci-select.py", "scripts/ci-selection.toml", "scripts/test-ci-select.py"):
            mode, runtime, rows = pick(p)
            self.assertEqual((mode, runtime, rows[0][1]), ("full", True, "package-gate-inputs"), p)

    def test_other_gate_scripts_and_ci_run_heavy_but_not_the_package_gate(self):
        for p in ("scripts/corpus.py", "scripts/perf.py", "scripts/ratchet.py",
                  ".github/workflows/nightly.yml",
                  "benchmarks/selection.toml"):
            mode, runtime, rows = pick(p)
            self.assertEqual((mode, runtime, rows[0][1]), ("full", False, "gates-and-infra"), p)

    def test_mixed_paths(self):
        self.assertEqual(pick("docs/verification.md", "crates/robigo-runtime/src/vm.rs")[:2], ("full", True))
        self.assertEqual(pick("tests/r/x.R", "scripts/corpus.py")[:2], ("full", False))
        # a package-gate input mixed with light files still runs the gate
        self.assertEqual(pick("docs/foo.md", "tests/packages/rv.lock")[:2], ("full", True))
        self.assertEqual(pick("tests/r/x.R", "scripts/build-natives.sh")[:2], ("full", True))

    def test_unknown_path_runs_everything(self):
        mode, runtime, rows = pick("something-new.txt")
        self.assertEqual((mode, runtime), ("full", True))
        self.assertIsNone(rows[0][1])

    def test_one_unknown_among_safe_runs_everything(self):
        self.assertEqual(pick("docs/a.md", "weird/file")[:2], ("full", True))

    def test_empty_diff_runs_everything(self):
        self.assertEqual(pick()[:2], ("full", True))

    def test_glob(self):
        g = sel.glob_re
        self.assertTrue(g("docs/**").match("docs/ledger/a.md"))
        self.assertTrue(g("*.md").match("README.md"))
        self.assertFalse(g("*.md").match("a/b.md"))
        self.assertTrue(g("crates/**").match("crates/x/README.md"))
        self.assertTrue(g("tests/**").match("tests/packages/rv.lock"))


class Config(unittest.TestCase):
    def test_every_rule_pattern_matches_a_tracked_file(self):
        files = subprocess.run(["git", "-c", "safe.directory=*", "ls-files"], cwd=ROOT,
                               capture_output=True, text=True, check=True).stdout.split("\n")
        with open(sel.CONFIG, "rb") as f:
            raw = sel.tomllib.load(f)["rules"]
        for r in raw:
            for pat in r["paths"]:
                rx = sel.glob_re(pat)
                self.assertTrue(any(rx.match(f) for f in files), f"rule {r['name']}: {pat} matches no file")

    def test_bad_mode_rejected(self):
        with tempfile.TemporaryDirectory() as t:
            p = Path(t) / "s.toml"
            p.write_text('[[rules]]\nname="r"\npaths=["a"]\nmode="skip"\n')
            with self.assertRaises(ValueError):
                sel.load(str(p))

    def test_no_rules_rejected(self):
        with tempfile.TemporaryDirectory() as t:
            p = Path(t) / "s.toml"
            p.write_text("x=1\n")
            with self.assertRaises((ValueError, KeyError)):
                sel.load(str(p))


class Cli(unittest.TestCase):
    def run_cli(self, *args, env=None):
        e = {**os.environ, **(env or {})}
        e.pop("GITHUB_EVENT_NAME", None)
        e.update(env or {})
        return subprocess.run([sys.executable, str(HERE / "ci-select.py"), *args],
                              capture_output=True, text=True, env=e, cwd=ROOT)

    def outputs(self, *args, env=None):
        with tempfile.TemporaryDirectory() as t:
            out = Path(t) / "o"
            p = self.run_cli(*args, "--github-output", env={"GITHUB_OUTPUT": str(out), **(env or {})})
            self.assertEqual(p.returncode, 0, p.stderr)
            return dict(l.split("=", 1) for l in out.read_text().splitlines() if "=" in l), p.stdout

    def test_outputs_for_light_tests_runtime_and_gate_inputs(self):
        o, _ = self.outputs("--changed", "docs/verification.md")
        self.assertEqual((o["mode"], o["heavy"], o["runtime"]), ("light", "false", "false"))
        o, _ = self.outputs("--changed", "tests/r/strings/escape.R")
        self.assertEqual((o["mode"], o["heavy"], o["runtime"]), ("full", "true", "false"))
        o, _ = self.outputs("--changed", "tests/packages/rv.lock")
        self.assertEqual((o["mode"], o["heavy"], o["runtime"]), ("full", "true", "true"))
        o, _ = self.outputs("--changed", "tests/cran/expected.json")
        self.assertEqual((o["mode"], o["heavy"], o["runtime"]), ("full", "true", "false"))
        o, _ = self.outputs("--changed", "crates/robigo-runtime/src/vm.rs")
        self.assertEqual((o["mode"], o["heavy"], o["runtime"]), ("full", "true", "true"))
        o, _ = self.outputs("--changed", "scripts/build-natives.sh")
        self.assertEqual((o["mode"], o["heavy"], o["runtime"]), ("full", "true", "true"))
        o, _ = self.outputs("--changed", "scripts/package-suite.py")
        self.assertEqual((o["mode"], o["heavy"], o["runtime"]), ("full", "true", "true"))
        o, _ = self.outputs("--changed", ".github/actions/package-suite/action.yml")
        self.assertEqual((o["mode"], o["heavy"], o["runtime"]), ("full", "true", "true"))
        o, _ = self.outputs("--changed", "crates/x/README.md")
        self.assertEqual((o["mode"], o["heavy"], o["runtime"]), ("full", "true", "true"))
        o, _ = self.outputs("--changed", "docs/foo.md")
        self.assertEqual((o["mode"], o["heavy"], o["runtime"]), ("light", "false", "false"))
        o, _ = self.outputs("--changed", "scripts/corpus.py")
        self.assertEqual((o["mode"], o["heavy"], o["runtime"]), ("full", "true", "false"))
        o, _ = self.outputs("--changed", "unknown/x")
        self.assertEqual((o["mode"], o["heavy"], o["runtime"]), ("full", "true", "true"))

    def test_paths_from_file_and_stdin(self):
        with tempfile.TemporaryDirectory() as t:
            f = Path(t) / "paths"
            f.write_text("docs/a.md\n\nREADME.md\n")
            o, _ = self.outputs("--paths-from", str(f))
            self.assertEqual(o["mode"], "light")
        p = subprocess.run([sys.executable, str(HERE / "ci-select.py"), "--paths-from", "-"],
                           input="docs/a.md\ncrates/robigo-runtime/src/vm.rs\n",
                           capture_output=True, text=True, cwd=ROOT)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn("crates/robigo-runtime/src/vm.rs", p.stdout)

    def test_summary_lists_the_package_gate_only_when_it_runs(self):
        _, out = self.outputs("--event", "pull_request", "--changed", "tests/r/strings/escape.R")
        self.assertIn("- **skip:** the package regression gate", out)
        _, out = self.outputs("--event", "pull_request", "--changed", "crates/robigo-runtime/src/vm.rs")
        self.assertIn("perf, the package regression gate, lint", out)
        self.assertIn("- **skip:** nothing", out)
        _, out = self.outputs("--event", "pull_request", "--changed", "docs/a.md")
        self.assertIn("- **skip:** build, test, corpora, fuzz, cran, smoke, perf and the package regression gate", out)

    def test_other_events_run_everything(self):
        for ev in ("push", "workflow_dispatch", "schedule"):
            o, _ = self.outputs("--event", ev)
            self.assertEqual((o["mode"], o["heavy"]), ("full", "true"), ev)

    def test_explicit_changed_is_a_dry_run(self):
        o, _ = self.outputs("--event", "push", "--changed", "docs/a.md")
        self.assertEqual(o["mode"], "light")

    def test_unreadable_diff_runs_everything(self):
        o, out = self.outputs("--event", "pull_request", "--base", "no-such-ref-xyz")
        self.assertEqual((o["mode"], o["heavy"], o["runtime"]), ("full", "true", "true"))
        self.assertIn("Selection failed", out)

    def test_broken_mapping_runs_everything(self):
        with tempfile.TemporaryDirectory() as t:
            p = Path(t) / "s.toml"
            p.write_text("not toml [")
            o, _ = self.outputs("--config", str(p), "--changed", "docs/a.md")
            self.assertEqual((o["mode"], o["heavy"], o["runtime"]), ("full", "true", "true"))


if __name__ == "__main__":
    unittest.main()
