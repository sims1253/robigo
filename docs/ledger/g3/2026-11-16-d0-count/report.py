#!/usr/bin/env python3
"""D0c's count from the traces: report.py FUNTAB REGISTRY RT_PRIMS TRACEDIR OUT_TSV
(or report.py --label LOGDIR: the invocations.tsv of a tracer log)

FUNTAB is funtab.py's table; REGISTRY the names of M1's rows
(robigo-rtbuiltins' registry.rs); RT_PRIMS the names.c rows robigo-rt
implements outside the registry (prims.rs, rtapi's dotcode.rs). TRACEDIR
holds split.gdb.py's traces: build/NNN.split for the build's invocations, in
Makefile order, build/invocations.tsv naming each, and e1.split for `-e 1`.

Writes OUT_TSV, one line per row reached, in boot order, and prints the
summary and the per-file list."""
import collections
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))


def label(stdin):
    s = open(stdin).read()
    if "makebasedb.R" in s:
        return "makebasedb"
    m = re.search(r'makeLazyLoading\("(\w+)"\)', s)
    if m:
        return "makeLazyLoading(%s)" % m.group(1)
    m = re.search(r"tools:::(\.?\w+)\(.*?'([^']*)'\)", s)
    if m:
        return "%s(%s)" % (m.group(1), m.group(2).rsplit("/", 1)[-1])
    m = re.search(r"tools:::(\w+)\(", s)
    return m.group(1) if m else "?"



if sys.argv[1] == "--label":
    import glob
    for p in sorted(glob.glob(os.path.join(sys.argv[2], "*.stdin"))):
        print("%s\t%s" % (os.path.basename(p)[:3], label(p)))
    sys.exit(0)
funtab_p, registry_p, rtprims_p, tdir, out_p = sys.argv[1:6]

# --- names.c ---------------------------------------------------------------
rows = {}  # name -> dict
by_cfun = collections.defaultdict(list)
for line in open(funtab_p):
    name, cfun, code, ev, arity, file, fline, nline = line.rstrip("\n").split("\t")
    if name in rows:  # La_rs, La_rs_cmplx: names.c lists them twice
        continue
    rows[name] = dict(name=name, cfun=cfun, eval=ev, file=file.replace("src/main/", "").replace("src/", ""), nline=int(nline))
    by_cfun[cfun].append(name)
registry = {l.strip() for l in open(registry_p) if l.strip()}
rtprims = {l.strip() for l in open(rtprims_p) if l.strip()}
assert registry <= rows.keys(), registry - rows.keys()
assert rtprims <= rows.keys(), rtprims - rows.keys()

from f4rule import EVAL_FILES, LANG, KERNEL_CFUNS


def category(name):
    r = rows[name]
    if name in registry:
        return "M1"
    if r["file"] in EVAL_FILES:
        return "evaluator"
    if name in LANG:
        return "language"
    if r["cfun"] in KERNEL_CFUNS:
        return "kernel"
    return "port"


# --- the traces ------------------------------------------------------------
def read_split(p):
    """(name, phase, seq) per row reached, in first-call order."""
    got = []
    for line in open(p):
        seq, phase, cfun, name, hits = line.rstrip("\n").split("\t")
        if name.startswith("?"):
            continue
        if name == "":
            assert len(by_cfun[cfun]) == 1, cfun
            name = by_cfun[cfun][0]
        assert rows[name]["cfun"] == cfun, (name, cfun)
        got.append((name, phase, int(seq)))
    return got


import glob
labels = dict(l.rstrip("\n").split("\t") for l in open(os.path.join(tdir, "build", "invocations.tsv")))
invs = []
for p in sorted(glob.glob(os.path.join(tdir, "build", "*.split"))):
    n = os.path.basename(p)[:3]
    invs.append((n, labels[n], read_split(p)))
e1 = read_split(os.path.join(tdir, "e1.split"))
lab = {n: l for n, l, _ in invs}
tr = {n: t for n, _, t in invs}

# The foreman's three paths (m2m3-plan.md §B, D0c).
n_basedb = [n for n, l, _ in invs if l == "makebasedb"]
n_mll = [n for n, l, _ in invs if l.startswith("makeLazyLoading")]
n_other = [n for n, l, _ in invs if n not in n_basedb + n_mll]
paths = {
    "1: stage 0 + makebasedb.R": {x[0] for n in n_basedb for x in tr[n]},
    "2: makeLazyLoading (compiler, tools)": {x[0] for n in n_mll for x in tr[n]},
    "3: -e 1 from the databases": {x[0] for x in e1},
}
other = {x[0] for n in n_other for x in tr[n]}
boot = set().union(*paths.values())

# Boot order: A stage 0 (the startup of an invocation with base from source:
# every build invocation but tools' makeLazyLoading, which finds base.rdb),
# B -e 1, C the build's scripts in Makefile order (and the startup with
# base.rdb of tools' makeLazyLoading), D the Makefiles' other R steps
# (outside the three paths; reported, not counted).
order = {}


def put(name, key):
    if name not in order or key < order[name]:
        order[name] = key


src_boot = [n for n in n_basedb + n_mll if lab[n] != "makeLazyLoading(tools)"]
for i, n in enumerate(src_boot):
    for name, phase, seq in tr[n]:
        if phase == "startup":
            put(name, ("A", i, seq))
for name, phase, seq in e1:
    put(name, ("B", 0, seq))
for i, n in enumerate(sorted(n_basedb + n_mll)):
    for name, phase, seq in tr[n]:
        put(name, ("C", i, seq))
for i, n in enumerate(n_other):
    for name, phase, seq in tr[n]:
        put(name, ("D", i, seq))
TIER = {"A": "stage 0", "B": "-e 1", "C": "build", "D": "other steps"}

reached = boot | other
with open(out_p, "w") as f:
    f.write("# name\tcfun\tfile\ttier\tcategory\trt\tpaths\n")
    for name in sorted(reached, key=lambda n: order[n]):
        r = rows[name]
        ps = "".join(k[0] for k, v in paths.items() if name in v) or "-"
        f.write("\t".join([name, r["cfun"], r["file"], TIER[order[name][0]], category(name),
                           "rt" if name in rtprims else "", ps]) + "\n")


# --- the summary --------------------------------------------------------------
def counts(s):
    c = collections.Counter(category(n) for n in s)
    return c


print("names.c rows (Unix):", len(rows))
print("registry:", len(registry), "rt outside the registry:", len(rtprims - registry))
print("evaluator-owned rows in names.c:", sum(1 for r in rows.values() if r["file"] in EVAL_FILES))
print("designed-away rows in names.c (not evaluator-owned):",
      sum(1 for n in rows if rows[n]["file"] not in EVAL_FILES and (n in LANG or rows[n]["cfun"] in KERNEL_CFUNS)))
print()
print("invocations:", ", ".join("%s %s" % (n, l) for n, l, _ in invs))
cats = ["M1", "evaluator", "language", "kernel", "port"]
print("%-40s %5s  %s" % ("path", "rows", "  ".join("%9s" % c for c in cats)))
for k, v in list(paths.items()) + [("boot set (1-3)", boot), ("other steps (not counted)", other),
                                    ("other steps, new rows", other - boot), ("all", reached)]:
    c = counts(v)
    print("%-40s %5d  %s" % (k, len(v), "  ".join("%9d" % c[x] for x in cats)))
port = sorted((n for n in boot if category(n) == "port"), key=lambda n: order[n])
print()
print("D0 count (boot set, port):", len(port))
print("  of which robigo-rt has outside the registry:", len([n for n in port if n in rtprims]),
      [n for n in port if n in rtprims])
f4b = [n for n in boot if rows[n]["file"] not in EVAL_FILES and n not in LANG and rows[n]["cfun"] not in KERNEL_CFUNS]
print("F4-B reading (evaluator-owned and designed away only, M1's rows counted):", len(f4b))
print("F4-A less robigo-rt's Rust outside the registry:", len([n for n in port if n not in rtprims]))
# per file, boot order
files = collections.OrderedDict()
for n in port:
    files.setdefault(rows[n]["file"], []).append(n)
print()
for fi, ns in files.items():
    tiers = sorted({TIER[order[n][0]] for n in ns}, key=lambda t: list(TIER.values()).index(t))
    print("%-16s %3d  %-18s %s" % (fi, len(ns), "/".join(tiers), " ".join(ns)))
extra = sorted((n for n in other - boot if category(n) == "port"), key=lambda n: order[n])
print()
print("other steps add (port):", len(extra), " ".join("%s(%s)" % (n, rows[n]["file"]) for n in extra))
