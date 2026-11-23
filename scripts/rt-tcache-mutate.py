#!/usr/bin/env python3
"""The translation cache as untrusted input (C1, crates/robigo-rt/src/tcache.rs).

Copies this checkout's rdb tree and its cache (.robigo/rt-home-db and
.robigo/rt-home-db-cache) into a scratch checkout, corrupts one package's
cache file at random positions (bit flips, overwritten bytes, a cut end), and
starts robigo-rt from the copied tree (ROBIGO_RT_CHECKOUT, which points the
binary at a checkout's whole tree: the cache is read only beside R_HOME). Every start must end as a start ends: by exit
status 0 or 1 (an R error), never by a signal or a Rust panic. Damage, which
breaks an entry's keyed checksum, makes that entry a miss: it is translated as
with no cache. A start that does not end in START_LIMIT (30 s) counts as failed
too. --rehash fixes the checksum after damaging an entry, which is forgery,
not damage: a forged entry carries no safety guarantee (the cache is trusted
as the tree's base.rdb is; issue #976 is the verifier work that would add
one). So in that mode a crash or a panic is reported, not a failure of the
cache's contract. The script's own output is the count of starts by
outcome.

    scripts/rt-tcache-mutate.py --robigo-rt BIN [--runs N] [--seed S] [--pkg base [--pkg compiler]] [--swap-runs N]
"""

import argparse
import os
import random
import resource
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# Every start's address space is capped, by this script for each start it
# makes (not left to the caller): mutated input that makes robigo-rt
# allocate without end (a forged entry, or hostile byte code: #976's seed
# 300014 grew by 0.8 GB a second) then ends in an R allocation error, not
# in the shared machine's OOM killer. robigo-rt starts in well under 1 GB.
MEM_CAP = 2 << 30
# A start's limit: a start takes a fraction of a second, so one past this is
# a hang. In the damage modes the first hang ends the run with its report,
# so the script reports well before rt-ci's job could be stopped (the step
# has no limit of its own below the job's).
START_LIMIT = 30


def cap_memory():
    resource.setrlimit(resource.RLIMIT_AS, (MEM_CAP, MEM_CAP))


def start(cmd, home, env):
    """One robigo-rt start under the memory cap, its output captured; a
    start that does not end in START_LIMIT seconds raises TimeoutExpired."""
    return subprocess.run(cmd, cwd=home, env=env, capture_output=True, timeout=START_LIMIT, preexec_fn=cap_memory)


def check_cap(cmd, home, env):
    """Fail unless the cap is in force: an allocation of more than it must
    be an R error."""
    p = start(cmd[:2] + ["-e", f"x <- numeric({2 * MEM_CAP // 8})"], home, dict(env, ROBIGO_TRANSLATION_CACHE="0"))
    if p.returncode != 1 or b"cannot allocate" not in p.stderr:
        sys.exit(f"rt-tcache-mutate: the memory cap is not in force (exit {p.returncode}): "
                 f"{p.stderr.decode(errors='replace')[-300:]}")

# a start that runs translated base code: closures, promises, a JIT compile
SCRIPT = """
f <- function(x, y = x * 2) { s <- 0; for (i in seq_len(x)) s <- s + i * y; s }
print(f(10)); print(f(3, 1))
print(paste0("a", 1:3, collapse = "+"))
print(sapply(list(1:3, 4:6), function(v) sum(v^2)))
g <- compiler::cmpfun(function(a) if (a > 1) a else -a)
print(g(-2))
print(tryCatch(stop("boom"), error = function(e) conditionMessage(e)))
"""


def mutate(data, rng):
    """One corruption of `data` (bytes): a bit flip, a few overwritten
    bytes, or a cut end."""
    b = bytearray(data)
    kind = rng.choice(["flip", "flip", "bytes", "cut"])
    if kind == "flip":
        i = rng.randrange(len(b))
        b[i] ^= 1 << rng.randrange(8)
    elif kind == "bytes":
        i = rng.randrange(len(b))
        for j in range(i, min(len(b), i + rng.randrange(1, 9))):
            b[j] = rng.randrange(256)
    else:
        b = b[: rng.randrange(len(b))]
    return bytes(b), kind


MASK = (1 << 64) - 1


def checksum(b):
    """format.rs's `checksum`: a 64-bit multiply-rotate hash (a row's is over its
    key, the row's first 20 bytes, then the entry's data: `keyed_checksum`)."""
    k = 0x9E3779B97F4A7C15
    h = (len(b) * k) & MASK
    n = len(b) // 8 * 8
    for i in range(0, n, 8):
        h = ((h ^ int.from_bytes(b[i:i + 8], "little")) * k) & MASK
        h = ((h << 29) | (h >> 35)) & MASK
    h = ((h ^ int.from_bytes(b[n:].ljust(8, b"\0"), "little")) * k) & MASK
    return h ^ (h >> 32)


def rows(data):
    """The entry table: per entry (row offset, data offset, data length)."""
    at = 8
    at += 1 + data[at]
    at += 4 + int.from_bytes(data[at:at + 4], "little")
    n = int.from_bytes(data[at:at + 4], "little")
    at += 4
    out = []
    for i in range(n):
        r = at + i * 36
        out.append((r, int.from_bytes(data[r + 20:r + 24], "little"), int.from_bytes(data[r + 24:r + 28], "little")))
    return out


def mutate_rehashed(data, table, rng):
    """One entry's data damaged, its checksum made to match again: the
    entry is then decoded and checked, as hostile input would be."""
    b = bytearray(data)
    r, at, n = rng.choice(table)
    kind = rng.choice(["flip", "flip", "bytes"])
    if kind == "flip":
        i = at + rng.randrange(n)
        b[i] ^= 1 << rng.randrange(8)
    else:
        i = at + rng.randrange(n)
        for j in range(i, min(at + n, i + rng.randrange(1, 9))):
            b[j] = rng.randrange(256)
    b[r + 28:r + 36] = checksum(bytes(b[r:r + 20]) + bytes(b[at:at + n])).to_bytes(8, "little")
    return bytes(b), kind + " rehashed"


def swap_runs(runs, rng, tmp, home, env, cmd):
    """--swap: swap two records of equal stored length that both have
    entries, start, and require the start to see stale records (a miss for
    each entry of the swapped records) and to end as a start ends."""
    rdb = os.path.join(tmp, ".robigo", "rt-home-db", "library", "base", "R", "base.rdb")
    keys = start(cmd[:2] + ["-e", 'k <- readRDS(file.path(R.home(), "library/base/R/base.rdx"))$variables; '
                            'for (x in k) cat(x[1], x[2], "\\n")'],
                 home, dict(env, ROBIGO_TRANSLATION_CACHE="0"))
    if keys.returncode != 0:
        sys.exit(f"rt-tcache-mutate: listing base.rdb's records failed (exit {keys.returncode}): "
                 f"{keys.stderr.decode(errors='replace')[-300:]}")
    recs = sorted({tuple(map(int, l.split())) for l in keys.stdout.decode().splitlines() if l.strip()})
    cache = open(os.path.join(tmp, ".robigo", "rt-home-db-cache", "base.rtc"), "rb").read()
    keyed = {int.from_bytes(cache[r:r + 8], "little") for r, _, _ in rows(cache)}
    by_len = {}
    for off, n in recs:
        if off in keyed:
            by_len.setdefault(n, []).append(off)
    pairs = [(v[i], v[j], n) for n, v in by_len.items() for i in range(len(v)) for j in range(i + 1, len(v))]
    if not pairs:
        sys.exit("rt-tcache-mutate: no two keyed records of equal length to swap")
    orig = open(rdb, "rb").read()
    stale_runs, bad = 0, []
    for k in range(runs):
        x, y, n = rng.choice(pairs)
        b = bytearray(orig)
        b[x:x + n], b[y:y + n] = orig[y:y + n], orig[x:x + n]
        open(rdb, "wb").write(bytes(b))
        try:
            p = start(cmd, home, dict(env, ROBIGO_TRANSLATION_CACHE_STATS="1"))
        except subprocess.TimeoutExpired:
            bad.append((k, f"a hang ({START_LIMIT} s)"))
            break
        err = p.stderr.decode(errors="replace")
        if p.returncode < 0 or p.returncode > 1 or "panicked" in err:
            bad.append((k, f"exit {p.returncode}: {err[-300:]}"))
        line = [l for l in err.splitlines() if l.startswith("translation cache: hit ")]
        if line and " stale record 0" not in line[-1]:
            stale_runs += 1
    open(rdb, "wb").write(orig)
    print(f"rt-tcache-mutate: {runs} starts with two records of base.rdb swapped ({len(pairs)} pairs to choose "
          f"from): {stale_runs} saw stale records, crashed {len(bad)}")
    for k, why in bad[:5]:
        print(f"  run {k}: {why}")
    if stale_runs == 0:
        print("  no start saw a stale record: the swap reached no record a start fetches (raise --runs)")
    return 1 if bad or stale_runs == 0 else 0


def damage_runs(a, pkg, rng, tmp, home, env, cmd, ref):
    """--runs starts, each with `pkg`'s cache file damaged at random
    (--rehash: forged); the file is put back after. 0 when every start ended
    as a start ends (always, for forgery, which only reports)."""
    cache = os.path.join(tmp, ".robigo", "rt-home-db-cache", f"{pkg}.rtc")
    with open(cache, "rb") as f:
        data = f.read()
    table = rows(data)
    # the Python mirror of format.rs's checksum, checked against the real
    # cache (not an assert: `python -O` would drop it)
    if not all(checksum(data[r:r + 20] + data[at:at + n]) == int.from_bytes(data[r + 28:r + 36], "little")
               for r, at, n in table[:50]):
        sys.exit("rt-tcache-mutate: the checksum does not match format.rs's")
    outcomes = {"same": 0, "other output": 0, "R error": 0, "hang": 0}
    bad = []
    try:
        for n in range(a.runs):
            m, kind = mutate_rehashed(data, table, rng) if a.rehash else mutate(data, rng)
            with open(cache, "wb") as f:
                f.write(m)
            try:
                p = start(cmd, home, env)
            except subprocess.TimeoutExpired:
                if a.rehash:
                    outcomes["hang"] += 1
                    continue
                # (the first hang ends the run: its report now)
                bad.append((n, kind, f"a hang ({START_LIMIT} s)", ""))
                break
            err = p.stderr.decode(errors="replace")
            if p.returncode < 0 or p.returncode > 1 or "panicked" in err:
                bad.append((n, kind, p.returncode, err[-600:]))
            elif p.returncode == 1:
                outcomes["R error"] += 1
            elif p.stdout == ref.stdout:
                outcomes["same"] += 1
            else:
                outcomes["other output"] += 1
    finally:
        with open(cache, "wb") as f:
            f.write(data)
    print(f"rt-tcache-mutate: {a.runs} corrupted {pkg}.rtc (seed {a.seed}): "
          + ", ".join(f"{k} {v}" for k, v in outcomes.items()) + f", crashed {len(bad)}")
    for n, kind, rc, err in bad[:5]:
        print(f"  run {n} ({kind}): exit {rc}\n{err}")
    # forgery (--rehash) has no guarantee to fail: its crashes are reported
    return 1 if bad and not a.rehash else 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--robigo-rt", required=True)
    ap.add_argument("--pkg", action="append", help="the package whose cache is damaged (again for more; default base)")
    ap.add_argument("--runs", type=int, default=200, help="damaged starts per package")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--swap-runs", type=int, default=0,
                    help="then this many starts with two records of the same stored length swapped in the copy's "
                         "base.rdb, the cache as it is (each key position then holds another, well-formed "
                         "function): every entry for them must miss as a stale record, never run")
    ap.add_argument("--swap", action="store_true", help="only the swapped starts (--runs of them), no damage")
    ap.add_argument("--rehash", action="store_true",
                    help="damage one entry and fix its checksum: forgery, not damage. A forged entry has no "
                         "safety guarantee (the cache is trusted as base.rdb is, issue #976), so this mode "
                         "reports crashes, panics and hangs without failing")
    a = ap.parse_args()
    pkgs = a.pkg or ["base"]
    tree = os.path.join(ROOT, ".robigo", "rt-home-db")
    rng = random.Random(a.seed)
    env = dict(os.environ, ROBIGO_RT_BOOT="rdb", R_DEFAULT_PACKAGES="NULL")
    env.pop("R_HOME", None)
    with tempfile.TemporaryDirectory(prefix="rt-tcache-mutate-") as tmp:
        shutil.copytree(tree, os.path.join(tmp, ".robigo", "rt-home-db"), symlinks=True)
        shutil.copytree(tree + "-cache", os.path.join(tmp, ".robigo", "rt-home-db-cache"))
        home = os.path.join(tmp, "home")
        os.mkdir(home)
        env["ROBIGO_RT_CHECKOUT"] = tmp
        env["HOME"] = home
        script = os.path.join(tmp, "s.R")
        with open(script, "w") as f:
            f.write(SCRIPT)
        cmd = [a.robigo_rt, "--vanilla", script]
        check_cap(cmd, home, env)
        ref = start(cmd, home, env)
        if ref.returncode != 0:
            sys.exit(f"rt-tcache-mutate: the unmutated start failed:\n{ref.stderr.decode(errors='replace')}")
        # the unmutated start must use the cache: a run that never reads it
        # (a stale tree, another build's binary) would prove nothing
        stats = start(cmd, home, dict(env, ROBIGO_TRANSLATION_CACHE_STATS="1")).stderr.decode(errors="replace")
        hit = [l for l in stats.splitlines() if l.startswith("translation cache: hit ")]
        if not hit or hit[-1].startswith("translation cache: hit 0,"):
            sys.exit(f"rt-tcache-mutate: the unmutated start does not use the cache (rebuild the tree with this "
                     f"binary: scripts/rt-home-db.sh): {hit[-1] if hit else stats[-300:]}")
        if a.swap:
            return swap_runs(a.runs, rng, tmp, home, env, cmd)
        status = 0
        for pkg in pkgs:
            status |= damage_runs(a, pkg, rng, tmp, home, env, cmd, ref)
            if status:
                return status
        if a.swap_runs:
            status |= swap_runs(a.swap_runs, rng, tmp, home, env, cmd)
        return status


if __name__ == "__main__":
    sys.exit(main())
