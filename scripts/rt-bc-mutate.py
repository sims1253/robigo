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

Every start runs under an address-space cap (`ulimit -v` in a shell that
execs it, set here, not left to the caller), checked in force before the
first: hostile byte code once
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

Fixed scripts (FIXED) run once each, with FIXED_LIMIT, and must print
their "ok" line:
- tests/bc-mutate/pool.R, under gctorture: it stops compiles midway
  (malformed byte code, the compile's depth bound) and then compiles
  correct code, which must give its values. A compile's buffers come from
  the isolate's pool (#988), so an aborted compile that gave them back with
  an object in them shows there.
- tests/bc-mutate/or2nd-inline.R: AND2ND/OR2ND with no AND1ST/OR1ST and an
  integer below, called directly and from a compiled caller's loop (an
  inline run), with GNU R 4.6.2's values.
"""
import argparse
import concurrent.futures
import hashlib
import os
import signal
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GEN = os.path.join(ROOT, "tests", "bc-mutate", "gen.R")
CORPUS = os.path.join(ROOT, "tests", "bc-mutate", "corpus.txt")
# the fixed scripts: (title, script, environment, the line it prints when
# it passes)
FIXED = [
    # compiles stopped midway (malformed byte code, the depth bound) and
    # then correct ones, under gctorture: the compile buffers' pool given
    # back clean
    ("compile buffers after aborted compiles, under gctorture (tests/bc-mutate/pool.R)",
     os.path.join(ROOT, "tests", "bc-mutate", "pool.R"), {"ROBIGO_GCTORTURE": "1"}, b"pool ok"),
    # the VM and the inline run keep AND2ND/OR2ND's first operand as bcEval
    ("AND2ND/OR2ND's first operand, run directly and inline (tests/bc-mutate/or2nd-inline.R)",
     os.path.join(ROOT, "tests", "bc-mutate", "or2nd-inline.R"), {}, b"or2nd ok"),
]
FIXED_OK = {script: ok for _, script, _, ok in FIXED}
# the generator the corpus was found with: its entries name mutations of
# this file, so a change to it needs the corpus found again (or a proof that
# it draws the same mutations: layer 1b's BCM_POOL draws from a stream of
# its own, and the code vectors of seeds 1, 7, 300014 and 700001, 200
# mutations each, are the old generator's with it and without it)
GEN_SHA256 = "53a44f973658967ab258f2ca48ec0331be0751e3568c61126e5244934e9ee1d9"

MEM_CAP = 2 << 30
# how long a start may run before the SIGINT, and how soon it must then end
LIMIT = 30
GRACE = 3
FIXED_LIMIT = 120


def capped(cmd):
    """`cmd` under the address-space cap, set by a shell that then execs it
    (`ulimit -v`, in KiB): no `preexec_fn`, which runs Python code between
    fork and exec, where another of the pool's threads may hold a lock the
    child then waits on forever (the subprocess documentation's warning)."""
    return ["/bin/sh", "-c", f'ulimit -v {MEM_CAP // 1024} && exec "$@"', "rt-bc-mutate", *cmd]


def env_for(checkout, home, **extra):
    env = dict(os.environ, ROBIGO_RT_BOOT="rdb", R_DEFAULT_PACKAGES="NULL", ROBIGO_RT_CHECKOUT=checkout,
               HOME=home, LANGUAGE="en", LC_ALL="C.UTF-8", **extra)
    env.pop("R_HOME", None)
    return env


def check_cap(rt, checkout, home):
    """Fail unless the cap is in force: an allocation past it is an R error."""
    p = subprocess.run(capped([rt, "--vanilla", "-e", f"x <- numeric({2 * MEM_CAP // 8})"]), cwd=home,
                       env=env_for(checkout, home), capture_output=True, timeout=120)
    if p.returncode != 1 or b"cannot allocate" not in p.stderr:
        sys.exit(f"rt-bc-mutate: the memory cap is not in force (exit {p.returncode}): "
                 f"{p.stderr.decode(errors='replace')[-300:]}")


def start(rt, checkout, home, limit, script=GEN, ready=None, **extra):
    """One start of the generator: (outcome, detail). Outcomes: "done",
    "R error", "interrupted" (at the limit, ended by SIGINT), or a failure:
    "signal N", "panic", "exit N", "not interruptible". `ready` (the
    tests' fake children): a file the child makes when it is ready for the
    interrupt; the limit counts from then."""
    p = subprocess.Popen(capped([rt, "--vanilla", script]), cwd=home, env=env_for(checkout, home, **extra),
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if ready:
        while not os.path.exists(ready) and p.poll() is None:
            time.sleep(0.01)
    try:
        out, err = p.communicate(timeout=limit)
    except subprocess.TimeoutExpired:
        p.send_signal(signal.SIGINT)
        try:
            out, err = p.communicate(timeout=GRACE)
        except subprocess.TimeoutExpired:
            p.kill()
            out, err = p.communicate()
            return "not interruptible", last_mutation(err)
        # robigo-rt installs its SIGINT handler at the end of its start, as
        # GNU R's setup_Rmainloop does: a start the limit reaches before its
        # first mutation (a boot slowed by load or gctorture) ends by the
        # signal's default action, which is not a finding. After the first
        # mutation the handler is in, and such an end is one.
        if p.returncode == -signal.SIGINT and last_mutation(err) == "before any mutation":
            return (f"{os.path.basename(script)} past the limit" if script in FIXED_OK else "interrupted"), ""
        # ended by the interrupt as R code ends (the error it raises: exit 1,
        # "Execution halted"), or failed while it unwound
        failed = ended(p, err)
        if failed:
            return failed
        # (a fixed script must finish: a well-formed program, not a mutation)
        return (f"{os.path.basename(script)} past the limit" if script in FIXED_OK else "interrupted"), ""
    failed = ended(p, err)
    if failed:
        return failed
    if p.returncode == 0 and FIXED_OK.get(script, b"done") in out:
        return "done", ""
    if p.returncode == 1 and script not in FIXED_OK:
        return "R error", ""
    return f"exit {p.returncode}", last_mutation(err) + ": " + tail(err, 300)


def ended(p, err):
    """A start's failure by how it ended, or None: a Rust panic, a signal,
    or an exit status other than R's 0 and 1."""
    if b"panicked at" in err or p.returncode == 101:
        return "panic", last_mutation(err) + ": " + tail(err, 400)
    if p.returncode < 0:
        return f"signal {-p.returncode}", last_mutation(err) + ": " + tail(err, 300)
    if p.returncode not in (0, 1):
        return f"exit {p.returncode}", last_mutation(err) + ": " + tail(err, 300)
    return None


def last_mutation(err):
    m = [l for l in err.decode(errors="replace").splitlines() if l.startswith("mutation ")]
    return m[-1] if m else "before any mutation"


def tail(err, n):
    """`err`'s end without the generator's "mutation K" lines."""
    lines = [l for l in err.decode(errors="replace").splitlines() if l.strip() and not l.startswith("mutation ")]
    return " | ".join(lines)[-n:]


OK = ("done", "R error", "interrupted")


def failure_lines(bad):
    """--failures-out's lines for the failing starts `bad` ([(title,
    outcome, detail)]), one per failure: a random start's whose mutation is
    known as tests/bc-mutate/corpus.txt has them (seed, mutation, what, and
    "pool" for a start with BCM_POOL), which corpus() reads back; any other
    (a fixed script, a corpus entry, a start that failed before its first
    mutation or said no number) as a comment line, which corpus() skips, so
    the file holds every failure the run saw."""
    out = []
    for n, o, d in bad:
        parts = n.split()
        k = d.split(":", 1)[0].removeprefix("mutation ").strip() if d.startswith("mutation ") else ""
        if parts[0] == "random" and k.isdigit():
            out.append(f"{parts[1]} {k} {o.replace(' ', '-')}" + (" pool" if "pool" in parts[2:] else ""))
        else:
            out.append("# " + f"{n}: {o}, {d}".replace("\n", " "))
    return out


def corpus(path=None):
    """tests/bc-mutate/corpus.txt's entries: (seed, mutation, what, pool),
    pool True for a line that ends in "pool" (the start ran with
    BCM_POOL=1)."""
    out = []
    with open(path or CORPUS) as f:
        for line in f:
            line = line.split("#", 1)[0].split()
            if line:
                out.append((int(line[0]), int(line[1]), line[2] if len(line) > 2 else "",
                            len(line) > 3 and line[3] == "pool"))
    return out


def gen_env(seed, batch, pool):
    """A generator start's environment."""
    # (BCM_POOL set either way: a caller's own BCM_POOL is not inherited)
    return {"BCM_SEED": str(seed), "BCM_BATCH": str(batch), "BCM_POOL": "1" if pool else "0"}


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--robigo-rt", required=True)
    ap.add_argument("--checkout", default=ROOT, help="the checkout whose .robigo trees robigo-rt boots from")
    ap.add_argument("--corpus-only", action="store_true")
    ap.add_argument("--processes", type=int, default=24, help="random starts after the corpus")
    ap.add_argument("--batch", type=int, default=100, help="mutations per random start")
    ap.add_argument("--seed", default="1",
                    help="random start p's seed is SEED * 100000 + p; 'auto': from GITHUB_RUN_ID, else today's "
                         "date (the nightly's search, .github/workflows/nightly.yml rt-bc-mutate: printed first, so a failure can be "
                         "replayed)")
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
    runs = [(title, {"script": script, **env}) for title, script, env, _ in FIXED]
    for seed, k, was, pool in corpus():
        runs.append((f"corpus {seed} {k} ({was}){' pool' if pool else ''}", gen_env(seed, k, pool)))
    if not a.corpus_only:
        for p in range(a.processes):
            s = a.seed * 100000 + p
            # (every other start also mutates the constants slot)
            pool = p % 2 == 1
            runs.append((f"random {s} x{a.batch}{' pool' if pool else ''}", gen_env(s, a.batch, pool)))
    with tempfile.TemporaryDirectory(prefix="rt-bc-mutate-") as home:
        check_cap(rt, a.checkout, home)
        with concurrent.futures.ThreadPoolExecutor(a.jobs) as ex:
            results = list(ex.map(lambda r: (r[0], *start(rt, a.checkout, home,
                                                          FIXED_LIMIT if r[1].get("script") in FIXED_OK else a.limit,
                                                          **r[1])), runs))
    counts = {}
    for _, outcome, _ in results:
        counts[outcome] = counts.get(outcome, 0) + 1
    bad = [(n, o, d) for n, o, d in results if o not in OK]
    print(f"rt-bc-mutate: {len(results)} starts ({len(runs) - len(FIXED) - (0 if a.corpus_only else a.processes)} "
          f"from the corpus, {len(FIXED)} fixed): "
          + ", ".join(f"{k} {v}" for k, v in sorted(counts.items())))
    for n, o, d in bad[:int(os.environ.get("BCM_SHOW", "10"))]:
        print(f"  {n}: {o}, {d}")
    if a.failures_out and bad:
        with open(a.failures_out, "a") as f:
            for line in failure_lines(bad):
                f.write(line + "\n")
        print(f"rt-bc-mutate: failures written to {a.failures_out}")
    return 1 if bad and not a.report_only else 0


if __name__ == "__main__":
    sys.exit(main())
