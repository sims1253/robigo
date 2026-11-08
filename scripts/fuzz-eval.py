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

With --tiers the oracle is skipped: the mutant runs on robigo twice, in the
default tier and with ROBIGO_JIT=0. The VM and the JIT must print the same
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


def oracle_version(rscript):
    try:
        v = subprocess.run([str(rscript), "--vanilla", "-e", "cat(format(getRversion()))"],
                           capture_output=True, text=True, timeout=60).stdout.strip()
    except (OSError, PermissionError, subprocess.TimeoutExpired):
        return ""
    return v.splitlines()[-1] if v else ""


def limits():
    resource.setrlimit(resource.RLIMIT_AS, (MEMORY_LIMIT, MEMORY_LIMIT))
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))


def run(cmd, timeout, env=None):
    try:
        p = subprocess.run(cmd, capture_output=True, timeout=timeout, preexec_fn=limits,
                           env=env or ENV)
        return p.stdout, p.stderr, p.returncode
    except subprocess.TimeoutExpired:
        return None, None, "timeout"


def first_diff(a, b):
    for i, (x, y) in enumerate(zip(a, b)):
        if x != y:
            return i
    return min(len(a), len(b))


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
        version = oracle_version(rscript)
        if version != "4.5.0":
            print(f"oracle {rscript} is R {version or '(unknown)'}, not 4.5.0; refusing (false diffs)", file=sys.stderr)
            sys.exit(2)
        robigo = str(pathlib.Path(a.robigo).resolve())
        if not os.path.exists(robigo):
            print(f"{robigo} is missing; run scripts/build.sh", file=sys.stderr)
            sys.exit(2)
        env = dict(ENV)
        try:
            home = subprocess.run([str(rscript), "RHOME"], capture_output=True, text=True,
                                  timeout=60).stdout.strip()
        except (OSError, PermissionError, subprocess.TimeoutExpired):
            home = ""
        env["R_HOME"] = home.splitlines()[-1] if home else ""

    out = ROOT / ".robigo/fuzz/eval"
    out.mkdir(parents=True, exist_ok=True)
    # Prefer the ratchet's scripts: those behave identically under GNU R and
    # robigo today, so a difference is a new regression, not a known gap.
    markers = ROOT / "tests/r/passing"
    marked = {str(p.relative_to(markers))[:-len(".pass")] for p in markers.rglob("*.pass")}
    scripts = [p for p in sorted((ROOT / "tests/r").rglob("*.R"))
               if str(p.relative_to(ROOT / "tests/r")) in marked]
    if not scripts:
        scripts = sorted((ROOT / "tests/r").rglob("*.R"))
    seeds = [p.read_bytes()[:65536] for p in scripts]
    print(f"oracle {rscript or '-'} (R {version}), {len(seeds)} seed scripts "
          f"(ratcheted: {bool(marked)})")

    lock = threading.Lock()
    counts = Counter()
    diff_buckets = {}
    crash_buckets = {}

    def record(buckets, key, data, stderr, prefix):
        with lock:
            buckets[key] = buckets.get(key, 0) + 1
            if buckets[key] == 1:
                name = hashlib.sha1(str(key).encode()).hexdigest()[:10]
                (out / f"{prefix}-{name}.R").write_bytes(data)
                (out / f"{prefix}-{name}.err").write_text(stderr[:3000])
                return name
        return None

    def worker(index):
        rnd = random.Random(a.seed * 1000003 + index)
        path = out / f"input-{index}"
        deadline = time.time() + a.seconds
        while time.time() < deadline:
            data = fuzz.mutate(rnd, "parse", rnd.choice(seeds))
            path.write_bytes(data)
            if a.tiers:
                r0 = run([robigo, "--vanilla", str(path)], TIMEOUT, env=env)
                r0b = run([robigo, "--vanilla", str(path)], TIMEOUT, env=env)
                r1 = run([robigo, "--vanilla", str(path)], TIMEOUT,
                         env=dict(env, ROBIGO_JIT="0"))
                # crashes are findings even in a run that also timed out:
                # classify every finished result before skipping the rest.
                for r in (r0, r0b, r1):
                    if r[2] == "timeout":
                        continue
                    crash = fuzz.classify(r[2], r[1].decode("utf-8", "replace"))
                    if crash:
                        name = record(crash_buckets, crash, data, r[1].decode("utf-8", "replace"), "crash")
                        if name:
                            print(f"CRASH {crash} ({out / ('crash-' + name + '.R')})", flush=True)
                if r0[2] == "timeout" or r0b[2] == "timeout" or r1[2] == "timeout":
                    with lock:
                        counts["(timeout)"] += 1
                    continue
                # unseeded RNG, the clock or tmpdir names differ run-to-run
                # within one tier: an unstable input is skipped, exactly as
                # against the oracle.
                if (norm(r0[0]), norm(r0[1]), r0[2]) != (norm(r0b[0]), norm(r0b[1]), r0b[2]):
                    with lock:
                        counts["(unstable)"] += 1
                    continue
                got = (norm(r0[0]), norm(r0[1]), r0[2])
                exp = (norm(r1[0]), norm(r1[1]), r1[2])
                stderr_text = r0[1].decode("utf-8", "replace") + r1[1].decode("utf-8", "replace")
                if got == exp:
                    with lock:
                        counts["(ok)"] += 1
                    continue
                which = "out" if got[0] != exp[0] else "err" if got[1] != exp[1] else "status"
                if which == "status":
                    key = ("tier:status", f"{exp[2]}->{got[2]}")
                else:
                    stream = {"out": 0, "err": 1}[which]
                    key = (f"tier:{which}", first_diff(exp[stream], got[stream]))
                with lock:
                    counts["(diff)"] += 1
                with lock:
                    capped = len(diff_buckets) >= a.max_diffs and key not in diff_buckets
                if not capped:
                    name = record(diff_buckets, key, data, stderr_text, "tier")
                    if name:
                        print(f"DIFF   {which} tiers differ at {key[1]} ({out / ('tier-' + name + '.R')})", flush=True)
                continue
            o1 = run([str(rscript), "--vanilla", str(path)], TIMEOUT, env=env)
            if o1[2] == "timeout":
                with lock:
                    counts["(oracle timeout)"] += 1
                continue
            o2 = run([str(rscript), "--vanilla", str(path)], TIMEOUT, env=env)
            if o2[2] == "timeout":
                with lock:
                    counts["(oracle timeout)"] += 1
                continue
            exp = (norm(o1[0]), norm(o1[1]), o1[2])
            again = (norm(o2[0]), norm(o2[1]), o2[2])
            if exp != again:
                with lock:
                    counts["(unstable in GNU R)"] += 1
                continue
            r = run([robigo, "--vanilla", str(path)], TIMEOUT, env=env)
            if r[2] == "timeout":
                with lock:
                    counts["(robigo timeout)"] += 1
                continue
            got = (norm(r[0]), norm(r[1]), r[2])
            crash = fuzz.classify(r[2], r[1].decode("utf-8", "replace"))
            stderr_text = r[1].decode("utf-8", "replace") + o1[1].decode("utf-8", "replace")
            if crash:
                name = record(crash_buckets, crash, data, stderr_text, "crash")
                if name:
                    print(f"CRASH {crash} ({out / ('crash-' + name + '.R')})", flush=True)
                continue
            if got != exp:
                which = "out" if got[0] != exp[0] else "err" if got[1] != exp[1] else "status"
                if which == "status":
                    key = (which, f"{exp[2]}->{got[2]}")
                else:
                    stream = {"out": 0, "err": 1}[which]
                    at = first_diff(exp[stream], got[stream])
                    key = (which, at, exp[stream][max(0, at - 40):at + 60])
                with lock:
                    counts["(diff)"] += 1
                with lock:
                    capped = len(diff_buckets) >= a.max_diffs and key not in diff_buckets
                if not capped:
                    name = record(diff_buckets, key, data, stderr_text, "diff")
                    if name:
                        print(f"DIFF   {which} {key[1]} ...{(key[2][:80] if len(key) > 2 else '')!r} ({out / ('diff-' + name + '.R')})", flush=True)
        path.unlink(missing_ok=True)

    with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool:
        list(pool.map(worker, range(a.jobs)))

    (out / "buckets.json").write_text(json.dumps({
        "counts": dict(counts),
        "crash_buckets": crash_buckets,
        "diff_buckets": [{"which": k[0], "at": k[1],
                          "expect": k[2].decode("utf-8", "replace") if len(k) > 2 else "", "n": n}
                         for k, n in sorted(diff_buckets.items(), key=lambda kv: -kv[1])[:50]],
    }, indent=1))
    unstable = counts.get("(unstable in GNU R)", 0) + counts.get("(unstable)", 0)
    timeouts = counts.get("(robigo timeout)", 0) + counts.get("(timeout)", 0) + counts.get("(oracle timeout)", 0)
    print(f"\n{sum(counts.values())} runs: {counts.get('(diff)', 0)} diffs in {len(diff_buckets)} buckets, "
          f"{len(crash_buckets)} crash bucket(s), {unstable} unstable, {timeouts} timeouts")
    for key, n in sorted(crash_buckets.items(), key=lambda kv: -kv[1]):
        print(f"  {n:6d}  crash  {key}")
    for key, n in sorted(diff_buckets.items(), key=lambda kv: -kv[1])[:15]:
        print(f"  {n:6d}  {key[0]}@{key[1]}  {(key[2][:70] if len(key) > 2 else b'')!r}")
    sys.exit(1 if (crash_buckets or (a.fail_on_diff and diff_buckets)) else 0)


if __name__ == "__main__":
    main()
