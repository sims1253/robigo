#!/usr/bin/env python3
"""Report which of GNU R's .Primitive/.Internal functions rr implements.

Usage: scripts/builtin-coverage.py [path/to/rr]
Compares `rr --list-builtins` with scripts/names-4.5.0.tsv (extracted from
GNU R 4.5.0's src/main/names.c: name, C function, C file, kind).
"""
import collections, os, subprocess, sys

here = os.path.dirname(os.path.abspath(__file__))
rr = sys.argv[1] if len(sys.argv) > 1 else "rr"
have = {}
for line in subprocess.run([rr, "--list-builtins"], capture_output=True, text=True, check=True).stdout.splitlines():
    name, kind = line.split("\t")
    have[name] = kind
total = collections.Counter()
done = collections.Counter()
missing = collections.defaultdict(list)
wrong_kind = []
for line in open(os.path.join(here, "names-4.5.0.tsv")):
    name, cfun, cfile, kind = line.rstrip("\n").split("\t")
    total[cfile] += 1
    if name in have:
        done[cfile] += 1
        if have[name] != kind:
            wrong_kind.append(f"{name} (R: {kind}, rr: {have[name]})")
    else:
        missing[cfile].append(name)
print(f"{sum(done.values())}/{sum(total.values())} builtins implemented\n")
for f, n in total.most_common():
    pct = 100 * done[f] // n
    print(f"{done[f]:4d}/{n:<4d} {pct:3d}%  {f}")
if "-v" in sys.argv:
    for f in sorted(missing):
        print(f"\n{f}: {' '.join(missing[f])}")
if wrong_kind:
    print("\nkind mismatches:", ", ".join(wrong_kind))
