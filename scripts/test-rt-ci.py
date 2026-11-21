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

    def test_the_shards_cover_every_step_once(self):
        # CI's three rt jobs run the shards; together they are every step,
        # each in one shard (the R_HOME tree, seconds, in all of them)
        crates = ["robigo-rt", "robigo-rtcore"]
        everything = [s[0] for s in rt.steps(crates)]
        seen = {}
        for shard in [s for s in rt.SHARDS if s not in rt.OWN_STEPS]:
            titles = [s[0] for s in rt.shard_steps(crates, shard)]
            self.assertEqual(titles, [t for t in everything if t in titles], f"{shard}: steps' order")
            self.assertTrue(any(rt.shard_of(t) == shard for t in titles), f"{shard} runs nothing of its own")
            for t in titles:
                seen.setdefault(t, []).append(shard)
        self.assertEqual(sorted(seen), sorted(everything))
        for t, shards in seen.items():
            if rt.shard_of(t) is None:
                self.assertEqual(shards, [s for s in rt.SHARDS if s not in rt.OWN_STEPS], t)
            else:
                self.assertEqual(shards, [rt.shard_of(t)], t)
        # where each long step goes (the split's point): the crates' tests
        # where the crates' test build is, the long conformance runs apart
        where = {t: rt.shard_of(t) for t in everything}
        self.assertEqual(where["the crates' tests (quick profile)"], "modes")
        self.assertEqual(where["ROBIGO_GC_MOVE (move every movable object at every collection), with gctorture"],
                         "modes")
        self.assertEqual(where["ROBIGO_MATERIALIZE=1 (every argument slot materialized)"], "modes")
        self.assertEqual([t for t, s in where.items() if s == "gctorture"],
                         ["conformance under gctorture (E3)",
                          f"conformance under ROBIGO_GC_MOVE={rt.GC_MOVE_YOUNG} with gctorture (E3)"])
        self.assertEqual([t for t, s in where.items() if s == "builtin-diff"],
                         ["builtin differential and its planted divergences (A4, scripts/builtin-diff.py ci)"])
        self.assertEqual([t for t, s in where.items() if s == "feature-builds"],
                         ["conformance under ROBIGO_RETRY_TORTURE=1 (E9: every build's retry runs once)",
                          "ROBIGO_POISON_DEAD=1 (dead slots poisoned), robigo-rt's tests and the planted early clear",
                          "ROBIGO_POISON_DEAD=1, conformance"])

    def test_each_step_builds_in_its_profile(self):
        # the crates' tests in the quick profile; everything that runs the
        # binary users get (each conformance step and mode, boot_db) on
        # release; the release-crate-tests shard runs the crates' tests on
        # release; neither shard of its own is in a plain run
        crates = ["robigo-heap", "robigo-rt", "robigo-rtcore"]
        for title, _, cmd in rt.steps(crates):
            if cmd[:2] != ["cargo", "test"]:
                continue
            if rt.CONFORMANCE in cmd and "--skip" not in cmd or "boot_db" in cmd or "--no-run" in cmd:
                self.assertEqual(cmd[2], "--release", title)
            else:
                self.assertEqual(cmd[2:4], ["--profile", "quick"], title)
        # the release binary is built before the first step that runs it
        titles = [t for t, _, _ in rt.steps(crates)]
        built = titles.index("the release binary and the conformance, built (cargo test --release --no-run)")
        self.assertLess(built, titles.index("base's database tree, written by this robigo-rt (scripts/rt-home-db.sh)"))
        self.assertTrue(all(c[2] != "--release" for t, _, c in rt.steps(crates)[:built] if c[:2] == ["cargo", "test"]))
        rel = rt.shard_steps(crates, "release-crate-tests")
        self.assertEqual([c[2] for _, _, c in rel if c[:2] == ["cargo", "test"]], ["--release"])
        self.assertEqual(rel[0][0], "the R_HOME tree, tools.so built (scripts/rt-home.sh)")
        plain = " ".join(t for t, _, _ in rt.steps(crates))
        self.assertNotIn("release profile", plain)
        self.assertEqual(set(rt.OWN_STEPS), {"debug-assertions", "release-crate-tests"})

    def test_every_crate_test_step_builds_what_the_first_built(self):
        # cargo turns a selected crate's dev-dependency features on, so a
        # step over fewer crates (no --features of its own) rebuilds what
        # the crates' first step built: the MATERIALIZE step did, for 6 minutes
        crates = ["robigo-heap", "robigo-rt", "robigo-rtcore"]
        pkgs = [x for c in crates for x in ("-p", c)]
        for title, _, cmd in rt.steps(crates):
            if cmd[:2] == ["cargo", "test"] and "--features" not in cmd and "--test" not in cmd:
                self.assertEqual([x for x in cmd if x == "-p" or x.startswith("robigo")], pkgs, title)

    def budgets(self):
        import math
        import re
        with open(os.path.join(HERE, "..", ".github", "workflows", "ci.yml")) as f:
            yml = f.read()
        timeouts = {}
        for job in re.split(r"\n(?=  [a-z0-9-]+:\n)", yml):
            m = re.search(r"rt-ci\.py --shard ([a-z-]+)", job)
            if m:
                self.assertNotIn(m.group(1), timeouts, f"two jobs run shard {m.group(1)}")
                timeouts[m.group(1)] = int(re.search(r"timeout-minutes: (\d+)", job).group(1))
        return math, timeouts

    def test_each_shard_has_a_budget_its_timeout_doubles(self):
        # the three relations; the numbers are the measurements'
        math, timeouts = self.budgets()
        self.assertEqual(sorted(rt.BUDGETS), sorted(rt.SHARDS))
        self.assertEqual(sorted(timeouts), sorted(rt.SHARDS), "each shard has one CI job")
        for shard, (budget, measured, where) in rt.BUDGETS.items():
            self.assertEqual(budget, math.ceil(measured * 1.2), f"{shard}: the measured time plus 20%, rounded up")
            self.assertEqual(timeouts[shard], 2 * budget, f"{shard}: its job's timeout is twice its budget")
            self.assertRegex(where, r"^20\d\d-\d\d-\d\d", f"{shard}: when it was measured")
        self.assertEqual(rt.WARN_AT, 0.9)

    def test_the_timing_table_warns_at_90_percent_of_the_budget(self):
        budget = rt.BUDGETS["modes"][0]
        below = budget * 0.9 * 60 - 6
        at = budget * 0.9 * 60 + 6
        self.assertIsNone(rt.budget_warning("modes", below / 60))
        self.assertNotIn("::warning::", rt.timings_report([("release tests", below, True)], "modes"))
        report = rt.timings_report([("release tests", at, True)], "modes")
        self.assertIn("::warning::rt: shard modes took", report)
        self.assertIn(f"{2 * budget}-minute timeout", report)
        # a plain run has no budget
        self.assertNotIn("budget", rt.timings_report([("release tests", at * 3, True)]))

    def test_the_case_times_report_lists_the_cases_over_half_their_limit(self):
        import tempfile
        path = os.path.join(tempfile.mkdtemp(), "t.tsv")
        with open(path, "w") as f:
            f.write("# secs\tlimit\tstatus\tcase\n"
                    "503.0\t600\tpass\ttests/rt/jit/serialize-write.R\n"
                    "301.0\t600\tfail\ttests/rt/a.R\n"
                    "300.0\t600\tpass\ttests/rt/half.R\n"
                    "11.0\t20\tpass\ttests/rt/b.R\n"
                    "0.4\t20\tpass\ttests/rt/c.R\n")
        self.assertEqual([r[3] for r in rt.slow_cases(path)],
                         ["tests/rt/jit/serialize-write.R", "tests/rt/a.R", "tests/rt/b.R"])
        report = rt.slow_cases_report("conformance under x", path)
        self.assertIn("3 case(s) over 50% of their limit", report)
        self.assertIn("  503.0 s of  600 s  pass  tests/rt/jit/serialize-write.R", report)
        self.assertIn("no case over", rt.slow_cases_report("y", os.path.join(os.path.dirname(path), "none.tsv")))

    def test_every_conformance_step_writes_its_case_times_and_lists_them_even_when_it_fails(self):
        # the runner's ROBIGO_CONF_TIMES_REPORT, a file per step, for every
        # conformance step rt-ci runs (the shards', the nightly's): not for
        # a step that runs no conformance
        from unittest import mock
        import contextlib, io, tempfile
        seen = []

        def fake(cmd, cwd, env):
            seen.append(env.get("ROBIGO_CONF_TIMES_REPORT"))
            if env.get("ROBIGO_CONF_TIMES_REPORT"):
                with open(env["ROBIGO_CONF_TIMES_REPORT"], "w") as f:
                    f.write("# secs\tlimit\tstatus\tcase\n700.0\t600\tfail\ttests/rt/x.R\n")
            return mock.Mock(returncode=0 if len(seen) == 1 else 1)
        plan = [("crates' tests", {}, ["cargo", "test", "--", "--skip", rt.CONFORMANCE]),
                ("conformance under gctorture (E3)", {}, rt.conformance_command())]
        out = io.StringIO()
        with mock.patch.object(rt, "TIMES_DIR", tempfile.mkdtemp()), mock.patch.object(rt.subprocess, "run", fake), \
                contextlib.redirect_stdout(out):
            self.assertEqual(rt.run_steps(plan, []), 1)
        self.assertIsNone(seen[0])
        self.assertTrue(seen[1] and seen[1].endswith("conformance-under-gctorture-E3.tsv"), seen)
        self.assertIn("1 case(s) over 50% of their limit", out.getvalue())
        self.assertIn("tests/rt/x.R", out.getvalue())
        # the nightly's full-move step is a conformance step like the others
        for _, _, cmd in rt.full_move_steps():
            self.assertTrue(rt.CONFORMANCE in cmd and "--exact" in cmd)

    def test_a_step_whose_report_fails_is_timed_as_failed(self):
        # the command passed, the crosscheck report did not (it compared
        # nothing: there is none), so the job fails and its time row says so
        from unittest import mock
        import tempfile
        missing = os.path.join(tempfile.mkdtemp(), "none.txt")
        done = []
        ok = mock.Mock(returncode=0)
        with mock.patch.object(rt.subprocess, "run", return_value=ok):
            r = rt.run_steps([("crosscheck", {"ROBIGO_BC_CROSSCHECK_REPORT": missing}, ["true"])], done)
        self.assertEqual(r, 1)
        self.assertEqual([(t, ok_) for t, _, ok_ in done], [("crosscheck", False)])

    def test_the_debug_assertions_shard_builds_apart(self):
        # its own build (RUSTFLAGS) in its own target directory, never in the
        # plain run, and the conformance and the crates' tests both on it
        from unittest import mock
        with mock.patch.object(rt, "target_dir", return_value="/t/target"):
            steps = rt.shard_steps(["robigo-rt"], "debug-assertions")
        self.assertEqual([t for t, _, _ in steps][0], "the R_HOME tree, tools.so built (scripts/rt-home.sh)")
        # every step, the R_HOME tree included (rt-home.sh builds robigo-heap
        # for its headers), builds with the flag into the shard's target
        for _, e, _ in steps:
            self.assertEqual(e["RUSTFLAGS"], "-C debug-assertions=on")
            self.assertEqual(e["CARGO_TARGET_DIR"], "/t/target-debug-assertions")
        built = [(e, c) for _, e, c in steps[1:]]
        self.assertEqual(len(built), 2)
        self.assertEqual(built[0][1][:5], ["cargo", "test", "--profile", "quick", "--locked"])
        self.assertEqual(built[1][1][:4], ["cargo", "test", "--release", "--locked"])
        self.assertIn(rt.CONFORMANCE, built[1][1])
        self.assertNotIn("debug assertions", " ".join(t for t, _, _ in rt.steps(["robigo-rt"])))

    def test_full_move_and_shard_are_refused_together(self):
        # --full-move runs the nightly's one step: with --shard as well the
        # shard would be dropped silently
        import contextlib, io
        err = io.StringIO()
        with self.assertRaises(SystemExit) as e, contextlib.redirect_stderr(err):
            rt.main(["--full-move", "--shard", "modes"])
        self.assertEqual(e.exception.code, 2)
        self.assertIn("--full-move and --shard select different steps", err.getvalue())

    def test_the_step_times_are_reported_slowest_first(self):
        r = rt.timings_report([("a", 60.0, True), ("b", 600.0, True), ("c", 30.0, False)])
        lines = r.splitlines()
        self.assertEqual(lines[0], "rt: step times (wall), slowest first:")
        self.assertIn("10.00 min  b", lines[1])
        self.assertIn("FAILED  c", lines[3])
        self.assertIn("11.50 min  in all", lines[4])

    def test_conformance_runs_alone_in_its_own_steps(self):
        steps = rt.steps(["robigo-rt", "robigo-rtcore"])
        # (a step's mode is its environment but the runner's job count)
        own = [({k: v for k, v in env.items() if k != "ROBIGO_CONF_JOBS"}, cmd) for _, env, cmd in steps
               if rt.CONFORMANCE in cmd and "--skip" not in cmd]
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
                                 "ROBIGO_BC_CROSSCHECK", "ROBIGO_RETRY_TORTURE", "ROBIGO_POISON_DEAD"} & set(env), env)
            # a crosscheck step writes its report, which `main` reads
            if "ROBIGO_BC_CROSSCHECK" in env:
                self.assertIn("ROBIGO_BC_CROSSCHECK_REPORT", env)
        # the crosscheck step exists at all: dropping it from steps() would
        # leave every assertion above green, and V3's gate (`main` failing on
        # an unexpected or untranslated closure) would leave CI unnoticed
        self.assertEqual(len([e for e, _ in own if "ROBIGO_BC_CROSSCHECK" in e]), 1)
        for title, _, cmd in steps:
            # (a build-only step, --no-run, runs no test at all)
            if cmd[0] == "cargo" and rt.CONFORMANCE not in cmd[-1:] and "--exact" not in cmd and "--no-run" not in cmd:
                self.assertEqual(cmd[-2:], ["--skip", rt.CONFORMANCE], title)
        # The test exists under that name: after a rename the conformance step
        # would run nothing, and still pass.
        src = (HERE.parent / "crates/robigo-rtapi/tests/conformance.rs").read_text()
        self.assertIn(f"fn {rt.CONFORMANCE}()", src)

    def test_the_r_home_tree_is_made_before_any_conformance_step(self):
        # the cases that load tools' native code need it (r-home-tools)
        steps = rt.steps(["robigo-rt", "robigo-rtcore"])
        names = [c for _, _, c in steps]
        tree = names.index(["scripts/rt-home.sh"])
        first = next(i for i, c in enumerate(names) if rt.CONFORMANCE in c and "--exact" in c)
        self.assertLess(tree, first)

    def test_the_check_features_are_built_for_the_last_steps_alone(self):
        # every other step tests the binary users get; the last ones, the
        # modes compiled in (one build for all of them) and each step's on:
        # ROBIGO_RETRY_TORTURE with the collector modes' case timeout, then
        # ROBIGO_POISON_DEAD on robigo-rt's tests and the conformance
        steps = rt.steps(["robigo-rt", "robigo-rtcore"])
        featured = [i for i, (_, _, cmd) in enumerate(steps) if "--features" in cmd]
        self.assertEqual(featured, list(range(len(steps) - 3, len(steps))), "the feature builds come last")
        for i in featured:
            cmd = steps[i][2]
            self.assertEqual(cmd[cmd.index("--features") + 1], rt.CHECK_FEATURES)
        retry = [t for t, e, _ in steps if e.get("ROBIGO_RETRY_TORTURE") == "1"]
        self.assertEqual(retry, [steps[featured[0]][0]])
        self.assertEqual([t for t, e, _ in steps if "ROBIGO_RETRY_TORTURE" in e], retry)
        self.assertEqual(steps[featured[0]][1].get("ROBIGO_CONF_TIMEOUT"), "600")
        poison = [t for t, e, _ in steps if e.get("ROBIGO_POISON_DEAD") == "1"]
        self.assertEqual(poison, [steps[i][0] for i in featured[1:]])
        self.assertIn("robigo-rt", steps[featured[1]][2])
        self.assertIn(rt.CONFORMANCE, steps[featured[2]][2])
        # each feature exists where the steps name it
        for f in rt.CHECK_FEATURES.split(","):
            crate, feature = f.split("/")
            manifest = (HERE.parent / "crates" / crate / "Cargo.toml").read_text()
            self.assertIn(f"\n{feature} = [", manifest)

    def test_conformance_steps_use_every_cpu(self):
        # they run one test alone, so the runner may run a case per CPU
        steps = rt.steps(["robigo-rt", "robigo-rtcore"]) + rt.full_move_steps()
        conf = [(t, e) for t, e, c in steps if rt.CONFORMANCE in c and "--exact" in c]
        self.assertTrue(conf)
        for title, env in conf:
            self.assertEqual(env.get("ROBIGO_CONF_JOBS"), str(os.cpu_count() or 2), title)
        others = [t for t, e, c in steps if "ROBIGO_CONF_JOBS" in e and not (rt.CONFORMANCE in c and "--exact" in c)]
        self.assertEqual(others, [])

    def test_the_moving_conformance_step_is_young_and_the_full_run_nightly(self):
        # H8: the job's GC_MOVE + gctorture conformance step moves the young
        # objects, with YOUNG_MOVE_SLOW's longer timeouts and no other step's;
        # the nightly's --full-move run moves everything, with
        # FULL_MOVE_SLOW's; each names a case that exists
        steps = rt.steps(["robigo-rt"])
        moving = [e for t, e, c in steps if "ROBIGO_GC_MOVE" in e and "--exact" in c]
        self.assertEqual(len(moving), 1)
        self.assertEqual(moving[0]["ROBIGO_GC_MOVE"], rt.GC_MOVE_YOUNG)
        self.assertEqual([e for t, e, _ in steps if "ROBIGO_CONF_TIMEOUTS" in e], moving)
        young = dict(item.rsplit("=", 1) for item in moving[0]["ROBIGO_CONF_TIMEOUTS"].split(","))
        self.assertEqual(set(young), set(rt.YOUNG_MOVE_SLOW))
        for case, secs in young.items():
            self.assertTrue((HERE.parent / case).is_file(), case)
            self.assertGreater(int(secs), int(moving[0]["ROBIGO_CONF_TIMEOUT"]), case)
        # (the step's own limit stays: a hung case still fails in ten minutes)
        self.assertEqual(moving[0]["ROBIGO_CONF_TIMEOUT"], "600")
        self.assertTrue(rt.GC_MOVE_YOUNG.startswith("young:") and int(rt.GC_MOVE_YOUNG[6:]) >= 1)
        full = rt.full_move_steps()
        self.assertEqual(len(full), 1)
        _, env, cmd = full[0]
        self.assertEqual((env["ROBIGO_GC_MOVE"], env["ROBIGO_GCTORTURE"]), ("1", "1"))
        self.assertIn(rt.CONFORMANCE, cmd)
        named = dict(item.rsplit("=", 1) for item in env["ROBIGO_CONF_TIMEOUTS"].split(","))
        self.assertEqual(set(named), set(rt.FULL_MOVE_SLOW))
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

    def test_the_crosscheck_report_passes_only_with_every_difference_classed(self):
        import tempfile

        def check(text):
            with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as f:
                f.write(text)
            try:
                return rt.bc_report_check(f.name)
            finally:
                os.unlink(f.name)

        loop = "expected (a loop's shape (checkSkipLoopCntxt, cmpForBody))"
        ok = (f"{loop}: for (v in x) v: at 2: translated Jmp regs [] imm [] consts 0 target Some(4); "
              "from source Loop regs [] imm [] consts 0 target Some(6)\n"
              "same: function(x) x + 1\n"
              "skipped: f(x): the from-source compile would force a binding\n")
        self.assertEqual(check(ok), ([], {loop: 1, "same": 1, "skipped": 1, "compared": 2}))
        bad = ("unexpected: g(x): at 0: translated GetVar regs [0] imm [] consts 1 target None; "
               "from source GetFun regs [0] imm [0] consts 2 target None")
        self.assertEqual(check(ok + bad + "\n")[0], [bad])
        # a malformed line, a kind matched only by its prefix, or nothing
        # compared (only skipped closures, an empty or a missing report)
        # fails rather than passing
        self.assertEqual(len(check(ok + "garbage\n")[0]), 1)
        self.assertEqual(len(check(ok + "sameness: h(x)\n")[0]), 1)
        self.assertEqual(len(check(ok + "skipped (why): h(x)\n")[0]), 1)
        self.assertEqual(check("skipped: f(x): the from-source compile would force a binding\n")[1]["compared"], 0)
        self.assertEqual(len(check("skipped: f(x): the from-source compile would force a binding\n")[0]), 1)
        self.assertEqual(len(check("")[0]), 1)
        self.assertEqual(len(rt.bc_report_check("/nonexistent/robigo-bc-report.txt")[0]), 1)

    def test_this_workspace_holds_the_premise(self):
        m = rt.metadata()
        crates = rt.rt_crates(m, PATS)
        self.assertIn("robigo-rt", crates)
        self.assertNotIn("robigo-cli", crates)
        self.assertEqual(rt.leaks(m, crates), [])


if __name__ == "__main__":
    unittest.main()
