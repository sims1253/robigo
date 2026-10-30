#!/usr/bin/env python3
"""The test ratchets: one marker file per case.

    scripts/ratchet.py check                # every marker names an existing case
    scripts/ratchet.py migrate [--dry-run]  # legacy passing.txt -> markers

A ratchet is a directory of empty marker files named `<case>.pass`, and
nothing else:

    tests/r/passing/<script path relative to tests/r>.pass
    tests/corpora/<name>/passing/<case id>.pass

One file per case means two branches that ratchet different cases add
different files, so merging them never conflicts; the single
`tests/r/passing.txt` (and `tests/corpora/*/passing.txt`) they replace
conflicted on every addition. The loaders are
crates/robigo-cli/tests/conformance.rs and scripts/corpus.py; this script
converts the old lists and checks the result.
"""
import argparse
import importlib.util
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

spec = importlib.util.spec_from_file_location("corpus", os.path.join(ROOT, "scripts", "corpus.py"))
corpus = importlib.util.module_from_spec(spec)
spec.loader.exec_module(corpus)

SUFFIX = corpus.MARKER
CONFORMANCE = os.path.join(ROOT, "tests", "r", "passing")
CONFORMANCE_LEGACY = os.path.join(ROOT, "tests", "r", "passing.txt")


def rel(p):
    return os.path.relpath(p, ROOT)


def markers(d, prefix=""):
    """Case ids of the marker files under d; any other file is an error."""
    ids = set()
    if not os.path.isdir(d):
        return ids
    for root, _, files in os.walk(d):
        for fn in files:
            p = os.path.join(root, fn)
            if not fn.endswith(SUFFIX):
                sys.exit(f"{rel(p)}: not a ratchet marker (markers are named <case>{SUFFIX})")
            ids.add(prefix + os.path.relpath(p, d)[:-len(SUFFIX)].replace(os.sep, "/"))
    return ids


def read_list(p):
    with open(p) as f:
        return {l.strip() for l in f if l.strip() and not l.startswith("#")}


def conformance_exists(cid):
    # Conformance ids are repository-relative ("tests/r/strings/x.R").
    return os.path.isfile(os.path.join(ROOT, cid))


def ratchets():
    """(name, marker directory, legacy list, does the case exist) of every ratchet."""
    out = [("conformance", CONFORMANCE, CONFORMANCE_LEGACY, conformance_exists)]
    for name in sorted(corpus.manifest()):
        if not os.path.isdir(corpus.cdir(name)):
            continue
        known = {cid for cid, _ in corpus.load_cases(name)}
        out.append((name, corpus.passing_dir(name),
                    os.path.join(corpus.cdir(name), "passing.txt"), known.__contains__))
    return out


def check():
    problems = []
    total = 0
    for name, d, legacy, exists in ratchets():
        prefix = "tests/r/" if d == CONFORMANCE else ""
        ids = markers(d, prefix)
        if os.path.exists(legacy):
            # Read, not an error: branches that predate the markers keep
            # working until `scripts/ratchet.py migrate` converts the list.
            print(f"note: {rel(legacy)} is a legacy list; run scripts/ratchet.py migrate", file=sys.stderr)
            ids |= read_list(legacy)
        for cid in sorted(i for i in ids if not exists(i)):
            problems.append(f"{rel(d)}: marker names no case: {cid}")
        print(f"{name}: {len(ids)} ratcheted case(s)")
        total += len(ids)
    for p in problems:
        print(f"ratchet: {p}", file=sys.stderr)
    print(f"total: {total} ratcheted case(s)")
    return 1 if problems else 0


def convert(d, ids, exists, legacy, dry_run, prefix=""):
    """Write one marker per entry, then prove no entry was lost.

    Markers that the list does not mention are kept: they came from the
    other side of a merge (this branch's migration) and the ratchet is
    their union. `same set` means the directory held nothing else, as in
    the one-commit migration."""
    bad = sorted(i for i in ids if not exists(i))
    if bad:
        print(f"{rel(legacy)}: {len(bad)} entr(y/ies) name no case (e.g. {bad[0]})", file=sys.stderr)
        return False
    before = markers(d, prefix)
    if not dry_run:
        for cid in sorted(ids):
            p = os.path.join(d, *cid[len(prefix):].split("/")) + SUFFIX
            if not os.path.exists(p):
                os.makedirs(os.path.dirname(p), exist_ok=True)
                open(p, "w").close()
    after = markers(d, prefix) if not dry_run else before | ids
    lost = ids - after
    print(f"{rel(legacy)}: {len(ids)} entries -> {len(after)} markers, "
          f"{'same set' if after == ids else 'every entry kept' if not lost else 'LOST ENTRIES'}"
          f"{' [dry run]' if dry_run else ''}")
    if lost:
        print(f"  lost: {sorted(lost)[:5]}", file=sys.stderr)
        return False
    if not dry_run:
        os.remove(legacy)
        print(f"  removed {rel(legacy)}")
    return True


def migrate(dry_run):
    legacy_lists = [r for r in ratchets() if os.path.exists(r[2])]
    if not legacy_lists:
        print("no legacy passing.txt lists; nothing to migrate")
        return 0
    ok = True
    for _name, d, legacy, exists in legacy_lists:
        prefix = "tests/r/" if d == CONFORMANCE else ""
        ok &= convert(d, read_list(legacy), exists, legacy, dry_run, prefix)
    return 0 if ok else 1


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("check")
    p = sub.add_parser("migrate")
    p.add_argument("--dry-run", action="store_true", help="report, write and remove nothing")
    a = ap.parse_args()
    sys.exit((check() if a.cmd == "check" else migrate(a.dry_run)) or 0)


if __name__ == "__main__":
    main()
