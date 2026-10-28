#!/usr/bin/env python3
"""Allocator calls per benchmark, from callgrind's call graph.

    scripts/alloc-count.py ROOT ROBIGO OUT.json [FILTER]


Counts calls of __rust_alloc / __rust_alloc_zeroed / __rust_realloc (every Box/Rc/Vec allocation
goes through them) from the callgrind call graph, in total and attributed to the calling function (so a boxed
double scalar, an Rc<Vector> with its Vec buffer, counts 2). Programs run as in scripts/perf.py.
"""
import json, os, re, sys, tempfile, subprocess, collections, concurrent.futures
root, robigo, out = sys.argv[1:4]; rx = sys.argv[4] if len(sys.argv) > 4 else None
sys.path.insert(0, os.path.join(root, "scripts"))
import perf
ALLOC = ("__rust_alloc", "__rust_alloc_zeroed", "__rust_realloc")
def count(cgfile):
    tot = 0; by = collections.Counter(); fn = None; cfn = None
    names = {}
    def name(s):
        m = re.match(r"\((\d+)\)(?: (.*))?", s)
        if not m: return s
        if m.group(2): names[m.group(1)] = m.group(2)
        return names.get(m.group(1), s)
    for line in open(cgfile, errors="replace"):
        if line.startswith("fn="): fn = name(line[3:].strip())
        elif line.startswith("cfn="): cfn = name(line[4:].strip())
        elif line.startswith("calls=") and cfn and cfn.endswith(ALLOC):
            n = int(line[6:].split()[0]); tot += n; by[re.sub(r"'\d+$", "", fn)] += n
    return tot, by
def one(p):
    name, _, text, pk = p
    env = dict(os.environ, **perf.BASE_ENV, R_DEFAULT_PACKAGES=pk, R_HOME=perf.r_home())
    with tempfile.TemporaryDirectory() as tmp:
        f = os.path.join(tmp, "bench.R"); open(f, "w").write(text); cg = os.path.join(tmp, "cg")
        r = subprocess.run(["valgrind", "--tool=callgrind", f"--callgrind-out-file={cg}", robigo, "--vanilla", f],
                           cwd=tmp, env=env, capture_output=True, timeout=7200)
        m = re.search(rb"refs:\s+([\d,]+)", r.stderr)
        tot, by = count(cg)
        return name, {"instr": int(m.group(1).replace(b",", b"")) if m else None, "allocs": tot,
                      "top": by.most_common(12)}
progs = [p for p in perf.programs(perf.conf()) if not rx or re.search(rx, p[0])]
res = {}
with concurrent.futures.ThreadPoolExecutor(12) as ex:
    for n, r in ex.map(one, progs): res[n] = r
json.dump(res, open(out, "w"), indent=1, sort_keys=True)
for n in sorted(res): print(f"{n:42} {res[n]['allocs']:>12,}")
