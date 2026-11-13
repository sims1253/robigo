#!/usr/bin/env python3
"""List the cases whose GNU R output differs between two oracle versions.

    scripts/oracle-diff.py                       # 4.5.0 -> 4.6.2, every suite
    scripts/oracle-diff.py conformance fastr     # some suites
    scripts/oracle-diff.py --ids                 # also one line per changed case
    scripts/oracle-diff.py --json diff.json      # everything, machine-readable
    scripts/oracle-diff.py --check               # fail on cases --to lacks or recorded stale

Suites: `conformance` (tests/r) and every corpus in tests/corpora/corpora.toml.
Case ids are the same in every version's recordings (docs/verification.md,
"Oracles"). A case is compared the way its harness compares robigo with
GNU R: a conformance script byte for byte (stdout, stderr, exit status), a
corpus case after masking (addresses, timings, both recordings' line
masks, IGNORE_RDIFF blocks). A case unstable in either recording is
counted apart, not as changed.

`missing` counts cases one side has not recorded; `stale` counts cases
recorded from another version of their source (corpora record the case's
hash, conformance recordings of versions other than 4.5.0 a `.Rsha`).
"""
import argparse
import hashlib
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import corpus  # noqa: E402

ROOT = corpus.ROOT
TESTS_R = os.path.join(ROOT, "tests", "r")


def conformance_base(script, version):
    """The recording of `script` (a path relative to tests/r) for GNU R
    `version`, without extension; as crates/robigo-cli/tests/conformance.rs."""
    stem = script[: -len(".R")]
    if version == corpus.GATED_R_VERSION:
        return os.path.join(TESTS_R, stem)
    return os.path.join(TESTS_R, f"R-{version}", stem)


def read_conformance(script, version):
    base = conformance_base(script, version)
    try:
        with open(base + ".Rout", "rb") as f:
            out = f.read()
    except FileNotFoundError:
        return None

    def opt(ext, default):
        try:
            with open(base + ext, "rb") as f:
                return f.read()
        except FileNotFoundError:
            return default

    status = opt(".Rstatus", b"0").strip()
    sha = opt(".Rsha", b"").strip().decode() or None
    return {"out": out, "err": opt(".Rerr", b""), "status": int(status or 0), "sha": sha}


def diff_conformance(v_from, v_to):
    scripts = sorted(os.path.relpath(os.path.join(d, f), TESTS_R)
                     for d, _, fs in os.walk(TESTS_R) for f in fs if f.endswith(".R"))
    res = new_result()
    for s in scripts:
        with open(os.path.join(TESTS_R, s), "rb") as f:
            src_sha = hashlib.sha256(f.read()).hexdigest()
        a, b = read_conformance(s, v_from), read_conformance(s, v_to)
        for side, rec in (("from", a), ("to", b)):
            if rec is None:
                res["missing_" + side].append(s)
            elif rec["sha"] and rec["sha"] != src_sha:
                res["stale_" + side].append(s)
        if a is None or b is None:
            continue
        res["compared"] += 1
        kinds = [k for k, x, y in (("stdout", a["out"], b["out"]), ("stderr", a["err"], b["err"]),
                                   ("status", a["status"], b["status"])) if x != y]
        if kinds:
            res["changed"][s] = kinds
    return res


def diff_corpus(name, v_from, v_to):
    conf = corpus.corpus_conf(name)
    cases = corpus.load_cases(name) if os.path.isdir(corpus.cdir(name)) else []
    exp_a = corpus.load_expected(name, corpus.expected_file(conf, v_from))
    exp_b = corpus.load_expected(name, corpus.expected_file(conf, v_to))
    skip = corpus.host_dependent(conf)
    res = new_result()
    for cid, src in cases:
        if cid in skip:
            continue
        a, b = exp_a.get(cid), exp_b.get(cid)
        for side, rec in (("from", a), ("to", b)):
            if rec is None:
                res["missing_" + side].append(cid)
            elif rec["sha"] != corpus.sha(src):
                res["stale_" + side].append(cid)
        if a is None or b is None:
            continue
        if a.get("unstable") or b.get("unstable"):
            res["unstable"].append(cid)
            continue
        res["compared"] += 1
        # Either recording's varying lines are masked on both sides.
        both = dict(a)
        for key in ("mask_out", "mask_err"):
            m = sorted(set(a.get(key, [])) | set(b.get(key, [])))
            if m:
                both[key] = m
        kinds = corpus.classify(both, corpus.dec(b["out"]), corpus.dec(b["err"]), b["status"])
        if kinds:
            res["changed"][cid] = kinds
    return res


def new_result():
    return {"compared": 0, "changed": {}, "unstable": [], "missing_from": [], "missing_to": [],
            "stale_from": [], "stale_to": []}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("suites", nargs="*", help="conformance and/or corpus names (default: all)")
    ap.add_argument("--from", dest="v_from", default=corpus.GATED_R_VERSION)
    ap.add_argument("--to", dest="v_to", default="4.6.2")
    ap.add_argument("--ids", action="store_true", help="print each changed case: suite, id, what differs")
    ap.add_argument("--json", help="write the full result to this file")
    ap.add_argument("--markdown", action="store_true", help="the summary as a markdown table")
    ap.add_argument("--check", action="store_true", help="exit 1 if --to lacks a case or recorded a stale one")
    a = ap.parse_args()
    v_from, v_to = corpus.r_version(a.v_from), corpus.r_version(a.v_to)
    suites = a.suites or ["conformance"] + list(corpus.manifest())
    results = {}
    for s in suites:
        results[s] = diff_conformance(v_from, v_to) if s == "conformance" else diff_corpus(s, v_from, v_to)

    cols = ["compared", "changed", "stdout", "stderr", "status", "unstable", "missing", "stale"]

    def row(r):
        kinds = [k for ks in r["changed"].values() for k in ks]
        return [r["compared"], len(r["changed"]), kinds.count("stdout"), kinds.count("stderr"), kinds.count("status"),
                len(r["unstable"]), f"{len(r['missing_from'])}/{len(r['missing_to'])}",
                f"{len(r['stale_from'])}/{len(r['stale_to'])}"]

    rows = [[s] + row(r) for s, r in results.items()]
    if len(rows) > 1:
        tot = [sum(r[i] for r in rows) for i in range(1, 7)]
        mis = [sum(int(r[7].split("/")[j]) for r in rows) for j in (0, 1)]
        sta = [sum(int(r[8].split("/")[j]) for r in rows) for j in (0, 1)]
        rows.append(["total"] + tot + [f"{mis[0]}/{mis[1]}", f"{sta[0]}/{sta[1]}"])
    print(f"GNU R {v_from} -> {v_to}: cases whose recorded output differs "
          f"(missing and stale: {v_from}/{v_to})")
    if a.markdown:
        print("| suite | " + " | ".join(cols) + " |\n|---|" + "---:|" * len(cols))
        for r in rows:
            print("| " + " | ".join(map(str, r)) + " |")
    else:
        print(f"{'suite':16}" + "".join(f"{c:>10}" for c in cols))
        for r in rows:
            print(f"{r[0]:16}" + "".join(f"{x:>10}" for x in r[1:]))
    if a.ids:
        for s, r in results.items():
            for cid, kinds in sorted(r["changed"].items()):
                print(f"{s}\t{cid}\t{','.join(kinds)}")
    if a.json:
        with open(a.json, "w") as f:
            json.dump({"from": v_from, "to": v_to, "suites": results}, f, indent=1, sort_keys=True)
    if a.check:
        bad = {s: len(r["missing_to"]) + len(r["stale_to"]) for s, r in results.items()}
        if any(bad.values()):
            for s, r in results.items():
                for kind in ("missing_to", "stale_to"):
                    for cid in r[kind][:5]:
                        print(f"  {kind.replace('_to', '')} in {v_to}: {s} {cid}", file=sys.stderr)
            print(f"{sum(bad.values())} case(s) not recorded, or recorded stale, for GNU R {v_to}", file=sys.stderr)
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
