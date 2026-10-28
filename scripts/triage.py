#!/usr/bin/env python3
"""First difference per failing case of a corpus run: the triage tool for
the script corpora (docs/ledger/corpus-scripts.md).

    scripts/corpus.py run gnur-demos --keep-output   # writes the results
    scripts/triage.py gnur-demos [--id RE] [--tests]

For every failing case it prints which streams differ and the first
differing hunk of each: `R|` is GNU R's recorded line, `o|` is robigo's.
For an echo transcript it also names the echoed expression (the last `> `
line above the difference), which is how a failure is traced to the
statement and then classified by root cause.
"""
import argparse
import json
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts"))
import corpus  # noqa: E402


def first_diff(want, got):
    """(line, want lines around it, got lines around it), or None."""
    a, b = want.split("\n"), got.split("\n")
    for i, (x, y) in enumerate(zip(a, b)):
        if x != y:
            return i, a[max(0, i - 2):i + 3], b[max(0, i - 2):i + 3], a
    if len(a) != len(b):
        i = min(len(a), len(b))
        return i, a[max(0, i - 2):i + 3], b[max(0, i - 2):i + 3], a
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("corpus")
    ap.add_argument("--results", help="results json (default .robigo/results/<corpus>.json)")
    ap.add_argument("--id", help="only cases whose id matches this regex")
    ap.add_argument("--tests", action="store_true", help="only the pass/fail list, no diffs")
    a = ap.parse_args()

    corpus.corpus_conf(a.corpus)
    res_path = a.results or os.path.join(corpus.STATE, "results", a.corpus + ".json")
    with open(res_path) as f:
        doc = json.load(f)
    srcs = {c[0]: c[1] for c in corpus.load_cases(a.corpus)}
    want = corpus.load_expected(a.corpus)
    rx = re.compile(a.id) if a.id else None
    fails = 0
    for r in doc["results"]:
        if r["pass"] or (rx and not rx.search(r["id"])):
            continue
        fails += 1
        if a.tests:
            print(f"FAIL {r['id']} ({', '.join(r['differs'])})")
            continue
        exp = want[r["id"]]
        print(f"\n===== {r['id']}: {' '.join(r['differs'])}")
        for stream in ("stdout", "stderr"):
            if stream not in r["differs"]:
                continue
            key = "out" if stream == "stdout" else "err"
            d = first_diff(corpus.normalize(corpus.dec(exp[key])).decode("utf-8", "replace"),
                           corpus.normalize(corpus.dec(r[key])).decode("utf-8", "replace"))
            if d is None:
                print(f"-- {stream}: same after normalisation")
                continue
            line, w, g, all_w = d
            print(f"-- {stream}: first differs at line {line} (0-based)")
            for s in w:
                print("  R| " + s)
            for s in g:
                print("  o| " + s)
            if stream == "stdout":
                echo = [l for l in all_w[:line] if l.startswith("> ")]
                if echo:
                    print("  last echoed: " + echo[-1])
        if "status" in r["differs"]:
            print(f"-- status: GNU R {exp['status']}, robigo {r.get('status')}")
    print(f"\n{doc['summary']['pass']}/{doc['summary']['cases']} pass, {fails} failing")


if __name__ == "__main__":
    main()
