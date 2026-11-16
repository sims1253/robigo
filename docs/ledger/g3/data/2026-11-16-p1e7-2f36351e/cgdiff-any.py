#!/usr/bin/env python3
"""Per-function instruction differences (self and inclusive) of two
callgrind runs of any command, divided by UNITS.

cgdiff-any.py UNITS --gnur WORK.R BASE.R [--top N] [--grep RX]
cgdiff-any.py UNITS --robigo BIN WORK.R BASE.R [--top N] [--grep RX]
"""
import argparse
import os
import re
import subprocess
import tempfile

VALGRIND = os.path.expanduser("~/.cache/rr-tools/bin/valgrind")
ANNOTATE = os.path.join(os.path.dirname(VALGRIND), "callgrind_annotate")
R_HOME = os.path.expanduser("~/.cache/robigo-research/R-4.6.2-build")
LINE = re.compile(r"^\s*([\d,]+)\s+(?:\([^)]*\)\s+)?(.*)$")


def run(cmd, env, out):
    subprocess.run([VALGRIND, "--tool=callgrind", f"--callgrind-out-file={out}", *cmd], env=env, check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def table(cg, inclusive):
    txt = subprocess.run([ANNOTATE, "--auto=no", f"--inclusive={'yes' if inclusive else 'no'}", "--threshold=100",
                          "--show-percs=no", cg], capture_output=True, text=True, check=True).stdout
    d = {}
    started = False
    for ln in txt.splitlines():
        if "file:function" in ln:
            started = True
            continue
        if not started:
            continue
        m = LINE.match(ln)
        if m:
            name = re.sub(r"'\d+", "", m.group(2).split(":", 1)[-1].split(" [")[0]).strip()
            name = re.sub(r"\s*\([\d,]+x\)$", "", name)
            d[name] = d.get(name, 0) + int(m.group(1).replace(",", ""))
    return d


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("units", type=float)
    ap.add_argument("--gnur", nargs=2)
    ap.add_argument("--robigo", nargs=3)
    ap.add_argument("--top", type=int, default=30)
    ap.add_argument("--grep", default="")
    a = ap.parse_args()
    if a.gnur:
        work, base = a.gnur
        env = {**os.environ, "R_HOME": R_HOME, "LD_LIBRARY_PATH": os.path.join(R_HOME, "lib"),
               "R_DEFAULT_PACKAGES": "NULL"}
        cmd = lambda s: [os.path.join(R_HOME, "bin/exec/R"), "--vanilla", "--no-echo", "-f", s]
    else:
        binp, work, base = a.robigo
        env = {**os.environ, "ROBIGO_COMPILE": "1", "R_DEFAULT_PACKAGES": "NULL"}
        cmd = lambda s: [binp, s]
    keep = os.environ.get("CG_KEEP")
    with tempfile.TemporaryDirectory() as d:
        d = keep or d
        w, b = os.path.join(d, "w"), os.path.join(d, "b")
        if not (keep and os.path.exists(w) and os.path.exists(b)):
            run(cmd(work), env, w)
            run(cmd(base), env, b)
        for inc in (False, True):
            tw, tb = table(w, inc), table(b, inc)
            rows = sorted(((tw.get(k, 0) - tb.get(k, 0)) / a.units, k) for k in set(tw) | set(tb))
            rows = [r for r in rows if re.search(a.grep, r[1])]
            print(f"== {'inclusive' if inc else 'self'}, per unit")
            for c, k in sorted(rows, key=lambda r: -abs(r[0]))[: a.top]:
                print(f"{c:10.1f}  {k[:110]}")


if __name__ == "__main__":
    main()
