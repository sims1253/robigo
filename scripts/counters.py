#!/usr/bin/env python3
"""The counter gate: work that should be zero on a hot path is counted, and
what is counted is held, so a regression of ten times on one path is a red
run at the change that made it, not a figure hidden in a whole benchmark's
instruction total (docs/verification.md, "Counter gate").

    scripts/counters.py build             # -> .robigo/bin/robigo-counters
    scripts/counters.py check [--robigo BIN] [--filter RX] [-v]
    scripts/counters.py check --record    # rewrite tests/counters/known.toml

A counted build (`--features counters`, crates/robigo-runtime/src/counters.rs)
prints its counts at exit: vector copies (copy-on-modify), promises,
environments, allocations, VM ops by kind and JIT outcomes. Each fixture
`tests/counters/NAME.R` is a loop over `n` iterations (the gate defines `n`)
whose header bounds counters per iteration, in every tier or in the tiers
named (`ast`: the interpreter, `vm`: the bytecode VM, `jit`: the VM with the
JIT, as `compiled_matches_interpreted` runs them):

    #| vector_copies <= 0
    #| jit vm_ops <= 0
    #| ast,vm allocs <= 2

The gate runs each fixture in each tier at two lengths and holds the
difference per iteration, so startup and warm-up cancel and the numbers are
exact: the counts are deterministic, so a bound is too.

A bound that fails today is listed in tests/counters/known.toml with the
number measured and its cause (the model is blaze's harness/v2/guards.py): a
listed bound may fail by no more than what is recorded, so it cannot get
worse; one listed that passes fails the run until it is taken out, so the
list only shrinks; one not listed that fails fails the run. `--record`
writes the list again from a run, keeping each entry's cause; it never
raises an entry's number.
"""
import argparse
import concurrent.futures
import fcntl
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "tests" / "counters"
KNOWN = FIXTURES / "known.toml"
BINARY = ROOT / ".robigo" / "bin" / "robigo-counters"
# Two loop lengths: their difference per iteration is the steady state.
LENGTHS = (1000, 3000)
ENV = {"LANGUAGE": "en", "LC_ALL": "C.UTF-8", "TZ": "UTC", "R_ENVIRON_USER": "", "R_PROFILE_USER": "",
       "R_DEFAULT_PACKAGES": "NULL"}
PREFIX = "robigo-counters: "
TIERS = {"ast": {"ROBIGO_COMPILE": "0", "ROBIGO_JIT": "0"}, "vm": {"ROBIGO_COMPILE": "1", "ROBIGO_JIT": "0"},
         "jit": {"ROBIGO_COMPILE": "1", "ROBIGO_JIT": "1"}}
BOUND = re.compile(r"#\|\s*(?:([a-z,]+)\s+)?([A-Za-z_:][A-Za-z0-9_:]*)\s*<=\s*([0-9.]+)\s*$")


@dataclass(frozen=True)
class Fixture:
    name: str
    text: str
    bounds: dict  # (tier, counter) -> most per iteration


@dataclass(frozen=True)
class Known:
    fixture: str
    tier: str
    counter: str
    measured: float
    cause: str

    @property
    def key(self):
        return (self.fixture, self.tier, self.counter)


@dataclass(frozen=True)
class Judged:
    fixture: str
    tier: str
    counter: str
    verdict: str  # pass, fail
    value: float | None
    limit: float
    detail: str


def read_fixture(path):
    text = path.read_text()
    bounds = {}
    for line in text.splitlines():
        if line.startswith("#|"):
            m = BOUND.match(line)
            if not m:
                raise ValueError(f"{path.name}: a bound reads `#| [tiers] counter <= number`, not {line!r}")
            tiers = m.group(1).split(",") if m.group(1) else list(TIERS)
            if set(tiers) - set(TIERS):
                raise ValueError(f"{path.name}: the tiers are {', '.join(TIERS)}, not {m.group(1)!r}")
            for tier in tiers:
                if (tier, m.group(2)) in bounds:
                    raise ValueError(f"{path.name}: `{m.group(2)}` is bounded twice in the {tier} tier")
                bounds[tier, m.group(2)] = float(m.group(3))
    if not bounds:
        raise ValueError(f"{path.name}: no `#| [tiers] counter <= number` bound")
    return Fixture(path.stem, text, bounds)


def fixtures(rx=None):
    return [read_fixture(p) for p in sorted(FIXTURES.glob("*.R")) if not rx or re.search(rx, p.stem)]


def read_known(text, all_fixtures):
    """The list known.toml holds; an entry of no bound, a field missing or of
    another type, and one bound twice are each a ValueError naming it."""
    data = tomllib.loads(text)
    if set(data) - {"known"} or not isinstance(data.get("known", []), list):
        raise ValueError(f"known.toml: its tables are [[known]], not {sorted(data)}")
    bounds = {(f.name, *b) for f in all_fixtures for b in f.bounds}
    out = []
    for at, e in enumerate(data.get("known", [])):
        where = f"known.toml: [[known]] {at + 1}"
        if not isinstance(e, dict) or set(e) != {"fixture", "tier", "counter", "measured", "cause"}:
            raise ValueError(f"{where}: its fields are fixture, tier, counter, measured and cause")
        if isinstance(e["measured"], bool) or not isinstance(e["measured"], (int, float)):
            raise ValueError(f"{where}: `measured` is a number, not {e['measured']!r}")
        if not isinstance(e["cause"], str) or not e["cause"].strip():
            raise ValueError(f"{where}: `cause` says in a line what fails and why, and is empty")
        k = Known(e["fixture"], e["tier"], e["counter"], float(e["measured"]), e["cause"])
        if k.key not in bounds:
            raise ValueError(f"{where}: {k.fixture} [{k.tier}] `{k.counter}` is no bound of tests/counters")
        if any(o.key == k.key for o in out):
            raise ValueError(f"{where}: {k.fixture} [{k.tier}] `{k.counter}` is listed twice")
        out.append(k)
    return out


def render_known(known):
    lines = [
        "# Counter bounds known to fail (scripts/counters.py; docs/verification.md,",
        "# \"Counter gate\"). Each entry is one bound of one fixture in tests/counters,",
        "# the number per iteration measured when it was recorded, and the cause. A",
        "# listed bound may fail by no more than `measured`; one listed that passes",
        "# fails the run until it is taken out; one not listed that fails fails the",
        "# run. The list only shrinks. Written by: scripts/counters.py check --record",
        "",
    ]
    for k in sorted(known, key=lambda k: k.key):
        m = int(k.measured) if k.measured == int(k.measured) else k.measured
        lines += ["[[known]]", f"fixture = {json.dumps(k.fixture)}", f"tier = {json.dumps(k.tier)}",
                  f"counter = {json.dumps(k.counter)}",
                  f"measured = {m}", f"cause = {json.dumps(k.cause)}", ""]
    return "\n".join(lines)


def judge(fixture, tier, per_iter, known):
    """Every bound of `fixture` in `tier` held to `per_iter` (counter ->
    number per iteration), with the known failures allowed as far as recorded."""
    listed = {k.key: k for k in known}
    out = []
    for (t, counter), limit in fixture.bounds.items():
        if t != tier:
            continue
        was = listed.get((fixture.name, tier, counter))
        value = per_iter.get(counter, 0.0 if counter.startswith("op:") else None)

        def judged(verdict, detail):
            return Judged(fixture.name, tier, counter, verdict, value, limit, detail)

        if value is None:
            out.append(judged("fail", "not counted: is the binary built with --features counters?"))
            continue
        said = f"{value:g} per iteration against at most {limit:g}"
        if value <= limit:
            if was is None:
                out.append(judged("pass", said))
            else:
                out.append(judged("fail", f"{said}: it passes and is listed as a known failure ({was.cause}); "
                                  "take it out of tests/counters/known.toml, the list only shrinks"))
        elif was is None:
            out.append(judged("fail", said))
        elif value <= was.measured:
            out.append(judged("pass", f"{said}: a known failure, recorded at {was.measured:g} ({was.cause})"))
        else:
            out.append(judged("fail", f"{said}: a known failure that got worse than the {was.measured:g} "
                              f"recorded ({was.cause})"))
    return out


def recorded(judged, before):
    """The known list a run gives: each bound over its limit, with the number
    measured, keeping the cause of an entry that was there. An entry never
    gets a higher number than it had: one that got worse stays failing."""
    was = {k.key: k for k in before}
    out = []
    for j in judged:
        if j.value is None or j.value <= j.limit:
            continue
        k = was.get((j.fixture, j.tier, j.counter))
        out.append(Known(j.fixture, j.tier, j.counter, j.value if k is None else min(j.value, k.measured),
                         "not yet said" if k is None else k.cause))
    return out


def parse_counts(stderr):
    """The counts of the `robigo-counters:` line a counted build prints at exit."""
    lines = [ln for ln in stderr.splitlines() if ln.startswith(PREFIX)]
    if not lines:
        return None
    counts = json.loads(lines[-1][len(PREFIX):])
    ops = counts.pop("ops", {})
    counts.update({f"op:{k}": v for k, v in ops.items()})
    return counts


def per_iteration(short, long, lengths):
    """Counter -> (long - short) / (difference of the lengths), exact to 1e-6."""
    d = lengths[1] - lengths[0]
    return {k: round((long.get(k, 0) - short.get(k, 0)) / d, 6) for k in set(short) | set(long)}


def run(robigo, fixture, tier, n):
    env = {k: v for k, v in os.environ.items() if not k.startswith("ROBIGO_")}
    env.update(ENV, **TIERS[tier])
    with tempfile.TemporaryDirectory(prefix="robigo-counters-") as tmp:
        f = Path(tmp) / f"{fixture.name}.R"
        f.write_text(f"n <- {n}L\n" + fixture.text)
        p = subprocess.run([robigo, "--vanilla", str(f)], cwd=tmp, env=env, capture_output=True, text=True,
                           timeout=600)
    counts = parse_counts(p.stderr)
    if p.returncode != 0 or counts is None:
        raise RuntimeError(f"{fixture.name} [{tier}] (n = {n}): exit {p.returncode}\n{p.stderr[-2000:]}")
    return counts


def measure(robigo, fxs, lengths=LENGTHS):
    """(fixture name, tier) -> counter -> number per iteration."""
    with concurrent.futures.ThreadPoolExecutor(min(16, os.cpu_count() or 4)) as ex:
        jobs = {(f.name, t, n): ex.submit(run, robigo, f, t, n) for f in fxs for t in TIERS for n in lengths}
        return {(f.name, t): per_iteration(jobs[f.name, t, lengths[0]].result(), jobs[f.name, t, lengths[1]].result(),
                                           lengths)
                for f in fxs for t in TIERS}


def build():
    target = json.loads(subprocess.check_output(["cargo", "metadata", "--format-version", "1", "--no-deps"],
                                                cwd=ROOT))["target_directory"]
    Path(target).mkdir(parents=True, exist_ok=True)
    BINARY.parent.mkdir(parents=True, exist_ok=True)
    # scripts/build.sh's lock: one build at a time on the shared target.
    with open(Path(target) / ".robigo-build.lock", "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        subprocess.run(["cargo", "build", "--locked", "--profile", "quick", "-p", "robigo-cli", "--features",
                        "counters"], cwd=ROOT, check=True)
        tmp = BINARY.with_suffix(".tmp")
        shutil.copy(Path(target) / "quick" / "robigo", tmp)
        tmp.replace(BINARY)
    print(BINARY.relative_to(ROOT))


def check(args):
    every = fixtures()
    fxs = [f for f in every if not args.filter or re.search(args.filter, f.name)]
    if not fxs:
        sys.exit("no fixture in tests/counters" + (f" matches --filter {args.filter!r}" if args.filter else ""))
    known = read_known(KNOWN.read_text() if KNOWN.exists() else "", every)
    measured = measure(args.robigo, fxs)
    judged = []
    for f in fxs:
        for t in TIERS:
            js = judge(f, t, measured[f.name, t], known)
            judged += js
            print(f"{f.name} [{t}]:")
            for j in js:
                print(f"  {'ok  ' if j.verdict == 'pass' else 'FAIL'} {j.counter}: {j.detail}")
            if args.verbose:
                for k, v in sorted(measured[f.name, t].items()):
                    if v and (t, k) not in f.bounds:
                        print(f"       {k}: {v:g}")
    if args.record:
        unmeasured = [j for j in judged if j.value is None]
        if unmeasured:
            sys.exit("not counted, so tests/counters/known.toml is left as it is: "
                     + ", ".join(f"{j.fixture} [{j.tier}] {j.counter}" for j in unmeasured))
        ran = {f.name for f in fxs}
        kept = [k for k in known if k.fixture not in ran]
        KNOWN.write_text(render_known(kept + recorded(judged, known)))
        print(f"wrote {KNOWN.relative_to(ROOT)}")
        return 0
    bad = [j for j in judged if j.verdict != "pass"]
    print(f"{len(judged) - len(bad)} of {len(judged)} bounds hold" + (f"; {len(bad)} fail" if bad else ""))
    return 1 if bad else 0


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("build", help="build the counted binary into .robigo/bin/robigo-counters")
    c = sub.add_parser("check", help="run the fixtures and hold their bounds")
    c.add_argument("--robigo", default=str(BINARY), help="a binary built with --features counters")
    c.add_argument("--filter", help="only fixtures whose name matches this regex")
    c.add_argument("--record", action="store_true", help="write tests/counters/known.toml from this run")
    c.add_argument("-v", "--verbose", action="store_true", help="also print every other counter that moved")
    args = ap.parse_args(argv)
    if args.cmd == "build":
        build()
        return 0
    return check(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
