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
    scripts/rt-ci.py --freeze-check  # the nightly's run: gctorture, the freeze checked at every collection

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
import re
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

_spec = importlib.util.spec_from_file_location("ci_select", os.path.join(ROOT, "scripts", "ci-select.py"))
ci_select = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(ci_select)
sys.path.insert(0, os.path.join(ROOT, "scripts"))
import conf_slots  # noqa: E402


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


# the boot heap's freeze (robigo-heap's freeze.rs), named on in its checked
# steps so that they do not depend on the default
FREEZE_ON = {"ROBIGO_GC_FREEZE": "1"}
# and off: on by default since the fold (step 2, the ledger's
# 2026-11-23-h-freeze-fold.md), so the steps that test the heap without it
# say so, and the moving collector's steps, which leave it off anyway,
# turn it off explicitly
FREEZE_OFF = {"ROBIGO_GC_FREEZE": "0"}


# where the freeze's check writes a failure (appended to by every process):
# a step with a line in it fails, whatever the case's own output (an
# unratcheted case's panic would otherwise pass: Sol r7 S14)
FREEZE_REPORT = os.path.join(tempfile.gettempdir(), f"robigo-rt-ci-freeze-check-{os.getpid()}.txt")


# where the byte-code crosscheck step writes (V3: one line per translated
# closure whose code differs from the from-source compiler's)
BC_REPORT = os.path.join(tempfile.gettempdir(), f"robigo-rt-ci-bc-crosscheck-{os.getpid()}.txt")
# the same under the rdb boot (base's closures from base.rdb, so many more)
BC_REPORT_RDB = os.path.join(tempfile.gettempdir(), f"robigo-rt-ci-bc-crosscheck-rdb-{os.getpid()}.txt")
# The fewest closures each crosscheck report must compare. Under the rdb
# boot base.rdb's closures and, since #942, the compiler namespace's: 97,485
# on main b0c49e869, the same with 24 conformance jobs and with 1, so the
# count is fixed for a given tree. The floor catches lost coverage (a boot
# that stops loading the compiler's namespace compares about half), not
# noise. Counts recorded before the report's records were written whole
# (95,302 on #961, CI's 93,591) were low by merged lines. The plain
# crosscheck compares 191 (the cases' own closures): any comparison at all
# is its floor.
BC_FLOORS = {BC_REPORT_RDB: 96000}

# the kinds of line the crosscheck writes (crates/robigo-rt/src/compile/crosscheck.rs),
# each the line's first word, then ":" (or a class in parentheses, then ":")
BC_KINDS = ("same", "expected", "unexpected", "untranslated", "skipped")
# a record's start straight after other text (no space or quote before it):
# where a second record begins in a merged line ("...; from source Bsame:
# ..."), not a kind's name inside an R string in a deparsed body. Every
# merge also leaves an empty line, which bc_report_check counts: that is the
# complete check; this one only names the suspect lines once it fired
BC_SECOND_RECORD = re.compile(r"[^\s\"'`](?:same|unexpected|untranslated|skipped|expected \(.+?\)): ")


def bc_report_check(path, floor=1):
    """The crosscheck report's errors and its counts, each kind's (and each
    expected class's) and "compared". An error is an unexpected difference,
    an untranslated closure, a line of no kind the crosscheck writes, or
    fewer than `floor` closures compared (BC_FLOORS): every closure compared
    has its line (same, expected or unexpected), so a step that compares
    fewer than it should, or has no report at all, is a broken setup, never
    a pass. And a report that lost or merged records:
    - the conformance's parallel cases append to one file; a record written
      in two parts let another process's land between them, giving one line
      with two records and an empty line. crosscheck.rs now writes each
      record whole under the file's lock, and each empty line is an error
      (once there is one, the lines where a second record starts are named);
    - a record that could not be written leaves `<path>.lost` (crosscheck.rs
      makes it when the write fails), which is an error whatever case lost it:
      a case without a ratchet marker fails without failing the run.
    So neither can lower the count silently."""
    errors = []
    if os.path.exists(path + ".lost"):
        errors.append(f"the crosscheck could not write a record to {path} ({path}.lost): the report is incomplete")
    try:
        with open(path) as f:
            raw = [line.rstrip("\n") for line in f]
    except FileNotFoundError:
        raw = []
    empty = sum(1 for x in raw if not x.strip())
    if empty:
        errors.append(f"{empty} empty line(s) in the crosscheck report {path}: records were merged")
    # (only where empty lines show a merge: a clean record's closure may
    # hold a kind's name, "notsame: ...", which the pattern would match)
    merged = [x for x in raw if empty and x.strip() and not x.startswith("    ") and BC_SECOND_RECORD.search(x)]
    for x in merged[:5]:
        errors.append(f"two records in one line: {x[:300]}")
    if len(merged) > 5:
        errors.append(f"... and {len(merged) - 5} more line(s) holding two records")
    lines = [x for x in raw if x.strip()]
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
    if compared < floor:
        errors.append(f"compared {compared} closures, fewer than the {floor} the crosscheck report {path} "
                      f"must (BC_FLOORS)")
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
    # The three below, alone in this step (2026-11-21): each passed, and each
    # timed out at 600 s in it for five delegates at load 45-130. B measured
    # them on the workstation, A1 too (at load 18-24):
    #   finalizers-self-ref-envs.R  346 s (B), 267 s (A1)
    #   active-binding-read.R       302 s (B)
    #   serialize-write.R           503 s (B), 185 s (A1); it loads the compiler
    "tests/r/memory/finalizers-self-ref-envs.R": 1200,
    "tests/rt/jit/active-binding-read.R": 1200,
    "tests/rt/jit/serialize-write.R": 1200,
    # (tests/rt/jit/lazy-promise-code.R, 347 s here, is `volume` since #966:
    # its small twin runs in this step instead, in about a second)
}

# The cases the young:16 step also starts first, at its 600 s: under 300 s
# alone there (A1, 2026-11-21, load 22-48), so they need no allowance, but
# started in id order they ran alone at the step's end (#963's first run,
# loaded: 416-453 s)
YOUNG_MOVE_FIRST = {
    "tests/rt/capi/closure-mutation.R": 600,     # 177 s alone
    "tests/rt/bytecode/jit.R": 600,              # 111 s alone
    "tests/r/lang/tailcall-frames.R": 600,       # 152 s alone
}


# The cases the two gctorture conformance steps start first, at the step's
# own limit (600 s): each takes minutes under gctorture, and started in id
# order it ran alone at the step's end. (tests/rt/jit/shared-body.R, 177 s
# under gctorture and 200 s under GC_MOVE=young:16, is `volume` since #966:
# its small twin runs in these steps instead.) In #952's run 44927530340 the two steps
# took 2.7 and 3.1 minutes longer than in #941's, its 8 new cases' doing.
# The longest cases alone under the plain gctorture step (A1, 2026-11-21,
# load 20-34): serialize-write.R 256 s, active-binding-read.R 181 s,
# finalizers-self-ref-envs.R 139 s, shared-body.R 76 s (177 s at load 45),
# c-stack-overflow.R 72 s; under retry torture none is over 300 s
# (serialize-write.R 269 s, active-binding-read.R 169 s, shared-body.R 161 s),
# so neither step gives a case longer than its limit
GCTORTURE_FIRST = {
    "tests/r/conditions/c-stack-overflow.R": 600,
    "tests/rt/jit/serialize-write.R": 600,
    "tests/rt/jit/active-binding-read.R": 600,
    "tests/r/memory/finalizers-self-ref-envs.R": 600,
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


def db_tree(binary, env=None):
    """The step that writes base's database tree with `binary`, the release
    robigo-rt its shard built first (rt_binary; scripts/rt-home-db.sh is
    given it by name)."""
    return ("base's database tree, written by this robigo-rt (scripts/rt-home-db.sh)", env or {},
            ["scripts/rt-home-db.sh", "--robigo-rt", binary])


def conformance_command():
    test = ["cargo", "test", "--release", "--locked"]
    return [*test, "-p", "robigo-rtapi", "--test", "conformance", "--", "--exact", CONFORMANCE, "--nocapture"]


# (a case under a collector mode runs many times longer than the runner's
# default 20 s allow)
SLOW = {"ROBIGO_CONF_TIMEOUT": "600"}

# The torture modes' conformance steps torture each case, not its start:
# the runtime holds them until the start is over (robigo-heap's
# ROBIGO_TORTURE_AFTER_START, a test hook; GNU R itself tortures the start,
# init_gctorture in InitMemory). The start keeps its own steps, one per boot
# (START_TORTURE), under the moving collector and gctorture from the start.
AFTER_START = {"ROBIGO_TORTURE_AFTER_START": "1"}

# The start itself, tortured: under the moving collector and gctorture from
# the heap's creation (no ROBIGO_TORTURE_AFTER_START), each boot runs this,
# whose start is the point (stage 0 or the rdb loader and its lazy-load
# fetches, the system profile, the compiler's load when the JIT is on): an
# unrooted object anywhere in it is moved under its holder and read as
# poison. The JIT is on (R_ENABLE_JIT=3, set, not the default's), so the
# compiler's namespace loads in the start; the script then fetches a few of
# base's lazy-loaded bindings and runs a loop the JIT compiles. (A package
# load, library(stats), waits for the tree to have one.)
START_SCRIPT = ('invisible(lapply(c("paste", "format", "sprintf", "regmatches", "Reduce", "tryCatch"), '
                'function(f) get(f, baseenv()))); '
                'f <- function(n) { s <- 0; for (i in seq_len(n)) s <- s + i; s }; '
                'stopifnot(f(10) == 55, f(20) == 210); cat("started\\n")')
# (GNU R's boot, rdb's, loads the compiler's namespace in the start when the
# JIT is on, main.c R_init_jit_enabled; M1's loader sets the JIT up without it)
START_COMPILER = 'stopifnot("compiler" %in% loadedNamespaces()); '


def start_torture_step(boot):
    """The step that runs START_SCRIPT under `boot` (ROBIGO_RT_BOOT's value,
    "" for M1's loader) with the start tortured."""
    name = {"": "M1's loader", "rdb": "the rdb boot"}[boot]
    return (f"the start tortured: {name} under ROBIGO_GC_MOVE=1 and gctorture from the heap's creation",
            # (set outright: an inherited hold or R_GCTORTURE=0 would take
            # the start's torture away)
            {"ROBIGO_RT_BOOT": boot, "R_GCTORTURE": "1", "R_GCTORTURE_WAIT": "1", "ROBIGO_GCTORTURE": "1",
             "ROBIGO_GC_MOVE": "1", **FREEZE_OFF, "ROBIGO_TORTURE_AFTER_START": "0", "R_ENABLE_JIT": "3"},
            [rt_binary(), "--vanilla", "-e", (START_COMPILER if boot == "rdb" else "") + START_SCRIPT])


# TEMPORARY (2026-11-23): two cases given 60 s in the conformance steps
# that run at the runner's default 20 s, because each failed local gates
# three times tonight on a box at load 46-66 (24 CPUs):
#   tests/rt/env/large-frames.R       6.7-7.3 s alone under the rdb boot (E),
#                                     20.0 s at load ~26; 5.6-6.3 s on CI
#   tests/rt/stress/finalizer-depths.R 12.1-15.6 s on a loaded box, 25 s once
# To be re-decided from the first quiet measurement with the machine-wide
# slot pool (ROBIGO_CONF_SLOTS_DIR), which starts a case's clock when it
# takes its slot: dropped then if both sit well under 10 s.
DEFAULT_STEP_SLOW = {
    "tests/rt/env/large-frames.R": 60,
    "tests/rt/stress/finalizer-depths.R": 60,
}


def every_cpu(steps):
    """The steps, with the conformance runner given a case per CPU in each
    conformance step: those run one test alone (`--exact`), so the cases may
    use every CPU (the runner's default, 2, is for a cargo test that shares
    the machine with other test binaries). The runner starts the cases named
    in ROBIGO_CONF_TIMEOUTS first. And each conformance step at the default
    timeout (no ROBIGO_CONF_TIMEOUT) gets DEFAULT_STEP_SLOW's allowances."""
    jobs = str(os.cpu_count() or 2)
    out = []
    for t, e, c in steps:
        if CONFORMANCE in c and "--exact" in c:
            e = {**e, "ROBIGO_CONF_JOBS": jobs}
            # (the effective environment: the step's over the caller's, as
            # run_steps runs it. A global timeout, the step's or the
            # caller's, means no default-timeout allowances; the case lists
            # merge, the defaults first, then the caller's, then the step's)
            # (present, even empty, is set: the runner reads it so)
            if "ROBIGO_CONF_TIMEOUT" not in e and "ROBIGO_CONF_TIMEOUT" not in os.environ:
                def named(v):
                    out = {}
                    for i in v.split(","):
                        if not i.strip():
                            continue
                        k, _, s = i.rpartition("=")
                        if not k.strip() or not s.strip().isdigit():
                            # (the runner's own message for it)
                            raise SystemExit(f"rt-ci: ROBIGO_CONF_TIMEOUTS: {i!r} is not CASE=SECS")
                        if k.strip() in out:
                            # (the runner refuses one list naming a case twice)
                            raise SystemExit(f"rt-ci: ROBIGO_CONF_TIMEOUTS names {k.strip()!r} twice")
                        out[k.strip()] = int(s)
                    return out
                e["ROBIGO_CONF_TIMEOUTS"] = case_timeouts({**DEFAULT_STEP_SLOW,
                                                           **named(os.environ.get("ROBIGO_CONF_TIMEOUTS", "")),
                                                           **named(e.get("ROBIGO_CONF_TIMEOUTS", ""))})
        out.append((t, e, c))
    return out


def full_move_steps():
    """The nightly's full run (`--full-move`): the conformance slice under
    gctorture, every collection moving everything it may."""
    full_slow = case_timeouts(FULL_MOVE_SLOW)
    return every_cpu([
        ("conformance under ROBIGO_GC_MOVE=1 with gctorture (every collection moves everything)",
         {"ROBIGO_GCTORTURE": "1", "ROBIGO_GC_MOVE": "1", **FREEZE_OFF, **AFTER_START, **SLOW,
          "ROBIGO_CONF_TIMEOUTS": full_slow},
         conformance_command()),
    ])


# The cases that need longer than the slow modes' 600 s in the nightly's
# every-collection freeze check (`--freeze-check`), and how long they get:
# there every allocation traces the heap three times (the collection, the
# check's independent trace, its comparison). Measured under gctorture at
# ROBIGO_GC_FREEZE_CHECK=1 on the 24-CPU workstation, the seven at once
# (load 35-40, 2026-11-21; jit.R alone: 536 s), each given about twice that:
FREEZE_CHECK_SLOW = {
    "tests/r/jit/loop-closed-form-super.R": 1500,  # 719 s
    "tests/rt/bytecode/jit.R": 2400,  # 1,150 s
    "tests/rt/capi/closure-mutation.R": 2400,  # 1,160 s
    "tests/r/conditions/c-stack-overflow.R": 2800,  # 1,387 s
    "tests/rt/jit/active-binding-read.R": 2900,  # 1,431 s
    "tests/rt/jit/serialize-write.R": 3600,  # 1,795 s
    "tests/r/memory/finalizers-self-ref-envs.R": 4400,  # 2,180 s
}


def freeze_check_steps():
    """The nightly's every-collection freeze check (`--freeze-check`): the
    conformance slice under gctorture with the freeze's invariant checked at
    every collection. CI's E3 step checks every 16th (the ledger's amendment
    to #948 (c), docs/ledger/g3/2026-11-21-h-boot-freeze-check.md); this run
    keeps the every-collection check, moved, not lost."""
    return every_cpu([
        # (the tools cases need the tree: without it they skip, Sol r7 S13)
        ("the R_HOME tree, tools.so built (scripts/rt-home.sh)", {}, ["scripts/rt-home.sh"]),
        ("conformance under gctorture, the freeze checked at every collection",
         {"ROBIGO_GCTORTURE": "1", **FREEZE_ON, "ROBIGO_GC_FREEZE_CHECK": "1", "ROBIGO_GC_FREEZE_CHECK_REPORT": FREEZE_REPORT,
          **AFTER_START, **SLOW, "ROBIGO_CONF_TIMEOUTS": case_timeouts(FREEZE_CHECK_SLOW)},
         conformance_command()),
    ])


# The cargo features that compile ROBIGO_RETRY_TORTURE and ROBIGO_POISON_DEAD
# in (robigo-rtcore's and robigo-rt's Cargo.toml): only the last steps build
# with them, all with both, so that they share one build.
CHECK_FEATURES = "robigo-rtcore/retry-torture,robigo-rt/poison-dead"


def rt_binary(target=None):
    """The release robigo-rt the release tests built in `target` (default:
    cargo's target directory, as `metadata()` gives it), which
    scripts/rt-home-db.sh is given by name. The one place that names the
    binary's path in a target."""
    return os.path.join(target or metadata()["target_directory"], "release", "robigo-rt")


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
        db_tree(rt_binary()),
        start_torture_step(""),
        start_torture_step("rdb"),
        ("the crates' tests (quick profile)", TREE_RUNS, [*crate, *pkgs, *skip]),
        ("boot from base's database (ROBIGO_RT_BOOT=rdb)", {"ROBIGO_RT_HOME_DB": "1"},
         [*test, "-p", "robigo-rtapi", "--test", "boot_db", *skip]),
        # the boot that replaces M1's (L5, ROBIGO_RT_BOOT=rdb: base from the
        # tree's base.rdb, as GNU R boots): the ratcheted conformance under it,
        # plain and with the byte-code crosscheck, each failing as its M1
        # step does (a ratcheted case that fails; an unexpected or
        # untranslated closure). Not yet the default anywhere (L's list)
        # The tree has its translation cache (C1, crates/robigo-rt/src/tcache.rs,
        # beside it: rt-home-db.sh), so these steps run with it; the crosscheck
        # step compares every cached translation with a fresh one too. The
        # same cases with the cache off must pass as well: the cache changes
        # nothing a case sees
        ("conformance under the rdb boot (ROBIGO_RT_BOOT=rdb)", {"ROBIGO_RT_BOOT": "rdb"}, conformance),
        ("conformance under the rdb boot without the translation cache (ROBIGO_TRANSLATION_CACHE=0)",
         {"ROBIGO_RT_BOOT": "rdb", "ROBIGO_TRANSLATION_CACHE": "0"}, conformance),
        # the cache as damaged input: a corrupted file's entries are misses,
        # every start ends as a start does (no signal, no panic, no hang); and
        # a database changed under its stamp (two records swapped) makes its
        # entries stale-record misses, never other code
        ("the translation cache as damaged input (scripts/rt-tcache-mutate.py)", {},
         ["python3", "scripts/rt-tcache-mutate.py", "--robigo-rt", rt_binary(), "--pkg", "base", "--pkg", "compiler",
          "--runs", "300", "--seed", "1", "--swap-runs", "40"]),
        ("conformance under the rdb boot with ROBIGO_BC_CROSSCHECK=1 (base.rdb's closures against the "
         "from-source compiler's)",
         {"ROBIGO_RT_BOOT": "rdb", "ROBIGO_BC_CROSSCHECK": "1", "ROBIGO_BC_CROSSCHECK_REPORT": BC_REPORT_RDB},
         conformance),
        ("conformance (tests/rt, ratcheted tests/r; GNU R 4.6.2)", {}, conformance),
        # the boot heap's freeze is on by default (the steps above run with
        # it, its check off); the heap without it, under each boot
        ("conformance with the boot heap's freeze off (ROBIGO_GC_FREEZE=0)", FREEZE_OFF, conformance),
        ("conformance under the rdb boot with the boot heap's freeze off (ROBIGO_RT_BOOT=rdb, ROBIGO_GC_FREEZE=0)",
         {"ROBIGO_RT_BOOT": "rdb", **FREEZE_OFF}, conformance),
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
        # the boot heap's freeze (robigo-heap's freeze.rs): its invariant
        # checked at every collection here, where every allocation collects
        # (the move modes below leave the freeze off)
        ("gctorture (collect at every allocation) and the GC stats check",
         {**TREE_RUNS, **{"ROBIGO_GCTORTURE": "1", "ROBIGO_GC_STATS": "check", **FREEZE_ON, "ROBIGO_GC_FREEZE_CHECK": "1"}},
         [*crate, *pkgs, *skip]),
        # docs/g3.md §15.3 E3: the rt slice under the collector modes. The
        # freeze's check at every 16th collection: at every one it traces
        # the heap twice per allocation (tests/rt/bytecode/jit.R: 536 s
        # against 97 s without it, past the case timeout; about 1.3x at 16)
        ("conformance under gctorture (E3)",
         {"ROBIGO_GCTORTURE": "1", **FREEZE_ON, "ROBIGO_GC_FREEZE_CHECK": "16", "ROBIGO_GC_FREEZE_CHECK_REPORT": FREEZE_REPORT,
          **AFTER_START, **slow, "ROBIGO_CONF_TIMEOUTS": case_timeouts(GCTORTURE_FIRST)}, conformance),
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
         {**TREE_RUNS, **{"ROBIGO_GCTORTURE": "1", "ROBIGO_GC_MOVE": "1", **FREEZE_OFF}}, [*crate, *pkgs, *skip]),
        # H8: the young objects at each collection, everything at every N-th;
        # the nightly runs it with every collection moving everything
        (f"conformance under ROBIGO_GC_MOVE={GC_MOVE_YOUNG} with gctorture (E3)",
         {"ROBIGO_GCTORTURE": "1", "ROBIGO_GC_MOVE": GC_MOVE_YOUNG, **FREEZE_OFF, **AFTER_START, **slow,
          "ROBIGO_CONF_TIMEOUTS": case_timeouts({**GCTORTURE_FIRST, **YOUNG_MOVE_FIRST, **YOUNG_MOVE_SLOW})}, conformance),
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
         {"ROBIGO_RETRY_TORTURE": "1", **AFTER_START, **slow},
         [*test, "-p", "robigo-rtapi", "--features", CHECK_FEATURES, "--test", "conformance", "--",
          "--exact", CONFORMANCE, "--nocapture"]),
        # C1 (#979, the bot's round 2): a translation-cache verification whose
        # fresh translation stops for a retry compares nothing. Under
        # ROBIGO_RETRY_TORTURE every fresh translation of a verification
        # stops once inside it, so the case is certain, not lucky: base's
        # database tree written by this build (whose binary the step above
        # made), then the boot test that verifies a start under the knob
        ("base's database tree for ROBIGO_RETRY_TORTURE's build (scripts/rt-home-db.sh)", {},
         ["scripts/rt-home-db.sh", "--robigo-rt", rt_binary()]),
        # (TREE_RUNS: a missing or stale tree fails, not skips; and the test
        # fails, not skips, if the build has no retry-torture)
        ("a translation-cache verification under ROBIGO_RETRY_TORTURE=1 compares nothing for a retry "
         "(tests/boot_db.rs)", {**TREE_RUNS, "ROBIGO_REQUIRE_RETRY_TORTURE": "1"},
         [*test, "-p", "robigo-rtapi", "--features", CHECK_FEATURES, "--test", "boot_db", "--",
          "--exact", "a_verification_stopped_for_a_retry_compares_nothing"]),
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
# 20%, rounded up. Its job's timeout-minutes is twice the budget, but no
# less than TIMEOUT_FLOOR (timeout_of), and the end-of-log table warns at
# WARN_AT of the budget, so a drift shows long before the
# timeout (scripts/test-rt-ci.py pins the three relations). When a warning
# fires, rebalance or remeasure: the dates say how old each measurement is.
BUDGETS = {  # shard: (budget, measured minutes, where measured)
    # each the slowest of main's three runs of 2026-11-23 (45104024474,
    # 45106626186, 45108072544), before #979; #979's steps added at C1's
    # slow-runner estimates (its two modes steps timed locally, C1,
    # 2026-11-23, scaled by 1.6: run 45072863916's conformance steps against
    # the same local run's, 1.34 against 0.84 min)
    "modes": (13, 10.57, "2026-11-23, run 45106626186: 9.67 (7.32 and 7.45 in the other two), and #979's rdb "
              "boot's conformance without the translation cache, 0.29 (0.18 locally), and the cache's "
              "damaged-input step, 0.61 (0.38 locally)"),
    "gctorture": (26, 21.53, "2026-11-23, run 45104024474, a slow runner (15.24 and 15.82 in the other two)"),
    "feature-builds": (17, 13.90, "2026-11-23, run 45104024474: 13.30 (11.01 and 12.66 in the other two), and "
                       "#979's tree for the retry-torture build and its verification test, estimated 0.6 (0.3 "
                       "and 0.05 minutes locally)"),
    "builtin-diff": (12, 9.34, "2026-11-23, run 45106626186 (8.01 and 7.49 in the other two)"),
    "debug-assertions": (5, 3.85, "2026-11-23, run 45108072544 (2.58 and 2.94 in the other two)"),
    "release-crate-tests": (17, 14.03, "2026-11-23, run 45104024474 (10.26 and 10.19 in the other two)"),
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
        db_tree(rt_binary(env["CARGO_TARGET_DIR"]), env),
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
        db_tree(rt_binary()),
        ("the crates' tests (release profile)", TREE_RUNS, [*test, "--", "--skip", CONFORMANCE]),
    ])


def shard_of(title):
    """The shard a step of `steps` runs in, or None for every shard."""
    if title.startswith("the R_HOME tree"):
        return None
    if title.startswith("conformance under gctorture") or title.startswith("conformance under ROBIGO_GC_MOVE"):
        return "gctorture"
    # the boot heap's freeze off under M1's loader: with E3's freeze check,
    # in the freeze's shard (modes was at its budget: the foreman,
    # 2026-11-22). Its rdb twin stays in modes, where base's database tree
    # is built
    if title == "conformance with the boot heap's freeze off (ROBIGO_GC_FREEZE=0)":
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
        return builds_first(debug_assertions_steps(crates))
    if shard == "release-crate-tests":
        return builds_first(release_crate_tests_steps(crates))
    return builds_first([s for s in steps(crates) if shard_of(s[0]) in (None, shard)])


# the environment that decides what cargo builds (the modes' variables are
# read by the tests at run time, not by the build)
BUILD_ENV = ("RUSTFLAGS", "CARGO_TARGET_DIR")


def builds_first(plan):
    """`plan` with each crate-test build in a step of its own (`--no-run`,
    the same command and build environment) before the first step that runs
    it: the build of the test binaries is then its own line in the times
    (#989's gate: debug-assertions' crate tests took 3.0 min, nearly all of
    it that build under load). The run's line still holds the doctests'
    compilation and, in a local run, its wait for a slot. A build the plan
    already has before the run (release-crate-tests') is not made twice,
    and one it has only after the run moves ahead of it."""
    def key(env, cmd):
        return (tuple(cmd[:cmd.index("--")] if "--" in cmd else [x for x in cmd if x != "--no-run"]),
                tuple((k, env.get(k)) for k in BUILD_ENV))
    def is_build(c):
        return c[:2] == ["cargo", "test"] and "--no-run" in c
    later = {}
    for i, (_t, e, c) in enumerate(plan):
        if is_build(c):
            later.setdefault(key(e, c), i)
    built, moved, out = set(), set(), []
    for i, (t, e, c) in enumerate(plan):
        if i in moved:
            continue
        if is_build(c):
            built.add(key(e, c))
        elif crate_test_threads(c) and key(e, c) not in built:
            k = key(e, c)
            built.add(k)
            if k in later:
                moved.add(later[k])
                out.append(plan[later[k]])
            else:
                out.append((f"{t}: its tests built (--no-run)", {x: e[x] for x in BUILD_ENV if x in e},
                            [*c[:c.index("--")], "--no-run"]))
        out.append((t, e, c))
    return out


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


def conf_slots_dir(env=None):
    """The machine-wide pool of conformance case slots a local run shares
    with every other local rt-ci run of this user (robigo-conformance's
    ROBIGO_CONF_SLOTS_DIR: a case takes a slot before it starts, its time
    limit counting from its start). The rule is conf_slots.default_dir's:
    the caller's own ROBIGO_CONF_SLOTS_DIR wins (empty: no pool), CI has
    none. None: leave it as it is."""
    return conf_slots.default_dir(env)


# A crate-test step runs at least this many test threads, however few slots
# were free: up to 3 threads over its slots per run cost 24 CPUs'
# conformance cases nothing measurable, and the gctorture modes' long tests
# are not left to one thread. Measured 2026-11-23, built beforehand,
# alternating: ROBIGO_GC_MOVE's crate step (the longest gctorture one) took
# 1.72/1.86 min at one thread and 1.57/1.21 at four (load 16-34); the
# debug-assertions step 0.53/0.71 and 0.56/0.59 (its 3.0 min in #989's gate
# was its build under load, now builds_first's own step). It never waits
# for more slots: a step that waits on several at once is the shape that
# starves under load.
CRATE_TEST_MIN_THREADS = 4


def crate_test_threads(cmd):
    """Whether `cmd` is a run of the crates' own tests (a cargo test that
    runs tests, other than a conformance step), which take their threads
    from the slot pool: libtest runs one thread per CPU, and two local runs'
    crate-test steps at once, or one beside another run's conformance cases,
    stacked to twice the box."""
    return (cmd[:2] == ["cargo", "test"] and "--" in cmd and "--no-run" not in cmd
            and not (CONFORMANCE in cmd and "--exact" in cmd))


def metadata():
    p = subprocess.run(["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"],
                       cwd=ROOT, stdout=subprocess.PIPE, text=True, check=True)
    return json.loads(p.stdout)


def main(argv):
    # robigo-rt's R_HOME is its own tree; an inherited one (the CI image's
    # GNU R, /usr/lib/R) it ignores with GNU R's warning on stdout, which
    # every compared output would then carry: none for what this runs
    os.environ.pop("R_HOME", None)
    slots = conf_slots_dir()
    if slots is not None:
        os.environ["ROBIGO_CONF_SLOTS_DIR"] = slots
    elif os.environ.get("ROBIGO_CONF_SLOTS_DIR"):
        # the caller's, absolute: the runner resolves it from its package's
        # directory, this from the caller's
        os.environ["ROBIGO_CONF_SLOTS_DIR"] = conf_slots.absolute(os.environ["ROBIGO_CONF_SLOTS_DIR"])
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--list", action="store_true", help="print the rt crates and stop")
    ap.add_argument("--check", action="store_true", help="check the premise and stop")
    ap.add_argument("--full-move", action="store_true",
                    help="only the conformance slice with every collection moving everything (the nightly's run)")
    ap.add_argument("--freeze-check", action="store_true",
                    help="only the conformance slice under gctorture with the freeze checked at every collection "
                         "(the nightly's run)")
    ap.add_argument("--shard", choices=SHARDS, help="only this CI job's steps (default: every step)")
    a = ap.parse_args(argv)
    if a.full_move and a.shard:
        # (--full-move is the nightly's one step; a shard would be ignored)
        ap.error("--full-move and --shard select different steps: give one")
    if a.freeze_check and (a.shard or a.full_move):
        ap.error("--freeze-check, --full-move and --shard select different steps: give one")

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

    raw = os.environ.get("RT_CI_MIN_FREE_GB", str(MIN_FREE_GB))
    try:
        min_gb = float(raw)
    except ValueError:
        print(f"::error::rt-ci: RT_CI_MIN_FREE_GB={raw!r} is not a number of GB (0 turns the check off)")
        return 2
    if min_gb > 0:
        problem = free_space_problem([meta["target_directory"], tempfile.gettempdir()], min_gb)
        if problem:
            # (workflow commands on stdout: Actions annotates only those)
            print(f"::error::{problem}")
            return 2
    plan = (full_move_steps() if a.full_move else freeze_check_steps() if a.freeze_check else shard_steps(crates, a.shard) if a.shard else builds_first(steps(crates)))
    done = []
    try:
        return run_steps(plan, done)
    finally:
        print(timings_report(done, a.shard), flush=True)


# Every conformance step writes each case's wall time (the runner's
# ROBIGO_CONF_TIMES_REPORT) into this directory, one file per step, and its
# end lists the cases over SLOW_FRACTION of their limit: report-only. The
# rule (docs/ledger/g3/2026-11-21-a1-rt-shards.md): a case over half its
# step's limit alone is named in that step with an allowance and its
# measured time, or is made smaller. RT_CI_TIMES_DIR keeps the files where
# CI can upload them, and they balance the shards.
TIMES_DIR = os.environ.get("RT_CI_TIMES_DIR") or os.path.join(tempfile.gettempdir(), f"robigo-rt-ci-times-{os.getpid()}")
SLOW_FRACTION = 0.5


def times_file(index, title):
    """The times report of a plan's `index`-th step, titled `title`: the index
    keeps two steps whose titles differ only in punctuation apart."""
    slug = re.sub(r"[^A-Za-z0-9]+", "-", title).strip("-")[:80]
    return os.path.join(TIMES_DIR, f"{index:02d}-{slug}.tsv")


def slow_cases(path, fraction=SLOW_FRACTION):
    """A times report's cases over `fraction` of their limit, slowest first:
    (secs, limit, status, case)."""
    try:
        with open(path) as f:
            lines = [line.rstrip("\n").split("\t") for line in f if line.strip() and not line.startswith("#")]
    except FileNotFoundError:
        return []
    rows = [(float(s), int(lim), status, case) for s, lim, status, case in lines]
    return [r for r in rows if r[0] > fraction * r[1]]


def slow_cases_report(title, path):
    """The end of a conformance step's log: its cases over half their limit."""
    rows = slow_cases(path)
    if not rows:
        return f"rt: case times: {title}: no case over {SLOW_FRACTION:.0%} of its limit ({path})"
    lines = [f"rt: case times: {title}: {len(rows)} case(s) over {SLOW_FRACTION:.0%} of their limit "
             f"(name each with an allowance and its time, or make it smaller; {path}):"]
    lines += [f"  {s:7.1f} s of {lim:4d} s  {status}  {case}" for s, lim, status, case in rows]
    return "\n".join(lines)


# rt-ci stops at its start when cargo's target directory or TMPDIR has less
# free space than this (GB): a release link of the test binaries needs about
# 5 GB, and a full disk otherwise fails a run mid-link, where it reads as a
# code failure (2026-11-21, the shared workstation's /). RT_CI_MIN_FREE_GB
# overrides it (0: no check)
MIN_FREE_GB = 8


def existing_ancestor(path):
    """`path`, or its nearest ancestor that exists (cargo's target directory
    does not exist on a fresh checkout until the first build), or None."""
    p = os.path.abspath(path)
    while not os.path.exists(p):
        parent = os.path.dirname(p)
        if parent == p:
            return None
        p = parent
    return p


def free_space_problem(paths, min_gb):
    """The message for the first of `paths` with under `min_gb` GB free, or
    None. Each path's volume is measured at its nearest existing ancestor;
    one that cannot be measured (no ancestor exists, or the measurement
    fails) is reported on stderr and skipped: this check gives a clear
    message, and must never be what fails a run."""
    import shutil
    for p in paths:
        at = existing_ancestor(p)
        try:
            if at is None:
                raise FileNotFoundError(p)
            free = shutil.disk_usage(at).free / 1e9
        except OSError as e:
            print(f"::warning::rt-ci: cannot tell the free space for {p} ({e}): not checked")
            continue
        if free < min_gb:
            return (f"rt-ci: {free:.1f} GB free on the volume of {p} (< {min_gb} GB): a release link of the "
                    f"test binaries needs about 5 GB; free space first, or set RT_CI_MIN_FREE_GB "
                    f"(docs/verification.md; killed tools' leftovers in /tmp: its rule under \"A tool's "
                    f"temporary directories\")")
    return None


def run_steps(plan, done):
    """Run `plan`'s steps in order, recording (title, seconds, ok) in `done`;
    stop at the first that fails."""
    for index, (title, env, cmd) in enumerate(plan):
        report = env.get("ROBIGO_GC_PIN_REPORT")
        if report and os.path.exists(report):
            os.remove(report)
        bc = env.get("ROBIGO_BC_CROSSCHECK_REPORT")
        for stale in (bc, bc and bc + ".lost"):
            if stale and os.path.exists(stale):
                os.remove(stale)
        freeze = env.get("ROBIGO_GC_FREEZE_CHECK_REPORT")
        if freeze and os.path.exists(freeze):
            os.remove(freeze)
        times = None
        if CONFORMANCE in cmd and "--exact" in cmd:
            times = times_file(index, title)
            os.makedirs(TIMES_DIR, exist_ok=True)
            if os.path.exists(times):
                os.remove(times)
            env = {**env, "ROBIGO_CONF_TIMES_REPORT": times}
        print(f"::group::{title}", flush=True)
        t0 = time.monotonic()
        # as many threads as slots, CRATE_TEST_MIN_THREADS at least: one it
        # waits for, then whatever else of the pool is free (all of it on a
        # quiet box), held until the step ends. The tests get no pool of their own: the step's slots
        # count their threads already, and a test waiting for a slot its
        # own step holds would wait forever
        try:
            pool = conf_slots.from_env({**os.environ, **env}) if crate_test_threads(cmd) else None
            held = pool.take(pool.n, who=f"rt-ci's step '{title}'") if pool else None
        except conf_slots.PoolError as e:
            print(f"::error::rt: {title}: {e}")
            print("::endgroup::", flush=True)
            done.append((title, 0.0, False))
            return 2
        if held:
            with held:
                threads = max(held.count, CRATE_TEST_MIN_THREADS)
                print(f"rt-ci: {held.count} of {pool.n} slots, --test-threads={threads}", flush=True)
                r = subprocess.run([*cmd, f"--test-threads={threads}"], cwd=ROOT,
                                   env={**os.environ, **env, "ROBIGO_CONF_SLOTS_DIR": ""})
        else:
            r = subprocess.run(cmd, cwd=ROOT, env={**os.environ, **env})
        done.append((title, time.monotonic() - t0, r.returncode == 0))
        print("::endgroup::", flush=True)
        if times:
            # (also when the step failed: a timeout is when the times matter)
            print(slow_cases_report(title, times), flush=True)
        if r.returncode != 0:
            print(f"::error::rt: {title} failed")
            return r.returncode
        if freeze and os.path.exists(freeze):
            with open(freeze) as f:
                lines = [l.rstrip("\n") for l in f if l.strip()]
            for line in lines:
                print(f"::error::rt: {line}")
            if lines:
                print(f"::error::rt: {title}: the freeze's check failed {len(lines)} time(s)")
                done[-1] = (title, done[-1][1], False)
                return 1
        if report:
            hits = pin_report_hits(report)
            for line in hits:
                print(f"::error::rt: {line}")
            if hits:
                print(f"::error::rt: {title}: {len(hits)} report(s)")
                done[-1] = (title, done[-1][1], False)
                return 1
        if bc:
            errors, classes = bc_report_check(bc, BC_FLOORS.get(bc, 1))
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
