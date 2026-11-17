#!/usr/bin/env python3
"""names.c's R_FunTab on Unix as rows: `funtab.py NAMES_C LIBR_SO` prints
NAME<TAB>CFUN<TAB>CODE<TAB>EVAL<TAB>ARITY<TAB>FILE<TAB>LINE (FILE as src/main/x.c:line
of CFUN's definition, from libR.so's debug info)."""
import re
import subprocess
import sys

names_c, libr = sys.argv[1], sys.argv[2]
defs = {}
nm = subprocess.run(["nm", "-l", "--defined-only", libr], capture_output=True, text=True, check=True).stdout
for line in nm.splitlines():
    parts = line.split(None, 3)
    if len(parts) == 4 and parts[1] in "tT" and re.match(r"(R_)?do_", parts[2]):
        loc = parts[3].split("\t")[-1]
        m = re.search(r"/src/(.*):(\d+)$", loc)
        # nmath has static helpers named like builtins (do_search)
        if m and not m.group(1).startswith("nmath/"):
            defs.setdefault(parts[2], ("src/" + m.group(1), m.group(2)))

row = re.compile(r'^\{"((?:[^"\\]|\\.)*)",\s*(?:/\*.*?\*/\s*)?(\w+),\s*([^,]+?),\s*(\d+),\s*([^,]+?),')
lineno = 0
win32 = False  # inside names.c's `#ifdef Win32` (Unix keeps the #else part)
for line in open(names_c):
    lineno += 1
    if line.startswith("#ifdef Win32"):
        win32 = True
    elif line.startswith("#else") and win32:
        win32 = False
    elif line.startswith("#endif"):
        win32 = False
    if win32:
        continue
    m = row.match(line)
    if not m:
        continue
    name, cfun, code, ev, arity = m.groups()
    name = name.encode().decode("unicode_escape")
    f, l = defs.get(cfun, ("?", "0"))
    print("\t".join([name, cfun, code, ev, arity, f, l, str(lineno)]))
