#!/usr/bin/env python3
"""Which of GNU R's .Primitive/.Internal functions robigo implements, and
the drift gate on what it does not.

    scripts/builtin-coverage.py [ROBIGO] [-v]        # the report
    scripts/builtin-coverage.py --check [ROBIGO]     # the gate (CI's build job)
    scripts/builtin-coverage.py --update [ROBIGO]    # rewrite the baseline

The report compares `robigo --list-builtins` with scripts/names-4.5.0.tsv
(extracted from GNU R 4.5.0's src/main/names.c: name, C function, C file,
kind).

The gate compares three things with scripts/builtin-coverage.baseline:

  missing  names.c builtins robigo does not register;
  kind     builtins registered as the wrong kind (.Primitive vs .Internal);
  gap      "robigo: ... not implemented yet" / "... not supported" errors in
           the Rust sources: a builtin that is registered but refuses part
           of its job (AGENTS.md, porting contract rule 6).

It fails when one appears that the baseline does not list: a builtin that
was dropped or registered wrong, or a new refusal nobody classified. Add a
deliberate new gap with --update (the diff to the baseline is the review).
Entries the tree no longer has are reported as stale and do not fail;
--update removes them. Positive controls come first: if `c` and `paste0`
are not seen on both sides, or no gap at all is extracted, the extraction is
broken and the gate says so instead of passing (exit 2).
"""
import argparse
import collections
import os
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
HERE = ROOT / "scripts"
BASELINE = HERE / "builtin-coverage.baseline"
GAP = re.compile(r'"(robigo: [^"\n]*?not (?:yet )?(?:implemented|supported)[^"\n]*)"')
CONTROLS = ("c", "paste0")


def registered(robigo):
    have = {}
    out = subprocess.run([robigo, "--list-builtins"], capture_output=True, text=True, check=True).stdout
    for line in out.splitlines():
        name, kind = line.split("\t")
        have[name] = kind
    return have


def names():
    rows = []
    for line in open(HERE / "names-4.5.0.tsv"):
        name, cfun, cfile, kind = line.rstrip("\n").split("\t")
        rows.append((name, cfun, cfile, kind))
    return rows


def gaps():
    """Every refusal message in the Rust sources, outside tests."""
    found = set()
    for p in sorted((ROOT / "crates").rglob("*.rs")):
        if "tests" in p.relative_to(ROOT).parts:
            continue
        for line in p.read_text(encoding="utf-8").splitlines():
            if "assert" in line or line.lstrip().startswith("//"):  # a test of the message, a doc comment
                continue
            found.update(GAP.findall(line))
    return found


def state(robigo):
    have, rows = registered(robigo), names()
    entries = set()
    for name, _, _, kind in rows:
        if name not in have:
            entries.add(("missing", name))
        elif have[name] != kind:
            entries.add(("kind", f"{name} (R: {kind}, robigo: {have[name]})"))
    entries |= {("gap", g) for g in gaps()}
    return have, rows, entries


def control(have, rows, entries):
    known = {r[0] for r in rows}
    broken = [f"{c!r} not in {where}" for c in CONTROLS
              for where, s in (("names-4.5.0.tsv", known), ("robigo --list-builtins", have)) if c not in s]
    if not any(k == "gap" for k, _ in entries):
        broken.append("no 'not implemented yet' message extracted from crates/")
    if broken:
        print("builtin-coverage: CONTROL FAILED (the extraction is broken, not the tree): " + "; ".join(broken))
        sys.exit(2)


def read_baseline():
    out = set()
    if BASELINE.exists():
        for line in BASELINE.read_text(encoding="utf-8").splitlines():
            if line.strip() and not line.startswith("#"):
                kind, _, rest = line.partition(" ")
                out.add((kind, rest))
    return out


def write_baseline(entries):
    order = {"missing": 0, "kind": 1, "gap": 2}
    with open(BASELINE, "w", encoding="utf-8") as f:
        f.write("# What robigo does not implement, as scripts/builtin-coverage.py --check\n"
                "# compares it: `missing NAME` (a names.c builtin robigo does not register),\n"
                "# `kind ...` (registered as the wrong kind), `gap MESSAGE` (a refusal in the\n"
                "# sources). Regenerate with --update; never edit by hand.\n")
        for kind, what in sorted(entries, key=lambda e: (order[e[0]], e[1])):
            f.write(f"{kind} {what}\n")


def report(have, rows, verbose):
    total, done = collections.Counter(), collections.Counter()
    missing, wrong_kind = collections.defaultdict(list), []
    for name, _, cfile, kind in rows:
        total[cfile] += 1
        if name in have:
            done[cfile] += 1
            if have[name] != kind:
                wrong_kind.append(f"{name} (R: {kind}, robigo: {have[name]})")
        else:
            missing[cfile].append(name)
    print(f"{sum(done.values())}/{sum(total.values())} builtins implemented\n")
    for f, n in total.most_common():
        print(f"{done[f]:4d}/{n:<4d} {100 * done[f] // n:3d}%  {f}")
    if verbose:
        for f in sorted(missing):
            print(f"\n{f}: {' '.join(missing[f])}")
    if wrong_kind:
        print("\nkind mismatches:", ", ".join(wrong_kind))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("robigo", nargs="?", default="robigo")
    ap.add_argument("-v", action="store_true", help="list the missing builtins")
    g = ap.add_mutually_exclusive_group()
    g.add_argument("--check", action="store_true")
    g.add_argument("--update", action="store_true")
    a = ap.parse_args()
    have, rows, entries = state(a.robigo)
    if not (a.check or a.update):
        report(have, rows, a.v)
        return 0
    control(have, rows, entries)
    if a.update:
        write_baseline(entries)
        print(f"wrote {BASELINE.relative_to(ROOT)}: {len(entries)} entries")
        return 0
    base = read_baseline()
    new, stale = sorted(entries - base), sorted(base - entries)
    count = collections.Counter(k for k, _ in entries)
    print(f"builtin-coverage: {len(rows) - count['missing']}/{len(rows)} builtins registered, "
          f"{count['kind']} kind mismatches, {count['gap']} refusal messages; baseline {len(base)} entries")
    if stale:
        print(f"\nresolved ({len(stale)}; --update removes them from the baseline):")
        for k, w in stale:
            print(f"  {k} {w}")
    if new:
        print(f"\nDRIFT ({len(new)}): not in {BASELINE.relative_to(ROOT)}:")
        for k, w in new:
            print(f"  {k} {w}")
        print("\nA missing or wrong-kind builtin is a regression. A new gap is a refusal nobody\n"
              "classified: implement it, or run --update and let the review see the baseline diff.")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
