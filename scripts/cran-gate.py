#!/usr/bin/env python3
"""CRAN packages with compiled code, installed from source into robigo.

    scripts/cran-gate.py run [PKG...]      # install + test, compare with GNU R
    scripts/cran-gate.py try PKG[@VER]     # explore: fetch, install, test, report
    scripts/cran-gate.py install PKG...    # robigo CMD INSTALL into the cache library
    scripts/cran-gate.py test PKG...       # run the test files with robigo
    scripts/cran-gate.py record PKG...     # GNU R in the oracle image, write expected
    scripts/cran-gate.py list              # the manifest

The metric is `tests/cran/packages.tsv`: one package per line (name,
version, the directory of its test files inside the tarball). `run`
installs each one with `robigo CMD INSTALL` from source — robigo's own
`R_HOME`, its headers, its `libR.so` — runs its test files through
`crates/robigo-capi/gate/shim.R` (a minimal tinytest/testthat that counts
expectations), and compares the count with GNU R 4.5.0's, recorded in
`tests/cran/expected.json` by `record` inside the `robigo-oracle:4.5.0`
docker image. A package that installs and matches is a pass; the ratchet
is the manifest, so it never shrinks by accident: `record` is the only
writer of expected.json and it runs GNU R, never robigo.

Package sources are downloaded into `$ROBIGO_CRAN_CACHE` (default
`~/.cache/robigo-cran`, never committed): `src/` the tarballs, `build/`
the unpacked trees, `lib/` robigo's installs, `lib-oracle/` GNU R's.

A C API bug can show up as a hang, not a failure, so each robigo step has a
time limit (`$ROBIGO_CRAN_INSTALL_TIMEOUT`, `$ROBIGO_CRAN_TEST_TIMEOUT`, in
seconds). A step over its limit has its whole process group killed and the
package's outcome is `timeout`.
"""
import argparse
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import tarfile
import time
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MANIFEST = os.path.join(ROOT, "tests", "cran", "packages.tsv")
EXPECTED = os.path.join(ROOT, "tests", "cran", "expected.json")
SHIM = os.path.join(ROOT, "crates", "robigo-capi", "gate", "shim.R")
CACHE = os.environ.get("ROBIGO_CRAN_CACHE", os.path.expanduser("~/.cache/robigo-cran"))
REPO = os.environ.get("ROBIGO_CRAN_REPO", "https://cran.r-project.org/src/contrib")
ORACLE_IMAGE = os.environ.get("ROBIGO_ORACLE_IMAGE", "robigo-oracle:4.5.0")
ROBIGO = os.environ.get("ROBIGO", os.path.join(ROOT, ".robigo", "bin", "robigo"))
# The oracle's environment, as scripts/regen-expected.sh uses it.
ENV = dict(os.environ, LANGUAGE="en", LC_ALL="C.UTF-8", TZ="UTC")
ENV.pop("R_HOME", None)
ENV.pop("R_PROFILE_USER", None)
# On CI the whole gate, compiling included, takes about a minute for two
# packages: at least ten times that for one step, more for installs because
# the popular set has bigger C++ than fastmap and digest.
INSTALL_TIMEOUT = int(os.environ.get("ROBIGO_CRAN_INSTALL_TIMEOUT", 900))
TEST_TIMEOUT = int(os.environ.get("ROBIGO_CRAN_TEST_TIMEOUT", 600))
# How long a timed-out step gets to stop on SIGTERM before SIGKILL.
KILL_GRACE = 5


def die(msg):
    sys.exit(f"cran-gate: {msg}")


def memory_cap(cmd):
    """Run under the shared machine's memory cap (docs/ledger/core.md)."""
    cap = os.environ.get("ROBIGO_NO_MEMORY_CAP")
    if cap:
        return cmd
    run = shutil.which("systemd-run")
    if not run:
        return cmd
    return [run, "--user", "--scope", "-q", "-p", "MemoryMax=3G",
            "-p", "MemorySwapMax=0"] + cmd


class StepTimeout(Exception):
    """A step went over its time limit: the outcome `timeout`."""

    def __init__(self, step, limit, output):
        super().__init__(f"{step} timed out after {limit}s")
        self.step, self.limit, self.output = step, limit, output


def run_step(step, cmd, limit, env=ENV):
    """Run `cmd` in its own process group, killed whole after `limit` seconds.

    R CMD INSTALL spawns compilers and test processes, so killing only the
    direct child would leave them running. `systemd-run --scope` execs the
    command, so it is the group's leader under the memory cap too.
    """
    p = subprocess.Popen(memory_cap(cmd), env=env, start_new_session=True, text=True,
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        out, err = p.communicate(timeout=limit)
    except subprocess.TimeoutExpired:
        os.killpg(p.pid, signal.SIGTERM)
        try:
            p.wait(KILL_GRACE)
        except subprocess.TimeoutExpired:
            pass
        # The leader exiting says nothing about the rest of its group.
        try:
            os.killpg(p.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        try:
            out, err = p.communicate(timeout=KILL_GRACE)
        except subprocess.TimeoutExpired:
            # A process that left the group still holds the pipes.
            out, err = "", ""
        raise StepTimeout(step, limit, out + err) from None
    return subprocess.CompletedProcess(cmd, p.returncode, out, err)


def manifest():
    """[(name, version, tests dir)], in file order."""
    out = []
    if not os.path.exists(MANIFEST):
        return out
    for line in open(MANIFEST):
        line = line.split("#", 1)[0].strip()
        if line:
            name, version, tests = line.split()
            out.append((name, version, tests))
    return out


def entry(name):
    for e in manifest():
        if e[0] == name:
            return e
    return None


def fetch(name, version):
    src = os.path.join(CACHE, "src", f"{name}_{version}.tar.gz")
    build = os.path.join(CACHE, "build", name)
    if not os.path.exists(src):
        os.makedirs(os.path.dirname(src), exist_ok=True)
        url = f"{REPO}/{name}_{version}.tar.gz"
        print(f"fetching {url}", flush=True)
        # CRAN now and then drops a connection; retry a few times before giving up.
        for attempt in range(1, 4):
            try:
                with urllib.request.urlopen(url, timeout=120) as r, open(src, "wb") as f:
                    shutil.copyfileobj(r, f)
                break
            except OSError:
                if os.path.exists(src):
                    os.remove(src)
                if attempt == 3:
                    raise
                time.sleep(5 * attempt)
    stamp = os.path.join(build, ".version")
    have = open(stamp).read().strip() if os.path.exists(stamp) else None
    if have != version:
        shutil.rmtree(build, ignore_errors=True)
        os.makedirs(build, exist_ok=True)
        with tarfile.open(src) as t:
            t.extractall(build)
        open(stamp, "w").write(version)
    return src, os.path.join(build, name)


def robigo_install(src, lib):
    os.makedirs(lib, exist_ok=True)
    env = dict(ENV, MAKEFLAGS=os.environ.get("MAKEFLAGS", "-j2"))
    p = run_step("install", [ROBIGO, "CMD", "INSTALL", f"--library={lib}", src],
                 INSTALL_TIMEOUT, env=env)
    if p.returncode != 0:
        tail = "\n".join((p.stdout + p.stderr).strip().splitlines()[-25:])
        die(f"robigo CMD INSTALL {os.path.basename(src)} failed:\n{tail}")
    return p.stdout + p.stderr


def oracle_install(src, lib):
    """Install with GNU R 4.5.0 in the oracle image (the same sources)."""
    os.makedirs(lib, exist_ok=True)
    src_rel = os.path.relpath(src, CACHE)
    lib_rel = os.path.relpath(lib, CACHE)
    cmd = ["docker", "run", "--rm", "--memory", "3g", "--memory-swap", "3g",
           "-v", f"{CACHE}:/cache", "-w", "/cache", ORACLE_IMAGE,
           "sh", "-c",
           f"R CMD INSTALL --library=/cache/{lib_rel} /cache/{src_rel} >/dev/null 2>&1"]
    p = subprocess.run(cmd, capture_output=True, text=True)
    if p.returncode != 0:
        die(f"GNU R could not install {os.path.basename(src)}: {p.stdout}{p.stderr}")


def shim_cmd(pkg, tests, lib):
    return [ROBIGO, "--vanilla", "--quiet", "-f", SHIM, "--args", pkg, tests, lib]


def report(out):
    """The shim's verdict: (version, passed, failed, failure lines)."""
    m = None
    fails = []
    for line in out.splitlines():
        m2 = re.match(r"^(\S+) (\S+): (\d+) expectations passed, (\d+) failed$", line)
        if m2:
            m = (m2.group(1), m2.group(2), int(m2.group(3)), int(m2.group(4)))
        elif line.startswith(" - "):
            fails.append(line[3:])
    if m is None:
        die(f"no verdict from the shim:\n{out}")
    return m, fails


def robigo_test(name, version, tests, quiet=False):
    lib = os.path.join(CACHE, "lib")
    src, tree = fetch(name, version)
    if not os.path.isdir(os.path.join(lib, name)):
        robigo_install(src, lib)
    tdir = os.path.join(tree, tests)
    if not os.path.isdir(tdir):
        die(f"{name}: no tests directory {tests} in {tree}")
    p = run_step("test", shim_cmd(name, tdir, lib), TEST_TIMEOUT)
    out = p.stdout + p.stderr
    (_, got_version, passed, failed), fails = report(out)
    return got_version, passed, failed, fails, lib, src, tree


def cmd_list(_):
    for name, version, tests in manifest():
        print(f"{name:16} {version:12} {tests}")


def cmd_install(a):
    lib = os.path.join(CACHE, "lib")
    for spec in a.pkgs:
        name, version = split_spec(spec)
        src, _ = fetch(name, version)
        print(robigo_install(src, lib).strip())


def split_spec(spec):
    name, _, version = spec.partition("@")
    if version:
        return name, version
    e = entry(name)
    if not e:
        die(f"{name}: no version given and not in {os.path.relpath(MANIFEST, ROOT)}")
    return e[0], e[1]


def cmd_test(a):
    for spec in a.pkgs:
        name, version = split_spec(spec)
        e = entry(name)
        tests = e[2] if e else a.tests
        got_version, passed, failed, fails, *_ = robigo_test(name, version, tests)
        print(f"{name} {got_version}: {passed} passed, {failed} failed")
        for f in fails:
            print(f" - {f}")


def cmd_try(a):
    """Explore: fetch, install, test one package and report where it stops."""
    name, version = split_spec(a.pkg)
    e = entry(name)
    tests = a.tests or (e[2] if e else None)
    lib = os.path.join(CACHE, "lib")
    src, tree = fetch(name, version)
    try:
        robigo_install(src, lib)
    except SystemExit as ex:
        print(f"{name} {version}: install FAILED")
        print(ex)
        return
    if tests is None:
        for cand in ("tests/testthat", "inst/tinytest", "tests", "inst/tests"):
            if os.path.isdir(os.path.join(tree, cand)):
                tests = cand
                break
        if tests is None:
            print(f"{name} {version}: installed, no test directory found")
            return
    got_version, passed, failed, fails, *_ = robigo_test(name, version, tests)
    print(f"{name} {got_version}: {passed} passed, {failed} failed  ({tests})")
    for f in fails[: a.show]:
        print(f" - {f}")
    if failed > a.show:
        print(f" - ... {failed - a.show} more")


def cmd_record(a):
    """GNU R 4.5.0 in the oracle image: what the packages' tests do there."""
    exp = json.load(open(EXPECTED)) if os.path.exists(EXPECTED) else {}
    specs = a.pkgs or [f"{n}@{v}" for n, v, _ in manifest()]
    for spec in specs:
        name, version = split_spec(spec)
        e = entry(name)
        tests = a.tests or (e[2] if e else None)
        if tests is None:
            die(f"{name}: pass --tests")
        src, tree = fetch(name, version)
        lib = os.path.join(CACHE, "lib-oracle")
        oracle_install(src, lib)
        tdir = os.path.join(tree, tests)
        cmd = ["docker", "run", "--rm", "--memory", "3g", "--memory-swap", "3g",
               "-v", f"{CACHE}:/cache", "-v", f"{ROOT}:/repo:ro",
               "-e", "LANGUAGE=en", "-e", "LC_ALL=C.UTF-8", "-e", "TZ=UTC",
               ORACLE_IMAGE, "R", "--vanilla", "--quiet", "-f", "/repo/crates/robigo-capi/gate/shim.R",
               "--args", name, "/cache/" + os.path.relpath(tdir, CACHE),
               "/cache/" + os.path.relpath(lib, CACHE)]
        p = subprocess.run(cmd, capture_output=True, text=True)
        out = (p.stdout or "") + (p.stderr or "")
        (_, got_version, passed, failed), fails = report(out)
        exp[name] = {"version": got_version, "tests": tests,
                     "passed": passed, "failed": failed, "failures": fails}
        print(f"{name} {got_version}: {passed} passed, {failed} failed  (GNU R)")
        for f in fails:
            print(f" - {f}")
    os.makedirs(os.path.dirname(EXPECTED), exist_ok=True)
    with open(EXPECTED, "w") as f:
        json.dump(dict(sorted(exp.items())), f, indent=1)
        f.write("\n")
    print(f"recorded {len(specs)} package(s) in {os.path.relpath(EXPECTED, ROOT)}")


def cmd_run(a):
    exp = json.load(open(EXPECTED)) if os.path.exists(EXPECTED) else {}
    specs = a.pkgs or [f"{n}@{v}" for n, v, _ in manifest()]
    ok, bad = [], []
    for spec in specs:
        name, version = split_spec(spec)
        e = entry(name)
        tests = e[2] if e else a.tests
        want = exp.get(name)
        try:
            got_version, passed, failed, fails, *_ = robigo_test(name, version, tests)
        except StepTimeout as ex:
            outcome = f"{ex.step} after {ex.limit}s"
            bad.append((name, f"timeout ({outcome})"))
            print(f"{name} {version}: TIMEOUT ({outcome})")
            if a.diff:
                print(ex.output)
            continue
        except SystemExit as ex:
            bad.append((name, "install failed"))
            print(f"{name} {version}: FAIL (install)")
            if a.diff:
                print(ex)
            continue
        if want is None:
            print(f"{name} {got_version}: {passed} passed, {failed} failed  (not recorded)")
            bad.append((name, "not recorded"))
            continue
        same = (passed, failed) == (want["passed"], want["failed"])
        if same and failed:
            # the same expectations fail, but not necessarily the same ones
            same = fails == want["failures"]
        if same:
            ok.append(name)
            print(f"{name} {got_version}: {passed} passed, {failed} failed  (GNU R: same)")
        else:
            bad.append((name, f"{passed}/{failed} vs GNU R {want['passed']}/{want['failed']}"))
            print(f"{name} {got_version}: {passed} passed, {failed} failed "
                  f"(GNU R {want['version']}: {want['passed']}/{want['failed']})")
            if a.diff:
                extra = [f for f in fails if f not in want["failures"]]
                gone = [f for f in want["failures"] if f not in fails]
                for f in extra:
                    print(f"  robigo only: {f}")
                for f in gone:
                    print(f"  GNU R only:  {f}")
    print(f"\n{len(ok)}/{len(ok) + len(bad)} package(s) match GNU R"
          + (f"; failing: {', '.join(n for n, _ in bad)}" if bad else ""))
    if bad:
        for name, why in bad:
            print(f" - {name}: {why}")
        sys.exit(1)


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list").set_defaults(fn=cmd_list)
    q = sub.add_parser("install"); q.add_argument("pkgs", nargs="+"); q.set_defaults(fn=cmd_install)
    q = sub.add_parser("test"); q.add_argument("pkgs", nargs="+")
    q.add_argument("--tests", default="tests/testthat"); q.set_defaults(fn=cmd_test)
    q = sub.add_parser("try"); q.add_argument("pkg")
    q.add_argument("--tests"); q.add_argument("--show", type=int, default=20)
    q.set_defaults(fn=cmd_try)
    q = sub.add_parser("record"); q.add_argument("pkgs", nargs="*")
    q.add_argument("--tests"); q.set_defaults(fn=cmd_record)
    q = sub.add_parser("run"); q.add_argument("pkgs", nargs="*")
    q.add_argument("--tests"); q.add_argument("--diff", action="store_true")
    q.set_defaults(fn=cmd_run)
    a = p.parse_args()
    try:
        a.fn(a)
    except StepTimeout as ex:
        die(str(ex))


if __name__ == "__main__":
    main()
