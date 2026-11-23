#!/usr/bin/env python3
"""The cargo-fuzz targets in CI (fuzz/fuzz_targets; docs/verification.md,
"Fuzzing").

    scripts/fuzz-ci.py check                        # cargo check the fuzz crate
    scripts/fuzz-ci.py plan --base origin/main      # the targets a branch owes
    scripts/fuzz-ci.py run --targets parse,rtc --seconds 60
    scripts/fuzz-ci.py run --all --seconds 0        # replay every seed corpus

`check` builds nothing with the sanitizers: it is what keeps a target
compiling on every pull request. `plan` names the targets whose inputs the
branch changes (FILTERS, by path prefix), and why. `run` builds each target,
replays its seeds (fuzz/seeds/<target>) and the corpus a run writes
(fuzz/corpus/<target>), then fuzzes it for `--seconds`. A failure leaves its
input under fuzz/artifacts/<target>/ and prints the command that repeats it.
"""

import argparse
import glob
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FUZZ = os.path.join(ROOT, "fuzz")
# The nightly the fuzz crate builds with (cargo-fuzz needs -Zsanitizer); the
# miri job pins the same one
TOOLCHAIN = "nightly-2026-11-02"
# libFuzzer's limit for one input, in both phases (its default, 1200 s, is
# longer than a job may wait: -max_total_time is checked between inputs)
INPUT_TIMEOUT = 60

# Each target's inputs, by path prefix: a change under one of them is one
# that can change what the target finds (its harness and seeds are its own:
# OWN)
FILTERS = {
    "parse": ["crates/robigo-syntax/"],
    # (unserialize, the interpreter the harness makes for every input, and
    # the value, memory, symbol and environment code the decoder builds with)
    "rds": ["crates/robigo-runtime/src/serialize", "crates/robigo-runtime/src/interp.rs",
            "crates/robigo-runtime/src/value.rs", "crates/robigo-runtime/src/memory.rs",
            "crates/robigo-runtime/src/sym.rs", "crates/robigo-runtime/src/env.rs"],
    # (tcache.rs's fuzz_decode, the format, and the instruction set it decodes)
    "rtc": ["crates/robigo-rt/src/tcache", "crates/robigo-rt/src/ir.rs", "fuzz/rtc_seeds.py"],
}
# ...and what every target depends on: the fuzz crate's manifest, the
# dependencies and the compiler, and the step itself
ALL = ["fuzz/Cargo.toml", "fuzz/Cargo.lock", "Cargo.lock", "rust-toolchain.toml", "scripts/fuzz-ci.py",
       ".github/workflows/ci.yml", ".github/workflows/nightly.yml"]


def own(t):
    """A target's own files: its harness and its seeds."""
    return [f"fuzz/fuzz_targets/{t}.rs", f"fuzz/seeds/{t}/"]


def targets():
    """The fuzz targets, by their files in fuzz/fuzz_targets."""
    return sorted(os.path.basename(p)[:-3] for p in glob.glob(os.path.join(FUZZ, "fuzz_targets", "*.rs")))


def plan(changed):
    """{target: [the changed paths that owe it]} for the paths `changed`
    (None: git could not say, so every target is owed)."""
    out = {}
    for t in targets():
        if changed is None:
            out[t] = ["(git cannot compare this checkout with its base)"]
            continue
        hit = [p for p in changed if any(p.startswith(x) for x in FILTERS.get(t, []) + own(t) + ALL)]
        if hit:
            out[t] = hit
    return out


def changed_paths(base):
    """The paths HEAD changes against its merge base with `base`, or None."""
    def git(*a):
        try:
            p = subprocess.run(["git", *a], cwd=ROOT, capture_output=True, text=True)
        except OSError:
            return None
        return p.stdout if p.returncode == 0 else None
    mb = git("merge-base", "HEAD", base)
    if mb is None:
        return None
    # (NUL-separated bytes: a name git would quote, non-ASCII or with a
    # control character, still meets the prefixes)
    try:
        p = subprocess.run(["git", "diff", "--name-only", "-z", "--no-renames", mb.strip(), "HEAD"], cwd=ROOT,
                           capture_output=True)
    except OSError:
        return None
    if p.returncode != 0:
        return None
    return sorted({os.fsdecode(x) for x in p.stdout.split(b"\0") if x})


def summary(lines):
    """Print `lines`, and add them to the job's step summary in CI."""
    text = "\n".join(lines)
    print(text, flush=True)
    path = os.environ.get("GITHUB_STEP_SUMMARY")
    if path:
        with open(path, "a") as f:
            f.write(text + "\n")


def cargo_fuzz(*args):
    return subprocess.run(["cargo", f"+{TOOLCHAIN}", "fuzz", *args], cwd=ROOT).returncode


def run(names, seconds):
    """Build, replay and fuzz each target; 1 if any failed."""
    status = 0
    for t in names:
        corpus = os.path.join(FUZZ, "corpus", t)
        os.makedirs(corpus, exist_ok=True)
        dirs = [corpus, os.path.join(FUZZ, "seeds", t)]
        if cargo_fuzz("build", "-O", t):
            summary([f"fuzz {t}: the build failed"])
            status = 1
            continue
        # replay: every input of both directories once, no mutation
        steps = [("replay", ["-runs=0"])]
        if seconds > 0:
            steps.append((f"{seconds} s", [f"-max_total_time={seconds}"]))
        timeout = f"-timeout={INPUT_TIMEOUT}"
        for what, opts in steps:
            t0 = time.time()
            if cargo_fuzz("run", "-O", t, *dirs, "--", timeout, *opts):
                # (by time, not by name: libFuzzer names an input by its
                # hash, so a failure seen before rewrites the same file)
                found = sorted(p for p in glob.glob(os.path.join(FUZZ, "artifacts", t, "*"))
                               if os.path.getmtime(p) >= t0 - 1)
                lines = [f"fuzz {t} ({what}): FAILED"]
                for p in found:
                    rel = os.path.relpath(p, ROOT)
                    lines.append(f"  input {rel}; repeat it: cargo +{TOOLCHAIN} fuzz run -O {t} {rel} -- {timeout}")
                if not found:
                    lines.append("  (no input saved: see the log above)")
                summary(lines)
                status = 1
                break
            summary([f"fuzz {t} ({what}): ok"])
    return status


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("check")
    pl = sub.add_parser("plan")
    pl.add_argument("--base", default="origin/main")
    pl.add_argument("--github-output", action="store_true", help="write targets=... to $GITHUB_OUTPUT")
    r = sub.add_parser("run")
    r.add_argument("--targets", default="", help="comma-separated")
    r.add_argument("--all", action="store_true")
    r.add_argument("--seconds", type=int, default=0)
    a = p.parse_args(argv)
    if a.cmd == "check":
        return subprocess.run(["cargo", f"+{TOOLCHAIN}", "check", "--locked", "--manifest-path",
                               os.path.join(FUZZ, "Cargo.toml"), "--bins"], cwd=ROOT).returncode
    if a.cmd == "plan":
        owed = plan(changed_paths(a.base))
        lines = ["fuzz targets this branch owes (their inputs changed):"]
        for t in targets():
            if t in owed:
                shown = owed[t][:5] + ([f"... {len(owed[t]) - 5} more"] if len(owed[t]) > 5 else [])
                lines.append(f"  {t}: runs ({', '.join(shown)})")
            else:
                lines.append(f"  {t}: skipped (no change under {', '.join(FILTERS.get(t, []) + own(t))} "
                             "or the files every target depends on)")
        summary(lines)
        if a.github_output and os.environ.get("GITHUB_OUTPUT"):
            with open(os.environ["GITHUB_OUTPUT"], "a") as f:
                f.write(f"targets={','.join(t for t in targets() if t in owed)}\n")
        return 0
    names = targets() if a.all else [t for t in a.targets.split(",") if t]
    unknown = [t for t in names if t not in targets()]
    if unknown:
        sys.exit(f"fuzz-ci: no target {', '.join(unknown)} in fuzz/fuzz_targets")
    return run(names, a.seconds)


if __name__ == "__main__":
    sys.exit(main())
