#!/usr/bin/env python3
"""CI's `rt` job: G3's runtime crates (docs/g3.md §14.1, §16).

    scripts/rt-ci.py            # premise check, release tests, gctorture: every step
    scripts/rt-ci.py --shard NAME  # one CI job's steps (SHARDS); CI runs each in parallel
    scripts/rt-ci.py --shard debug-assertions  # the crates' tests and the conformance with
                                # debug assertions on (a build of their own, in
                                # the target directory + "-debug-assertions";
                                # the plain run above does not include it)
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
robigo-rtcore's `retry-torture` feature made for that step alone. The
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
    full_slow = ",".join(f"{c}={secs}" for c, secs in sorted(FULL_MOVE_SLOW.items()))
    return every_cpu([
        ("conformance under ROBIGO_GC_MOVE=1 with gctorture (every collection moves everything)",
         {"ROBIGO_GCTORTURE": "1", "ROBIGO_GC_MOVE": "1", **SLOW, "ROBIGO_CONF_TIMEOUTS": full_slow},
         conformance_command()),
    ])


# The cargo feature that compiles ROBIGO_RETRY_TORTURE in (robigo-rtcore's
# Cargo.toml): only its step builds with it.
RETRY_TORTURE_FEATURE = "robigo-rtcore/retry-torture"


def steps(crates):
    """(title, extra environment, command) for each step of the job."""
    pkgs = [x for c in crates for x in ("-p", c)]
    test = ["cargo", "test", "--release", "--locked"]
    skip = ["--", "--skip", CONFORMANCE]
    conformance = conformance_command()
    slow = SLOW
    out = [
        ("release tests", {}, [*test, *pkgs, *skip]),
        # robigo-rt's R_HOME tree with tools' native code (A5): the
        # conformance cases that load it (`.requires` r-home-tools) run
        ("the R_HOME tree, tools.so built (scripts/rt-home.sh)", {}, ["scripts/rt-home.sh"]),
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
         {"ROBIGO_NO_INPLACE": "1"}, [*test, *pkgs, *skip]),
        # with a collection due almost always, the check makes any &Cx
        # request of LARGE bytes or more panic where it is made: such a
        # request goes through cx.alloc_rooted (docs/g3.md §3 item 3)
        ("gctorture (collect at every allocation) and the GC stats check",
         {"ROBIGO_GCTORTURE": "1", "ROBIGO_GC_STATS": "check"}, [*test, *pkgs, *skip]),
        # docs/g3.md §15.3 E3: the rt slice under the collector modes
        ("conformance under gctorture (E3)", {"ROBIGO_GCTORTURE": "1", **slow}, conformance),
        # every argument a PROMSXP at call entry (GNU R's representation,
        # through the same binding-kind transitions): docs/g3.md §16's
        # ROBIGO_MATERIALIZE=1
        ("ROBIGO_MATERIALIZE=1 (every argument slot materialized)",
         {"ROBIGO_MATERIALIZE": "1"}, [*test, "-p", "robigo-rt", *skip]),
        # docs/g3.md §16 (H3): the debug moving collector, at every
        # collection gctorture makes due
        ("ROBIGO_GC_MOVE (move every movable object at every collection), with gctorture",
         {"ROBIGO_GCTORTURE": "1", "ROBIGO_GC_MOVE": "1"}, [*test, *pkgs, *skip]),
        # H8: the young objects at each collection, everything at every N-th;
        # the nightly runs it with every collection moving everything
        (f"conformance under ROBIGO_GC_MOVE={GC_MOVE_YOUNG} with gctorture (E3)",
         {"ROBIGO_GCTORTURE": "1", "ROBIGO_GC_MOVE": GC_MOVE_YOUNG, **slow}, conformance),
        # D5's frame recycling (docs/g3.md §16): every frame that would be recycled
        # is poisoned and held back instead, so a frame that escaped without
        # its ESCAPED bit panics where it is used next
        ("ROBIGO_FRAME_QUARANTINE=1 (recycled frames poisoned and held back)",
         {"ROBIGO_FRAME_QUARANTINE": "1"}, [*test, *pkgs, *skip]),
        ("ROBIGO_FRAME_QUARANTINE=1, conformance",
         {"ROBIGO_FRAME_QUARANTINE": "1"},
         conformance),
        # docs/g3.md §16 (H3): the pin report; `main` fails the job on any line
        ("ROBIGO_GC_PIN_REPORT (no weak reference decided while a C word reaches its key)",
         {"ROBIGO_GC_PIN_REPORT": PIN_REPORT}, [*test, *pkgs, *skip]),
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
        # once, so each build's retry runs. The mode is compiled in only
        # with robigo-rtcore's feature, built for this step alone and last:
        # every step above tests the binary users get (cargo keeps both
        # builds). With ROBIGO_GC_MOVE=1 as well it takes about 20 minutes,
        # so that run is by hand (g3.md §16).
        ("conformance under ROBIGO_RETRY_TORTURE=1 (E9: every build's retry runs once)",
         {"ROBIGO_RETRY_TORTURE": "1", **slow},
         [*test, "-p", "robigo-rtapi", "--features", RETRY_TORTURE_FEATURE, "--test", "conformance", "--",
          "--exact", CONFORMANCE, "--nocapture"]),
    ]
    return every_cpu(out)


# CI's three rt jobs (.github/workflows/ci.yml), each one shard of the
# steps, built once and run in parallel: before 2026-11-21 one job ran them
# all in 29-38 minutes, against a 40-minute limit. Each is named for what it
# runs. Measured on main's 2f16dc4c (minutes, its 8.4-minute release build
# included where the shard's first cargo step makes it):
#   modes         release tests, the conformance and its observation modes,
#                 gctorture, quarantine, the pin report, stress, overshoot,
#                 E9's small limit and failures, the byte-code crosscheck:
#                 about 16
#   gc-move       the moving collector, its unit tests and the conformance
#                 under it with gctorture: about 19
#   feature-builds  the steps that build with a feature of their own: the
#                 builtin differential's plants, and retry-torture: about 15
#   debug-assertions  the crates' release tests and the conformance on a
#                 release build with `-C debug-assertions=on` (heap's
#                 debug_assert!s see compiled code; not opt-level 0, where
#                 conditions/eval-depth-limit.R fails only because debug C
#                 frames are larger): about 9 (its own build), and only in
#                 its own job: a plain local run does not build it
# The R_HOME tree (seconds) is made in every shard; every other step is in
# exactly one (scripts/test-rt-ci.py checks both).
SHARDS = ("modes", "gc-move", "feature-builds", "debug-assertions")


def target_dir():
    """cargo's target directory for this checkout (CARGO_TARGET_DIR, the
    configs' build.target-dir, else ROOT/target)."""
    return metadata()["target_directory"]


def debug_assertions_steps(crates):
    """The debug-assertions shard: the R_HOME tree, the crates' release tests
    and the conformance, all on one release build with debug assertions on,
    in a target directory of its own (the normal build stays as it is)."""
    pkgs = [x for c in crates for x in ("-p", c)]
    test = ["cargo", "test", "--release", "--locked"]
    env = {"RUSTFLAGS": "-C debug-assertions=on", "CARGO_TARGET_DIR": target_dir() + "-debug-assertions"}
    return every_cpu([
        ("the R_HOME tree, tools.so built (scripts/rt-home.sh)", {}, ["scripts/rt-home.sh"]),
        ("release tests with debug assertions (-C debug-assertions=on)", env,
         [*test, *pkgs, "--", "--skip", CONFORMANCE]),
        ("conformance with debug assertions (-C debug-assertions=on)", env, conformance_command()),
    ])


def shard_of(title):
    """The shard a step of `steps` runs in, or None for every shard."""
    if title.startswith("the R_HOME tree"):
        return None
    if "GC_MOVE" in title:
        return "gc-move"
    if title.startswith("builtin differential") or "RETRY_TORTURE" in title:
        return "feature-builds"
    return "modes"


def shard_steps(crates, shard):
    """The steps of one shard, in `steps`' order (debug-assertions: its own,
    which `steps` does not have)."""
    if shard == "debug-assertions":
        return debug_assertions_steps(crates)
    return [s for s in steps(crates) if shard_of(s[0]) in (None, shard)]


def timings_report(done):
    """Each step's wall time, slowest first, for the end of the log: the next
    drift shows here before it becomes a timeout."""
    lines = ["rt: step times (wall), slowest first:"]
    for title, secs, ok in sorted(done, key=lambda d: -d[1]):
        lines.append(f"  {secs / 60:6.2f} min  {'' if ok else 'FAILED  '}{title}")
    lines.append(f"  {sum(d[1] for d in done) / 60:6.2f} min  in all")
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
        print(timings_report(done), flush=True)


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
