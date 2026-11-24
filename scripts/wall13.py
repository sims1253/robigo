#!/usr/bin/env python3
"""Wall time under docs/g3.md §13's rule: robigo-rt (the shipped, PGO build,
scripts/pgo-build.sh -p robigo-rtapi) against GNU R 4.6.2 and the rivals, one
process per run, for each program of benchmarks/benchmarks.toml that --filter
selects.

    scripts/wall13.py --robigo-rt BIN [--filter RE] [--impls gnur,rir,fastr] [--rounds 11] --out FILE
    scripts/wall13.py --report FILE [--markdown]

Every runtime starts with base alone (R_DEFAULT_PACKAGES=NULL): robigo-rt
attaches no packages yet, and GNU R loading utils and stats inside the clock
would be unfair to it. A program GNU R cannot run on base alone is reported.

The rule: each timed sample is a batch of back-to-back processes, run until
they took 0.05 s in all; at least 10 interleaved rounds, the order rotated each round
so that no runtime always runs first; medians compared; two runtimes within
2% tie, and a tie does not count as faster. A runtime's answer must be GNU
R's stdout (as scripts/rivals.py requires), checked once per program.

GNU R is the 4.6.2 oracle image (robigo-oracle:4.6.2), §13's rival; Ř and
FastR are rivals.py's images (`scripts/rivals.py setup`). Their containers
are started once (rivals.py's Runner), and every clock reads where the
program runs, so `docker exec` is never timed. robigo-rt runs on the host
with no R_HOME (its own tree: an inherited GNU R one only draws a warning).

The JSON records the machine (CPU model and signature, logical CPUs, host
name) and robigo-rt's binary (sha256, build ID, the checkout's git
describe), so runs on the 5900X and on m0pad can be told apart and compared.
There is no CI job for it: GitHub's runners vary too much for a 2% tie, and
the self-hosted ones were removed (2026-11-07). It runs by hand on both.
"""
import argparse
import json
import os
import platform
import shlex
import statistics
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts"))
import rivals  # noqa: E402
from perf import binary_identity, conf, cpu_signature  # noqa: E402

rivals.IMAGES["gnur"] = ("robigo-oracle:4.6.2", ["Rscript"])
BATCH_S = 0.05
TIE = 0.02
MIN_ROUNDS = 10


def run_rt(bin_, file, pk, timeout, shared):
    """(seconds, stdout, status) of one robigo-rt process, with the clock
    wrapper rivals.Runner uses for the containers."""
    env = {k: v for k, v in os.environ.items() if k != "R_HOME"}
    env.update(rivals.BASE_ENV, R_DEFAULT_PACKAGES=pk)
    fd, tfile = tempfile.mkstemp(dir=shared, suffix=".time")
    os.close(fd)
    clock = 'S=$(date +%s.%N); "$@"; C=$?; E=$(date +%s.%N); echo "$S $E $C" >&3'
    inner = ["timeout", "-s", "KILL", str(int(timeout)), bin_, "--vanilla", file]
    p = subprocess.run(["bash", "-c", f"exec 3>{shlex.quote(tfile)}; {clock}", "bash", *inner],
                       cwd=shared, env=env, capture_output=True)
    try:
        with open(tfile) as f:
            s, e, status = f.read().split()
        os.unlink(tfile)
    except (OSError, ValueError):
        return None, p.stderr.decode("utf-8", "replace")[-300:], -1
    return float(e) - float(s), p.stdout.decode("utf-8", "replace"), int(status)


def verdict(ratio):
    return "tie" if abs(ratio - 1) <= TIE else ("faster" if ratio < 1 else "slower")


# Every runtime starts with base alone: robigo-rt attaches no default
# packages yet, so a GNU R that loads the suite's (utils, stats) would pay
# for them inside the clock. A program that needs them is reported, not timed.
PACKAGES = "NULL"


def measure(prog, impls, runner, a, shared, log):
    """One program's samples under the rule."""
    name, shot, _, _ = prog
    pk = PACKAGES
    file = os.path.join(shared, "shot.R")
    with open(file, "w") as f:
        f.write(shot)

    def once(impl):
        if impl == "robigo-rt":
            secs, out, status = run_rt(a.robigo_rt, file, pk, a.timeout, shared)
        else:
            secs, out, status = runner.run(impl, file, pk, a.timeout, None)
        if secs is None or status != 0:
            raise RuntimeError(f"{impl}: failed: {out[-300:]}")
        return secs, out

    rec = {"samples": {i: [] for i in impls}}
    answers = {}
    for impl in impls:
        try:
            _, answers[impl] = once(impl)
        except RuntimeError as e:
            if impl == "gnur":
                rec["error"] = f"GNU R fails on base alone (the program needs packages): {e}"
                return rec
            raise
    wrong = [i for i in impls if answers[i] != answers["gnur"]]
    if wrong:
        rec["error"] = f"not GNU R's answer: {', '.join(wrong)}"
        return rec
    rec["runs"] = {i: [] for i in impls}
    for r in range(a.rounds):
        order = impls[r % len(impls):] + impls[:r % len(impls)]
        for impl in order:
            # a batch: processes back to back until they took BATCH_S in all
            total, k = 0.0, 0
            while total < BATCH_S:
                total += once(impl)[0]
                k += 1
            rec["samples"][impl].append(total / k)
            rec["runs"][impl].append(k)
        print(f"{name}: round {r + 1}/{a.rounds}", file=log, flush=True)
    rec["median"] = {i: statistics.median(rec["samples"][i]) for i in impls}
    rec["robigo_rt_vs"] = {}
    for i in impls:
        if i != "robigo-rt":
            ratio = rec["median"]["robigo-rt"] / rec["median"][i]
            rec["robigo_rt_vs"][i] = {"ratio": ratio, "verdict": verdict(ratio)}
    return rec


def machine():
    model, sig = cpu_signature()
    return {"cpu": model, "cpu_signature": sig, "cpus": os.cpu_count(), "host": platform.node()}


# a 1-minute load above this share of the CPUs makes the run's ties
# meaningless (§13 measures on an idle machine): the report says so
BUSY = 0.25


def busy_note(doc):
    loads = [x[0] for x in doc.get("load", {}).values()]
    if loads and max(loads) > BUSY * doc["machine"]["cpus"]:
        return (f"WARNING: the machine was busy (1-minute load up to {max(loads):.1f} on {doc['machine']['cpus']} "
                "CPUs): these medians are not §13's evidence")
    return None


def report(doc, markdown=False):
    impls = doc["impls"]
    rivals_ = [i for i in impls if i != "robigo-rt"]
    m = doc["machine"]
    print(f"{m['cpu']} ({m['cpus']} CPUs, {m['host']}); robigo-rt {doc['robigo_rt']['checkout']} "
          f"sha256 {doc['robigo_rt']['sha256'][:16]}; {doc['rounds']} rounds, ties within {TIE:.0%}")
    head = ["program", *[f"{i} s" for i in impls], *[f"vs {i}" for i in rivals_]]
    rows = []
    for name, rec in doc["programs"].items():
        if "error" in rec or "median" not in rec:
            rows.append([name, rec.get("error", "not measured"), *[""] * (len(head) - 2)])
            continue
        rows.append([name, *[f"{rec['median'][i]:.4f}" for i in impls],
                     *[f"{v['ratio']:.3f} {v['verdict']}" for v in rec["robigo_rt_vs"].values()]])
    if markdown:
        print("| " + " | ".join(head) + " |")
        print("|" + "|".join("---" for _ in head) + "|")
        for r in rows:
            print("| " + " | ".join(r) + " |")
    else:
        w = [max(len(str(x)) for x in col) for col in zip(head, *rows)]
        for r in [head, *rows]:
            print("  ".join(str(x).ljust(n) for x, n in zip(r, w)))
    note = busy_note(doc)
    if note:
        print(note)
    for i in rivals_:
        vs = [rec["robigo_rt_vs"][i]["verdict"] for rec in doc["programs"].values() if "robigo_rt_vs" in rec]
        print(f"robigo-rt vs {i}: faster {vs.count('faster')}, tie {vs.count('tie')}, slower {vs.count('slower')}"
              f" (of {len(vs)})")


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--robigo-rt", help="robigo-rt's shipped (PGO) binary")
    ap.add_argument("--filter", default="", help="programs whose name matches (default: all)")
    ap.add_argument("--impls", default="gnur", help="rivals beside GNU R 4.6.2: gnur,rir,fastr")
    ap.add_argument("--rounds", type=int, default=11)
    ap.add_argument("--timeout", type=float, default=300.0)
    ap.add_argument("--out")
    ap.add_argument("--report", help="print a JSON file this wrote")
    ap.add_argument("--markdown", action="store_true")
    a = ap.parse_args(argv)
    if a.report:
        with open(a.report) as f:
            report(json.load(f), a.markdown)
        return 0
    if not a.robigo_rt or not a.out:
        ap.error("--robigo-rt and --out are needed")
    a.robigo_rt = os.path.abspath(a.robigo_rt)
    if a.rounds < MIN_ROUNDS:
        sys.exit(f"§13 asks for at least {MIN_ROUNDS} runs per runtime")
    rivals_ = [i for i in a.impls.split(",") if i]
    if "gnur" not in rivals_:
        rivals_.insert(0, "gnur")
    for i in rivals_:
        if i not in rivals.IMAGES:
            sys.exit(f"no rival {i!r} (have: {', '.join(rivals.IMAGES)})")
    impls = ["robigo-rt", *rivals_]
    progs = rivals.programs(conf(), a.filter)
    if not progs:
        sys.exit(f"no program matches {a.filter!r}")
    # (before any run) base's database tree written by this binary: it names no boot, which
    # boots rdb once that is the default (scripts/rt_trees.py)
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    import rt_trees
    rt_trees.ensure(a.robigo_rt)
    doc = {"rule": "docs/g3.md §13", "machine": machine(),
           "robigo_rt": binary_identity(a.robigo_rt, "robigo-rtapi"),
           "images": {i: rivals.IMAGES[i][0] for i in rivals_}, "impls": impls, "rounds": a.rounds,
           "batch_s": BATCH_S, "tie": TIE, "programs": {}, "load": {"start": os.getloadavg()}}
    os.makedirs(rivals.RIVALS, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=rivals.RIVALS) as shared:
        os.chmod(shared, 0o777)
        runner = rivals.Runner(rivals_, shared)
        try:
            for prog in progs:
                try:
                    doc["programs"][prog[0]] = measure(prog, impls, runner, a, shared, sys.stderr)
                except RuntimeError as e:
                    doc["programs"][prog[0]] = {"error": str(e)}
                with open(a.out, "w") as f:
                    json.dump(doc, f, indent=1)
        finally:
            runner.close()
    doc["load"]["end"] = os.getloadavg()
    with open(a.out, "w") as f:
        json.dump(doc, f, indent=1)
    report(doc, a.markdown)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
