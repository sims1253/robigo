#!/usr/bin/env python3
"""Weighted coverage of the libR census (tests/capi/symbols.tsv) by
robigo-capi's *implemented* exports: sum of n_pkgs over implemented
symbols / total weight. Stubs (src/stubs.rs) do not count.

  crates/robigo-capi/coverage.py [--missing N]
"""
import os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import importlib.util
spec = importlib.util.spec_from_file_location("gen", os.path.join(os.path.dirname(os.path.abspath(__file__)), "gen-stubs.py"))
gen = importlib.util.module_from_spec(spec); spec.loader.exec_module(gen)

rows = gen.census()
impl = gen.implemented()
total = sum(w for _, _, w in rows)
have = [(s, w) for s, _, w in rows if s in impl]
got = sum(w for _, w in have)
print(f"symbols {len(have)}/{len(rows)}, weighted {got}/{total} = {100*got/total:.1f}%")
if "--missing" in sys.argv:
    n = int(sys.argv[sys.argv.index("--missing") + 1])
    miss = [(w, s) for s, _, w in rows if s not in impl]
    miss.sort(reverse=True)
    for w, s in miss[:n]:
        print(f"{w:4} {s}")
