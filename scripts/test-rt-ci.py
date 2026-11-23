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


# The caller's settings that change what rt-ci.py plans and where its pool
# is: no test may see them, and every test sees the same CPU count
# (every ROBIGO_CONF_* variable, and these)
CALLER_ENV = ("GITHUB_ACTIONS", "XDG_RUNTIME_DIR", "RT_CI_MIN_FREE_GB")
CPUS = 8
BRANCH_CHANGES = rt.branch_changes


class RtCi(unittest.TestCase):
    def setUp(self):
        from unittest import mock
        env = mock.patch.dict(os.environ)
        env.start()
        self.addCleanup(env.stop)
        for k in [k for k in os.environ if k in CALLER_ENV or k.startswith("ROBIGO_CONF_")]:
            os.environ.pop(k)
        # the pool's fallback under a directory of the test's own: no test
        # makes the user's real /tmp pool (main() sets one up)
        import shutil, tempfile
        tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, tmp, ignore_errors=True)
        fallback = mock.patch.object(rt.conf_slots, "FALLBACK_ROOT", tmp)
        fallback.start()
        self.addCleanup(fallback.stop)
        # (and the steps' times under it: RT_CI_TIMES_DIR was read at import)
        times = mock.patch.object(rt, "TIMES_DIR", os.path.join(tmp, "times"))
        times.start()
        self.addCleanup(times.stop)
        cpus = mock.patch.object(os, "cpu_count", return_value=CPUS)
        cpus.start()
        self.addCleanup(cpus.stop)
        # main() looks at this checkout's own branch for base-subset's
        # inputs: no test may depend on what the branch under test changes
        changes = mock.patch.object(rt, "branch_changes", return_value=[])
        changes.start()
        self.addCleanup(changes.stop)

    def test_the_cache_mutation_starts_are_memory_capped(self):
        # every robigo-rt start of scripts/rt-tcache-mutate.py goes through
        # its capped `start` (RLIMIT_AS), and the script checks at its own
        # start that the cap is in force
        src = (HERE / "rt-tcache-mutate.py").read_text()
        self.assertEqual(src.count("subprocess.run("), 1)
        self.assertIn("preexec_fn=cap_memory", src)
        self.assertIn("resource.RLIMIT_AS", src)
        self.assertIn("check_cap(cmd, home, env)", src)

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
        everything = [s[0] for s in rt.builds_first(rt.steps(crates))]
        seen = {}
        for shard in [s for s in rt.SHARDS if s not in rt.OWN_STEPS]:
            titles = [s[0] for s in rt.shard_steps(crates, shard)]
            self.assertEqual(titles, [t for t in everything if t in titles], f"{shard}: steps' order")
            self.assertTrue(any(rt.shard_of(t) == shard for t in titles), f"{shard} runs nothing of its own")
            for t in titles:
                seen.setdefault(t, []).append(shard)
        self.assertEqual(sorted(seen), sorted(everything))
        shared = (rt.RELEASE_BUILD_TITLE, rt.db_tree("")[0])
        for t, shards in seen.items():
            if rt.shard_of(t) is None:
                self.assertEqual(shards, [s for s in rt.SHARDS if s not in rt.OWN_STEPS], t)
            elif t in shared:
                # the release build and its tree: in modes and each shard
                # that runs the binary at the default boot
                self.assertEqual(shards, [s for s in rt.SHARDS if s == "modes" or s in rt.TREE_SHARDS], t)
            else:
                self.assertEqual(shards, [rt.shard_of(t)], t)
        # where each long step goes (the split's point): the crates' tests
        # where the crates' test build is, the long conformance runs apart
        # (each build step goes where its tests run: the lists below name
        # the runs)
        for t in everything:
            if t.endswith(": its tests built (--no-run)"):
                self.assertEqual(rt.shard_of(t), rt.shard_of(t.removesuffix(": its tests built (--no-run)")), t)
        where = {t: rt.shard_of(t) for t in everything if not t.endswith(": its tests built (--no-run)")}
        self.assertEqual(where["the crates' tests (quick profile)"], "modes")
        self.assertEqual(where["ROBIGO_GC_MOVE (move every movable object at every collection), with gctorture"],
                         "modes")
        self.assertEqual(where["ROBIGO_MATERIALIZE=1 (every argument slot materialized)"], "modes")
        self.assertEqual([t for t, s in where.items() if s == "gctorture"],
                         ["conformance under M1's loader with the boot heap's freeze off (ROBIGO_RT_BOOT=m1, "
                          "ROBIGO_GC_FREEZE=0)",
                          "conformance under gctorture (E3)",
                          f"conformance under ROBIGO_GC_MOVE={rt.GC_MOVE_YOUNG} with gctorture (E3)"])
        self.assertEqual([t for t, s in where.items() if s == "builtin-diff"],
                         ["builtin differential and its planted divergences (A4, scripts/builtin-diff.py ci)"])
        self.assertEqual([t for t, s in where.items() if s == "feature-builds"],
                         ["ROBIGO_RETRY_TORTURE's build (with poison-dead), built (cargo test --release --no-run)",
                          "base's database tree for ROBIGO_RETRY_TORTURE's build (scripts/rt-home-db.sh)",
                          "conformance under ROBIGO_RETRY_TORTURE=1 (E9: every build's retry runs once)",
                          "a translation-cache verification under ROBIGO_RETRY_TORTURE=1 compares nothing for a "
                          "retry (tests/boot_db.rs)",
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
        self.assertEqual({c[2] for _, _, c in rel if c[:2] == ["cargo", "test"]}, {"--release"})
        self.assertEqual(rel[0][0], "the R_HOME tree, tools.so built (scripts/rt-home.sh)")
        plain = " ".join(t for t, _, _ in rt.steps(crates))
        self.assertNotIn("release profile", plain)
        self.assertEqual(set(rt.OWN_STEPS), {"debug-assertions", "release-crate-tests"})

    def test_the_database_tree_comes_before_every_run_of_the_crates_tests(self):
        # robigo-rtapi's boot_db tests read .robigo/rt-home-db: in every shard
        # that runs the crates' tests the tree is made (rebuilt when stale)
        # first, and each run says it must be there, so it never skips
        from unittest import mock
        crates = ["robigo-heap", "robigo-rt", "robigo-rtapi", "robigo-rtcore"]
        with mock.patch.object(rt, "target_dir", return_value="/t/target"):
            for shard in rt.SHARDS:
                steps = rt.shard_steps(crates, shard)
                # (the runs that include robigo-rtapi's tests, where boot_db is)
                runs = [i for i, (_, _, c) in enumerate(steps) if c[:2] == ["cargo", "test"] and "--no-run" not in c
                        and "--skip" in c and "--test" not in c and "robigo-rtapi" in c]
                tree = [i for i, (t, _, _) in enumerate(steps) if t.startswith("base's database tree")]
                if not runs:
                    continue
                self.assertEqual(len(tree), 1, shard)
                self.assertLess(tree[0], runs[0], shard)
                for i in runs:
                    self.assertEqual(steps[i][1].get("ROBIGO_RT_HOME_DB"), "1", f"{shard}: {steps[i][0]}")
                # the release binary that writes the tree is built before it
                built = [i for i, (_, _, c) in enumerate(steps) if "--no-run" in c and "--release" in c]
                self.assertTrue(built and built[0] < tree[0], shard)

    def test_the_rdb_boot_runs_the_conformance_plain_and_crosschecked_after_its_tree(self):
        # plain, crosschecked, and with the translation cache off (the tree's
        # cache must change nothing a case sees)
        steps = rt.steps(["robigo-rt", "robigo-rtapi"])
        titles = [t for t, _, _ in steps]
        # (the default boot is rdb: a step that names no boot runs it)
        boot_keys = {"ROBIGO_RT_BOOT", "ROBIGO_CONF_JOBS", "ROBIGO_CONF_TIMEOUTS", "ROBIGO_BC_CROSSCHECK",
                     "ROBIGO_BC_CROSSCHECK_REPORT", "ROBIGO_GC_FREEZE", "ROBIGO_TRANSLATION_CACHE"}
        rdb = [i for i, (_, e, c) in enumerate(steps) if e.get("ROBIGO_RT_BOOT", "rdb") == "rdb"
               and rt.CONFORMANCE in c and "--skip" not in c and set(e) <= boot_keys]
        # plain, crosschecked, with the boot heap's freeze off, and without
        # the translation cache
        self.assertEqual(len(rdb), 4)
        tree = titles.index("base's database tree, written by this robigo-rt (scripts/rt-home-db.sh)")
        self.assertTrue(all(tree < i for i in rdb))
        self.assertEqual({rt.shard_of(titles[i]) for i in rdb}, {"modes"})
        # (ROBIGO_CONF_TIMEOUTS: the default-timeout steps' allowances, set apart)
        self.assertEqual(sorted(sorted(k for k in steps[i][1] if k != "ROBIGO_CONF_TIMEOUTS") for i in rdb),
                         [["ROBIGO_BC_CROSSCHECK", "ROBIGO_BC_CROSSCHECK_REPORT", "ROBIGO_CONF_JOBS", "ROBIGO_RT_BOOT"],
                          ["ROBIGO_CONF_JOBS"],
                          ["ROBIGO_CONF_JOBS", "ROBIGO_GC_FREEZE"],
                          ["ROBIGO_CONF_JOBS", "ROBIGO_RT_BOOT", "ROBIGO_TRANSLATION_CACHE"]])
        self.assertEqual(sum(steps[i][1].get("ROBIGO_TRANSLATION_CACHE") == "0" for i in rdb), 1)

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
            self.assertEqual(timeouts[shard], max(2 * budget, rt.TIMEOUT_FLOOR),
                             f"{shard}: its job's timeout is twice its budget, at least the floor")
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
        self.assertIn(f"{rt.timeout_of(budget)}-minute timeout", report)
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

    def test_the_case_times_report_lists_a_case_with_an_allowance_under_half_of_it(self):
        # a case given longer than the step's limit shows its time against
        # its allowance even when it is under half of it; without the step's
        # limit, the report is the over-half one alone
        import tempfile
        path = os.path.join(tempfile.mkdtemp(), "t.tsv")
        with open(path, "w") as f:
            f.write("# secs\tlimit\tstatus\tcase\n"
                    "332.9\t780\tpass\ttests/r/conditions/c-stack-overflow.R\n"
                    "200.0\t600\tpass\ttests/rt/a.R\n")
        report = rt.slow_cases_report("conformance under gctorture (E3)", path, 600)
        self.assertIn("no case over 50% of its limit", report)
        self.assertIn("the cases with an allowance above the step's 600 s:", report)
        self.assertIn("  332.9 s of  780 s  pass  tests/r/conditions/c-stack-overflow.R", report)
        self.assertNotIn("tests/rt/a.R", report)
        self.assertNotIn("c-stack-overflow", rt.slow_cases_report("x", path))
        # over half of its allowance: listed once, with the over-half cases
        with open(path, "a") as f:
            f.write("700.0\t780\tpass\ttests/r/b.R\n")
        report = rt.slow_cases_report("x", path, 600)
        self.assertEqual(report.count("tests/r/b.R"), 1)
        self.assertIn("1 case(s) over 50%", report)
        self.assertEqual(rt.step_timeout({"ROBIGO_CONF_TIMEOUT": "600"}), 600)
        self.assertEqual(rt.step_timeout({}), 20)

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
        self.assertTrue(seen[1] and seen[1].endswith("01-conformance-under-gctorture-E3.tsv"), seen)
        # two titles that differ only in punctuation have one slug, so the
        # step's index is what keeps their files apart
        self.assertEqual(rt.times_file(3, "conformance under X=1"), rt.times_file(3, "conformance under X-1"))
        self.assertNotEqual(rt.times_file(3, "conformance under X=1"), rt.times_file(4, "conformance under X-1"))
        self.assertIn("1 case(s) over 50% of their limit", out.getvalue())
        self.assertIn("tests/rt/x.R", out.getvalue())
        # the nightly's full-move step is a conformance step like the others
        # (its last: the tree and the build come first)
        self.assertTrue(rt.CONFORMANCE in rt.full_move_steps()[-1][2] and "--exact" in rt.full_move_steps()[-1][2])

    def test_the_free_space_floor_measures_a_path_that_does_not_exist_yet_at_its_ancestor(self):
        # cargo's target directory does not exist on a fresh checkout (CI's
        # first build): its volume is measured at the nearest ancestor
        from unittest import mock
        import collections, contextlib, io, tempfile
        base = tempfile.mkdtemp()
        missing = os.path.join(base, "target")
        deeper = os.path.join(base, "no", "such", "target")
        self.assertEqual(rt.existing_ancestor(missing), base)
        self.assertEqual(rt.existing_ancestor(deeper), base)
        self.assertIsNone(rt.free_space_problem([missing, deeper], 0.0))
        usage = collections.namedtuple("usage", "total used free")
        seen = []

        def du(p):
            seen.append(p)
            return usage(100e9, 97e9, 3e9)
        with mock.patch("shutil.disk_usage", du):
            self.assertIn("3.0 GB free on the volume of " + deeper, rt.free_space_problem([deeper], 8))
        self.assertEqual(seen, [base])
        # a measurement that fails is a warning, never a crash
        # (on stdout: Actions annotates only workflow commands there)
        out = io.StringIO()
        with mock.patch("shutil.disk_usage", side_effect=PermissionError("denied")), contextlib.redirect_stdout(out):
            self.assertIsNone(rt.free_space_problem([missing], 8))
        self.assertIn("::warning::rt-ci: cannot tell the free space", out.getvalue())
        # and so is a path with no existing ancestor
        out = io.StringIO()
        with mock.patch.object(rt, "existing_ancestor", return_value=None), contextlib.redirect_stdout(out):
            self.assertIsNone(rt.free_space_problem(["/nowhere"], 8))
        self.assertIn("cannot tell", out.getvalue())

    def test_a_run_stops_at_its_start_below_the_free_space_floor(self):
        from unittest import mock
        import collections, contextlib, io
        usage = collections.namedtuple("usage", "total used free")
        # main's premise mocked: no `cargo metadata` run, whatever the workspace
        hermetic = contextlib.ExitStack()
        hermetic.enter_context(mock.patch.object(rt, "metadata", return_value={"target_directory": "/x"}))
        hermetic.enter_context(mock.patch.object(rt, "rt_crates", return_value=["robigo-rt"]))
        hermetic.enter_context(mock.patch.object(rt, "leaks", return_value=[]))
        hermetic.enter_context(mock.patch.object(rt, "steps", return_value=[]))
        with hermetic, mock.patch("shutil.disk_usage", return_value=usage(100e9, 95e9, 5e9)):
            self.assertIn("5.0 GB free", rt.free_space_problem(["/x"], 8))
            self.assertIsNone(rt.free_space_problem(["/x"], 4))
            for value, said in (("8", "free space first"), ("8GB", "is not a number of GB")):
                out = io.StringIO()
                with mock.patch.object(rt, "run_steps") as run, contextlib.redirect_stdout(out), \
                        mock.patch.dict(os.environ, {"RT_CI_MIN_FREE_GB": value}):
                    self.assertEqual(rt.main([]), 2, value)
                run.assert_not_called()
                self.assertIn("::error::", out.getvalue())
                self.assertIn(said, out.getvalue())
            # RT_CI_MIN_FREE_GB=0 turns the check off
            with mock.patch.object(rt, "run_steps", return_value=0) as run, contextlib.redirect_stdout(io.StringIO()), \
                    mock.patch.dict(os.environ, {"RT_CI_MIN_FREE_GB": "0"}):
                self.assertEqual(rt.main([]), 0)
            run.assert_called_once()

    def test_a_local_run_shares_the_machine_wide_slot_pool(self):
        import tempfile
        d = tempfile.mkdtemp()
        import shutil
        self.addCleanup(shutil.rmtree, d, ignore_errors=True)
        # (the fallback under a directory of the test's own, not /tmp's)
        from unittest import mock
        fallback = mock.patch.object(rt.conf_slots, "FALLBACK_ROOT", os.path.join(d, "tmp"))
        fallback.start()
        self.addCleanup(fallback.stop)
        os.makedirs(os.path.join(d, "tmp"))
        mine = os.path.join(d, "tmp", f"robigo-conf-slots-{os.getuid()}")
        self.assertEqual(rt.conf_slots_dir({"XDG_RUNTIME_DIR": d}), os.path.join(d, "robigo-conf-slots"))
        self.assertEqual(rt.conf_slots_dir({}), mine)
        self.assertEqual(rt.conf_slots_dir({"TMPDIR": d}), mine)
        self.assertEqual(rt.conf_slots_dir({"XDG_RUNTIME_DIR": os.path.join(d, "missing")}), mine)
        # the same directory for every checkout's run: the pool is the user's
        self.assertEqual(rt.conf_slots_dir({"XDG_RUNTIME_DIR": d}), rt.conf_slots_dir({"XDG_RUNTIME_DIR": d}))
        # CI has none; the caller's own setting (empty: none) wins
        self.assertIsNone(rt.conf_slots_dir({"GITHUB_ACTIONS": "true", "XDG_RUNTIME_DIR": d}))
        self.assertIsNone(rt.conf_slots_dir({"ROBIGO_CONF_SLOTS_DIR": ""}))
        self.assertIsNone(rt.conf_slots_dir({"ROBIGO_CONF_SLOTS_DIR": "/x"}))
        # an existing pool that others could write in is not used
        import contextlib, io
        os.chmod(os.path.join(d, "robigo-conf-slots"), 0o777)
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertIsNone(rt.conf_slots_dir({"XDG_RUNTIME_DIR": d}))
        self.assertIn("not a private directory", out.getvalue())
        # a regular file where the pool would be: refused with its mode, too
        f = os.path.join(d, "f")
        os.makedirs(f)
        open(os.path.join(f, "robigo-conf-slots"), "w").close()
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertIsNone(rt.conf_slots_dir({"XDG_RUNTIME_DIR": f}))
        self.assertIn("not a private directory", out.getvalue())

    def test_the_crates_tests_take_their_threads_from_the_slot_pool(self):
        import contextlib, fcntl, io, shutil, subprocess, tempfile
        from unittest import mock
        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d, ignore_errors=True)
        os.chmod(d, 0o700)
        crate = ["cargo", "test", "--profile", "quick", "--locked", "-p", "x", "--", "--skip", rt.CONFORMANCE]
        seen = []

        def fake_run(cmd, cwd=None, env=None):
            # the slots a step holds while it runs, as another run sees them
            busy = 0
            for i in range(8):
                with open(os.path.join(d, f"slot-{i}"), "a") as f:
                    try:
                        fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    except BlockingIOError:
                        busy += 1
            seen.append((cmd, env.get("ROBIGO_CONF_SLOTS_DIR"), busy))
            return subprocess.CompletedProcess(cmd, 0)

        def run(cmd, env):
            seen.clear()
            with mock.patch.object(rt.subprocess, "run", fake_run), contextlib.redirect_stdout(io.StringIO()), \
                    mock.patch.dict(os.environ, env, clear=False):
                self.assertEqual(rt.run_steps([("step", {}, cmd)], []), 0)
            return seen[0]

        pool = {"ROBIGO_CONF_SLOTS_DIR": d, "ROBIGO_CONF_SLOTS": "8"}
        # a quiet pool: every slot, as many threads; its tests get no pool
        self.assertEqual(run(crate, pool), ([*crate, "--test-threads=8"], "", 8))
        # another run holds one slot: the step takes the other seven
        other = open(os.path.join(d, "slot-1"), "a")
        fcntl.flock(other, fcntl.LOCK_EX)
        self.assertEqual(run(crate, pool), ([*crate, "--test-threads=7"], "", 8))
        other.close()
        # a busy pool: one slot free, still CRATE_TEST_MIN_THREADS threads,
        # without waiting for more slots
        self.assertEqual(rt.CRATE_TEST_MIN_THREADS, 4)
        others = [open(os.path.join(d, f"slot-{i}"), "a") for i in range(7)]
        for f in others:
            fcntl.flock(f, fcntl.LOCK_EX)
        self.assertEqual(run(crate, pool), ([*crate, "--test-threads=4"], "", 8))
        for f in others:
            f.close()
        # no pool (CI, or the caller's empty one): the command as it was
        self.assertEqual(run(crate, {"ROBIGO_CONF_SLOTS_DIR": ""}), (crate, "", 0))
        # the conformance steps take a slot per case in the runner, and a
        # build runs no test: neither changes
        conf = rt.conformance_command()
        self.assertEqual(run(conf, pool), (conf, d, 0))
        build = ["cargo", "test", "--release", "--locked", "--no-run"]
        self.assertEqual(run(build, pool), (build, d, 0))

    def test_a_take_that_fails_fails_the_step_cleanly(self):
        # Pool.take checks the directory again on every take: its PoolError
        # is the step's ::error::, with the group closed and the step's line
        import contextlib, io, shutil, tempfile
        from unittest import mock
        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d, ignore_errors=True)
        os.chmod(d, 0o700)
        crate = ["cargo", "test", "-p", "x", "--", "--skip", rt.CONFORMANCE]
        done, out = [], io.StringIO()
        def fail(*a, **k):
            raise rt.conf_slots.PoolError("ROBIGO_CONF_SLOTS_DIR gone")
        with mock.patch.object(rt.conf_slots.Pool, "take", fail), \
                mock.patch.object(rt.subprocess, "run") as run, contextlib.redirect_stdout(out), \
                mock.patch.dict(os.environ, {"ROBIGO_CONF_SLOTS_DIR": d, "ROBIGO_CONF_SLOTS": "2"}):
            self.assertEqual(rt.run_steps([("step", {}, crate)], done), 2)
        run.assert_not_called()
        self.assertIn("::error::rt: step: ROBIGO_CONF_SLOTS_DIR gone", out.getvalue())
        self.assertIn("::endgroup::", out.getvalue())
        self.assertEqual(done, [("step", 0.0, False)])

    def test_every_crate_test_step_is_one_the_pool_bounds(self):
        # every cargo test step that runs tests other than the conformance
        # has a `--` for the thread count to follow (a step with none would
        # run unbounded beside other runs)
        crates = ["robigo-rt", "robigo-rtapi", "robigo-rtcore"]
        for shard in rt.SHARDS:
            for title, _env, cmd in rt.shard_steps(crates, shard):
                if cmd[:2] == ["cargo", "test"] and "--no-run" not in cmd \
                        and not (rt.CONFORMANCE in cmd and "--exact" in cmd):
                    self.assertTrue(rt.crate_test_threads(cmd), f"{shard}: {title}: {cmd}")

    def test_the_torture_steps_torture_the_cases_and_one_step_per_boot_the_start(self):
        # every conformance step under a torture mode holds it through the
        # start (ROBIGO_TORTURE_AFTER_START=1); the start keeps one step per
        # boot under the moving collector and gctorture from the creation
        crates = ["robigo-rt", "robigo-rtapi", "robigo-rtcore"]
        plans = rt.steps(crates) + rt.full_move_steps() + rt.freeze_check_steps()
        torture = ("ROBIGO_GCTORTURE", "ROBIGO_GC_MOVE", "ROBIGO_RETRY_TORTURE")
        seen = 0
        for title, env, cmd in plans:
            if rt.CONFORMANCE in cmd and "--exact" in cmd and any(k in env for k in torture):
                seen += 1
                self.assertEqual(env.get("ROBIGO_TORTURE_AFTER_START"), "1", title)
                if "ROBIGO_GC_MOVE" in env:
                    self.assertEqual(env.get("ROBIGO_GC_FREEZE"), "0", title)
        self.assertGreaterEqual(seen, 5)
        starts = [(t, e, c) for t, e, c in rt.steps(crates) if t.startswith("the start tortured")]
        self.assertEqual(sorted(e["ROBIGO_RT_BOOT"] for _t, e, _c in starts), ["m1", "rdb"])
        for title, env, cmd in starts:
            self.assertEqual(env["ROBIGO_TORTURE_AFTER_START"], "0", title)
            self.assertEqual((env["R_GCTORTURE"], env["R_GCTORTURE_WAIT"], env["ROBIGO_GCTORTURE"], env["ROBIGO_GC_MOVE"]),
                             ("1", "1", "1", "1"), title)
            # (the freeze is off under any move mode; said, as every GC_MOVE step says it)
            self.assertEqual(env["ROBIGO_GC_FREEZE"], "0", title)
            self.assertEqual(cmd[0], rt.rt_binary())
        # the rdb one after base's database tree is written
        titles = [t for t, _e, _c in rt.steps(crates)]
        tree = next(i for i, t in enumerate(titles) if t.startswith("base's database tree"))
        self.assertGreater(titles.index(next(t for t, e, _c in starts if e["ROBIGO_RT_BOOT"] == "rdb")), tree)

    def test_each_shards_crate_tests_are_built_in_a_step_of_their_own(self):
        # a crate-test step's time is its tests' alone: its build (the same
        # command with --no-run, the same RUSTFLAGS and target) comes first,
        # once per build, and a plan's own build is not made twice
        crates = ["robigo-rt", "robigo-rtapi", "robigo-rtcore"]
        plans = {shard: rt.shard_steps(crates, shard) for shard in rt.SHARDS}
        plans["local"] = rt.builds_first(rt.steps(crates))
        for name, plan in plans.items():
            built = []
            for title, env, cmd in plan:
                if cmd[:2] == ["cargo", "test"] and "--no-run" in cmd:
                    built.append((cmd[:cmd.index("--no-run")], {k: env.get(k) for k in rt.BUILD_ENV}))
                elif rt.crate_test_threads(cmd):
                    want = (cmd[:cmd.index("--")], {k: env.get(k) for k in rt.BUILD_ENV})
                    self.assertIn(want, built, f"{name}: {title}")
            self.assertEqual(len(built), len({repr(b) for b in built}), f"{name}: a build made twice")
            self.assertEqual(rt.builds_first(plan), plan, name)
        # a matching build that comes only after the run moves ahead of it
        crate = ["cargo", "test", "-p", "x", "--", "--skip", rt.CONFORMANCE]
        late = rt.builds_first([("run", {}, crate), ("late build", {}, ["cargo", "test", "-p", "x", "--no-run"])])
        self.assertEqual([t for t, _e, _c in late], ["late build", "run"])
        da = [t for t, _e, _c in plans["debug-assertions"]]
        self.assertIn("the crates' tests with debug assertions (-C debug-assertions=on, quick profile): "
                      "its tests built (--no-run)", da)
        self.assertEqual(sum("built" in t for t, _e, c in plans["release-crate-tests"] if "--no-run" in c), 1)

    def test_the_end_to_end_crosscheck_test_clears_every_modes_variables(self):
        # robigo-rtapi's a_lost_crosscheck_record_fails_the_run runs a case
        # as the plain binary does, inside whatever crate-test step runs it:
        # each variable such a step sets is in its MODE_VARS, or is one that
        # does not change the case (the runner's own, the crosscheck's, the
        # tree's)
        import re
        src = (HERE.parent / "crates" / "robigo-rtapi" / "tests" / "conformance.rs").read_text()
        block = re.search(r"const MODE_VARS: &\[&str\] = &\[(.*?)\];", src, re.S).group(1)
        cleared = set(re.findall(r'"([A-Z_0-9]+)"', block))
        harmless = {"ROBIGO_RT_HOME_DB", "ROBIGO_TRANSLATION_CACHE", "RUSTFLAGS", "CARGO_TARGET_DIR"}
        crates = ["robigo-rt", "robigo-rtapi", "robigo-rtcore"]
        for shard in rt.SHARDS:
            for title, env, cmd in rt.shard_steps(crates, shard):
                if rt.crate_test_threads(cmd):
                    for k in env:
                        if k.startswith("ROBIGO_CONF_") or k.startswith("ROBIGO_BC_CROSSCHECK") or k in harmless:
                            continue
                        self.assertIn(k, cleared, f"{shard}: {title}: {k} is not in MODE_VARS")

    def test_every_step_that_can_boot_rdb_has_its_binarys_tree_before_it(self):
        # base's database tree is written by one binary (rt-home-db.sh's
        # stamp names it); a step that runs the release binary with no boot
        # set (rdb once it is the default) or ROBIGO_RT_BOOT=rdb needs the
        # tree that same build wrote, earlier in its own plan, under either
        # default. Walked in order: a release build replaces the binary (in
        # its target), a tree step records the binary it saw
        crates = ["robigo-rt", "robigo-rtapi", "robigo-rtcore"]
        plans = {shard: rt.shard_steps(crates, shard) for shard in rt.SHARDS}
        plans["local"] = rt.builds_first(rt.steps(crates))
        plans["full-move"] = rt.full_move_steps()
        plans["freeze-check"] = rt.freeze_check_steps()
        binary_scripts = ("scripts/rt-stress.sh", "scripts/rt-overshoot.py", "scripts/rt-fail-alloc.py",
                          "scripts/rt-tcache-mutate.py", "scripts/builtin-diff.py")

        def build_of(cmd):
            # (a release cargo command's build: its features, or plain)
            return cmd[cmd.index("--features") + 1] if "--features" in cmd else "plain"

        checked = 0
        for name, plan in plans.items():
            binary, tree = {}, {}
            for title, env, cmd in plan:
                target = env.get("CARGO_TARGET_DIR", "")
                # (any release cargo test of robigo-rtapi builds the binary and
                # may run it: the conformance, boot_db, the crates' tests that
                # spawn it, a --no-run build)
                release = cmd[:2] == ["cargo", "test"] and "--release" in cmd and "robigo-rtapi" in cmd
                if release:
                    binary[target] = build_of(cmd)
                if cmd[:1] == ["scripts/rt-home-db.sh"]:
                    # the tree is the named binary's: the release one of this target
                    exe = cmd[cmd.index("--robigo-rt") + 1]
                    self.assertEqual(exe, rt.rt_binary(target or None), f"{name}: {title}: another binary")
                    tree[target] = binary.get(target)
                    continue
                if "scripts/builtin-diff.py" in cmd:
                    binary[target] = "plain"       # (it builds the release binary first)
                # (or the binary run outright: the start-torture steps)
                runs = (release and "--no-run" not in cmd or any(x in cmd for x in binary_scripts)
                        or cmd[:1] == [rt.rt_binary(target or None)])
                # (unset or empty is rdb, the default)
                if not runs or env.get("ROBIGO_RT_BOOT", "rdb") not in ("", "rdb"):
                    continue
                checked += 1
                self.assertIsNotNone(tree.get(target), f"{name}: {title}: no tree before it")
                self.assertEqual(tree[target], binary.get(target),
                                 f"{name}: {title}: the tree was written by another build")
        self.assertGreater(checked, 20)

    # Every file that runs a robigo-rt binary, and how that binary gets base's
    # database tree written by itself (a binary naming no boot boots rdb once
    # it is the default). A file that starts running one fails the test below
    # until it is listed here with its way.
    TREE_CALLERS = {
        "scripts/rt-ci.py": "its plans: each binary's tree before its steps (the walk test above)",
        "scripts/builtin-diff.py": "run/show: rt_trees.ensure (ci: rt-ci's builtin-diff shard wrote it); the "
                                   "planted binary boots the plain one's tree; both with ROBIGO_TRANSLATION_CACHE=0",
        "scripts/corpus.py": "rt_trees.ensure in robigo_rt_path; CI's rt-corpora unpacks perf-rt-build's trees",
        "scripts/wall13.py": "rt_trees.ensure before the first run (local only)",
        "scripts/rt-build-gaps.py": "rt_trees.ensure before its probes (local only)",
        "scripts/rt-startup.py": "make_trees / --pack-trees / --trees-from: perf's rows and starts boot its trees",
        "scripts/perf.py": "rt-startup's trees (rt-starts --trees, rt-bars' make_trees)",
        "scripts/rt_trees.py": "is the way",
        "scripts/rt-home-db.sh": "writes the tree (boots stage0 itself)",
        "scripts/rt-base-build.sh": "names its boot (ROBIGO_RT_BOOT=stage0)",
        "scripts/rt-rdb-compare.sh": "reads trees, runs no binary at a default boot",
        "scripts/pgo-build.sh": "L's flip branch makes its training run's tree",
        "scripts/rt-stress.sh": "rt-ci's step after the tree; by hand, the checkout's tree",
        "scripts/rt-overshoot.py": "rt-ci's step after the tree; by hand, the checkout's tree",
        "scripts/rt-fail-alloc.py": "rt-ci's step after the tree; by hand, the checkout's tree",
        "scripts/rt-tcache-mutate.py": "rt-ci's step after the tree; by hand, the checkout's tree",
        ".github/workflows/ci.yml": "rt-ci's shards; rt-corpora unpacks dist/rt-trees.tar; perf-rt boots "
                                   "--trees dist/rt-trees.tar",
    }

    def test_every_robigo_rt_caller_says_how_its_binary_gets_its_tree(self):
        import re
        pat = re.compile(r"--robigo-rt\b|release/robigo-rt|ROBIGO_RT_BIN|dist/robigo-rt|CARGO_BIN_EXE_robigo-rt")
        root = HERE.parent
        found = set()
        for p in list(root.glob("scripts/*.py")) + list(root.glob("scripts/*.sh")) + list(root.glob(".github/workflows/*.yml")):
            rel = str(p.relative_to(root))
            if re.match(r"scripts/test[-_]", rel):
                continue
            if pat.search(p.read_text(errors="replace")):
                found.add(rel)
        self.assertEqual(sorted(found - set(self.TREE_CALLERS)), [], "a robigo-rt caller with no tree listed")
        self.assertEqual(sorted(set(self.TREE_CALLERS) - found), [], "a listed caller that no longer runs one")
        # rt-corpora's binary comes with its trees, unpacked before any run
        yml = (root / ".github" / "workflows" / "ci.yml").read_text()
        job = re.search(r"\n  rt-corpora:\n(.*?)\n  [a-z0-9-]+:\n", yml, re.S).group(1)
        self.assertLess(job.index("tar -xf dist/rt-trees.tar"), job.index("corpus.py run"))

    def test_rt_trees_makes_the_tree_only_when_it_is_not_this_binarys_whole_tree(self):
        import importlib.util, shutil, subprocess, tempfile
        from unittest import mock
        spec_ = importlib.util.spec_from_file_location("rt_trees", HERE / "rt_trees.py")
        tr = importlib.util.module_from_spec(spec_)
        spec_.loader.exec_module(tr)
        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d, ignore_errors=True)
        os.makedirs(os.path.join(d, "scripts"))
        for n in ("rt-home.sh", "rt-home-db.sh"):
            open(os.path.join(d, "scripts", n), "w").write(n)
        b = os.path.join(d, "robigo-rt")
        open(b, "wb").write(b"binary")
        db = os.path.join(d, ".robigo", "rt-home-db")
        files = [os.path.join(db, "library", "base", "R", "base.rdb"),
                 os.path.join(db, "library", "base", "R", "base.rdx"),
                 os.path.join(db, "library", "base", "R", "base"),
                 os.path.join(db, "library", "compiler", "R", "compiler.rdb"),
                 os.path.join(db, "library", "compiler", "R", "compiler.rdx"),
                 os.path.join(d, ".robigo", "rt-home-db-cache", "base.rtc"),
                 os.path.join(d, ".robigo", "rt-home-db-cache", "compiler.rtc")]
        def make_whole():
            for f in files:
                os.makedirs(os.path.dirname(f), exist_ok=True)
                open(f, "w").write("x")
            home = ["R-4.6.2-x", tr.script_hash(d, "rt-home.sh"), "natives"]
            os.makedirs(os.path.join(d, ".robigo", "rt-home"), exist_ok=True)
            open(os.path.join(d, ".robigo", "rt-home", ".rt-home"), "w").write(" ".join(home) + "\n")
            open(os.path.join(db, ".rt-home-db"), "w").write(
                " ".join(home + [tr.script_hash(d, "rt-home-db.sh"), tr.binary_hash(b)]) + "\n")
        ran = []
        def fake(cmd, **k):
            ran.append(cmd[0])
            return subprocess.CompletedProcess(cmd, 0, "", "")
        with mock.patch.object(tr.subprocess, "run", fake), mock.patch.dict(os.environ, {"ROBIGO_RT_CHECKOUT": "/else"}):
            tr.ensure(b, d)            # no tree: both scripts
            self.assertEqual(ran, ["scripts/rt-home.sh", "scripts/rt-home-db.sh"])
            # and the binary's children boot from this checkout, not an inherited one
            self.assertEqual(os.environ["ROBIGO_RT_CHECKOUT"], os.path.abspath(d))
            make_whole()
            ran.clear()
            tr.ensure(b, d)            # this binary's whole tree: nothing
            self.assertEqual(ran, [])
            open(b, "wb").write(b"another")
            tr.ensure(b, d)            # another binary's: made again
            self.assertEqual(ran, ["scripts/rt-home.sh", "scripts/rt-home-db.sh"])
            open(b, "wb").write(b"binary")
            make_whole()
            for gone in files[:5]:    # a database, an index, the loader: each sends the scripts to decide
                make_whole()
                os.remove(gone)
                ran.clear()
                tr.ensure(b, d)
                self.assertEqual(ran, ["scripts/rt-home.sh", "scripts/rt-home-db.sh"], gone)
            make_whole()
            open(os.path.join(d, "scripts", "rt-home-db.sh"), "w").write("changed")
            ran.clear()
            tr.ensure(b, d)            # the recipe changed: made again
            self.assertEqual(ran, ["scripts/rt-home.sh", "scripts/rt-home-db.sh"])
            # stale stamps and no binary: not fresh (no traceback), and the
            # scripts say what is missing (rt-home-db.sh's "no robigo-rt at")
            make_whole()
            open(os.path.join(d, "scripts", "rt-home.sh"), "w").write("changed too")
            os.remove(b)
            self.assertFalse(tr.fresh(b, d))
            ran.clear()
            tr.ensure(b, d)
            self.assertEqual(ran, ["scripts/rt-home.sh", "scripts/rt-home-db.sh"])

    def test_base_subsets_check_is_owed_when_a_branch_changes_its_inputs(self):
        owed = rt.base_subset_owed
        for path in ("tests/rt/env/large-frames.R", "tests/rt/a.R", "benchmarks/rt/calls-rows.toml",
                     "scripts/base-subset.sh", "scripts/base-subset-trace.R", "crates/robigo-rt/proto/base-wide.txt",
                     "scripts/r-src.sh", "scripts/base-wide.R", "benchmarks/suite/calls.R",
                     "tests/rt/capi/support/eval.c", "tests/rt/R-4.6.2/capi/eval.Rstatus", "tests/rt/R-4.7.0/capi/eval.Rstatus", "scripts/oracle-run.sh",
                     "scripts/perf.py", "scripts/oracle-versions"):
            self.assertEqual(owed(["docs/x.md", path]), path)
        for path in ("tests/rt/env/large-frames.Rout", "tests/r/basics/a.R", "benchmarks/rt/other.toml",
                     "crates/robigo-rt/proto/other.txt", "crates/robigo-rt/src/base-subset.rs", "scripts/rt-ci.py"):
            self.assertIsNone(owed([path]), path)
        self.assertIsNone(owed([]))
        # git cannot say: owed
        self.assertIn("cannot compare", owed(None))

    def test_branch_changes_see_commits_edits_and_new_files(self):
        import subprocess, shutil, tempfile
        from unittest import mock
        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d, ignore_errors=True)
        # (the caller's git config stays out: signing, hooks, templates)
        genv = dict(os.environ, GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_SYSTEM=os.devnull, GIT_CONFIG_NOSYSTEM="1")
        def git(*a):
            subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false",
                            "-c", "core.hooksPath=" + os.devnull, *a], cwd=d, check=True, capture_output=True, env=genv)
        git("init", "-q", "-b", "main")
        for f in ("kept", "edited"):
            open(os.path.join(d, f), "w").write("1\n")
        git("add", "kept", "edited")
        git("commit", "-q", "-m", "base")
        git("branch", "base")
        open(os.path.join(d, "committed"), "w").write("1\n")
        git("add", "committed")
        git("commit", "-q", "-m", "change")
        open(os.path.join(d, "edited"), "w").write("2\n")
        open(os.path.join(d, "untracked"), "w").write("1\n")
        isolated = mock.patch.dict(os.environ, {"GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_SYSTEM": os.devnull,
                                                "GIT_CONFIG_NOSYSTEM": "1"})
        isolated.start()
        self.addCleanup(isolated.stop)
        self.assertEqual(BRANCH_CHANGES(d, "base"), ["committed", "edited", "untracked"])
        self.assertIsNone(BRANCH_CHANGES(d, "no-such-branch"))
        # a staged change the worktree undoes is still a change (the commit
        # pushed will carry it)
        open(os.path.join(d, "kept"), "w").write("2\n")
        git("add", "kept")
        open(os.path.join(d, "kept"), "w").write("1\n")
        self.assertIn("kept", BRANCH_CHANGES(d, "base"))
        # a name that is not UTF-8
        bad = os.fsdecode(b"caf\xe9.R")
        open(os.path.join(os.fsencode(d), b"caf\xe9.R"), "w").close()
        self.assertIn(bad, BRANCH_CHANGES(d, "base"))
        # no git at all: git cannot say
        from unittest import mock
        with mock.patch.dict(os.environ, {"PATH": ""}):
            self.assertIsNone(BRANCH_CHANGES(d, "base"))

    def test_a_local_run_adds_base_subsets_check_or_stops_with_the_cure(self):
        import contextlib, io
        from unittest import mock
        def main(owed, problem, argv=()):
            out = io.StringIO()
            # main's premise mocked: no `cargo metadata` run, whatever the workspace
            with mock.patch.object(rt, "metadata", return_value={"target_directory": "/x", "packages": []}), \
                    mock.patch.object(rt, "rt_crates", return_value=["robigo-rt"]), \
                    mock.patch.object(rt, "leaks", return_value=[]), \
                    mock.patch.object(rt, "branch_changes", return_value=[]), \
                    mock.patch.object(rt, "base_subset_owed", return_value=owed), \
                    mock.patch.object(rt, "oracle_problem", return_value=problem), \
                    mock.patch.object(rt, "run_steps", return_value=0) as run, \
                    mock.patch.dict(os.environ, {"RT_CI_MIN_FREE_GB": "0"}), contextlib.redirect_stdout(out):
                status = rt.main(list(argv))
            plan = run.call_args[0][0] if run.called else None
            return status, plan, out.getvalue()
        status, plan, _ = main(None, None)
        self.assertEqual(status, 0)
        self.assertNotIn(rt.BASE_SUBSET_STEP, plan)
        status, plan, _ = main("tests/rt/a.R", None)
        self.assertEqual(plan[-1], rt.BASE_SUBSET_STEP)
        # no oracle: the run fails at its start (never a warning), naming the cure
        status, plan, out = main("tests/rt/a.R", "there is no robigo-oracle:4.6.2 image; scripts/build-oracle.sh --pull 4.6.2")
        self.assertEqual((status, plan), (2, None))
        self.assertIn("tests/rt/a.R", out)
        self.assertIn("scripts/build-oracle.sh --pull 4.6.2", out)
        # CI's shards never run it (CI has its own base-subset job)
        status, plan, _ = main("tests/rt/a.R", "missing", ["--shard", "modes"])
        self.assertEqual(status, 0)
        self.assertNotIn(rt.BASE_SUBSET_STEP, plan)

    def test_the_oracle_cure_names_the_pull(self):
        from unittest import mock
        import subprocess
        def fake(cmd, **kw):
            return subprocess.CompletedProcess(cmd, 1, "", "")
        with mock.patch.object(rt.subprocess, "run", fake):
            problem = rt.oracle_problem()
        self.assertIn("robigo-oracle:4.6.2", problem)
        self.assertIn("scripts/build-oracle.sh --pull 4.6.2", problem)
        self.assertIn("scripts/build-oracle.sh --image 4.6.2", problem)
        with mock.patch.object(rt.subprocess, "run", side_effect=FileNotFoundError):
            self.assertIn("no docker", rt.oracle_problem())
        # the version, image and cure follow the checkout's rt pin
        with mock.patch.object(rt, "rt_pin", return_value="4.7.0"), mock.patch.object(rt.subprocess, "run", fake):
            problem = rt.oracle_problem()
        for want in ("robigo-oracle:4.7.0", "GNU R 4.7.0", "--pull 4.7.0", "oracle_4_7_0=", "--image 4.7.0"):
            self.assertIn(want, problem)
        def r470(cmd, **kw):
            # (docker is not asked: a matching Rscript is enough)
            self.assertEqual(cmd[0], "Rscript", "docker was consulted though Rscript matched")
            return subprocess.CompletedProcess(cmd, 0, "4.7.0", "")
        with mock.patch.object(rt, "rt_pin", return_value="4.7.0"), mock.patch.object(rt.subprocess, "run", r470):
            self.assertIsNone(rt.oracle_problem())
        # the pin read from a checkout's scripts/oracle-versions
        import tempfile, shutil
        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d, ignore_errors=True)
        os.makedirs(os.path.join(d, "scripts"))
        with open(os.path.join(d, "scripts", "oracle-versions"), "w") as f:
            f.write("# pins\ngated=4.5.0\nrt=9.8.7\n")
        self.assertEqual(rt.rt_pin(d), "9.8.7")

    def test_the_lint_job_runs_lint_job_sh_and_nothing_else(self):
        # the workflow's lint job and scripts/lint-job.sh, which delegates
        # run before pushing, are one list: the job's only command is the
        # script, and the script runs fmt, lint.sh and every test suite the
        # job ran
        import re
        yml = (HERE.parent / ".github" / "workflows" / "ci.yml").read_text()
        job = re.search(r"\n  lint:\n(.*?)\n  [a-z0-9-]+:\n", yml, re.S).group(1)
        runs = re.findall(r"^\s+run: (.*)$", job, re.M)
        self.assertEqual(runs, ["scripts/lint-job.sh"])
        self.assertNotIn("run: |", job)
        script = (HERE / "lint-job.sh").read_text()
        checks = re.search(r"CHECKS=\((.*?)\n\)", script, re.S).group(1)
        listed = re.findall(r'"([^"]+)"', checks)
        self.assertEqual(listed[:2], ["cargo fmt --check", "scripts/lint.sh"])
        # every script suite is a test file that exists
        for c in listed[2:]:
            self.assertRegex(c, r"^python3 scripts/test[-_][\w-]+\.py$")
            self.assertTrue((HERE.parent / c.split()[1]).exists(), c)
        self.assertTrue(os.access(HERE / "lint-job.sh", os.X_OK))

    def test_a_crosscheck_that_compares_fewer_than_its_floor_fails(self):
        # the rdb boot's report: a boot that stopped loading the compiler's
        # namespace compares about half as many closures, without any error
        import tempfile
        path = os.path.join(tempfile.mkdtemp(), "bc.txt")
        with open(path, "w") as f:
            f.write("same: f\n" * 3 + "expected (a loop's shape (x)): g\n" + "skipped: h\n")
        errors, counts = rt.bc_report_check(path, 4)
        self.assertEqual((errors, counts["compared"]), ([], 4))
        errors, _ = rt.bc_report_check(path, 5)
        self.assertEqual(len(errors), 1)
        self.assertIn("compared 4 closures, fewer than the 5", errors[0])
        # the plain report's floor is one comparison; the rdb one's is set
        self.assertEqual(rt.BC_FLOORS.get(rt.BC_REPORT, 1), 1)
        self.assertGreater(rt.BC_FLOORS[rt.BC_REPORT_RDB], 46000)
        self.assertLessEqual(rt.BC_FLOORS[rt.BC_REPORT_RDB], 97485)   # the count on b0c49e869

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
        built = [(e, c) for _, e, c in steps[1:] if c[:2] == ["cargo", "test"] and "--no-run" not in c]
        self.assertEqual(len(built), 2)
        self.assertEqual(built[0][1][:5], ["cargo", "test", "--profile", "quick", "--locked"])
        self.assertEqual(built[1][1][:4], ["cargo", "test", "--release", "--locked"])
        self.assertIn(rt.CONFORMANCE, built[1][1])
        self.assertNotIn("debug assertions", " ".join(t for t, _, _ in rt.steps(["robigo-rt"])))

    def test_the_freeze_off_is_tested_under_each_boot(self):
        # the freeze is on by default (step 2); the heap without it is the
        # conformance under each boot with it off
        off = [(e, c) for t, e, c in rt.steps(["robigo-rt"]) if e.get("ROBIGO_GC_FREEZE") == "0"
               and rt.CONFORMANCE in c and "ROBIGO_GCTORTURE" not in e]
        boots = sorted(e.get("ROBIGO_RT_BOOT", "rdb") for e, _ in off)
        self.assertEqual(boots, ["m1", "rdb"])
        # every step of the moving collector turns it off explicitly, and no
        # step but those and the freeze-off ones does
        for t, e, _ in [*rt.steps(["robigo-rt"]), *rt.full_move_steps()]:
            if "ROBIGO_GC_MOVE" in e:
                self.assertEqual(e.get("ROBIGO_GC_FREEZE"), "0", t)
            elif e.get("ROBIGO_GC_FREEZE") == "0":
                self.assertIn("freeze off", t)
        # a step that names the freeze on is the check's
        for t, e, _ in rt.steps(["robigo-rt"]):
            if e.get("ROBIGO_GC_FREEZE") == "1":
                self.assertIn("ROBIGO_GC_FREEZE_CHECK", e, t)

    def test_full_move_and_shard_are_refused_together(self):
        # --full-move runs the nightly's one step: with --shard as well the
        # shard would be dropped silently
        import contextlib, io
        err = io.StringIO()
        with self.assertRaises(SystemExit) as e, contextlib.redirect_stderr(err):
            rt.main(["--full-move", "--shard", "modes"])
        self.assertEqual(e.exception.code, 2)
        self.assertIn("--full-move and --shard select different steps", err.getvalue())

    def test_the_freeze_check_runs_at_every_collection_in_the_nightly_only(self):
        # CI's E3 checks every 16th collection, with a full check (I2, I3,
        # the comparison) at the first check and every 16th after; the
        # nightly's --freeze-check every one, its slow cases named and given
        # longer (the ledger's amendment to #948 (c), and its note of
        # 2026-11-24)
        e3 = [e for t, e, _ in rt.steps(["robigo-rt"]) if t == "conformance under gctorture (E3)"]
        self.assertEqual(e3[0]["ROBIGO_GC_FREEZE_CHECK"], "16,16")
        # the check runs on the freeze (on by default; named, so the step
        # does not depend on the default)
        self.assertEqual(e3[0]["ROBIGO_GC_FREEZE"], "1")
        tests = [e for t, e, _ in rt.steps(["robigo-rt"]) if t.startswith("gctorture (collect at every allocation)")]
        self.assertEqual(tests[0]["ROBIGO_GC_FREEZE_CHECK"], "1")
        self.assertEqual(tests[0]["ROBIGO_GC_FREEZE"], "1")
        night = rt.freeze_check_steps()
        self.assertEqual(len(night), 4)
        # the R_HOME tree first: the tools cases skip without it (Sol r7 S13);
        # then the binary and base's database tree it writes
        self.assertEqual(night[0][2], ["scripts/rt-home.sh"])
        self.assertEqual(night[1], rt.release_build())
        self.assertEqual(night[2], rt.db_tree(rt.rt_binary()))
        _, env, cmd = night[3]
        self.assertEqual((env["ROBIGO_GC_FREEZE_CHECK"], env["ROBIGO_GCTORTURE"], env["ROBIGO_GC_FREEZE"]), ("1", "1", "1"))
        # a failed check fails the step, whatever the case's output (S14)
        self.assertEqual(env["ROBIGO_GC_FREEZE_CHECK_REPORT"], rt.FREEZE_REPORT)
        self.assertEqual(e3[0]["ROBIGO_GC_FREEZE_CHECK_REPORT"], rt.FREEZE_REPORT)
        self.assertNotIn("ROBIGO_GC_MOVE", env)
        self.assertIn(rt.CONFORMANCE, cmd)
        named = dict(item.rsplit("=", 1) for item in env["ROBIGO_CONF_TIMEOUTS"].split(","))
        self.assertEqual(set(named), set(rt.FREEZE_CHECK_SLOW))
        for case, secs in named.items():
            self.assertTrue((HERE.parent / case).is_file(), case)
            self.assertGreater(int(secs), int(env["ROBIGO_CONF_TIMEOUT"]), case)
        import contextlib, io
        with self.assertRaises(SystemExit), contextlib.redirect_stderr(io.StringIO()):
            rt.main(["--freeze-check", "--full-move"])

    def test_the_step_times_are_reported_slowest_first(self):
        r = rt.timings_report([("a", 60.0, True), ("b", 600.0, True), ("c", 30.0, False)])
        lines = r.splitlines()
        self.assertEqual(lines[0], "rt: step times (wall), slowest first:")
        self.assertIn("10.00 min  b", lines[1])
        self.assertIn("FAILED  c", lines[3])
        self.assertIn("11.50 min  in all", lines[4])

    def test_the_default_timeout_steps_give_the_two_slow_cases_their_allowance(self):
        # temporary (DEFAULT_STEP_SLOW): every conformance step at the
        # runner's 20 s names them, at 60 s; a step with its own timeout
        # (a collector mode's 600 s) does not
        crates = ["robigo-heap", "robigo-rt", "robigo-rtapi", "robigo-rtcore"]
        plans = rt.steps(crates) + rt.full_move_steps()
        for shard in rt.SHARDS:
            plans += rt.shard_steps(crates, shard)
        seen = 0
        for title, env, cmd in plans:
            if rt.CONFORMANCE not in cmd or "--exact" not in cmd:
                continue
            named = dict(i.rsplit("=", 1) for i in env.get("ROBIGO_CONF_TIMEOUTS", "").split(",") if i)
            if "ROBIGO_CONF_TIMEOUT" in env:
                self.assertFalse(set(rt.DEFAULT_STEP_SLOW) & set(named), title)
            else:
                seen += 1
                self.assertEqual(named, {c: str(s) for c, s in rt.DEFAULT_STEP_SLOW.items()}, title)
        self.assertGreater(seen, 5)
        self.assertTrue(all(s == 60 for s in rt.DEFAULT_STEP_SLOW.values()))
        # the caller's environment counts: its global timeout means none, its
        # case list is kept (and the step's wins over both)
        from unittest import mock
        conf = [s for s in rt.steps(crates) if rt.CONFORMANCE in s[2] and "--exact" in s[2]
                and "ROBIGO_CONF_TIMEOUT" not in s[1]][0]
        for v in ("180", ""):    # (present, even empty, is set)
            with mock.patch.dict(os.environ, {"ROBIGO_CONF_TIMEOUT": v}):
                self.assertNotIn("ROBIGO_CONF_TIMEOUTS", rt.every_cpu([(conf[0], {}, conf[2])])[0][1])
        for bad in ("tests/rt/a.R", "tests/rt/a.R=soon", "=90"):
            with mock.patch.dict(os.environ, {"ROBIGO_CONF_TIMEOUTS": bad}), \
                    self.assertRaisesRegex(SystemExit, "is not CASE=SECS"):
                rt.every_cpu([(conf[0], {}, conf[2])])
        with mock.patch.dict(os.environ, {"ROBIGO_CONF_TIMEOUTS": "tests/rt/a.R=90,tests/rt/a.R=91"}), \
                self.assertRaisesRegex(SystemExit, "names 'tests/rt/a.R' twice"):
            rt.every_cpu([(conf[0], {}, conf[2])])
        with mock.patch.dict(os.environ, {"ROBIGO_CONF_TIMEOUTS": "tests/rt/a.R=90,tests/rt/env/large-frames.R=120"}):
            env = rt.every_cpu([(conf[0], {}, conf[2])])[0][1]
        got = dict(i.rsplit("=", 1) for i in env["ROBIGO_CONF_TIMEOUTS"].split(","))
        self.assertEqual(got, {"tests/rt/a.R": "90", "tests/rt/env/large-frames.R": "120",
                               "tests/rt/stress/finalizer-depths.R": "60"})

    def test_conformance_runs_alone_in_its_own_steps(self):
        steps = rt.steps(["robigo-rt", "robigo-rtcore"])
        # (a step's mode is its environment but the runner's job count and
        # the case allowances every default-timeout step names)
        own = [({k: v for k, v in env.items() if k not in ("ROBIGO_CONF_JOBS", "ROBIGO_CONF_TIMEOUTS")}, cmd)
               for _, env, cmd in steps
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
                                 "ROBIGO_BC_CROSSCHECK", "ROBIGO_RETRY_TORTURE", "ROBIGO_POISON_DEAD", "ROBIGO_RT_BOOT",
                                 "ROBIGO_GC_FREEZE"} & set(env), env)
            # a crosscheck step writes its report, which `main` reads
            if "ROBIGO_BC_CROSSCHECK" in env:
                self.assertIn("ROBIGO_BC_CROSSCHECK_REPORT", env)
        # the crosscheck step exists at all: dropping it from steps() would
        # leave every assertion above green, and V3's gate (`main` failing on
        # an unexpected or untranslated closure) would leave CI unnoticed
        cross = [e for e, _ in own if "ROBIGO_BC_CROSSCHECK" in e]
        self.assertEqual(len(cross), 2)
        # (one per boot, each with a report of its own)
        self.assertEqual(sorted(e.get("ROBIGO_RT_BOOT", "rdb") for e in cross), ["m1", "rdb"])
        self.assertEqual(len({e["ROBIGO_BC_CROSSCHECK_REPORT"] for e in cross}), 2)
        for title, _, cmd in steps:
            # (a build-only step, --no-run, runs no test at all)
            if cmd[0] == "cargo" and rt.CONFORMANCE not in cmd[-1:] and "--exact" not in cmd and "--no-run" not in cmd:
                self.assertEqual(cmd[-2:], ["--skip", rt.CONFORMANCE], title)
        # The test exists under that name: after a rename the conformance step
        # would run nothing, and still pass.
        src = (HERE.parent / "crates/robigo-rtapi/tests/conformance.rs").read_text()
        self.assertIn(f"fn {rt.CONFORMANCE}()", src)

    def test_a_step_with_a_failed_freeze_check_fails(self):
        # the cases passed or failed as the ratchet allows, but a process
        # wrote a check failure: the step fails (Sol r7 S14)
        from unittest import mock
        import tempfile
        report = os.path.join(tempfile.mkdtemp(), "freeze.txt")

        def run(cmd, **kw):
            with open(report, "a") as f:
                f.write("robigo: ROBIGO_GC_FREEZE_CHECK: frozen 0x1 (type 4) points to the ordinary 0x2\n")
            return mock.Mock(returncode=0)
        done = []
        with mock.patch.object(rt.subprocess, "run", side_effect=run):
            r = rt.run_steps([("e3", {"ROBIGO_GC_FREEZE_CHECK_REPORT": report}, ["true"])], done)
        self.assertEqual(r, 1)
        self.assertEqual([(t, ok_) for t, _, ok_ in done], [("e3", False)])
        # a report left from an earlier run is removed first; the tally lines
        # are full checks, not failures
        def tally(cmd, **kw):
            with open(report, "a") as f:
                f.write(f"{rt.FULL_CHECK_TALLY}1 in process 11\n{rt.FULL_CHECK_TALLY}2 in process 11\n")
                f.write(f"{rt.FULL_CHECK_TALLY}1 in process 12\n")
            return mock.Mock(returncode=0)
        done = []
        with mock.patch.object(rt.subprocess, "run", side_effect=tally):
            self.assertEqual(rt.run_steps([("e3", {"ROBIGO_GC_FREEZE_CHECK_REPORT": report}, ["true"])], done), 0)
        self.assertEqual(rt.freeze_report(report), ([], 3, 2))
        # a torn or truncated tally is no full check: it is a failure line
        with open(report, "w") as f:
            f.write(f"{rt.FULL_CHECK_TALLY}\n{rt.FULL_CHECK_TALLY}3 in process \n")
            f.write(f"{rt.FULL_CHECK_TALLY}1{rt.FULL_CHECK_TALLY} in process 7\n{rt.FULL_CHECK_TALLY}2 in process 9\n")
        failures, full, processes = rt.freeze_report(report)
        self.assertEqual((len(failures), full, processes), (3, 1, 1))
        # a record cut short at the file's end, its newline missing
        with open(report, "w") as f:
            f.write(f"{rt.FULL_CHECK_TALLY}1 in process 1234\n{rt.FULL_CHECK_TALLY}1 in process 12")
        failures, full, processes = rt.freeze_report(report)
        self.assertEqual((len(failures), full, processes), (1, 1, 1))

    def test_a_step_whose_freeze_check_ran_no_full_check_fails(self):
        # a full-check rate no process reached is the silent failure: a step
        # with the check's report and no tally line fails
        from unittest import mock
        import tempfile
        report = os.path.join(tempfile.mkdtemp(), "freeze.txt")
        done = []
        with mock.patch.object(rt.subprocess, "run", return_value=mock.Mock(returncode=0)):
            self.assertEqual(rt.run_steps([("e3", {"ROBIGO_GC_FREEZE_CHECK_REPORT": report}, ["true"])], done), 1)
        self.assertEqual([(t, ok_) for t, _, ok_ in done], [("e3", False)])

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
        # the feature builds come last: the build (its binary replaces the
        # release one), the database tree that binary writes (no flag of its
        # own), retry torture's conformance, the cache's verification under
        # the knob (#979), then the poison-dead steps: the tree before any
        # step that runs the binary
        n = len(steps)
        self.assertEqual(featured, [n - 6, n - 4, n - 3, n - 2, n - 1], "the feature builds come last")
        self.assertIn("--no-run", steps[n - 6][2])
        self.assertEqual(steps[n - 5][2][:3], ["scripts/rt-home-db.sh", "--robigo-rt", rt.rt_binary()])
        self.assertIn("boot_db", steps[featured[2]][2])
        featured = featured[1:]
        for i in featured:
            cmd = steps[i][2]
            self.assertEqual(cmd[cmd.index("--features") + 1], rt.CHECK_FEATURES)
        retry = [t for t, e, _ in steps if e.get("ROBIGO_RETRY_TORTURE") == "1"]
        self.assertEqual(retry, [steps[featured[0]][0]])
        self.assertEqual([t for t, e, _ in steps if "ROBIGO_RETRY_TORTURE" in e], retry)
        self.assertEqual(steps[featured[0]][1].get("ROBIGO_CONF_TIMEOUT"), "600")
        poison = [t for t, e, _ in steps if e.get("ROBIGO_POISON_DEAD") == "1"]
        self.assertEqual(poison, [steps[i][0] for i in featured[2:]])
        self.assertIn("robigo-rt", steps[featured[2]][2])
        self.assertIn(rt.CONFORMANCE, steps[featured[3]][2])
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
            self.assertEqual(env.get("ROBIGO_CONF_JOBS"), str(CPUS), title)
        others = [t for t, e, c in steps if "ROBIGO_CONF_JOBS" in e and not (rt.CONFORMANCE in c and "--exact" in c)]
        self.assertEqual(others, [])

    # The runner's KNOWN_FEATURES, and those of them the environment decides
    # (crates/robigo-conformance/src/lib.rs, identity_features; this mirrors
    # them: keep the two in step). r-home-tools and bytecode-fidelity depend
    # on the build, not the environment, so a step neither keeps nor turns
    # them off.
    KNOWN = {"bytecode-fidelity", "collection-schedule", "default-vsize", "inplace-identity", "promise-identity",
             "r-home-tools", "volume"}
    BY_ENVIRONMENT = {"collection-schedule", "default-vsize", "inplace-identity", "promise-identity", "volume"}

    @staticmethod
    def kept_features(env):
        on = lambda name: env.get(name) == "1"
        kept = set()
        if not on("ROBIGO_NO_INPLACE"):
            kept.add("inplace-identity")
        if not on("ROBIGO_MATERIALIZE"):
            kept.add("promise-identity")
        if not on("ROBIGO_GCTORTURE") and not on("ROBIGO_RETRY_TORTURE"):
            kept |= {"volume", "collection-schedule"}
        if "R_VSIZE" not in env and "R_MAX_VSIZE" not in env:
            kept.add("default-vsize")
        return kept

    def list_name_problems(self, plans, root):
        """For every step that names cases in ROBIGO_CONF_TIMEOUTS, each name
        the runner refuses at its start ("names X, which is no case's id ...
        of an enabled case"): not exactly a case id (a `*.R` under tests/rt
        or tests/r, as the runner discovers them, named from the repository
        root), or a case whose .requires names a feature the step's
        environment turns off. And one more than the runner's start checks:
        a tests/r case without its passing-rt marker, which the runner
        accepts but never runs in CI, so the step's timeout for it would
        apply to nothing."""
        ids = {str(p.relative_to(root)) for suite in ("tests/rt", "tests/r") for p in (root / suite).rglob("*.R")
               if p.is_file()}
        problems = []
        for title, env, _ in plans:
            if "ROBIGO_CONF_TIMEOUTS" not in env:
                continue
            kept = self.kept_features(env)
            # (parsed as the runner's parse_case_timeouts: blank items
            # skipped, names trimmed)
            for item in env["ROBIGO_CONF_TIMEOUTS"].split(","):
                if not item.strip():
                    continue
                case = item.rsplit("=", 1)[0].strip()
                path = root / case
                if case not in ids:
                    problems.append(f"{title}: {case}: no case has this id")
                    continue
                if case.startswith("tests/r/"):
                    marker = root / "tests/r/passing-rt" / (case[len("tests/r/"):] + ".pass")
                    if not marker.is_file():
                        problems.append(f"{title}: {case}: not ratcheted for robigo-rt ({marker.relative_to(root)})")
                req = path.with_suffix(".requires")
                need = set(req.read_text().split()) if req.is_file() else set()
                unknown = sorted(need - self.KNOWN)
                if unknown:
                    problems.append(f"{title}: {case}: requires {', '.join(unknown)}, which the runner does not know")
                off = sorted((need & self.BY_ENVIRONMENT) - kept)
                if off:
                    problems.append(f"{title}: {case}: requires {', '.join(off)}, which this step turns off")
        return problems

    def test_every_case_a_step_names_is_one_it_runs(self):
        # (#961's lists named shared-body.R and lazy-promise-code.R, which
        # #966 made `volume`: the gctorture shard's runner refused to start,
        # ten minutes into its build. This finds it in seconds.)
        crates = ["robigo-heap", "robigo-rt", "robigo-rtapi", "robigo-rtcore"]
        plans = rt.steps(crates) + rt.full_move_steps()
        for shard in rt.SHARDS:
            plans += rt.shard_steps(crates, shard)
        self.assertEqual(self.list_name_problems(plans, HERE.parent), [])

    def test_a_named_case_its_step_turns_off_is_found(self):
        import tempfile
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        for case, requires in (("tests/rt/jit/big.R", "volume"), ("tests/rt/jit/ok.R", None),
                               ("tests/r/memory/unmarked.R", None), ("tests/rt/dynload/tools.R", "r-home-tools"),
                               ("tests/rt/jit/odd.R", "no-such-feature")):
            (root / case).parent.mkdir(parents=True, exist_ok=True)
            (root / case).write_text("1\n")
            if requires:
                (root / case).with_suffix(".requires").write_text(requires + "\n")
        plans = [("conformance under gctorture (E3)", {"ROBIGO_GCTORTURE": "1", "ROBIGO_CONF_TIMEOUTS":
                  "tests/rt/jit/big.R=600,tests/rt/jit/ok.R=600,tests/r/memory/unmarked.R=600,tests/rt/gone.R=600"},
                  ["cargo"]),
                 ("conformance (plain)", {"ROBIGO_CONF_TIMEOUTS": "tests/rt/jit/big.R=600"}, ["cargo"])]
        problems = self.list_name_problems(plans, root)
        self.assertEqual(len(problems), 3, problems)
        self.assertIn("conformance under gctorture (E3): tests/rt/jit/big.R: requires volume, which this step turns off",
                      problems)
        self.assertTrue(any("unmarked.R: not ratcheted" in p for p in problems), problems)
        self.assertTrue(any("tests/rt/gone.R: no case has this id" in p for p in problems), problems)
        # a build-dependent requirement is not the step's to turn off; an
        # unknown one the runner itself refuses
        build = [("conformance under gctorture (E3)", {"ROBIGO_GCTORTURE": "1", "ROBIGO_CONF_TIMEOUTS":
                  "tests/rt/dynload/tools.R=600,tests/rt/jit/odd.R=600"}, ["cargo"])]
        self.assertEqual(self.list_name_problems(build, root),
                         ["conformance under gctorture (E3): tests/rt/jit/odd.R: requires no-such-feature, "
                          "which the runner does not know"])
        # not ids, though the files exist; and an empty list names nothing
        (root / "tests/rt/R-4.6.2").mkdir(parents=True)
        (root / "tests/rt/R-4.6.2/ok.Rout").write_text("1\n")
        odd = [("step", {"ROBIGO_CONF_TIMEOUTS": "./tests/rt/jit/ok.R=600,tests/rt/R-4.6.2/ok.Rout=600"}, ["cargo"]),
               ("empty", {"ROBIGO_CONF_TIMEOUTS": rt.case_timeouts({})}, ["cargo"]),
               ("spaced", {"ROBIGO_CONF_TIMEOUTS": " tests/rt/jit/ok.R =600, "}, ["cargo"])]
        self.assertEqual(sorted(self.list_name_problems(odd, root)),
                         ["step: ./tests/rt/jit/ok.R: no case has this id",
                          "step: tests/rt/R-4.6.2/ok.Rout: no case has this id"])

    def test_the_moving_conformance_step_is_young_and_the_full_run_nightly(self):
        # H8: the job's GC_MOVE + gctorture conformance step moves the young
        # objects, with YOUNG_MOVE_SLOW's longer timeouts and no other step's;
        # the nightly's --full-move run moves everything, with
        # FULL_MOVE_SLOW's; each names a case that exists
        steps = rt.steps(["robigo-rt"])
        moving = [e for t, e, c in steps if "ROBIGO_GC_MOVE" in e and "--exact" in c]
        self.assertEqual(len(moving), 1)
        self.assertEqual(moving[0]["ROBIGO_GC_MOVE"], rt.GC_MOVE_YOUNG)
        # (and the plain gctorture step, which names GCTORTURE_FIRST at its
        # own limit: started first, not given longer)
        plain = [e for t, e, c in steps if t == "conformance under gctorture (E3)"]
        # (every default-timeout step names DEFAULT_STEP_SLOW: set apart)
        named = [t for t, e, _ in steps if "ROBIGO_CONF_TIMEOUTS" in e and "ROBIGO_CONF_TIMEOUT" in e]
        self.assertEqual(sorted(named), sorted(["conformance under gctorture (E3)",
                                                f"conformance under ROBIGO_GC_MOVE={rt.GC_MOVE_YOUNG} with gctorture (E3)"]))
        first = dict(item.rsplit("=", 1) for item in plain[0]["ROBIGO_CONF_TIMEOUTS"].split(","))
        self.assertEqual(first, {c: str(s) for c, s in {**rt.GCTORTURE_FIRST, **rt.GCTORTURE_SLOW}.items()})
        # GCTORTURE_SLOW's get longer (c-stack-overflow.R's interim 780 s);
        # every other case keeps the step's 600 s
        self.assertEqual(first["tests/r/conditions/c-stack-overflow.R"], "780")
        self.assertEqual(first["tests/rt/jit/serialize-write.R"], "600")
        for case, secs in first.items():
            self.assertTrue((HERE.parent / case).is_file(), case)
            if case in rt.GCTORTURE_SLOW:
                self.assertGreater(int(secs), int(plain[0]["ROBIGO_CONF_TIMEOUT"]), case)
            else:
                self.assertEqual(int(secs), int(plain[0]["ROBIGO_CONF_TIMEOUT"]), case)
        young = dict(item.rsplit("=", 1) for item in moving[0]["ROBIGO_CONF_TIMEOUTS"].split(","))
        self.assertEqual(set(young), set(rt.YOUNG_MOVE_SLOW) | set(rt.GCTORTURE_FIRST) | set(rt.YOUNG_MOVE_FIRST))
        for case, secs in young.items():
            self.assertTrue((HERE.parent / case).is_file(), case)
            # YOUNG_MOVE_SLOW's get longer; GCTORTURE_FIRST's and
            # YOUNG_MOVE_FIRST's are only started first
            if case in rt.YOUNG_MOVE_SLOW:
                self.assertGreater(int(secs), int(moving[0]["ROBIGO_CONF_TIMEOUT"]), case)
            else:
                self.assertEqual(int(secs), int(moving[0]["ROBIGO_CONF_TIMEOUT"]), case)
        # (the step's own limit stays: a hung case still fails in ten minutes)
        self.assertEqual(moving[0]["ROBIGO_CONF_TIMEOUT"], "600")
        self.assertTrue(rt.GC_MOVE_YOUNG.startswith("young:") and int(rt.GC_MOVE_YOUNG[6:]) >= 1)
        full = rt.full_move_steps()
        self.assertEqual([c for _t, _e, c in full[:3]],
                         [["scripts/rt-home.sh"], rt.release_build()[2], rt.db_tree(rt.rt_binary())[2]])
        self.assertEqual(len(full), 4)
        _, env, cmd = full[3]
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

    def test_a_crosscheck_report_with_merged_records_fails(self):
        # two processes' appends interleaved: a record, then another's, then
        # both newlines; the merged line counts once by its first word, so
        # the count fell under the floor on a 24-core box with no error
        import tempfile
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        good = "same: f: at 0: translated A; from source A\n"
        clean = os.path.join(tmp.name, "clean.txt")
        with open(clean, "w") as f:
            f.write(good * 3 + "skipped: { x }: the from-source compile would force a binding (y)\n"
                    + "same: g(\"same: inside a string\")\n"
                    + "same: h(\"notsame: a kind's name after a letter\")\n")
        errors, counts = rt.bc_report_check(clean, 5)
        self.assertEqual((errors, counts["compared"]), ([], 5))
        merged = os.path.join(tmp.name, "merged.txt")
        with open(merged, "w") as f:
            f.write(good + "same: f: at 1: translated B; from source Bsame: g: at 0: translated C; from source C\n\n"
                    + "skipped: { y }: force (z)expected (a loop's shape (x)): h: at 2: translated D; from source D\n\n")
        errors, counts = rt.bc_report_check(merged, 1)
        self.assertEqual(counts["compared"], 2)       # what a merge does to the count
        self.assertIn("2 empty line(s)", errors[0])
        self.assertEqual(sum(e.startswith("two records in one line: ") for e in errors), 2, errors)
        # a record a case could not write: its marker fails the report,
        # whatever the count
        open(clean + ".lost", "w").close()
        errors, counts = rt.bc_report_check(clean, 5)
        self.assertEqual(counts["compared"], 5)
        self.assertEqual(len(errors), 1)
        self.assertIn("could not write a record", errors[0])

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


def rust_src_snippets(root):
    """The places where the crates' compile-fail expectations (trybuild's
    tests/**/*.stderr) quote the standard library's source. rustc prints a
    location in std (`--> $RUST/core/src/marker.rs`, or `::: $RUST/...`)
    either way, but the source lines under it (the `|` gutter and the code)
    only where the rust-src component is installed; an expectation blessed
    there fails on CI's minimal toolchain (#954: robigo-rtcore's
    env_writer_not_sync.stderr, CI run 45063956014)."""
    import re
    location = re.compile(r"^\s*(-->|:::)\s+\$RUST/")
    out = []
    for path in sorted(root.glob("crates/*/tests/**/*.stderr")):
        lines = path.read_text(errors="replace").splitlines()
        for n, line in enumerate(lines, 1):
            if location.match(line) and n < len(lines) and lines[n].lstrip().startswith("|"):
                out.append(f"{path.relative_to(root)}:{n}: {line.strip()} (and the source under it)")
    return out


class CompileFailExpectations(unittest.TestCase):
    def test_no_expectation_depends_on_rust_src(self):
        found = rust_src_snippets(HERE.parent)
        self.assertEqual(found, [], "a compile-fail expectation quotes the standard library's source under a "
                         "$RUST/ location, which rustc shows only with the rust-src component: CI's toolchain has "
                         "none, so the test fails there. Keep the location line and drop the `|` lines under it "
                         "(or re-bless without rust-src: `rustup component remove rust-src`, then "
                         "TRYBUILD=overwrite):\n  " + "\n  ".join(found))

    def test_a_quoted_std_source_is_found_and_a_bare_location_is_not(self):
        import tempfile
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        ui = root / "crates/x/tests/ui"
        ui.mkdir(parents=True)
        # what B's machine wrote (#954), and what CI prints
        (ui / "with-rust-src.stderr").write_text(
            "note: required because it appears within the type `PhantomData<*const ()>`\n"
            " --> $RUST/core/src/marker.rs\n  |\n  | pub struct PhantomData<T: PointeeSized>;\n"
            "  |            ^^^^^^^^^^^\nnote: next\n")
        (ui / "without.stderr").write_text(
            "note: required because it appears within the type `PhantomData<*const ()>`\n"
            " --> $RUST/core/src/marker.rs\nnote: required because it appears within the type `X`\n"
            " --> src/clib.rs\n  |\n  | struct X {\n")
        self.assertEqual(rust_src_snippets(root),
                         ["crates/x/tests/ui/with-rust-src.stderr:2: --> $RUST/core/src/marker.rs "
                          "(and the source under it)"])

class BuildOraclePull(unittest.TestCase):
    """scripts/build-oracle.sh --pull: a failed pull names the ref, the
    login for an auth error, and the local build (docker faked on PATH)."""

    def run_pull(self, docker_stderr):
        import subprocess
        import tempfile
        with tempfile.TemporaryDirectory() as d:
            fake = Path(d, "docker")
            fake.write_text("#!/bin/sh\n[ \"$1\" = pull ] || exit 3\n"
                            f"echo '{docker_stderr}' >&2\nexit 1\n")
            fake.chmod(0o755)
            env = dict(os.environ, PATH=f"{d}:{os.environ['PATH']}", HOME=d)
            return subprocess.run([str(HERE / "build-oracle.sh"), "--pull", "4.6.2"], env=env,
                                  capture_output=True, text=True)

    def test_auth_error_says_login_and_fallback(self):
        p = self.run_pull("Error response from daemon: Head \"https://ghcr.io/v2/x\": unauthorized")
        self.assertEqual(p.returncode, 1)
        self.assertIn("unauthorized", p.stderr)  # docker's own error is kept
        self.assertRegex(p.stderr, r"docker pull ghcr\.io/\S+:oracle-4\.6\.2-\S+ failed")
        self.assertIn("docker login ghcr.io", p.stderr)
        self.assertIn("scripts/build-oracle.sh --image 4.6.2", p.stderr)

    def test_other_error_gives_fallback_without_login(self):
        p = self.run_pull("Error response from daemon: manifest unknown")
        self.assertEqual(p.returncode, 1)
        self.assertNotIn("docker login", p.stderr)
        self.assertIn("scripts/build-oracle.sh --image 4.6.2", p.stderr)


if __name__ == "__main__":
    unittest.main()
