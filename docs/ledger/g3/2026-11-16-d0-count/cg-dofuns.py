#!/usr/bin/env python3
"""The do_* functions of libR.so that a callgrind run executed, with their
call counts: `cg-dofuns.py OUT...` prints `NAME CALLS SELF_IR` per line.

A function counts when it executed at least one instruction (it has an `fn=`
block with cost). CALLS sums the `calls=` lines whose callee is it."""
import re
import sys
from collections import defaultdict

names = {}  # compressed id -> name, per file (callgrind ids are per file)
self_ir = defaultdict(int)
calls = defaultdict(int)


def resolve(tok):
    # "(12) name" defines, "(12)" refers
    m = re.match(r"\((\d+)\)(?: (.*))?$", tok)
    if not m:
        return tok
    if m.group(2) is not None:
        names[m.group(1)] = m.group(2)
    return names[m.group(1)]


for path in sys.argv[1:]:
    names.clear()
    fn = None
    pending_cfn = None
    pending_calls = 0
    with open(path, errors="replace") as f:
        for line in f:
            line = line.rstrip("\n")
            if line.startswith("fn="):
                fn = resolve(line[3:])
                pending_cfn = None
            elif line.startswith("cfn="):
                pending_cfn = resolve(line[4:])
            elif line.startswith("calls="):
                pending_calls = int(line[6:].split()[0])
                calls[pending_cfn] += pending_calls
                # the next line is the call's inclusive cost: skip it
                next(f)
            elif line and (line[0].isdigit() or line[0] in "+-*") and fn is not None:
                parts = line.split()
                if len(parts) >= 2:
                    try:
                        self_ir[fn] += int(parts[1])
                    except ValueError:
                        pass
            elif line.startswith(("cfi=", "cfl=", "fi=", "fe=", "fl=", "ob=", "cob=")):
                # file/object names are compressed too, in their own id space;
                # they never reach `names` lookups for functions, but their
                # definitions must not shadow function ids: keep separate
                pass

for fn in sorted(self_ir):
    base = fn.split("'")[0]
    if re.match(r"(R_)?do_[A-Za-z0-9_]+$", base) and self_ir[fn] > 0:
        print(base, calls.get(fn, 0), self_ir[fn])
