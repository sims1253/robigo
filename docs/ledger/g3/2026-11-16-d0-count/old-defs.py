#!/usr/bin/env python3
"""The old runtime's builtin tables: `old-defs.py BUILTINS_DIR` prints
`RUST_PATH NAME KIND` for every prim/special/internal row of the DEFS tables
in crates/robigo-runtime/src/builtins (RUST_PATH as callgrind names it)."""
import pathlib, re, sys
root = pathlib.Path(sys.argv[1])
pat = re.compile(r'\b(prim|special|internal)\("((?:[^"\\]|\\.)*)",\s*([A-Za-z_][A-Za-z_0-9:]*)')
for p in sorted(root.rglob("*.rs")):
    mod = "::".join(p.relative_to(root).with_suffix("").parts)
    if mod.endswith("::mod"):
        mod = mod[:-5]
    for m in pat.finditer(p.read_text()):
        kind, name, fn = m.groups()
        path = fn if "::" in fn else "robigo_runtime::builtins::%s::%s" % (mod, fn)
        print(path, name.encode().decode("unicode_escape"), kind, sep="\t")
