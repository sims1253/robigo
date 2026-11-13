#!/usr/bin/env python3
"""CI's `rt` job: G3's runtime crates (docs/g3.md §14.1, §16).

    scripts/rt-ci.py            # premise check, release tests, gctorture
    scripts/rt-ci.py --list     # the rt crates, one per line
    scripts/rt-ci.py --check    # the premise check alone

The rt crates are the workspace members under the `rt` rule of
scripts/ci-selection.toml, so the selection and this job cannot drift apart.

The premise behind that rule's skipped heavy jobs is that the robigo binary
cannot change when only rt crates do. It holds while no other workspace
member depends on an rt crate in any way (normal, build or dev: the skipped
`test` job would miss a dev-dependency too). This script fails first if one
does. At the flip (M7) the CLI depends on robigo-rt, this check fails, and
the `rt` rule has to go.

Then `cargo test --release` on the rt crates; robigo-rt's conformance
(`tests/rt` and the `tests/r` cases it has ratcheted, against GNU R 4.6.2,
crates/robigo-rt/tests/conformance.rs) with its report in the log; and the
crates' tests again under `ROBIGO_GCTORTURE=1` (a collection at every
allocation) with `ROBIGO_GC_STATS=check` (a large `&Cx` request while a
collection is due panics); under `ROBIGO_GC_MOVE=1` with gctorture (every
collection moves what it may and poisons the old copies: a raw `Sexp` kept
across a safepoint outside the roots reads poison or faults); and with
`ROBIGO_GC_PIN_REPORT` (a weak reference decided dead while a C word reaches
its key: the rt slice has no package code, so any report fails the job).
The verification modes of g3.md §16 add their steps here as they land;
conformance under them is one (T1's), not yet taken.
"""
import argparse
import importlib.util
import json
import os
import subprocess
import sys
import tempfile

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


# robigo-rt's conformance test (crates/robigo-rt/tests/conformance.rs): its
# own step, so that its report reaches the log; the others skip it.
CONFORMANCE = "rt_conformance"


# where the pin report step writes (appended to by every process)
PIN_REPORT = os.path.join(tempfile.gettempdir(), f"robigo-rt-ci-pin-report-{os.getpid()}.txt")


def pin_report_hits(path):
    """The pin report's lines (none if the file is missing)."""
    try:
        with open(path) as f:
            return [line.rstrip("\n") for line in f if line.strip()]
    except FileNotFoundError:
        return []


def steps(crates):
    """(title, extra environment, command) for each step of the job."""
    pkgs = [x for c in crates for x in ("-p", c)]
    test = ["cargo", "test", "--release", "--locked"]
    skip = ["--", "--skip", CONFORMANCE]
    return [
        ("release tests", {}, [*test, *pkgs, *skip]),
        ("conformance (tests/rt, ratcheted tests/r; GNU R 4.6.2)", {},
         [*test, "-p", "robigo-rt", "--test", "conformance", "--", "--exact", CONFORMANCE, "--nocapture"]),
        # with a collection due almost always, the check makes any &Cx
        # request of LARGE bytes or more panic where it is made: such a
        # request goes through cx.alloc_rooted (docs/g3.md §3 item 3)
        ("gctorture (collect at every allocation) and the GC stats check",
         {"ROBIGO_GCTORTURE": "1", "ROBIGO_GC_STATS": "check"}, [*test, *pkgs, *skip]),
        # docs/g3.md §16 (H3): the debug moving collector, at every
        # collection gctorture makes due
        ("ROBIGO_GC_MOVE (move every movable object at every collection), with gctorture",
         {"ROBIGO_GCTORTURE": "1", "ROBIGO_GC_MOVE": "1"}, [*test, *pkgs, *skip]),
        # docs/g3.md §16 (H3): the pin report; `main` fails the job on any line
        ("ROBIGO_GC_PIN_REPORT (no weak reference decided while a C word reaches its key)",
         {"ROBIGO_GC_PIN_REPORT": PIN_REPORT}, [*test, *pkgs, *skip]),
    ]


def metadata():
    p = subprocess.run(["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"],
                       cwd=ROOT, stdout=subprocess.PIPE, text=True, check=True)
    return json.loads(p.stdout)


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--list", action="store_true", help="print the rt crates and stop")
    ap.add_argument("--check", action="store_true", help="check the premise and stop")
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

    for title, env, cmd in steps(crates):
        report = env.get("ROBIGO_GC_PIN_REPORT")
        if report and os.path.exists(report):
            os.remove(report)
        print(f"::group::{title}", flush=True)
        r = subprocess.run(cmd, cwd=ROOT, env={**os.environ, **env})
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
                return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
