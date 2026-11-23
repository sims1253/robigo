#!/usr/bin/env python3
"""Hostile byte code through robigo-rt (issue #976): every start must end as
R code ends, whatever the byte code it is handed.

tests/bc-mutate/gen.R mutates the code vectors of compiler::cmpfun'd
closures, makes them closures again (.Internal(mkCode), .Internal(bcClose))
and calls them. robigo-rt runs it under the rdb boot. A start passes when it
ends by exit status 0 or 1 (done, or an R error), or when it is still running
at the limit (a mutation can make a well-formed endless loop) and a SIGINT
then ends it within GRACE seconds, as an R loop's poll does. It fails on a
signal, a Rust panic or abort, or a start that SIGINT does not end (a loop of
robigo-rt's own that never polls: a translator, verifier or VM loop).

Every start runs under an address-space cap (RLIMIT_AS, set here, not left
to the caller), checked in force before the first: hostile byte code once
made robigo-rt allocate 0.8 GB a second (corpus entry 300014 4), and the
machine is shared.

    scripts/rt-bc-mutate.py --robigo-rt BIN                  # the corpus, then the random runs
    scripts/rt-bc-mutate.py --robigo-rt BIN --corpus-only
    scripts/rt-bc-mutate.py --robigo-rt BIN --processes 60 --batch 100 --seed 3 --report-only

The corpus (tests/bc-mutate/corpus.txt) names mutations by seed and number:
each is replayed as it was found, mutations 1 to the number in one start
(alone, none of them crashes: what an earlier mutation's run left behind,
such as a translation kept on a constant both codes share, is part of the
case).

One start runs tests/bc-mutate/pool.R under gctorture, with POOL_LIMIT: it
stops compiles midway (malformed byte code, the compile's depth bound) and
then compiles correct code, which must give its values. A compile's buffers
come from the isolate's pool (#988), so an aborted compile that gave them
back with an object in them shows there.
"""
import argparse
import concurrent.futures
import hashlib
import os
import resource
import signal
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GEN = os.path.join(ROOT, "tests", "bc-mutate", "gen.R")
CORPUS = os.path.join(ROOT, "tests", "bc-mutate", "corpus.txt")
# compiles stopped midway (malformed byte code, the depth bound) and then
# correct ones, under gctorture: the compile buffers' pool given back clean
POOL = os.path.join(ROOT, "tests", "bc-mutate", "pool.R")
# the generator the corpus was found with: its entries name mutations of
# this file, so a change to it needs the corpus found again
GEN_SHA256 = "22dd95c23959f53435903e06596251c43170e4c27b5d3564689e80c1acfc5a2f"

MEM_CAP = 2 << 30
# how long a start may run before the SIGINT, and how soon it must then end
LIMIT = 30
GRACE = 3
POOL_LIMIT = 120


def cap_memory():
    resource.setrlimit(resource.RLIMIT_AS, (MEM_CAP, MEM_CAP))


def env_for(checkout, home, **extra):
    env = dict(os.environ, ROBIGO_RT_BOOT="rdb", R_DEFAULT_PACKAGES="NULL", ROBIGO_RT_CHECKOUT=checkout,
               HOME=home, LANGUAGE="en", LC_ALL="C.UTF-8", **extra)
    env.pop("R_HOME", None)
    return env


def check_cap(rt, checkout, home):
    """Fail unless the cap is in force: an allocation past it is an R error."""
    p = subprocess.run([rt, "--vanilla", "-e", f"x <- numeric({2 * MEM_CAP // 8})"], cwd=home,
                       env=env_for(checkout, home), capture_output=True, timeout=120, preexec_fn=cap_memory)
    if p.returncode != 1 or b"cannot allocate" not in p.stderr:
        sys.exit(f"rt-bc-mutate: the memory cap is not in force (exit {p.returncode}): "
                 f"{p.stderr.decode(errors='replace')[-300:]}")


def start(rt, checkout, home, limit, script=GEN, **extra):
    """One start of the generator: (outcome, detail). Outcomes: "done",
    "R error", "interrupted" (at the limit, ended by SIGINT), or a failure:
    "signal N", "panic", "exit N", "not interruptible"."""
    p = subprocess.Popen([rt, "--vanilla", script], cwd=home, env=env_for(checkout, home, **extra),
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE, preexec_fn=cap_memory)
    try:
        out, err = p.communicate(timeout=limit)
    except subprocess.TimeoutExpired:
        p.send_signal(signal.SIGINT)
        try:
            out, err = p.communicate(timeout=GRACE)
            # (pool.R must finish: a well-formed program, not a mutation)
            return ("pool.R past the limit" if script == POOL else "interrupted"), ""
        except subprocess.TimeoutExpired:
            p.kill()
            out, err = p.communicate()
            return "not interruptible", last_mutation(err)
    if b"panicked at" in err or p.returncode == 101:
        return "panic", last_mutation(err) + ": " + tail(err, 400)
    if p.returncode < 0:
        return f"signal {-p.returncode}", last_mutation(err) + ": " + tail(err, 300)
    if p.returncode == 0 and (b"pool ok" if script == POOL else b"done") in out:
        return "done", ""
    if p.returncode == 1 and script != POOL:
        return "R error", ""
    return f"exit {p.returncode}", last_mutation(err) + ": " + tail(err, 300)


def last_mutation(err):
    m = [l for l in err.decode(errors="replace").splitlines() if l.startswith("mutation ")]
    return m[-1] if m else "before any mutation"


def tail(err, n):
    """`err`'s end without the generator's "mutation K" lines."""
    lines = [l for l in err.decode(errors="replace").splitlines() if l.strip() and not l.startswith("mutation ")]
    return " | ".join(lines)[-n:]


OK = ("done", "R error", "interrupted")


def corpus():
    out = []
    with open(CORPUS) as f:
        for line in f:
            line = line.split("#", 1)[0].split()
            if line:
                out.append((int(line[0]), int(line[1]), line[2] if len(line) > 2 else ""))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--robigo-rt", required=True)
    ap.add_argument("--checkout", default=ROOT, help="the checkout whose .robigo trees robigo-rt boots from")
    ap.add_argument("--corpus-only", action="store_true")
    ap.add_argument("--processes", type=int, default=24, help="random starts after the corpus")
    ap.add_argument("--batch", type=int, default=100, help="mutations per random start")
    ap.add_argument("--seed", default="1",
                    help="random start p's seed is SEED * 100000 + p; 'auto': from GITHUB_RUN_ID, else today's "
                         "date (a search, the nightly's: printed first, so a failure can be replayed)")
    ap.add_argument("--failures-out", help="write each failing random start as a corpus line (seed, mutation, what)")
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--limit", type=int, default=LIMIT)
    ap.add_argument("--report-only", action="store_true", help="report failures without failing")
    a = ap.parse_args()
    with open(GEN, "rb") as f:
        sha = hashlib.sha256(f.read()).hexdigest()
    if sha != GEN_SHA256:
        sys.exit(f"rt-bc-mutate: tests/bc-mutate/gen.R changed (sha256 {sha}): its corpus names mutations of the "
                 "old generator; find the corpus again with the new one and update GEN_SHA256")
    if a.seed == "auto":
        import time
        src = os.environ.get("GITHUB_RUN_ID") or time.strftime("%Y%m%d")
        # (a start's seed, SEED * 100000 + p, stays an R integer)
        a.seed = int(src) % 20000
        print(f"rt-bc-mutate: seed {a.seed} (from {'GITHUB_RUN_ID' if os.environ.get('GITHUB_RUN_ID') else 'the date'} {src})")
    else:
        a.seed = int(a.seed)
    rt = os.path.abspath(a.robigo_rt)
    # the binary measured, named in the report (the commit its build.rs
    # embedded, and its sha256: an edit not committed shows only there)
    p = subprocess.run([os.path.join(ROOT, "scripts", "rt-builder.sh"), rt], capture_output=True, text=True)
    if p.returncode != 0:
        sys.exit(f"rt-bc-mutate: {p.stderr.strip()}")
    who = dict(l.split(": ", 1) for l in p.stdout.splitlines() if ": " in l)
    print(f"rt-bc-mutate: robigo-rt at commit {who.get('commit', '?')}, sha256 {who.get('sha256', '?')[:16]}, {rt}")
    runs = [("compile buffers after aborted compiles, under gctorture (tests/bc-mutate/pool.R)",
             {"script": POOL, "ROBIGO_GCTORTURE": "1"})]
    for seed, k, was in corpus():
        runs.append((f"corpus {seed} {k} ({was})", {"BCM_SEED": str(seed), "BCM_BATCH": str(k)}))
    if not a.corpus_only:
        for p in range(a.processes):
            s = a.seed * 100000 + p
            runs.append((f"random {s} x{a.batch}", {"BCM_SEED": str(s), "BCM_BATCH": str(a.batch)}))
    with tempfile.TemporaryDirectory(prefix="rt-bc-mutate-") as home:
        check_cap(rt, a.checkout, home)
        with concurrent.futures.ThreadPoolExecutor(a.jobs) as ex:
            results = list(ex.map(lambda r: (r[0], *start(rt, a.checkout, home,
                                                          POOL_LIMIT if r[1].get("script") == POOL else a.limit,
                                                          **r[1])), runs))
    counts = {}
    for _, outcome, _ in results:
        counts[outcome] = counts.get(outcome, 0) + 1
    bad = [(n, o, d) for n, o, d in results if o not in OK]
    print(f"rt-bc-mutate: {len(results)} starts ({len(runs) - 1 - (0 if a.corpus_only else a.processes)} from the "
          "corpus, 1 of pool.R): "
          + ", ".join(f"{k} {v}" for k, v in sorted(counts.items())))
    for n, o, d in bad[:int(os.environ.get("BCM_SHOW", "10"))]:
        print(f"  {n}: {o}, {d}")
    if a.failures_out and bad:
        # a random start's failure as tests/bc-mutate/corpus.txt has them:
        # its seed, the mutation it was at, what happened
        with open(a.failures_out, "a") as f:
            for n, o, d in bad:
                parts = n.split()
                if parts[0] == "random":
                    k = d.split(":", 1)[0].removeprefix("mutation ").strip() if d.startswith("mutation ") else "?"
                    f.write(f"{parts[1]} {k} {o.replace(' ', '-')}\n")
        print(f"rt-bc-mutate: failures written to {a.failures_out}")
    return 1 if bad and not a.report_only else 0


if __name__ == "__main__":
    sys.exit(main())
