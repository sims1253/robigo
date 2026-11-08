#!/usr/bin/env python3
"""Check the perf gate's path-scoped benchmark selection (scripts/perf-select.py,
benchmarks/selection.toml) and that the gate handles a narrowed run."""
import contextlib
import importlib.util
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
sys.path.insert(0, str(HERE))
import perf  # noqa: E402

spec = importlib.util.spec_from_file_location("select_", HERE / "perf-select.py")
sel = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sel)

GROUPS, RULES = sel.load()
ALL = [p[0] for p in perf.programs(perf.conf())]


def pick(*paths):
    mode, flt, rows = sel.decide(list(paths), GROUPS, RULES)
    names = [p[0] for p in perf.programs(perf.conf(), None, flt)] if mode == "subset" else \
        (ALL if mode == "full" else [])
    return mode, names, rows


class Selection(unittest.TestCase):
    def test_vm_change_runs_everything(self):
        # Every closure body, base R's included, runs on the bytecode VM.
        for p in ("crates/robigo-runtime/src/vm.rs", "crates/robigo-runtime/src/compile.rs"):
            mode, names, rows = pick(p)
            self.assertEqual((mode, rows[0][1]), ("full", "vm"), p)

    def test_core_numerics_run_everything(self):
        # %% (fasta), sum's long double (knucleotide) and the RNG (csv).
        for p in ("crates/robigo-numerics/src/arith.rs", "crates/robigo-numerics/src/ldouble.rs",
                  "crates/robigo-numerics/src/rng.rs"):
            self.assertEqual(pick(p)[0], "full", p)

    def test_jit_change_runs_loops_not_idioms(self):
        mode, names, _ = pick("crates/robigo-runtime/src/jit/closure.rs")
        self.assertEqual(mode, "subset")
        self.assertIn("robigo/fib", names)
        self.assertIn("rbench/shootout/nbody", names)
        self.assertIn("robigo/s4-generic", names)
        self.assertNotIn("rbench/shootout/knucleotide_brute_3", names)
        self.assertNotIn("robigo/csv", names)

    def test_evaluator_change_adds_idioms(self):
        mode, names, _ = pick("crates/robigo-runtime/src/interp.rs")
        self.assertEqual(mode, "subset")
        self.assertIn("robigo/fib", names)
        self.assertIn("rbench/shootout/knucleotide_brute_3", names)

    def test_builtins_change_runs_idioms(self):
        mode, names, rows = pick("crates/robigo-runtime/src/builtins/paste.rs")
        self.assertEqual(mode, "subset")
        self.assertEqual(rows[0][1], "builtins")
        self.assertIn("robigo/strings", names)
        self.assertNotIn("robigo/fib", names)

    def test_hot_builtin_also_runs_loops(self):
        mode, names, rows = pick("crates/robigo-runtime/src/builtins/subassign.rs")
        self.assertEqual(rows[0][1], "hot-builtins")
        self.assertIn("robigo/fib", names)
        self.assertIn("robigo/strings", names)

    def test_parser_change_runs_robigo_programs(self):
        mode, names, _ = pick("crates/robigo-syntax/src/lexer.rs")
        self.assertEqual(mode, "subset")
        self.assertTrue(names and all(n.startswith("robigo/") for n in names))

    def test_numerics_change(self):
        mode, names, _ = pick("crates/robigo-numerics/src/nmath/pt.rs")
        self.assertEqual(mode, "subset")
        self.assertIn("robigo/lm", names)
        self.assertNotIn("robigo/closures", names)

    def test_docs_only_runs_nothing(self):
        self.assertEqual(pick("docs/verification.md", "README.md", "tests/r/x/y.R")[0], "none")

    def test_mixed_is_the_union_and_ignores_docs(self):
        _, vm, _ = pick("crates/robigo-runtime/src/jit.rs")
        _, bi, _ = pick("crates/robigo-runtime/src/builtins/paste.rs")
        mode, both, _ = pick("crates/robigo-runtime/src/jit.rs", "docs/x.md",
                             "crates/robigo-runtime/src/builtins/paste.rs")
        self.assertEqual(mode, "subset")
        self.assertEqual(set(both), set(vm) | set(bi))

    def test_unknown_path_runs_everything(self):
        for p in ("crates/robigo-runtime/src/value.rs", "crates/new-crate/src/lib.rs", "something-new.txt",
                  "crates/robigo-runtime/src/jit_new.rs"):
            mode, names, rows = pick(p)
            self.assertEqual((mode, len(names)), ("full", len(ALL)), p)
            self.assertIsNone(rows[0][1])

    def test_one_unknown_path_among_known_ones_runs_everything(self):
        self.assertEqual(pick("crates/robigo-runtime/src/jit.rs", "docs/a.md", "weird/file")[0], "full")

    def test_infrastructure_runs_everything(self):
        for p in (".github/workflows/ci.yml", "benchmarks/benchmarks.toml", "benchmarks/selection.toml",
                  "scripts/perf.py", "Cargo.lock", "crates/robigo-runtime/Cargo.toml", "rlib/base/R/apply.R"):
            self.assertEqual(pick(p, "docs/a.md")[0], "full", p)

    def test_empty_diff_runs_everything(self):
        self.assertEqual(pick()[0], "full")

    def test_first_matching_rule_wins(self):
        self.assertEqual(pick("crates/robigo-runtime/src/builtins/s4.rs")[2][0][1], "s4")
        self.assertEqual(pick("crates/robigo-runtime/src/builtins/arith_exact.rs")[2][0][1], "hot-builtins")

    def test_glob(self):
        g = sel.glob_re
        self.assertTrue(g("docs/**").match("docs/ledger/a.md"))
        self.assertTrue(g("**/*.md").match("README.md"))
        self.assertTrue(g("**/*.md").match("a/b/c.md"))
        self.assertFalse(g("*.md").match("a/b.md"))
        self.assertFalse(g("crates/*/tests/**").match("crates/a/b/tests/x"))
        self.assertTrue(g("crates/*/tests/**").match("crates/a/tests/x/y.rs"))
        self.assertFalse(g("scripts/perf*.py").match("scripts/perfXpy"))


class Config(unittest.TestCase):
    def test_every_group_selects_benchmarks(self):
        for name, pats in GROUPS.items():
            for p in pats:
                self.assertTrue(perf.programs(perf.conf(), None, p), f"group {name}: {p} matches nothing")

    def test_every_group_is_used(self):
        used = {g for _, _, s in RULES for g in s}
        self.assertEqual(set(GROUPS) - used, set(), "unused groups")

    def test_every_rule_pattern_matches_a_tracked_file(self):
        files = subprocess.run(["git", "-c", "safe.directory=*", "ls-files"], cwd=ROOT, capture_output=True, text=True, check=True).stdout.split("\n")
        with open(sel.CONFIG, "rb") as f:
            raw = sel.tomllib.load(f)["rules"]
        for r in raw:
            for pat in r["paths"]:
                rx = sel.glob_re(pat)
                self.assertTrue(any(rx.match(f) for f in files), f"rule {r['name']}: {pat} matches no file")

    def test_bad_group_reference_is_rejected(self):
        with tempfile.TemporaryDirectory() as t:
            p = Path(t) / "s.toml"
            p.write_text('[groups]\na=["x"]\n[[rules]]\nname="r"\npaths=["a"]\nselect=["b"]\n')
            with self.assertRaises(ValueError):
                sel.load(str(p))

    def test_empty_select_is_rejected(self):
        # An emptied select would silently act like "none".
        with tempfile.TemporaryDirectory() as t:
            p = Path(t) / "s.toml"
            p.write_text('[groups]\na=["x"]\n[[rules]]\nname="r"\npaths=["a"]\nselect=[]\n')
            with self.assertRaises(ValueError):
                sel.load(str(p))


class Cli(unittest.TestCase):
    def run_cli(self, *args, env=None):
        e = {**os.environ, **(env or {})}
        e.pop("GITHUB_EVENT_NAME", None)
        e.update(env or {})
        return subprocess.run([sys.executable, str(HERE / "perf-select.py"), *args],
                              capture_output=True, text=True, env=e, cwd=ROOT)

    def outputs(self, *args, env=None):
        with tempfile.TemporaryDirectory() as t:
            out = Path(t) / "o"
            p = self.run_cli(*args, "--github-output", env={"GITHUB_OUTPUT": str(out), **(env or {})})
            self.assertEqual(p.returncode, 0, p.stderr)
            return dict(l.split("=", 1) for l in out.read_text().splitlines()), p.stdout

    def test_outputs_for_subset_none_full(self):
        o, _ = self.outputs("--changed", "crates/robigo-syntax/src/lexer.rs")
        self.assertEqual((o["mode"], o["run"]), ("subset", "true"))
        self.assertTrue(o["filter"].startswith("(?:"))
        o, _ = self.outputs("--changed", "docs/a.md")
        self.assertEqual((o["mode"], o["run"], o["filter"]), ("none", "false", ""))
        o, _ = self.outputs("--changed", "x/y")
        self.assertEqual((o["mode"], o["run"], o["filter"]), ("full", "true", ""))

    def test_other_events_run_everything(self):
        for ev in ("push", "workflow_dispatch", "schedule"):
            o, out = self.outputs("--event", ev)
            self.assertEqual((o["mode"], o["filter"]), ("full", ""), ev)
        # even with a docs-only change list
        o, _ = self.outputs("--event", "push", "--changed", "docs/a.md")
        self.assertEqual(o["mode"], "none")  # explicit --changed is a dry run of the mapping

    def test_unreadable_diff_runs_everything(self):
        o, out = self.outputs("--event", "pull_request", "--base", "no-such-ref-xyz")
        self.assertEqual((o["mode"], o["filter"]), ("full", ""))
        self.assertIn("Selection failed", out)

    def test_broken_mapping_runs_everything(self):
        with tempfile.TemporaryDirectory() as t:
            p = Path(t) / "s.toml"
            p.write_text("not toml [")
            o, out = self.outputs("--config", str(p), "--changed", "docs/a.md")
            self.assertEqual((o["mode"], o["filter"]), ("full", ""))

    def test_real_git_diff(self):
        with tempfile.TemporaryDirectory() as t:
            def git(*a):
                subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@t", *a], cwd=t, check=True,
                               capture_output=True)
            git("init", "-q")
            (Path(t) / "docs").mkdir()
            (Path(t) / "docs" / "a.md").write_text("a")
            git("add", "-A"); git("commit", "-qm", "base")
            (Path(t) / "docs" / "a.md").write_text("b")
            git("add", "-A"); git("commit", "-qm", "head")
            orig = sel.ROOT
            sel.ROOT = t
            try:
                self.assertEqual(sel.changed_paths("HEAD^1", "HEAD"), ["docs/a.md"])
            finally:
                sel.ROOT = orig


class NarrowedGate(unittest.TestCase):
    def compare(self, base, new):
        with tempfile.TemporaryDirectory() as t:
            b, n = Path(t) / "b.json", Path(t) / "n.json"
            b.write_text(json.dumps({"results": base}))
            n.write_text(json.dumps(new))
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                rc = perf.cmd_compare(SimpleNamespace(base=str(b), new=str(n), threshold=0.5))
            return rc, out.getvalue()

    @staticmethod
    def row(instr):
        return {"robigo": {"instr": instr, "status": 0}}

    def test_left_out_benchmarks_are_not_removed_and_threshold_is_unchanged(self):
        base = {"robigo/fib": self.row(1000), "robigo/csv": self.row(1000), "rbench/x/y": self.row(1000)}
        sel_ = {"filter": "^robigo/fib$", "suite": None}
        rc, out = self.compare(base, {"selection": sel_, "results": {"robigo/fib": self.row(1004)}})
        self.assertEqual(rc, 0)
        self.assertNotIn("REMOVED", out)
        self.assertIn("2 baseline benchmark(s) outside this run's selection", out)
        # 0.5% still fails
        rc, out = self.compare(base, {"selection": sel_, "results": {"robigo/fib": self.row(1006)}})
        self.assertEqual(rc, 1)
        self.assertIn("SLOWER", out)

    def test_a_selected_benchmark_that_vanished_is_still_reported(self):
        base = {"robigo/fib": self.row(1000), "robigo/csv": self.row(1000)}
        rc, out = self.compare(base, {"selection": {"filter": "^robigo/", "suite": None},
                                      "results": {"robigo/fib": self.row(1000)}})
        self.assertIn("REMOVED robigo/csv", out)

    def test_full_run_reports_removed_as_before(self):
        base = {"robigo/fib": self.row(1000), "robigo/csv": self.row(1000)}
        rc, out = self.compare(base, {"results": {"robigo/fib": self.row(1000)}})
        self.assertIn("REMOVED robigo/csv", out)


if __name__ == "__main__":
    unittest.main()
