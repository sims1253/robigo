#!/usr/bin/env python3
"""robigo-rt's link order file (crates/robigo-rtapi/rt-starts.order): the
functions a start runs, hottest first, so that the linker packs them
together and a start faults in fewer of the executable's pages
(docs/ledger/g3/2026-11-23-order-file.md).

    scripts/rt-order.py regenerate BINARY [OUT]
    scripts/rt-order.py generate BINARY OUT CG [CG ...]
    scripts/rt-order.py check BINARY [ORDER]
    scripts/rt-order.py coverage BINARY CG [ORDER]

- regenerate: the checked-in file (or OUT) from BINARY's three starts
  (scripts/rt-startup.py's interim, rdb-jit and rdb-nojit), each run once
  under callgrind with --dump-instr=yes from trees BINARY builds in a
  scratch home; then generate. BINARY is a release build of this checkout,
  linked with or without the old file: the order of the code does not
  change which functions run.
- generate: each CG is a callgrind file of BINARY run with --dump-instr=yes. Every executed
  instruction of BINARY is mapped to the function holding it (the symbol
  table, mangled names: the linker's), a function weighs the sum of its
  instructions' counts over all the files, and OUT gets one name per line,
  heaviest first.
- check: how many of ORDER's names (default the checked-in file) BINARY no
  longer has. A stale name is harmless (the linker skips it, and the
  function lands where it would without the file); this only reports, and
  exits 0.
- coverage: the share of the code a start ran (a CG of BINARY, as for
  generate) whose functions ORDER names. A new function a start runs that
  the file does not name falls outside the packed region: when this drops,
  regenerate the file.
"""
import bisect
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ORDER = os.path.join(ROOT, "crates", "robigo-rtapi", "rt-starts.order")


def symbols(binary):
    """{start address: (size, mangled name)} of the binary's text symbols."""
    out = {}
    for ln in subprocess.run(["nm", "-S", "--defined-only", binary], capture_output=True, text=True,
                             check=True).stdout.splitlines():
        p = ln.split()
        if len(p) == 4 and p[2] in "tTwW":
            a, size = int(p[0], 16), int(p[1], 16)
            if size and (a not in out or out[a][0] < size):
                out[a] = (size, p[3])
    return out


def executed(cg, binary):
    """[(address, count)] of `binary`'s executed instructions in the
    callgrind file `cg` (--dump-instr=yes; an object may be first named on a
    cob= line, and named by its id after)."""
    name = os.path.basename(binary)
    obs, ob, res = {}, None, []
    with open(cg, errors="replace") as f:
        for ln in f:
            if ln.startswith(("ob=", "cob=")):
                m = re.match(r"c?ob=\((\d+)\)(?: (.*))?", ln.strip())
                if m:
                    if m.group(2):
                        obs[m.group(1)] = m.group(2)
                    if ln.startswith("ob="):
                        ob = obs.get(m.group(1))
                elif ln.startswith("ob="):
                    ob = ln[3:].strip()
                continue
            m = re.match(r"^(0x[0-9a-f]+) \d+ (\d+)", ln)
            if m and ob and os.path.basename(ob) == name:
                n = int(m.group(2))
                if n:
                    res.append((int(m.group(1), 16), n))
    return res


def by_function(syms, ins):
    """{function start: summed count} of the executed instructions `ins`."""
    starts = sorted(syms)
    w = {}
    for a, n in ins:
        i = bisect.bisect_right(starts, a) - 1
        if i >= 0 and a < starts[i] + syms[starts[i]][0]:
            w[starts[i]] = w.get(starts[i], 0) + n
    return w


def read_order(path):
    with open(path) as f:
        return [ln.strip() for ln in f if ln.strip()]


def generate(binary, out, cgs):
    syms = symbols(binary)
    weight = {}
    for cg in cgs:
        for s, n in by_function(syms, executed(cg, binary)).items():
            weight[s] = weight.get(s, 0) + n
    if not weight:
        raise SystemExit(f"rt-order: no executed instruction of {binary} in {', '.join(cgs)} (--dump-instr=yes?)")
    order = sorted(weight, key=lambda s: (-weight[s], syms[s][1]))
    with open(out, "w") as f:
        for s in order:
            f.write(syms[s][1] + "\n")
    print(f"{out}: {len(order):,} functions, {sum(syms[s][0] for s in order) / 1024:,.0f} KiB of code")


def check(binary, order):
    """(names in `order`, of them not in `binary`)."""
    names = read_order(order)
    have = {n for _, n in symbols(binary).values()}
    return len(names), sum(1 for n in names if n not in have)


def coverage(binary, cg, order):
    """The share of the code bytes `binary` ran in `cg` that `order` names
    (functions' sizes, weighted by nothing: what the packed region holds)."""
    syms = symbols(binary)
    named = set(read_order(order))
    ran = by_function(syms, executed(cg, binary))
    total = sum(syms[s][0] for s in ran)
    covered = sum(syms[s][0] for s in ran if syms[s][1] in named)
    return covered / total if total else None


def regenerate(binary, out):
    """generate() from callgrind profiles of BINARY's three starts."""
    import importlib.util
    spec = importlib.util.spec_from_file_location("rt_startup", os.path.join(ROOT, "scripts", "rt-startup.py"))
    su = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(su)
    home = su.build_trees(binary, ROOT)
    work = su.new_home()
    cgs = []
    for name in ("interim", "rdb-jit", "rdb-nojit"):
        cg = os.path.join(work, f"{name}.cg")
        env = dict(su.clean_env(), ROBIGO_RT_CHECKOUT=os.path.abspath(home), **su.STARTS[name])
        p = subprocess.run([os.environ.get("VALGRIND", "valgrind"), "--tool=callgrind", "--dump-instr=yes",
                            "--compress-pos=no", f"--callgrind-out-file={cg}", os.path.abspath(binary),
                            "--vanilla", "-e", "1"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                           text=True)
        if p.returncode:
            raise SystemExit(f"rt-order: the {name} start failed under callgrind:\n{p.stderr[-2000:]}")
        cgs.append(cg)
    generate(binary, out, cgs)


def main():
    if len(sys.argv) < 3 or sys.argv[1] not in ("regenerate", "generate", "check", "coverage"):
        raise SystemExit(__doc__)
    cmd, binary = sys.argv[1], sys.argv[2]
    if cmd == "regenerate":
        regenerate(binary, sys.argv[3] if len(sys.argv) > 3 else ORDER)
    elif cmd == "generate":
        if len(sys.argv) < 5:
            raise SystemExit(__doc__)
        generate(binary, sys.argv[3], sys.argv[4:])
    elif cmd == "check":
        order = sys.argv[3] if len(sys.argv) > 3 else ORDER
        n, stale = check(binary, order)
        print(f"{order}: {n:,} names, {stale:,} not in {binary}" + (" (harmless; regenerate when many)" if stale else ""))
    else:
        if len(sys.argv) < 4:
            raise SystemExit(__doc__)
        order = sys.argv[4] if len(sys.argv) > 4 else ORDER
        c = coverage(binary, sys.argv[3], order)
        print(f"{order} covers {c:.1%} of the code {binary} ran in {sys.argv[3]}" if c is not None
              else f"no code of {binary} in {sys.argv[3]}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
