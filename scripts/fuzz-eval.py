#!/usr/bin/env python3
"""Differential fuzzing: generated R programs run on robigo and GNU R.

    scripts/fuzz-eval.py [--seconds N] [--jobs N] [--robigo BIN] [--seed N]
                         [--oracle RSCRIPT] [--gen mix|ast|template|bytes]
                         [--tiers [--variants N]] [--emi] [--fail-on-diff]
                         [--max-diffs N] [--reduce SECONDS]
    scripts/fuzz-eval.py --bisect FILE.R [--jit-opt OPT]

Inputs (--gen):

  ast       a conformance script mutated on GNU R's parse tree
            (scripts/rast.py): literals, operators, names and subexpressions
            replaced, statements dropped, duplicated or run in a hot loop;
            ~99% of mutants parse;
  template  a program built to stress the VM's and the JIT's assumptions
            (scripts/fuzz_gen.py): types, bindings, methods changed after
            the code is hot;
  bytes     scripts/fuzz.py's byte mutator (~13% of its mutants parse:
            mostly a parser test, which scripts/fuzz.py parse also is);
  mix       (default) ast 45%, template 45%, bytes 10%.

Seeds are `tests/r/**/*.R`, restricted to the ratchet (`tests/r/passing/`)
where markers exist, since those behave identically under GNU R and robigo
today. Every run uses R_DEFAULT_PACKAGES=NULL and --vanilla, like
scripts/regen-expected.sh.

Checks (scripts/rcheck.py):

  (default) each input runs on GNU R 4.5.0 twice and on robigo once; when
            GNU R agrees with itself and robigo's normalized stdout, stderr
            or status differs, that is a candidate conformance gap.
  --tiers   no oracle: the default tier (twice, for stability), the VM
            (ROBIGO_JIT=0) and the interpreter (ROBIGO_COMPILE=0) must print
            the same (docs/architecture.md). The bucket names the tier that
            disagrees with the other two: `jit`, `vm`, `interp`, or `all`.
            When they agree, the JIT tier runs again under --variants more
            ROBIGO_JIT_OPT settings (MATRIX: `eager,verify` always, then a
            random pick of the optimizations switched off one at a time,
            loops or closures alone, and seeded mixed VM/native selections),
            blamed as `jit[OPT]`. `eager` compiles at the first opportunity,
            so small programs reach the closure JIT too; `verify` runs
            Cranelift's verifier and register-allocation checker, whose
            failure is a crash.
  --emi     no oracle (equivalence modulo inputs, Le et al., PLDI'14): the
            input and the input with code that never runs inserted
            (rast.insert_dead) must print the same in the default tier,
            whenever they print the same in the interpreter.

A robigo crash (panic, stack overflow, signal, failed allocation) is a
finding exactly as in scripts/fuzz.py and always fails the run. Timeouts and
unstable inputs are counted but do not fail. Each bucket (what differs and
its first differing lines, scripts/rcheck.py's key) keeps one reproducer in
.robigo/fuzz/eval/ (`<check>-<hash>.R`, with a `.err`; a crash is `crash-`,
or `tier-crash-`/`emi-crash-` when a tier check found it); `--reduce SECONDS`
runs scripts/reduce.py on each new one afterwards (`.min.R`).

--bisect FILE.R names the loop or closure whose native code changes what
FILE prints: the JIT tier with ROBIGO_JIT_OPT=OPT (default `eager`) is
compared with the VM, then each native site of its report alone
(`only=ID`), then halves of them when no single one does it.

Differences are leads, not verdicts: some seeds are inherently flaky and a
diff may repeat a known corpus failure. Verify a lead by reducing it
(scripts/reduce.py) to a `tests/r/<area>/<name>.R` conformance script before
working on it (docs/workstreams.md, protocol rule 8).
"""
import argparse
import concurrent.futures
import hashlib
import json
import os
import pathlib
import random
import subprocess
import sys
import threading
import time
from collections import Counter

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
import fuzz  # noqa: E402
import fuzz_gen  # noqa: E402
import rast  # noqa: E402
import rcheck  # noqa: E402

MIX = [("ast", 45), ("template", 45), ("bytes", 10)]

# ROBIGO_JIT_OPT settings of --tiers (crates/robigo-runtime/src/jit/config.rs);
# {seed} is drawn per input. The first always runs.
MATRIX = ["eager,verify", "eager,verify,-summary", "eager,verify,-inline", "eager,verify,-loops",
          "eager,verify,-closures", "verify", "eager,verify,sample={seed}:0.5", "verify,sample={seed}:0.5",
          "eager,verify,-summary,-inline,sample={seed}:0.3"]


def variants(rnd, n):
    """MATRIX[0] and N - 1 others, seeds filled in (N < 0: all of them)."""
    rest = MATRIX[1:] if n < 0 else rnd.sample(MATRIX[1:], max(0, min(n - 1, len(MATRIX) - 1)))
    picked = ([MATRIX[0]] if n != 0 else []) + rest
    return [v.format(seed=rnd.randrange(1 << 31)) for v in picked]


def bisect(checker, path, opt, jobs):
    """Name the native sites whose code changes what PATH prints (see the
    module doc); exit status 0 when it found them."""
    ref = checker.robigo(path, "vm")
    full, sites = rcheck.split_report(checker.jit(path, f"{opt},report"))
    if rcheck.outcome(full) == rcheck.outcome(ref):
        sys.exit(f"{path}: the JIT tier with ROBIGO_JIT_OPT={opt} prints what the VM prints; nothing to bisect")
    native = [(k, i, label) for k, i, status, detail, label in sites if "runs " in detail]
    print(f"{len(native)} native site(s) of {len(sites)} in the report; JIT and VM differ:")
    print("  " + " | ".join(rcheck.signature(rcheck.outcome(ref), rcheck.outcome(full))))
    if not native:
        sys.exit("no native site: the difference is not native code's (the VM's own optimizations?)")

    def differs(ids):
        r, _ = rcheck.split_report(checker.jit(path, f"{opt},only={':'.join(ids)},report"))
        return rcheck.outcome(r) != rcheck.outcome(ref)

    ids = [i for _, i, _ in native]
    with concurrent.futures.ThreadPoolExecutor(jobs) as pool:
        alone = [i for i, d in zip(ids, pool.map(lambda i: differs([i]), ids)) if d]
    if not alone:
        # an interaction: halve while a half still differs on its own
        cur = ids
        while len(cur) > 1:
            h = len(cur) // 2
            if differs(cur[:h]):
                cur = cur[:h]
            elif differs(cur[h:]):
                cur = cur[h:]
            else:
                break
        alone = cur if differs(cur) else []
        how = "together"
    else:
        how = "each alone"
    if not alone:
        sys.exit("no subset of the native sites reproduces the difference (selection changes it?)")
    print(f"the native code of these site(s) changes the output ({how}):")
    for k, i, label in native:
        if i in alone:
            print(f"  {k:7} {i}  {label}")
    print(f"reproduce: ROBIGO_JIT_OPT={opt},only={':'.join(alone)} robigo {path}")


class Findings:
    """Counts, crash buckets and diff buckets, shared by the worker threads:
    one reproducer per bucket under OUT, at most MAX_DIFFS diff buckets."""

    def __init__(self, out, max_diffs):
        self.out, self.max_diffs = out, max_diffs
        self.lock = threading.Lock()
        self.counts = Counter()
        self.crashes = {}
        self.diffs = {}
        self.new = []  # reproducers written, in order

    def count(self, what):
        with self.lock:
            self.counts[what] += 1

    def record(self, v, data, prefix, extra=None):
        """Bucket verdict V of input DATA; True when it is a crash."""
        if v.kind == "crash":
            self.count("(crash)")
            # named after the check that found it, which reduce.py repeats
            # (a crash in one tier only does not reproduce in the default)
            p = "crash" if prefix == "diff" else f"{prefix}-crash"
            if name := self._record(self.crashes, v.key[1], data, v.detail, p, False, extra):
                print(f"CRASH {v.key[1]} ({self.out / (p + '-' + name + '.R')})", flush=True)
            return True
        if v.kind != "diff":
            self.count(f"({v.kind}{' ' + v.key if isinstance(v.key, str) else ''})")
            return False
        self.count("(diff)")
        if name := self._record(self.diffs, v.key, data, v.detail, prefix, True, extra):
            print(f"DIFF   {' | '.join(map(str, v.key))[:160]} ({self.out / (prefix + '-' + name + '.R')})", flush=True)
        return False

    def _record(self, buckets, key, data, stderr, prefix, cap, extra):
        with self.lock:
            if cap and key not in buckets and len(buckets) >= self.max_diffs:
                return None
            buckets[key] = buckets.get(key, 0) + 1
            if buckets[key] > 1:
                return None
            name = hashlib.sha1(str(key).encode()).hexdigest()[:10]
            (self.out / f"{prefix}-{name}.R").write_bytes(data)
            (self.out / f"{prefix}-{name}.err").write_text(stderr[:3000], encoding="utf-8")
            if extra is not None:
                (self.out / f"{prefix}-{name}.orig.R").write_bytes(extra)
            # a --tiers variant: the ROBIGO_JIT_OPT scripts/reduce.py reruns
            if stderr.startswith("[jit ROBIGO_JIT_OPT="):
                (self.out / f"{prefix}-{name}.opt").write_text(stderr.split("\n", 1)[0][len("[jit ROBIGO_JIT_OPT="):-1])
            self.new.append(self.out / f"{prefix}-{name}.R")
            return name

    def report(self):
        """Write buckets.json, print the summary; True if the run found a crash."""
        c = self.counts
        (self.out / "buckets.json").write_text(json.dumps({
            "counts": dict(c),
            "crash_buckets": self.crashes,
            "diff_buckets": [{"key": list(k), "n": n}
                             for k, n in sorted(self.diffs.items(), key=lambda kv: -kv[1])[:50]],
        }, indent=1))
        runs = sum(n for k, n in c.items() if not k.startswith("(gen "))
        unstable = sum(n for k, n in c.items() if k.startswith("(unstable"))
        timeouts = sum(n for k, n in c.items() if k.startswith("(timeout"))
        gens = ", ".join(f"{k[5:-1]} {n}" for k, n in sorted(c.items()) if k.startswith("(gen "))
        print(f"\n{runs} runs ({gens}): {c['(diff)']} diffs in {len(self.diffs)} buckets, "
              f"{len(self.crashes)} crash bucket(s), {unstable} unstable, {timeouts} timeouts")
        for key, n in sorted(self.crashes.items(), key=lambda kv: -kv[1]):
            print(f"  {n:6d}  crash  {key}")
        for key, n in sorted(self.diffs.items(), key=lambda kv: -kv[1])[:15]:
            print(f"  {n:6d}  {' | '.join(map(str, key))[:150]}")
        return bool(self.crashes)


class Inputs:
    """Generated inputs: `next(rnd)` -> (generator, bytes)."""

    def __init__(self, gen, parser, seeds):
        self.gen, self.parser = gen, parser
        self.raw = [s.read_bytes()[:65536] for s in seeds]
        self.trees = []
        if gen in ("ast", "mix"):
            for s in seeds:
                t = parser.parse(s.read_text(encoding="utf-8", errors="surrogateescape"))
                if t is not None:
                    self.trees.append(t)
            self.pool = [f for t in self.trees for f in rast.fragments(t)]

    def next(self, rnd):
        g = self.gen
        if g == "mix":
            g = rnd.choices([k for k, _ in MIX], [w for _, w in MIX])[0]
        if g == "ast":
            return g, rast.mutate(rnd, rnd.choice(self.trees), self.pool).encode("utf-8", "surrogateescape")
        if g == "template":
            return g, fuzz_gen.program(rnd).encode()
        return g, fuzz.mutate(rnd, "parse", rnd.choice(self.raw))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--seconds", type=int, default=60)
    ap.add_argument("--jobs", type=int, default=os.cpu_count())
    ap.add_argument("--robigo", default=str(ROOT / ".robigo/bin/robigo"))
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--oracle")
    ap.add_argument("--gen", default="mix", choices=["mix", "ast", "template", "bytes"])
    mode = ap.add_mutually_exclusive_group()
    mode.add_argument("--tiers", action="store_true", help="compare robigo's three tiers instead of GNU R")
    mode.add_argument("--emi", action="store_true", help="compare inputs with and without dead code")
    mode.add_argument("--bisect", metavar="FILE", help="name the native code that changes what FILE prints")
    ap.add_argument("--variants", type=int, default=3,
                    help="--tiers: ROBIGO_JIT_OPT settings per input beyond the three tiers (-1: all of MATRIX)")
    ap.add_argument("--jit-opt", default="eager", help="--bisect: the ROBIGO_JIT_OPT of the JIT tier")
    ap.add_argument("--fail-on-diff", action="store_true")
    ap.add_argument("--max-diffs", type=int, default=200)
    ap.add_argument("--reduce", type=int, metavar="SECONDS", help="reduce each new reproducer afterwards")
    a = ap.parse_args()
    # Inherited by every child (and this driver, which stays far below).
    rcheck.limit_resources()

    robigo = pathlib.Path(a.robigo).resolve()
    if not robigo.exists():
        sys.exit(f"{robigo} is missing; run scripts/build.sh")
    env = dict(rcheck.ENV)
    if a.bisect:
        if os.environ.get("R_HOME"):
            env["R_HOME"] = os.environ["R_HOME"]
        bisect(rcheck.Checker(robigo, None, env=env), pathlib.Path(a.bisect).resolve(), a.jit_opt, a.jobs)
        return
    if a.tiers or a.emi:
        rscript, version = None, "tier differential (no oracle)" if a.tiers else "EMI (no oracle)"
        if os.environ.get("R_HOME"):
            env["R_HOME"] = os.environ["R_HOME"]
    else:
        rscript, home = rcheck.require_oracle(a.oracle)
        version = "4.5.0"
        # The oracle's own home, for robigo's base library too: an empty or
        # foreign R_HOME makes GNU R warn on every run.
        env["R_HOME"] = home
    checker = rcheck.Checker(robigo, rscript, env=env)
    # Any GNU R parses (the parse trees of ast mutation and EMI).
    parser = rast.Parser(rscript or rcheck.find_oracle(a.oracle)) if (a.gen in ("ast", "mix") or a.emi) else None

    out = ROOT / ".robigo/fuzz/eval"
    out.mkdir(parents=True, exist_ok=True)
    # Prefer the ratchet's scripts: those behave identically under GNU R and
    # robigo today, so a difference is a new regression, not a known gap.
    markers = ROOT / "tests/r/passing"
    marked = {str(p.relative_to(markers))[:-len(".pass")] for p in markers.rglob("*.pass")}
    scripts = [p for p in sorted((ROOT / "tests/r").rglob("*.R")) if str(p.relative_to(ROOT / "tests/r")) in marked]
    ratcheted = bool(scripts)
    if not ratcheted:
        scripts = sorted((ROOT / "tests/r").rglob("*.R"))
    inputs = Inputs(a.gen, parser, scripts)
    print(f"oracle {rscript or '-'} (R {version}), {len(scripts)} seed scripts (ratcheted: {ratcheted}), "
          f"inputs: {a.gen}")

    findings = Findings(out, a.max_diffs)

    def worker(index):
        rnd = random.Random(a.seed * 1000003 + index)
        path, vpath = out / f"input-{index}.R", out / f"input-{index}.emi.R"
        deadline = time.time() + a.seconds
        while time.time() < deadline:
            g, data = inputs.next(rnd)
            findings.count(f"(gen {g})")
            path.write_bytes(data)
            if a.emi:
                tree = parser.parse(data.decode("utf-8", "surrogateescape"))
                variant = tree and rast.insert_dead(rnd, tree)
                if not variant:
                    findings.count("(no parse)")
                    continue
                vdata = variant.encode("utf-8", "surrogateescape")
                vpath.write_bytes(vdata)
                findings.record(checker.emi(path, vpath), vdata, "emi", extra=data)
            elif a.tiers:
                c = rcheck.Checker(robigo, None, env=env, variants=variants(rnd, a.variants))
                findings.record(c.tiers(path), data, "tier")
            else:
                findings.record(checker.oracle(path), data, "diff")
        path.unlink(missing_ok=True)
        vpath.unlink(missing_ok=True)

    with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool:
        list(pool.map(worker, range(a.jobs)))
    if parser:
        parser.close()

    crashed = findings.report()
    if a.reduce and findings.new:
        print(f"\nreducing {len(findings.new)} reproducer(s), {a.reduce}s each")
        for p in findings.new:
            cmd = [sys.executable, str(ROOT / "scripts/reduce.py"), str(p), "--robigo", str(robigo),
                   "--seconds", str(a.reduce), "--jobs", str(a.jobs)]
            if rscript:
                cmd += ["--oracle", str(rscript)]
            r = subprocess.run(cmd, capture_output=True, text=True)
            status = r.stderr.strip().splitlines()[-1] if r.stderr.strip() else f"exit {r.returncode}"
            print(f"  {p.name}: {status}")
    sys.exit(1 if (crashed or (a.fail_on_diff and findings.diffs)) else 0)


if __name__ == "__main__":
    main()
