#!/usr/bin/env python3
"""§10's count on the old runtime (4.5.0): old-runtime.py OLD_DEFS CALLGRIND_OUT
FUNTAB REGISTRY. The old runtime's builtins are Rust functions named in
its DEFS tables (old-defs.py); a callgrind run of its boot names the ones
that ran. Prints the rows reached and the same F4 classification as
report.py (a function shared by several rows counts all of them)."""
import collections, os, re, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from f4rule import EVAL_FILES as EVAL, LANG, KERNEL_CFUNS
sys.argv, (defs_p, cg_p, funtab_p, reg_p) = sys.argv[:1], sys.argv[1:5]
defs = collections.defaultdict(list)
for l in open(defs_p):
    p, n, k = l.rstrip("\n").split("\t")
    defs[p].append(n)
ran = set()
for l in open(cg_p, errors="replace"):
    m = re.match(r"c?fn=\(\d+\) (robigo_runtime::builtins::\S+)", l)
    if m:
        ran.add(m.group(1).split("'")[0])
rows = {}
for l in open(funtab_p):
    f = l.rstrip("\n").split("\t")
    rows.setdefault(f[0], dict(cfun=f[1], file=f[5].replace("src/main/", "")))
reg = {l.strip() for l in open(reg_p) if l.strip()}
names = sorted({n for p in ran for n in defs.get(p, [])})
c = collections.Counter()
port = []
for n in names:
    r = rows.get(n)
    if r is None:
        c["not in 4.6.2's names.c"] += 1
    elif n in reg:
        c["M1"] += 1
    elif r["file"] in EVAL:
        c["evaluator"] += 1
    elif n in LANG:
        c["language"] += 1
    elif r["cfun"] in KERNEL_CFUNS:
        c["kernel"] += 1
    else:
        c["port"] += 1
        port.append(n)
print(len(names), "rows;", dict(c))
print("port:", " ".join(port))
