#!/usr/bin/env python3
"""Mutation fuzzing of the code that reads untrusted bytes.

    scripts/fuzz.py parse [--seconds N] [--jobs N] [--robigo BIN] [--seed N]
    scripts/fuzz.py rds   [--seconds N] [--jobs N] [--robigo BIN] [--seed N]

`parse` mutates the conformance scripts and runs them through parse();
`rds` mutates an uncompressed saveRDS() file and runs it through readRDS().
GNU R answers such input with an R error at worst, so robigo may never end
the process instead: a Rust panic, a failed allocation, a stack overflow or
a fatal signal is a finding. Each finding is one bucket with one reproducer
under .robigo/fuzz/<target>/, and the script exits 1 if there are any.
Inputs that merely run past the timeout are reported but do not fail.
"""

import argparse
import collections
import hashlib
import json
import os
import random
import re
import resource
import signal
import subprocess
import sys
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ENV = {"PATH": "/usr/bin:/bin", "LC_ALL": "C.UTF-8", "R_DEFAULT_PACKAGES": "NULL", "HOME": "/tmp", "TZ": "UTC"}
for name in ("R_HOME", "ROBIGO_NATIVES", "ROBIGO_RDBFLAT"):
    if name in os.environ:
        ENV[name] = os.environ[name]
PROGRAMS = {
    "parse": 'invisible(tryCatch(parse(file = Sys.getenv("FUZZ_FILE"), keep.source = FALSE), error = function(e) NULL))',
    "rds": 'invisible(tryCatch(readRDS(Sys.getenv("FUZZ_FILE")), error = function(e) NULL))',
}
RDS_SEED = """saveRDS(list(1:10, c(1.5, NA), c(a = "x", b = NA), TRUE, NULL, list(p = 1, q = list(2)),
    factor(c("u", "v")), data.frame(a = 1:3, b = c("x", "y", "z")), function(x, y = 2) x + y * 3,
    quote(a + b(c)), y ~ x, as.environment(list(k = 1)), matrix(1:6, 2), 1i, as.raw(1:3)),
    Sys.getenv("FUZZ_FILE"), compress = FALSE)"""
TOKENS = [b"(", b")", b"{", b"}", b"[[", b"]]", b"\\(x)", b"|>", b"%%", b"`", b"'", b'"', b"\n", b"\\u{1F600}",
          b'r"---(', b"0x", b"1e", b"L", b"function", b"if", b"else", b"->>", b":=", b"\xff", b"\x00", b"#line 1",
          b"??", b"@", b"::", b"=>"]
WORDS = [b"\xff\xff\xff\xff", b"\x7f\xff\xff\xff", b"\x00\x00\x00\x00", b"\x00\x10\x00\x00"]
MEMORY_LIMIT = 6 << 30
TIMEOUT = 20


def mutate(rnd, target, data):
    b = bytearray(data)
    for _ in range(rnd.randint(1, 8)):
        if not b:
            b = bytearray(b"x")
        kind, p = rnd.randrange(6), rnd.randrange(len(b))
        if kind == 0:
            b[p] ^= 1 << rnd.randrange(8)
        elif kind == 1:
            b[p] = rnd.choice([0, 1, 0x7f, 0x80, 0xff])
        elif kind == 2:
            del b[p:p + rnd.randint(1, 64)]
        elif kind == 3:
            n = rnd.randint(1, 64)
            b[p:p] = b[p:p + n] * rnd.randint(1, 4)
        elif kind == 4 and target == "parse":
            b[p:p] = rnd.choice(TOKENS) * rnd.choice([1, 1, 1, 3, 50, 3000])
        elif kind == 5 and target == "rds":
            q = p & ~3
            b[q:q + 4] = rnd.choice(WORDS)
    return bytes(b)


def limits():
    resource.setrlimit(resource.RLIMIT_AS, (MEMORY_LIMIT, MEMORY_LIMIT))
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))


def classify(returncode, stderr):
    panic = re.search(r"panicked at ([^\n]+)", stderr)
    if panic:
        return "panic " + panic.group(1)
    if "overflowed its stack" in stderr:
        return "stack overflow"
    if "memory allocation of" in stderr:
        return "allocation failure"
    if returncode < 0:
        return "signal " + signal.Signals(-returncode).name
    return None


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("target", choices=sorted(PROGRAMS))
    ap.add_argument("--seconds", type=int, default=60)
    ap.add_argument("--jobs", type=int, default=os.cpu_count())
    ap.add_argument("--robigo", default=str(ROOT / ".robigo/bin/robigo"))
    ap.add_argument("--seed", type=int, default=0)
    a = ap.parse_args()
    robigo = str(Path(a.robigo).resolve())
    out = ROOT / ".robigo/fuzz" / a.target
    out.mkdir(parents=True, exist_ok=True)
    if a.target == "parse":
        seeds = [p.read_bytes()[:65536] for p in sorted((ROOT / "tests/r").rglob("*.R"))]
    else:
        path = out / "seed.rds"
        subprocess.run([robigo, "--vanilla", "-e", RDS_SEED], env={**ENV, "FUZZ_FILE": str(path)}, check=True)
        seeds = [path.read_bytes()]

    lock = threading.Lock()
    counts = collections.Counter()
    buckets = {}
    deadline = time.time() + a.seconds

    def record(key, data, stderr):
        with lock:
            counts[key] += 1
            if key not in buckets:
                name = hashlib.sha1(key.encode()).hexdigest()[:10]
                buckets[key] = name
                (out / f"crash-{name}").write_bytes(data)
                (out / f"crash-{name}.err").write_text(stderr[:3000])

    def worker(index):
        rnd = random.Random(a.seed * 1000003 + index)
        path = out / f"input-{index}"
        while time.time() < deadline:
            data = mutate(rnd, a.target, rnd.choice(seeds))
            path.write_bytes(data)
            try:
                p = subprocess.run([robigo, "--vanilla", "-e", PROGRAMS[a.target]], env={**ENV, "FUZZ_FILE": str(path)},
                                   capture_output=True, timeout=TIMEOUT, preexec_fn=limits)
            except subprocess.TimeoutExpired:
                with lock:
                    counts["(timeout)"] += 1
                continue
            with lock:
                counts["(runs)"] += 1
            stderr = p.stderr.decode("utf-8", "replace")
            key = classify(p.returncode, stderr)
            if key:
                record(key, data, stderr)
        path.unlink(missing_ok=True)

    with ThreadPoolExecutor(a.jobs) as pool:
        list(pool.map(worker, range(a.jobs)))
    (out / "buckets.json").write_text(json.dumps(buckets, indent=1))
    print(f"{a.target}: {counts['(runs)']} runs, {counts['(timeout)']} timeouts, {len(buckets)} findings")
    for key, name in buckets.items():
        print(f"  {counts[key]:6d}  {key}  ({out / ('crash-' + name)})")
    sys.exit(1 if buckets else 0)


if __name__ == "__main__":
    main()
