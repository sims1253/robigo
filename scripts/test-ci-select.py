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
sys.path.insert(0, str(HERE))

spec = importlib.util.spec_from_file_location("ci_select", HERE / "ci-select.py")
sel = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sel)

RULES = sel.load()


def pick(*paths):
    mode, runtime, rows = sel.decide(list(paths), RULES)
    return mode, runtime, rows


class Selection(unittest.TestCase):
    def test_docs_only_is_light(self):
        mode, runtime, rows = pick("docs/verification.md", "docs/ledger/test-infra.md")
        self.assertEqual((mode, runtime), ("light", False))
        self.assertTrue(all(r == "no-binary-effect" for _, r in rows))

    def test_readme_and_markdown_is_light(self):
        self.assertEqual(pick("README.md")[:2], ("light", False))
        self.assertEqual(pick("AGENTS.md", "docs/design.md")[:2], ("light", False))

    def test_test_data_is_light(self):
        self.assertEqual(pick("tests/r/strings/escape.R", "tests/corpora/fastr/passing/x.pass")[:2], ("light", False))
        self.assertEqual(pick("tests/packages/passing/rlang/a/b.pass")[:2], ("light", False))

    def test_runtime_changes_run_everything_and_mark_runtime(self):
        for p in ("crates/robigo-runtime/src/interp.rs", "crates/robigo-syntax/src/lib.rs",
                  "rlib/base/R/apply.R", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml"):
            mode, runtime, rows = pick(p)
            self.assertEqual((mode, runtime, rows[0][1]), ("full", True, "runtime"), p)

    def test_crate_tests_are_runtime(self):
        # under crates/**, so conservative: runtime (they compile against the runtime)
        self.assertEqual(pick("crates/robigo-runtime/tests/foo.rs")[:2], ("full", True))

    def test_gate_scripts_and_workflows_run_heavy_but_not_runtime(self):
        for p in ("scripts/package-suite.py", "scripts/corpus.py", "scripts/perf.py",
                  ".github/workflows/ci.yml", ".github/workflows/packages.yml",
                  ".github/actions/setup/action.yml", "benchmarks/selection.toml"):
            mode, runtime, rows = pick(p)
            self.assertEqual((mode, runtime, rows[0][1]), ("full", False, "gates-and-infra"), p)

    def test_mixed_with_runtime_runs_everything(self):
        self.assertEqual(pick("docs/verification.md", "crates/robigo-runtime/src/vm.rs")[:2], ("full", True))
        self.assertEqual(pick("tests/r/x.R", "scripts/ci-select.py")[:2], ("full", False))

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
        self.assertTrue(g("**/*.md").match("README.md"))
        self.assertTrue(g("**/*.md").match("a/b/c.md"))
        self.assertFalse(g("*.md").match("a/b.md"))
        self.assertTrue(g("crates/*/tests/**").match("crates/a/tests/x/y.rs"))


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

    def test_outputs_for_light_runtime_and_full(self):
        o, _ = self.outputs("--changed", "docs/verification.md")
        self.assertEqual((o["mode"], o["heavy"], o["runtime"]), ("light", "false", "false"))
        o, _ = self.outputs("--changed", "crates/robigo-runtime/src/vm.rs")
        self.assertEqual((o["mode"], o["heavy"], o["runtime"]), ("full", "true", "true"))
        o, _ = self.outputs("--changed", "scripts/package-suite.py")
        self.assertEqual((o["mode"], o["heavy"], o["runtime"]), ("full", "true", "false"))
        o, _ = self.outputs("--changed", "unknown/x")
        self.assertEqual((o["mode"], o["heavy"], o["runtime"]), ("full", "true", "true"))

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
