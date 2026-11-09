#!/usr/bin/env python3
"""Differential fuzzing: mutated conformance scripts run on robigo and GNU R.

    scripts/fuzz-eval.py [--seconds N] [--jobs N] [--robigo BIN] [--seed N]
                         [--oracle RSCRIPT] [--fail-on-diff] [--max-diffs N]
                         [--tiers]

Seeds are `tests/r/**/*.R` — restricted to the ratchet (`tests/r/passing/`)
where markers exist, since those inputs behave identically under GNU R and
robigo today. scripts/fuzz.py's mutator generates mutants. Each
mutant runs on GNU R 4.5.0 twice and on robigo once, all with
R_DEFAULT_PACKAGES=NULL and --vanilla, like scripts/regen-expected.sh. When
GNU R's normalized output (scripts/corpus.py's masking) is stable and
robigo's normalized stdout, stderr or exit status differs, that is a
candidate conformance gap: one bucket with one reproducer under
.robigo/fuzz/eval/, printed at the end. A robigo crash (panic, stack
overflow, signal, failed allocation) is a finding exactly as in
scripts/fuzz.py and always fails the run. Timeouts and unstable GNU R
output are reported but do not fail.

With --tiers the oracle is skipped: the mutant runs on robigo three times,
twice in the default tier (for stability) and once with ROBIGO_JIT=0. The VM and the JIT must print the same
thing (docs/architecture.md), so any difference is a miscompile candidate —
no oracle, no conformance-gap noise. Crashes count as above.

The oracle must be GNU R 4.5.0 (--oracle, $ROBIGO_ORACLE_R, or
~/.cache/robigo-research/R-4.5.0-build/bin/Rscript; scripts/build-oracle.sh
builds one). Newer R disagrees with 4.5.0 for reasons that are not robigo's
bugs.

Differences are leads, not verdicts: some seeds are inherently flaky and a
diff may repeat a known corpus failure. Verify a lead by reducing it to a
`tests/r/<area>/<name>.R` conformance script before working on it
(docs/workstreams.md, protocol rule 8).
"""
import argparse
import concurrent.futures
import hashlib
import json
import os
import pathlib
import random
import resource
import signal
import re
import subprocess
import sys
import threading
import time
from collections import Counter

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
import corpus  # noqa: E402
import fuzz  # noqa: E402

RTMP = re.compile(rb"/tmp/Rtmp[A-Za-z0-9]+")


def norm(b):
    """corpus.py's masking, plus the random R CMD SHLIB/dyn.load tmpdirs
    whose names differ on every run."""
    return corpus.normalize(RTMP.sub(b"/tmp/RtmpXXXXXX", b))

ENV = {"PATH": "/usr/bin:/bin", "LANGUAGE": "en", "LC_ALL": "C.UTF-8",
       "R_DEFAULT_PACKAGES": "NULL", "HOME": "/tmp", "TZ": "UTC",
       # A local machine's R_LIBS_USER would load the host R's packages into
       # robigo (with "built under R version x.y.z" warnings the oracle never
       # prints): run base-only, like CI and scripts/regen-expected.sh.
       "R_LIBS": "", "R_LIBS_USER": "", "R_LIBS_SITE": "",
       "R_ENVIRON": "", "R_ENVIRON_USER": "", "R_PROFILE": "", "R_PROFILE_USER": ""}
TIMEOUT = 30
MEMORY_LIMIT = 16 << 30


def find_oracle(user):
    candidates = []
    if user:
        candidates.append(pathlib.Path(user))
    if os.environ.get("ROBIGO_ORACLE_R"):
        candidates.append(pathlib.Path(os.environ["ROBIGO_ORACLE_R"]))
    candidates.append(pathlib.Path.home() / ".cache/robigo-research/R-4.5.0-build/bin/Rscript")
    candidates.append(pathlib.Path("/usr/bin/Rscript"))
    for c in candidates:
        if c.exists():
            return c
    print("no oracle found: pass --oracle, set ROBIGO_ORACLE_R, or run scripts/build-oracle.sh", file=sys.stderr)
    sys.exit(2)


def oracle_info(rscript):
    """(version, R_HOME) of the oracle, asked of R itself; ("", "") when it
    cannot answer (hung, unexecutable, garbage)."""
    try:
        lines = subprocess.run([str(rscript), "--vanilla", "-e", 'cat(format(getRversion()), R.home(), sep="\\n")'],
                               capture_output=True, text=True, timeout=60).stdout.splitlines()
    except (OSError, subprocess.TimeoutExpired):
        return "", ""
    return (lines[-2], lines[-1]) if len(lines) >= 2 else ("", "")


def run(cmd, timeout, env=None):
    # start_new_session: a timed-out mutant that spawned grandchildren
    # (system(), dyn.load) loses the whole process group, not just the
    # direct child. The rlimits are set once in main() and inherited: no
    # preexec_fn, which is unsafe under the worker threads.
    try:
        p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                             env=env or ENV, start_new_session=True)
    except OSError as e:
        # An environment failure, not a finding: stop every worker now
        # (sys.exit in a pool thread would wait out the whole sweep).
        print(f"cannot run {cmd[0]}: {e}", file=sys.stderr, flush=True)
        sys.stdout.flush()
        os._exit(2)
    try:
        out, err = p.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(os.getpgid(p.pid), signal.SIGKILL)
        except OSError:
            p.kill()
        out, err = p.communicate()
        return out, err, "timeout"
    return out, err, p.returncode


def first_diff(a, b):
    for i, (x, y) in enumerate(zip(a, b)):
        if x != y:
            return i
    return min(len(a), len(b))


def outcome(r):
    """A run's comparable result: normalized stdout and stderr, and status."""
    return norm(r[0]), norm(r[1]), r[2]


def text(r):
    return r[1].decode("utf-8", "replace")


class Findings:
    """Counts, crash buckets and diff buckets, shared by the worker threads:
    one reproducer per bucket under OUT, at most MAX_DIFFS diff buckets."""

    def __init__(self, out, max_diffs):
        self.out, self.max_diffs = out, max_diffs
        self.lock = threading.Lock()
        self.counts = Counter()
        self.crashes = {}
        self.diffs = {}

    def count(self, what):
        with self.lock:
            self.counts[what] += 1

    def crash(self, r, data):
        """Record R if it crashed (scripts/fuzz.py's classification)."""
        if r[2] == "timeout":
            return False
        kind = fuzz.classify(r[2], text(r))
        if not kind:
            return False
        self.count("(crash)")
        if name := self._record(self.crashes, kind, data, text(r), "crash", cap=False):
            print(f"CRASH {kind} ({self.out / ('crash-' + name + '.R')})", flush=True)
        return True

    def diff(self, exp, got, data, stderr, prefix):
        """Bucket a difference by the stream that differs, where, and the
        expected text around it."""
        self.count("(diff)")
        which = "out" if got[0] != exp[0] else "err" if got[1] != exp[1] else "status"
        if which == "status":
            key = (which, f"{exp[2]}->{got[2]}", b"")
        else:
            stream = {"out": 0, "err": 1}[which]
            at = first_diff(exp[stream], got[stream])
            key = (which, at, exp[stream][max(0, at - 40):at + 60])
        if name := self._record(self.diffs, key, data, stderr, prefix, cap=True):
            print(f"DIFF   {prefix} {which} {key[1]} ...{key[2][:80]!r} ({self.out / (prefix + '-' + name + '.R')})",
                  flush=True)

    def _record(self, buckets, key, data, stderr, prefix, cap):
        with self.lock:
            if cap and key not in buckets and len(buckets) >= self.max_diffs:
                return None
            buckets[key] = buckets.get(key, 0) + 1
            if buckets[key] > 1:
                return None
            name = hashlib.sha1(str(key).encode()).hexdigest()[:10]
            (self.out / f"{prefix}-{name}.R").write_bytes(data)
            (self.out / f"{prefix}-{name}.err").write_text(stderr[:3000], encoding="utf-8")
            return name

    def report(self):
        """Write buckets.json, print the summary; True if the run found a crash."""
        c = self.counts
        (self.out / "buckets.json").write_text(json.dumps({
            "counts": dict(c),
            "crash_buckets": self.crashes,
            "diff_buckets": [{"which": k[0], "at": k[1], "expect": k[2].decode("utf-8", "replace"), "n": n}
                             for k, n in sorted(self.diffs.items(), key=lambda kv: -kv[1])[:50]],
        }, indent=1))
        unstable = c["(unstable in GNU R)"] + c["(unstable)"]
        timeouts = c["(robigo timeout)"] + c["(timeout)"] + c["(oracle timeout)"]
        print(f"\n{sum(c.values())} runs: {c['(diff)']} diffs in {len(self.diffs)} buckets, "
              f"{len(self.crashes)} crash bucket(s), {unstable} unstable, {timeouts} timeouts")
        for key, n in sorted(self.crashes.items(), key=lambda kv: -kv[1]):
            print(f"  {n:6d}  crash  {key}")
        for key, n in sorted(self.diffs.items(), key=lambda kv: -kv[1])[:15]:
            print(f"  {n:6d}  {key[0]}@{key[1]}  {key[2][:70]!r}")
        return bool(self.crashes)


def check_tiers(f, robigo, path, data, env):
    """The default tier (twice, for stability) against ROBIGO_JIT=0."""
    cmd = [robigo, "--vanilla", str(path)]
    runs = [run(cmd, TIMEOUT, env), run(cmd, TIMEOUT, env), run(cmd, TIMEOUT, dict(env, ROBIGO_JIT="0"))]
    # A crash is the finding, even when another run timed out; the tiers
    # are not compared after one.
    if any([f.crash(r, data) for r in runs]):
        return None
    if any(r[2] == "timeout" for r in runs):
        return f.count("(timeout)")
    got, again, exp = map(outcome, runs)
    # Unseeded RNG, the clock or tmpdir names differ within one tier.
    if got != again:
        return f.count("(unstable)")
    if got == exp:
        return f.count("(ok)")
    f.diff(exp, got, data, text(runs[0]) + text(runs[2]), "tier")


def check_oracle(f, robigo, rscript, path, data, env):
    """robigo against GNU R, when GNU R agrees with itself."""
    oracle = []
    for _ in range(2):
        o = run([str(rscript), "--vanilla", str(path)], TIMEOUT, env)
        if o[2] == "timeout":
            return f.count("(oracle timeout)")
        oracle.append(o)
    exp = outcome(oracle[0])
    if exp != outcome(oracle[1]):
        return f.count("(unstable in GNU R)")
    r = run([robigo, "--vanilla", str(path)], TIMEOUT, env)
    if r[2] == "timeout":
        return f.count("(robigo timeout)")
    if f.crash(r, data):
        return None
    got = outcome(r)
    if got == exp:
        return f.count("(ok)")
    f.diff(exp, got, data, text(r) + text(oracle[0]), "diff")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--seconds", type=int, default=60)
    ap.add_argument("--jobs", type=int, default=os.cpu_count())
    ap.add_argument("--robigo", default=str(ROOT / ".robigo/bin/robigo"))
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--oracle")
    ap.add_argument("--tiers", action="store_true",
                    help="compare robigo's tiers (ROBIGO_JIT=0 vs default) instead of GNU R")
    ap.add_argument("--fail-on-diff", action="store_true")
    ap.add_argument("--max-diffs", type=int, default=200)
    a = ap.parse_args()
    # Inherited by every child (and this driver, which stays far below).
    resource.setrlimit(resource.RLIMIT_AS, (MEMORY_LIMIT, MEMORY_LIMIT))
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))

    if a.tiers:
        robigo = str(pathlib.Path(a.robigo).resolve())
        if not os.path.exists(robigo):
            print(f"{robigo} is missing; run scripts/build.sh", file=sys.stderr)
            sys.exit(2)
        rscript = None
        version = "tier differential (no oracle)"
        env = dict(ENV)
    else:
        rscript = find_oracle(a.oracle)
        version, home = oracle_info(rscript)
        if version != "4.5.0":
            print(f"oracle {rscript} is R {version or '(unknown)'}, not 4.5.0; refusing (false diffs)", file=sys.stderr)
            sys.exit(2)
        robigo = str(pathlib.Path(a.robigo).resolve())
        if not os.path.exists(robigo):
            print(f"{robigo} is missing; run scripts/build.sh", file=sys.stderr)
            sys.exit(2)
        # The oracle's own home, for robigo's base library too: an empty or
        # foreign R_HOME makes GNU R warn on every run.
        env = dict(ENV, R_HOME=home)

    out = ROOT / ".robigo/fuzz/eval"
    out.mkdir(parents=True, exist_ok=True)
    # Prefer the ratchet's scripts: those behave identically under GNU R and
    # robigo today, so a difference is a new regression, not a known gap.
    markers = ROOT / "tests/r/passing"
    marked = {str(p.relative_to(markers))[:-len(".pass")] for p in markers.rglob("*.pass")}
    scripts = [p for p in sorted((ROOT / "tests/r").rglob("*.R"))
               if str(p.relative_to(ROOT / "tests/r")) in marked]
    ratcheted = bool(scripts)
    if not ratcheted:
        scripts = sorted((ROOT / "tests/r").rglob("*.R"))
    seeds = [p.read_bytes()[:65536] for p in scripts]
    print(f"oracle {rscript or '-'} (R {version}), {len(seeds)} seed scripts "
          f"(ratcheted: {ratcheted})")

    findings = Findings(out, a.max_diffs)

    def worker(index):
        rnd = random.Random(a.seed * 1000003 + index)
        path = out / f"input-{index}"
        deadline = time.time() + a.seconds
        while time.time() < deadline:
            data = fuzz.mutate(rnd, "parse", rnd.choice(seeds))
            path.write_bytes(data)
            if a.tiers:
                check_tiers(findings, robigo, path, data, env)
            else:
                check_oracle(findings, robigo, rscript, path, data, env)
        path.unlink(missing_ok=True)

    with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool:
        list(pool.map(worker, range(a.jobs)))

    crashed = findings.report()
    sys.exit(1 if (crashed or (a.fail_on_diff and findings.diffs)) else 0)


if __name__ == "__main__":
    main()
