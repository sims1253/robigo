#!/usr/bin/env python3
"""Tests of scripts/prune-target.py.

    python3 scripts/test-prune-target.py

A synthetic target directory checks which units go, a held `.cargo-lock`
checks that pruning waits for a running build, and a real cargo project
checks that a pruned unit is rebuilt while a unit still in use stays fresh.
The last needs cargo; PRUNE_TEST_TOOLCHAINS="1.99.0 1.100.0" runs it under
each rustup toolchain (cargo 1.100 changed the layout of the build
directory), the default is the `cargo` on PATH.
"""

import atexit
import fcntl
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "prune-target.py"
TMP = Path(tempfile.mkdtemp(prefix="robigo-test-prune-"))
atexit.register(shutil.rmtree, TMP, ignore_errors=True)
OLD = time.time() - 10 * 86400
failures = 0


def check(cond, what):
    global failures
    print(("ok   " if cond else "FAIL ") + what)
    failures += not cond


def prune(target, *flags):
    r = subprocess.run([sys.executable, str(SCRIPT), *flags, str(target)],
                       check=False, capture_output=True, text=True)
    if r.returncode:
        sys.exit(f"prune-target.py failed:\n{r.stderr}")
    return r.stdout


def write(path, when=None):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("x" * 100)
    if when:
        os.utime(path, (when, when))
    return path


def age(root, when=OLD):
    for r, _, files in os.walk(root):
        for f in files:
            os.utime(os.path.join(r, f), (when, when))


def synthetic():
    t = TMP / "synthetic"
    p = t / "debug"
    write(p / ".cargo-lock")
    h = [f"{i:016x}" for i in range(1, 7)]
    # cargo >= 1.100: one directory per unit.
    fresh_new = write(p / f"build/foo/{h[0]}/fingerprint/lib-foo")
    old_new = write(p / f"build/foo/{h[1]}/out/libfoo-{h[1]}.rlib", OLD)
    write(p / f"build/foo/{h[1]}/fingerprint/lib-foo", OLD)
    gone_pkg = write(p / f"build/gone/{h[5]}/out/x", OLD).parent.parent.parent
    # cargo < 1.100: a unit's files share its hash across directories.
    old_legacy = [write(p / f".fingerprint/bar-{h[2]}/lib-bar", OLD),
                  write(p / f"deps/libbar-{h[2]}.rlib", OLD),
                  write(p / f"deps/bar-{h[2]}.d", OLD),
                  write(p / f"deps/bar-{h[2]}.long-type-12345.txt", OLD),
                  write(p / f"build/bar-{h[2]}/output", OLD)]
    # A fingerprint read by today's build keeps its old output.
    used_rlib = write(p / f"deps/libbaz-{h[3]}.rlib", OLD)
    write(p / f".fingerprint/baz-{h[3]}/lib-baz", OLD)
    os.utime(p / f".fingerprint/baz-{h[3]}/lib-baz", (time.time(), OLD))
    old_inc = write(p / "incremental/foo-1abc/s-x/dep-graph.bin", OLD).parent.parent
    fresh_inc = write(p / "incremental/foo-2abc/s-y/dep-graph.bin").parent.parent
    stray = write(p / "deps/libR.so", OLD)
    uplifted = write(p / "robigo", OLD)
    # A profile two levels down (cross target) is pruned too.
    cross = write(t / f"x86_64-unknown-linux-gnu/release/build/q/{h[4]}/out/q", OLD)
    write(t / "x86_64-unknown-linux-gnu/release/.cargo-lock")

    listing = prune(t, "-n")
    listed = {line.split(" ", 1)[1] for line in listing.splitlines()[:-1]}
    want = {str(old_new.parent.parent), str(cross.parent.parent), str(old_inc),
            str(gone_pkg / h[5])} | {str(x.parent if x.parent.name.endswith(h[2]) else x)
                                     for x in old_legacy}
    check(listed == want, "-n lists exactly the stale units")
    check(listing.splitlines()[-1].startswith("would delete 5 units"), "-n counts units")
    check(old_new.exists() and old_inc.exists(), "-n deletes nothing")

    out = prune(t)
    check(out.startswith("deleted 5 units"), "reports what it deleted")
    check(not old_new.parent.parent.exists(), "old unit (new layout) deleted")
    check(fresh_new.exists(), "fresh unit (new layout) kept")
    check(not gone_pkg.exists(), "package directory left empty removed")
    check(not any(x.exists() for x in old_legacy), "old unit (legacy layout) deleted whole")
    check(used_rlib.exists(), "unit whose fingerprint was read today kept")
    check(not old_inc.exists() and fresh_inc.exists(), "only the old incremental cache deleted")
    check(stray.exists() and uplifted.exists(), "files outside any unit kept")
    check(not cross.exists(), "nested profile pruned")
    check(prune(t).startswith("deleted 0 units"), "second run deletes nothing")


def waits_for_lock():
    t = TMP / "locked"
    unit = write(t / "debug/build/foo/0000000000000001/out/x", OLD)
    with open(write(t / "debug/.cargo-lock"), "rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_SH)  # what a running build holds
        proc = subprocess.Popen([sys.executable, str(SCRIPT), str(t)],
                                stdout=subprocess.DEVNULL)
        time.sleep(1)
        check(proc.poll() is None and unit.exists(), "waits while a build holds the lock")
    proc.wait(timeout=30)
    check(not unit.exists(), "prunes once the build is done")


PROJECT = {
    "Cargo.toml": '[workspace]\nmembers = ["app", "dep"]\nresolver = "2"\n',
    "dep/Cargo.toml": '[package]\nname = "dep"\nversion = "0.1.0"\nedition = "2021"\n'
                      '[features]\nx = []\n',
    "dep/src/lib.rs": 'pub fn v() -> u32 { if cfg!(feature = "x") { 2 } else { 1 } }\n',
    "dep/build.rs": "fn main() {}\n",
    "app/Cargo.toml": '[package]\nname = "app"\nversion = "0.1.0"\nedition = "2021"\n'
                      '[dependencies]\ndep = { path = "../dep" }\n[features]\nx = ["dep/x"]\n',
    "app/src/main.rs": 'fn main() { println!("{}", dep::v()); }\n',
}


def end_to_end(toolchain):
    name = toolchain or "path"
    root = TMP / f"cargo-{name}"
    for rel, text in PROJECT.items():
        write(root / rel).write_text(text)
    target = root / "target"
    cargo = ["cargo"] + ([f"+{toolchain}"] if toolchain else [])
    env = dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_BUILD_JOBS="2",
               CARGO_TERM_COLOR="never")

    def run(*features):
        r = subprocess.run([*cargo, "run", "--offline", "-p", "app", *features],
                           cwd=root, env=env, check=False, capture_output=True, text=True)
        if r.returncode:
            sys.exit(f"cargo failed:\n{r.stderr}")
        return r.stdout.strip(), "Compiling" in r.stderr

    # Sources older than the build, so aged outputs still look fresh.
    age(root, OLD - 86400)
    run()
    run("--features", "x")
    age(target)
    check(run("--features", "x") == ("2", False), f"[{name}] aged build still fresh")
    listing = prune(target, "-n")
    check("would delete 0 units" not in listing, f"[{name}] the unused variant is stale")
    prune(target)
    check(run("--features", "x") == ("2", False), f"[{name}] unit in use kept: no rebuild")
    check(run() == ("1", True), f"[{name}] pruned variant rebuilds and runs")


def main():
    synthetic()
    waits_for_lock()
    toolchains = os.environ.get("PRUNE_TEST_TOOLCHAINS", "").split() or [None]
    if shutil.which("cargo"):
        for tc in toolchains:
            end_to_end(tc)
    else:
        print("skip end-to-end: no cargo")
    if failures:
        sys.exit(f"{failures} check(s) failed")
    print("all passed")


if __name__ == "__main__":
    main()
