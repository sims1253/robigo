#!/usr/bin/env python3
"""CI's `rt` job: G3's runtime crates (docs/g3.md §14.1, §16).

    scripts/rt-ci.py            # premise check, the crates' tests, the conformance and its
                                # modes: every step (the crates' tests in the quick profile)
    scripts/rt-ci.py --shard NAME  # one CI job's steps (SHARDS); CI runs each in parallel
    scripts/rt-ci.py --shard debug-assertions  # the crates' tests and the conformance with
                                # debug assertions on (a build of their own, in
                                # the target directory + "-debug-assertions";
                                # the plain run above does not include it)
    scripts/rt-ci.py --shard release-crate-tests  # the crates' tests on the full release
                                # profile, as main's push runs do (not in the plain run)
    scripts/rt-ci.py --list     # the rt crates, one per line
    scripts/rt-ci.py --check    # the premise check alone
    scripts/rt-ci.py --full-move  # the nightly's run: every collection moves everything

The rt crates are the workspace members under the `rt` rule of
scripts/ci-selection.toml, so the selection and this job cannot drift apart.

The premise behind that rule's skipped heavy jobs is that the robigo binary
cannot change when only rt crates do. It holds while no other workspace
member depends on an rt crate in any way (normal, build or dev: the skipped
`test` job would miss a dev-dependency too). This script fails first if one
does. At the flip (M7) the CLI depends on robigo-rt, this check fails, and
the `rt` rule has to go.

Then `cargo test --release` on the rt crates; the `robigo-rt` binary's
conformance (`tests/rt` and the `tests/r` cases it has ratcheted, against GNU
R 4.6.2: crates/robigo-rtapi/tests/conformance.rs, in the binary's own
crate) with its report in the log, then again under each observation mode,
`ROBIGO_NO_INPLACE=1` and `ROBIGO_MATERIALIZE=1` (docs/g3.md §15.3 E5: the
production recordings, without the cases whose `.requires` names the
identity the mode gives up); the crates' tests under `ROBIGO_NO_INPLACE=1`;
and the
crates' tests again under `ROBIGO_GCTORTURE=1` (a collection at every
allocation) with `ROBIGO_GC_STATS=check` (a large `&Cx` request while a
collection is due panics); under `ROBIGO_GC_MOVE=1` with gctorture (every
collection moves what it may and poisons the old copies: a raw `Sexp` kept
across a safepoint outside the roots reads poison or faults); and with
`ROBIGO_GC_PIN_REPORT` (a weak reference decided dead while a C word reaches
its key: the rt slice's package code protects what it keeps, so any report
but those of experiment A's two probes, which hold unprotected pointers on
purpose, fails the job).
Then the conformance again under gctorture and under the moving collector
(E3; `young:N`, which moves the young objects at each collection and
everything at every N-th, H8), the stress script in a few fresh processes (E3, scripts/rt-stress.sh),
the overshoot of every slice (E4, scripts/rt-overshoot.py), and E9: the
conformance under a small vector limit, allocations refused once after
visible effects (scripts/rt-fail-alloc.py), and, last, the conformance with
every `&Cx` build's retry run once (`ROBIGO_RETRY_TORTURE=1`), in a build with
robigo-rtcore's `retry-torture` feature made for the last steps alone, and in
the same build robigo-rt's tests and the conformance with every dead slot
poisoned (`ROBIGO_POISON_DEAD=1`, robigo-rt's `poison-dead` feature). The
verification modes of g3.md §16 add their steps here as they land.
"""
import argparse
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

_spec = importlib.util.spec_from_file_location("ci_select", os.path.join(ROOT, "scripts", "ci-select.py"))
ci_select = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(ci_select)


def rt_patterns(rules):
    for name, pats, _mode, _runtime in rules:
        if name == "rt":
            return pats
    raise ValueError("scripts/ci-selection.toml has no rule named 'rt'")


def rt_crates(meta, pats, root=ROOT):
    """Names of the workspace members whose manifest the rt rule covers."""
    members = set(meta["workspace_members"])
    out = []
    for p in meta["packages"]:
        if p["id"] not in members:
            continue
        rel = os.path.relpath(p["manifest_path"], root).replace(os.sep, "/")
        if any(g.match(rel) for g in pats):
            out.append(p["name"])
    return sorted(out)


def leaks(meta, crates):
    """(member, rt dependency) for every other workspace member that depends on an rt crate."""
    members = set(meta["workspace_members"])
    rt = set(crates)
    out = []
    for p in meta["packages"]:
        if p["id"] not in members or p["name"] in rt:
            continue
        for d in p["dependencies"]:
            if d["name"] in rt:
                out.append((p["name"], d["name"]))
    return sorted(out)


# robigo-rtapi's conformance test of the binary (crates/robigo-rtapi/tests/conformance.rs): its
# own step, so that its report reaches the log; the others skip it.
CONFORMANCE = "rt_conformance"


# where the pin report step writes (appended to by every process)
PIN_REPORT = os.path.join(tempfile.gettempdir(), f"robigo-rt-ci-pin-report-{os.getpid()}.txt")


# where the byte-code crosscheck step writes (V3: one line per translated
# closure whose code differs from the from-source compiler's)
BC_REPORT = os.path.join(tempfile.gettempdir(), f"robigo-rt-ci-bc-crosscheck-{os.getpid()}.txt")
# the same under the rdb boot (base's closures from base.rdb, so many more)
BC_REPORT_RDB = os.path.join(tempfile.gettempdir(), f"robigo-rt-ci-bc-crosscheck-rdb-{os.getpid()}.txt")

# the kinds of line the crosscheck writes (crates/robigo-rt/src/compile/crosscheck.rs),
# each the line's first word, then ":" (or a class in parentheses, then ":")
BC_KINDS = ("same", "expected", "unexpected", "untranslated", "skipped")


def bc_report_check(path):
    """The crosscheck report's errors and its counts, each kind's (and each
    expected class's) and "compared". An error is an unexpected difference,
    an untranslated closure, a line of no kind the crosscheck writes, or
    nothing compared: every closure compared has its line (same, expected
    or unexpected), so a step that expects comparisons and has none, or no
    report at all, is a broken setup, never a pass."""
    try:
        with open(path) as f:
            lines = [line.rstrip("\n") for line in f if line.strip()]
    except FileNotFoundError:
        lines = []
    errors = []
    counts = {}
    compared = 0
    for x in lines:
        if x.startswith("    "):
            # (ROBIGO_BC_CROSSCHECK_DUMP's instruction lines)
            continue
        head = x.split(":", 1)[0] if ":" in x else ""
        kind = head.split(" ", 1)[0]
        if kind not in BC_KINDS or (kind != "expected" and head != kind):
            errors.append(f"malformed: {x}")
            continue
        if kind in ("unexpected", "untranslated"):
            errors.append(x)
        compared += kind in ("same", "expected", "unexpected")
        counts[head] = counts.get(head, 0) + 1
    if compared == 0:
        errors.append(f"compared 0 closures: the crosscheck report {path} names none")
    counts["compared"] = compared
    return errors, counts


# Experiment A's probes and the allocation-parity probe built on them
# (crates/robigo-rtapi/tests/support/capi_test.c, docs/g3.md §15.1) keep
# external pointers only in C locals on purpose: the report may name them,
# and nothing else may appear.
EXPECTED_PIN_ROUTINES = ("capi_pp_probe", "capi_eb_probe", "rt_collects_in")


def pin_report_hits(path):
    """The pin report's lines (none if the file is missing), but the probes'."""
    try:
        with open(path) as f:
            lines = [line.rstrip("\n") for line in f if line.strip()]
    except FileNotFoundError:
        return []
    return [x for x in lines if not any(f" routine={r} " in x for r in EXPECTED_PIN_ROUTINES)]


# H8: the moving collector's policy in the job's gctorture conformance step.
# `young:N` moves the young objects at each collection and everything at
# every N-th (crates/robigo-heap/src/gc/mover.rs): a torture collection then
# costs about a plain one. N was chosen by measurement
# (docs/ledger/g3/2026-11-17-h8-young-torture.md). The nightly's full run
# (`--full-move`, .github/workflows/nightly.yml) moves everything at every
# collection, so a path that runs once is still caught by some run.
GC_MOVE_YOUNG = "young:16"

# The cases that need longer than the slow modes' 600 s in the full run,
# and how long they get: there every collection moves the whole live heap.
# The cases still run: only their timeout differs.
FULL_MOVE_SLOW = {
    # 2.4M collections of a ~5k-object live heap: ~470 s here, ~536 s for
    # main's whole step on CI, so 600 s fails on a slow runner
    "tests/r/memory/finalizers-self-ref-envs.R": 1200,
}


# The cases that need longer than the slow modes' 600 s in the job's
# GC_MOVE=young:16 gctorture step, and how long they get. Each still runs;
# every other case keeps the 600 s, so a hung unlisted case still fails in
# ten minutes (a listed one in its own time). The runner starts the listed
# cases first, so they do not start late:
YOUNG_MOVE_SLOW = {
    # a C stack overflow's deep recursion, each frame under gctorture with
    # the moving collector. In that step on CI: about 360 s (run
    # 44865825323, 2026-11-21 04:22), about 600 s (run 44883150235, 08:27,
    # passed at the limit), over 600 s (run 44896845237, #927: failed);
    # 134-248 s on the 24-CPU workstation, with its load. Not a regression:
    # the step's settings count 832-839 G user instructions at dc404c7da
    # (run 44865825323's tree) and 822 G at main 6b5d719a8 (perf stat, two
    # runs each, 2026-11-21), so the spread on CI is the runner's
    "tests/r/conditions/c-stack-overflow.R": 1200,
}


# The cases the two gctorture conformance steps start first, at the step's
# own limit (600 s): each takes minutes under gctorture, and started in id
# order it ran alone at the step's end. tests/rt/jit/shared-body.R (it loads
# the compiler): 177 s under gctorture, 200 s under GC_MOVE=young:16 on the
# 24-CPU workstation (2026-11-21); in #952's run 44927530340 the two steps
# took 2.7 and 3.1 minutes longer than in #941's, its 8 new cases' doing
GCTORTURE_FIRST = {
    "tests/rt/jit/shared-body.R": 600,
    "tests/r/conditions/c-stack-overflow.R": 600,
}


def case_timeouts(slow):
    """ROBIGO_CONF_TIMEOUTS's value for `slow` (case -> seconds)."""
    return ",".join(f"{c}={secs}" for c, secs in sorted(slow.items()))


# The crates' own tests build in the `quick` profile (release's opt-level,
# no LTO, 256 codegen units): with release's thin LTO and one codegen unit
# each of the ~64 test binaries costs ~70-110 s to build (a cold build of all
# of them: 766 s at -j6, against 106 s in `quick`; their run takes the same
# time in both, 20 s). Everything that judges the binary users get stays on
# release: every conformance step and mode, the boot_db step and the
# scripts that run the binary, the differential, perf. (The crates' tests
# that spawn the binary through CARGO_BIN_EXE_robigo-rt, alloc, front_end,
# sigint, cli, now spawn the quick one: they test the crates' code, and the
# release binary keeps its own steps.) The crates' tests on the full release
# profile run in the release-crate-tests shard, on main's push runs.
CRATE_TEST = ["cargo", "test", "--profile", "quick", "--locked"]

# Every run of the crates' tests comes after base's database tree is made in
# its shard, and says it must be there: robigo-rtapi's boot_db tests then run
# instead of skipping, and a tree that is missing fails them
TREE_RUNS = {"ROBIGO_RT_HOME_DB": "1"}
DB_TREE = ("base's database tree, written by this robigo-rt (scripts/rt-home-db.sh)", ["scripts/rt-home-db.sh"])


def conformance_command():
    test = ["cargo", "test", "--release", "--locked"]
    return [*test, "-p", "robigo-rtapi", "--test", "conformance", "--", "--exact", CONFORMANCE, "--nocapture"]


# (a case under a collector mode runs many times longer than the runner's
# default 20 s allow)
SLOW = {"ROBIGO_CONF_TIMEOUT": "600"}


def every_cpu(steps):
    """The steps, with the conformance runner given a case per CPU in each
    conformance step: those run one test alone (`--exact`), so the cases may
    use every CPU (the runner's default, 2, is for a cargo test that shares
    the machine with other test binaries). The runner starts the cases named
    in ROBIGO_CONF_TIMEOUTS first."""
    jobs = str(os.cpu_count() or 2)
    return [(t, {**e, "ROBIGO_CONF_JOBS": jobs} if CONFORMANCE in c and "--exact" in c else e, c) for t, e, c in steps]


def full_move_steps():
    """The nightly's full run (`--full-move`): the conformance slice under
    gctorture, every collection moving everything it may."""
    full_slow = case_timeouts(FULL_MOVE_SLOW)
    return every_cpu([
        ("conformance under ROBIGO_GC_MOVE=1 with gctorture (every collection moves everything)",
         {"ROBIGO_GCTORTURE": "1", "ROBIGO_GC_MOVE": "1", **SLOW, "ROBIGO_CONF_TIMEOUTS": full_slow},
         conformance_command()),
    ])


# The cargo features that compile ROBIGO_RETRY_TORTURE and ROBIGO_POISON_DEAD
# in (robigo-rtcore's and robigo-rt's Cargo.toml): only the last steps build
# with them, all with both, so that they share one build.
CHECK_FEATURES = "robigo-rtcore/retry-torture,robigo-rt/poison-dead"


def steps(crates):
    """(title, extra environment, command) for each step of the job."""
    pkgs = [x for c in crates for x in ("-p", c)]
    test = ["cargo", "test", "--release", "--locked"]
    crate = CRATE_TEST
    skip = ["--", "--skip", CONFORMANCE]
    conformance = conformance_command()
    slow = SLOW
    out = [
        # robigo-rt's R_HOME tree with tools' native code (A5): the
        # conformance cases that load it (`.requires` r-home-tools) run
        ("the R_HOME tree, tools.so built (scripts/rt-home.sh)", {}, ["scripts/rt-home.sh"]),
        # the release binary users get, which the steps below run (base's
        # database tree, boot_db, the scripts): built as the conformance step
        # builds it (its features), since the crates' tests build quick
        ("the release binary and the conformance, built (cargo test --release --no-run)", {},
         [*test, "-p", "robigo-rtapi", "--test", "conformance", "--no-run"]),
        # L5's boot beside M1's loader (ROBIGO_RT_BOOT=rdb): base's database,
        # written by this robigo-rt booted from base's source, then base
        # booted from it (crates/robigo-rtapi/tests/boot_db.rs). Before every
        # step whose tests read it (robigo-rtapi's boot_db tests, in each run
        # of the crates' tests), so a stale tree is rebuilt first (its stamp
        # names the source tree, the script and the binary) and those tests
        # run instead of skipping on a fresh checkout (TREE_RUNS)
        ("base's database tree, written by this robigo-rt (scripts/rt-home-db.sh)", {}, ["scripts/rt-home-db.sh"]),
        ("the crates' tests (quick profile)", TREE_RUNS, [*crate, *pkgs, *skip]),
        ("boot from base's database (ROBIGO_RT_BOOT=rdb)", {"ROBIGO_RT_HOME_DB": "1"},
         [*test, "-p", "robigo-rtapi", "--test", "boot_db", *skip]),
        # the boot that replaces M1's (L5, ROBIGO_RT_BOOT=rdb: base from the
        # tree's base.rdb, as GNU R boots): the ratcheted conformance under it,
        # plain and with the byte-code crosscheck, each failing as its M1
        # step does (a ratcheted case that fails; an unexpected or
        # untranslated closure). Not yet the default anywhere (L's list)
        ("conformance under the rdb boot (ROBIGO_RT_BOOT=rdb)", {"ROBIGO_RT_BOOT": "rdb"}, conformance),
        ("conformance under the rdb boot with ROBIGO_BC_CROSSCHECK=1 (base.rdb's closures against the "
         "from-source compiler's)",
         {"ROBIGO_RT_BOOT": "rdb", "ROBIGO_BC_CROSSCHECK": "1", "ROBIGO_BC_CROSSCHECK_REPORT": BC_REPORT_RDB},
         conformance),
        ("conformance (tests/rt, ratcheted tests/r; GNU R 4.6.2)", {}, conformance),
        # docs/g3.md §15.3 E5: the same cases and recordings under each
        # observation mode, but those whose `.requires` names the identity
        # the mode gives up (robigo-conformance's KNOWN_FEATURES)
        ("conformance under ROBIGO_NO_INPLACE=1 (every in-place write copies)",
         {"ROBIGO_NO_INPLACE": "1"}, conformance),
        ("conformance under ROBIGO_MATERIALIZE=1 (every argument slot materialized)",
         {"ROBIGO_MATERIALIZE": "1"}, conformance),
        # the identity tests pin production and check the mode's side
        # themselves (tests/attrib.rs, tests/subset.rs)
        ("ROBIGO_NO_INPLACE=1 (every in-place write copies)",
         {**TREE_RUNS, **{"ROBIGO_NO_INPLACE": "1"}}, [*crate, *pkgs, *skip]),
        # with a collection due almost always, the check makes any &Cx
        # request of LARGE bytes or more panic where it is made: such a
        # request goes through cx.alloc_rooted (docs/g3.md §3 item 3)
        ("gctorture (collect at every allocation) and the GC stats check",
         {**TREE_RUNS, **{"ROBIGO_GCTORTURE": "1", "ROBIGO_GC_STATS": "check"}}, [*crate, *pkgs, *skip]),
        # docs/g3.md §15.3 E3: the rt slice under the collector modes
        ("conformance under gctorture (E3)",
         {"ROBIGO_GCTORTURE": "1", **slow, "ROBIGO_CONF_TIMEOUTS": case_timeouts(GCTORTURE_FIRST)}, conformance),
        # every argument a PROMSXP at call entry (GNU R's representation,
        # through the same binding-kind transitions): docs/g3.md §16's
        # ROBIGO_MATERIALIZE=1. Every rt crate, as the steps above: cargo
        # turns a selected crate's dev-dependency features on (robigo-rtcore's
        # robigo-heap/test-support), so `-p robigo-rt` alone rebuilt heap and
        # every crate above it (6 minutes in CI)
        ("ROBIGO_MATERIALIZE=1 (every argument slot materialized)",
         {**TREE_RUNS, **{"ROBIGO_MATERIALIZE": "1"}}, [*crate, *pkgs, *skip]),
        # docs/g3.md §16 (H3): the debug moving collector, at every
        # collection gctorture makes due
        ("ROBIGO_GC_MOVE (move every movable object at every collection), with gctorture",
         {**TREE_RUNS, **{"ROBIGO_GCTORTURE": "1", "ROBIGO_GC_MOVE": "1"}}, [*crate, *pkgs, *skip]),
        # H8: the young objects at each collection, everything at every N-th;
        # the nightly runs it with every collection moving everything
        (f"conformance under ROBIGO_GC_MOVE={GC_MOVE_YOUNG} with gctorture (E3)",
         {"ROBIGO_GCTORTURE": "1", "ROBIGO_GC_MOVE": GC_MOVE_YOUNG, **slow,
          "ROBIGO_CONF_TIMEOUTS": case_timeouts({**GCTORTURE_FIRST, **YOUNG_MOVE_SLOW})}, conformance),
        # D5's frame recycling (docs/g3.md §16): every frame that would be recycled
        # is poisoned and held back instead, so a frame that escaped without
        # its ESCAPED bit panics where it is used next
        ("ROBIGO_FRAME_QUARANTINE=1 (recycled frames poisoned and held back)",
         {**TREE_RUNS, **{"ROBIGO_FRAME_QUARANTINE": "1"}}, [*crate, *pkgs, *skip]),
        ("ROBIGO_FRAME_QUARANTINE=1, conformance",
         {"ROBIGO_FRAME_QUARANTINE": "1"},
         conformance),
        # docs/g3.md §16 (H3): the pin report; `main` fails the job on any line
        ("ROBIGO_GC_PIN_REPORT (no weak reference decided while a C word reaches its key)",
         {**TREE_RUNS, **{"ROBIGO_GC_PIN_REPORT": PIN_REPORT}}, [*crate, *pkgs, *skip]),
        # docs/g3.md §15.3 E3 and E4, on the release binary the steps above
        # built: the stress script in fresh processes (the ledger's runs take
        # 30 per profile), and the overshoot of every slice against max(32
        # MiB, 10% of the live heap)
        ("the stress script, 3 runs per profile (E3, scripts/rt-stress.sh)", {}, ["scripts/rt-stress.sh", "--runs", "3"]),
        ("overshoot on every slice (E4, scripts/rt-overshoot.py)", {}, ["python3", "scripts/rt-overshoot.py"]),
        # E9: the slice under a small vector heap and limit, and allocations
        # refused once after visible effects (ROBIGO_FAIL_ALLOC_AT)
        ("conformance under a small vector limit (E9)", {"R_VSIZE": "4M", "R_MAX_VSIZE": "16M"}, conformance),
        # V3: every closure whose body is GNU R's byte code translated and
        # compared with the from-source compiler's code of its source;
        # `main` fails the job on an unexpected difference (the expected
        # ones are the from-source port's known gaps, counted)
        ("conformance under ROBIGO_BC_CROSSCHECK=1 (translated byte code against the from-source compiler's)",
         {"ROBIGO_BC_CROSSCHECK": "1", "ROBIGO_BC_CROSSCHECK_REPORT": BC_REPORT}, conformance),
        ("injected allocation failures (E9, scripts/rt-fail-alloc.py)", {},
         ["python3", "scripts/rt-fail-alloc.py", "--points", "400"]),
        # A4 (docs/g3.md §16, ROBIGO_BUILTIN_DIFF): every row of
        # tests/builtin-diff/rows.toml against GNU R 4.6.2's records (CI's
        # builtin-diff-data job; locally made with docker), the known/
        # ratchet held, then the planted-divergence check on a build with
        # robigo-rtbuiltins/diff-plant (before the retry-torture step, which
        # rebuilds without it)
        ("builtin differential and its planted divergences (A4, scripts/builtin-diff.py ci)", {},
         ["python3", "scripts/builtin-diff.py", "ci"]),
        # E9 (docs/g3.md §16): every `&Cx` build's first allocation refused
        # once, so each build's retry runs. The mode is compiled in only with
        # robigo-rtcore's feature, in a build (CHECK_FEATURES, with robigo-rt's
        # poison-dead) made for the last steps alone, this one and the two
        # poison-dead steps after it: every step above tests the binary users
        # get (cargo keeps both builds). With ROBIGO_GC_MOVE=1 as well it takes
        # about 20 minutes, so that run is by hand (g3.md §16).
        ("conformance under ROBIGO_RETRY_TORTURE=1 (E9: every build's retry runs once)",
         {"ROBIGO_RETRY_TORTURE": "1", **slow},
         [*test, "-p", "robigo-rtapi", "--features", CHECK_FEATURES, "--test", "conformance", "--",
          "--exact", CONFORMANCE, "--nocapture"]),
        # C1-PZ (docs/g3.md §16): a dead slot (a register's last use, D9's
        # clears; the slots a scope's restore frees) holds a poison naming
        # where it died, and a read of it panics: robigo-rt's tests, with
        # tests/poison.rs's planted early clear, then the conformance
        ("ROBIGO_POISON_DEAD=1 (dead slots poisoned), robigo-rt's tests and the planted early clear",
         {"ROBIGO_POISON_DEAD": "1"},
         [*crate, "-p", "robigo-rt", "--features", CHECK_FEATURES, *skip]),
        ("ROBIGO_POISON_DEAD=1, conformance",
         {"ROBIGO_POISON_DEAD": "1"},
         [*test, "-p", "robigo-rtapi", "--features", CHECK_FEATURES, "--test", "conformance", "--",
          "--exact", CONFORMANCE, "--nocapture"]),
    ]
    return every_cpu(out)


# CI's rt jobs (.github/workflows/ci.yml), each one shard of the steps,
# built in each job and run in parallel. Each is named for what it runs.
# Before 2026-11-21 one job ran them all (29-38 minutes against a 40-minute
# limit); #910 split them in three; with the fourth job they were rebalanced
# from run 44883150235's step times, so that the crates' test build is paid
# where crate tests run and nowhere else, and since then that build is in the
# quick profile (CRATE_TEST):
#   modes         the crates' tests and every mode they run under
#                 (no-inplace, gctorture with the stats check, materialize,
#                 GC_MOVE, quarantine, the pin report), the conformance and
#                 its short modes, stress, overshoot, E9's small limit and
#                 failures, the byte-code crosscheck
#   gctorture     the two long conformance runs under gctorture: plain, and
#                 with GC_MOVE=young:16 (a conformance build only)
#   feature-builds  retry-torture's conformance, then, in its build
#                 (CHECK_FEATURES), the poison-dead steps
#   builtin-diff  A4's builtin differential with its planted divergences
#                 (their own build), against builtin-diff-data's GNU R records
#   debug-assertions  the crates' tests (quick) and the conformance (release)
#                 with `-C debug-assertions=on` (heap's
#                 debug_assert!s see compiled code; not opt-level 0, where
#                 conditions/eval-depth-limit.R fails only because debug C
#                 frames are larger), in a target of its own; only in its
#                 own job: a plain local run does not build it
#   release-crate-tests  the crates' tests on the full release profile, the
#                 code users get (main's push runs only, so a codegen-dependent
#                 failure of a crate test shows within one main run of its
#                 merge); a plain local run does not build it either
# The R_HOME tree (seconds) is made in every shard; every other step is in
# exactly one (scripts/test-rt-ci.py checks both).
SHARDS = ("modes", "gctorture", "feature-builds", "builtin-diff", "debug-assertions", "release-crate-tests")
# the shards whose steps are their own, not a part of a plain run's
OWN_STEPS = ("debug-assertions", "release-crate-tests")

# Each shard's budget, in minutes: its steps' total in a CI run plus about
# 20%, rounded up. Its job's timeout-minutes is twice the budget, and the
# end-of-log table warns at WARN_AT of it, so a drift shows long before the
# timeout (scripts/test-rt-ci.py pins the three relations). When a warning
# fires, rebalance or remeasure: the dates say how old each measurement is.
BUDGETS = {  # shard: (budget, measured minutes, where measured)
    "modes": (9, 6.99, "2026-11-21, run 44927530340 (#952): 6.49, and the two rdb-boot steps' estimated 0.5"),
    # the plain gctorture conformance moves into modes if GCTORTURE_FIRST
    # does not bring this back near 15 minutes
    "gctorture": (25, 20.62, "2026-11-21, run 44927530340 (#952), before GCTORTURE_FIRST"),
    "feature-builds": (12, 9.87, "2026-11-21, run 44927530340 (#952)"),
    "builtin-diff": (9, 7.42, "2026-11-21, run 44927530340 (#952)"),
    "debug-assertions": (5, 3.47, "2026-11-21, run 44927530340 (#952): 3.07, and the tree's estimated 0.4"),
    "release-crate-tests": (12, 9.82, "2026-11-21, main's run 44931071167: 9.42, and the tree's estimated 0.4"),
}
# A job's timeout is twice its budget, but no less than this: a build from a
# cold cache (a Cargo.lock change, an evicted entry) takes up to about 12
# minutes on its own (rt-debug-assertions' first run, #941)
TIMEOUT_FLOOR = 20


def timeout_of(budget):
    """A shard's job timeout, in minutes, for its budget."""
    return max(2 * budget, TIMEOUT_FLOOR)
WARN_AT = 0.9


def budget_warning(shard, minutes):
    """The warning for a shard whose steps took `minutes`, or None below
    WARN_AT of its budget."""
    if shard not in BUDGETS:
        return None
    budget, measured, where = BUDGETS[shard]
    if minutes < WARN_AT * budget:
        return None
    return (f"::warning::rt: shard {shard} took {minutes:.1f} min, {100 * minutes / budget:.0f}% of its "
            f"{budget}-minute budget ({measured} min measured {where}); rebalance the shards or remeasure "
            f"(scripts/rt-ci.py's BUDGETS) before it reaches the job's {timeout_of(budget)}-minute timeout")


def target_dir():
    """cargo's target directory for this checkout (CARGO_TARGET_DIR, the
    configs' build.target-dir, else ROOT/target)."""
    return metadata()["target_directory"]


def debug_assertions_steps(crates):
    """The debug-assertions shard: the R_HOME tree, the crates' tests (quick
    profile) and the conformance (release), all with debug assertions on, in
    a target directory of its own (the normal build stays as it is). The
    R_HOME step builds robigo-heap for its headers, so it gets the same
    flags and target: nothing in this shard builds into the normal one."""
    pkgs = [x for c in crates for x in ("-p", c)]
    env = {"RUSTFLAGS": "-C debug-assertions=on", "CARGO_TARGET_DIR": target_dir() + "-debug-assertions"}
    test = ["cargo", "test", "--release", "--locked"]
    return every_cpu([
        ("the R_HOME tree, tools.so built (scripts/rt-home.sh)", env, ["scripts/rt-home.sh"]),
        # the conformance's build (with the flag) first: its release binary
        # writes the database tree the crates' tests read
        ("the release binary and the conformance with debug assertions, built (cargo test --release --no-run)", env,
         [*test, "-p", "robigo-rtapi", "--test", "conformance", "--no-run"]),
        (DB_TREE[0] + " (with debug assertions)", env, DB_TREE[1]),
        ("the crates' tests with debug assertions (-C debug-assertions=on, quick profile)", {**env, **TREE_RUNS},
         [*CRATE_TEST, *pkgs, "--", "--skip", CONFORMANCE]),
        ("conformance with debug assertions (-C debug-assertions=on)", env, conformance_command()),
    ])


def release_crate_tests_steps(crates):
    """The release-crate-tests shard (main's push runs): the crates' tests on
    the full release profile (thin LTO, one codegen unit), the code users get,
    which the other shards' quick-profile crate tests do not build."""
    pkgs = [x for c in crates for x in ("-p", c)]
    test = ["cargo", "test", "--release", "--locked", *pkgs]
    return every_cpu([
        ("the R_HOME tree, tools.so built (scripts/rt-home.sh)", {}, ["scripts/rt-home.sh"]),
        # built first (the same command, so the run below builds nothing):
        # its release binary writes the database tree the tests read
        ("the crates' tests (release profile), built", {}, [*test, "--no-run"]),
        DB_TREE[:1] + ({},) + DB_TREE[1:],
        ("the crates' tests (release profile)", TREE_RUNS, [*test, "--", "--skip", CONFORMANCE]),
    ])


def shard_of(title):
    """The shard a step of `steps` runs in, or None for every shard."""
    if title.startswith("the R_HOME tree"):
        return None
    if title.startswith("conformance under gctorture") or title.startswith("conformance under ROBIGO_GC_MOVE"):
        return "gctorture"
    if title.startswith("builtin differential"):
        return "builtin-diff"
    if "RETRY_TORTURE" in title or "POISON_DEAD" in title:
        return "feature-builds"
    return "modes"


def shard_steps(crates, shard):
    """The steps of one shard, in `steps`' order (debug-assertions and
    release-crate-tests: their own, which `steps` does not have)."""
    if shard == "debug-assertions":
        return debug_assertions_steps(crates)
    if shard == "release-crate-tests":
        return release_crate_tests_steps(crates)
    return [s for s in steps(crates) if shard_of(s[0]) in (None, shard)]


def timings_report(done, shard=None):
    """Each step's wall time, slowest first, for the end of the log, and for
    a shard its budget's warning: the next drift shows here before it
    becomes a timeout."""
    lines = ["rt: step times (wall), slowest first:"]
    for title, secs, ok in sorted(done, key=lambda d: -d[1]):
        lines.append(f"  {secs / 60:6.2f} min  {'' if ok else 'FAILED  '}{title}")
    total = sum(d[1] for d in done) / 60
    lines.append(f"  {total:6.2f} min  in all")
    if shard in BUDGETS:
        budget = BUDGETS[shard][0]
        lines.append(f"  {total:6.2f} min of shard {shard}'s {budget}-minute budget ({100 * total / budget:.0f}%)")
        w = budget_warning(shard, total)
        if w:
            lines.append(w)
    return "\n".join(lines)


def metadata():
    p = subprocess.run(["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"],
                       cwd=ROOT, stdout=subprocess.PIPE, text=True, check=True)
    return json.loads(p.stdout)


def main(argv):
    # robigo-rt's R_HOME is its own tree; an inherited one (the CI image's
    # GNU R, /usr/lib/R) it ignores with GNU R's warning on stdout, which
    # every compared output would then carry: none for what this runs
    os.environ.pop("R_HOME", None)
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--list", action="store_true", help="print the rt crates and stop")
    ap.add_argument("--check", action="store_true", help="check the premise and stop")
    ap.add_argument("--full-move", action="store_true",
                    help="only the conformance slice with every collection moving everything (the nightly's run)")
    ap.add_argument("--shard", choices=SHARDS, help="only this CI job's steps (default: every step)")
    a = ap.parse_args(argv)
    if a.full_move and a.shard:
        # (--full-move is the nightly's one step; a shard would be ignored)
        ap.error("--full-move and --shard select different steps: give one")

    meta = metadata()
    crates = rt_crates(meta, rt_patterns(ci_select.load()))
    if not crates:
        print("rt-ci: no workspace member matches the rt rule", file=sys.stderr)
        return 1
    if a.list:
        print("\n".join(crates))
        return 0
    bad = leaks(meta, crates)
    if bad:
        for member, dep in bad:
            print(f"::error::{member} depends on {dep}: an rt-only change could change it, "
                  "so the `rt` rule in scripts/ci-selection.toml no longer holds")
        return 1
    print(f"rt crates: {' '.join(crates)}; no other workspace member depends on them")
    if a.check:
        return 0

    plan = full_move_steps() if a.full_move else shard_steps(crates, a.shard) if a.shard else steps(crates)
    done = []
    try:
        return run_steps(plan, done)
    finally:
        print(timings_report(done, a.shard), flush=True)


def run_steps(plan, done):
    """Run `plan`'s steps in order, recording (title, seconds, ok) in `done`;
    stop at the first that fails."""
    for title, env, cmd in plan:
        report = env.get("ROBIGO_GC_PIN_REPORT")
        if report and os.path.exists(report):
            os.remove(report)
        bc = env.get("ROBIGO_BC_CROSSCHECK_REPORT")
        if bc and os.path.exists(bc):
            os.remove(bc)
        print(f"::group::{title}", flush=True)
        t0 = time.monotonic()
        r = subprocess.run(cmd, cwd=ROOT, env={**os.environ, **env})
        done.append((title, time.monotonic() - t0, r.returncode == 0))
        print("::endgroup::", flush=True)
        if r.returncode != 0:
            print(f"::error::rt: {title} failed")
            return r.returncode
        if report:
            hits = pin_report_hits(report)
            for line in hits:
                print(f"::error::rt: {line}")
            if hits:
                print(f"::error::rt: {title}: {len(hits)} report(s)")
                done[-1] = (title, done[-1][1], False)
                return 1
        if bc:
            errors, classes = bc_report_check(bc)
            for c, n in sorted(classes.items()):
                print(f"rt: bc crosscheck: {n} x {c}")
            for line in errors:
                print(f"::error::rt: {line}")
            if errors:
                # (an error is a closure's line, a malformed line or a
                # report that compared nothing: bc_report_check)
                print(f"::error::rt: {title}: {len(errors)} report problem(s)")
                done[-1] = (title, done[-1][1], False)
                return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
