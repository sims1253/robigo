#!/usr/bin/env python3
"""The per-file table of the ledger entry from report.py's TSV: table.py BOOT_SET_TSV"""
import collections, sys
files = collections.OrderedDict()
for l in open(sys.argv[1]):
    if l.startswith("#"):
        continue
    name, cfun, file, tier, cat, rt, paths = l.rstrip("\n").split("\t")
    if cat != "port" or paths == "-":
        continue
    files.setdefault(file, {"early": [], "build": []})["early" if tier in ("stage 0", "-e 1") else "build"].append(
        "`%s`%s" % (name, "*" if rt else ""))
print("| # | file | rows | stage 0, `-e 1` (B5a) | build (B5b) |")
print("|---|---|---|---|---|")
for i, (f, d) in enumerate(files.items(), 1):
    print("| %d | %s | %d | %s | %s |" % (i, f, len(d["early"]) + len(d["build"]), ", ".join(d["early"]), ", ".join(d["build"])))
