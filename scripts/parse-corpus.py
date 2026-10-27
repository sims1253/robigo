#!/usr/bin/env python3
"""Parse-corpus harness: record GNU R's parse trees over large R corpora
and compare robigo's, with a ratchet of sources that must keep matching.

Corpora (see CORPORA below):
  rlib          every .R file in GNU R's own src/library (local cache)
  rtests        GNU R's tests/*.R (local cache)
  fastr         every snippet of tests/corpora/fastr/cases.jsonl
  gnur-tests    tests/corpora/gnur-tests/cases
  gnur-examples tests/corpora/gnur-examples/cases
  cran          R sources of CRAN packages (~/.cache/robigo-research/cran-src)

Sources are untrusted data: they are only ever parsed, never run or
sourced. Files that are not valid UTF-8 are recorded and parsed as Latin-1
(transcoded to UTF-8 on both sides).

Usage:
  scripts/parse-corpus.py list [CORPUS...]
  scripts/parse-corpus.py record [CORPUS...] [--force]
  scripts/parse-corpus.py check [CORPUS...] [--update] [--jobs N]
  scripts/parse-corpus.py show CORPUS ID
  scripts/parse-corpus.py fetch-cran [CORPUS...]

State:
  ~/.cache/robigo-research/parse-oracle/
    manifests/<corpus>.tsv      id \t path \t enc \t sha256
    records/<corpus>.tsv        id \t dump (dumps joined with \x01)
  tests/corpora/<corpus>/parse-dumps.tsv.gz   committed records (fastr,
                                              gnur-tests, gnur-examples)
  tests/parse/passing/<corpus>.txt            the ratchet
"""

import argparse
import gzip
import hashlib
import json
import os
import subprocess
import sys
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CACHE = Path(os.environ.get("ROBIGO_PARSE_CACHE", Path.home() / ".cache/robigo-research/parse-oracle"))
RSRC = Path(os.environ.get("ROBIGO_RSRC", Path.home() / ".cache/robigo-research/rsrc/R-4.5.0"))
CRAN_SRC = Path(os.environ.get("ROBIGO_CRAN_SRC", Path.home() / ".cache/robigo-research/cran-src"))
R_LIBS = Path.home() / "R/x86_64-pc-linux-gnu-library/4.5"

# The oracle's environment, as in scripts/corpus.py.
R_ENV = {"LANGUAGE": "en", "LC_ALL": "C.UTF-8", "TZ": "UTC", "R_ENVIRON_USER": "", "R_PROFILE_USER": ""}

RATCHET_DIR = ROOT / "tests/parse/passing"


def corpus_paths(name):
    """(label, source directory, committed-dumps path or None)."""
    if name == "rlib":
        return "GNU R src/library", RSRC / "src/library", None
    if name == "rtests":
        return "GNU R tests", RSRC / "tests", None
    if name == "fastr":
        return "FastR snippets", ROOT / "tests/corpora/fastr", ROOT / "tests/corpora/fastr/parse-dumps.tsv.gz"
    if name == "gnur-tests":
        return "GNU R regression tests", ROOT / "tests/corpora/gnur-tests", ROOT / "tests/corpora/gnur-tests/parse-dumps.tsv.gz"
    if name == "gnur-examples":
        return "help-page examples", ROOT / "tests/corpora/gnur-examples", ROOT / "tests/corpora/gnur-examples/parse-dumps.tsv.gz"
    if name == "cran":
        return "CRAN package sources", CRAN_SRC, None
    raise SystemExit(f"unknown corpus {name!r}")


def in_repo(name):
    """Corpora whose sources live in the repository: their manifests keep
    repo-relative paths so a cache built in one worktree works in another."""
    _, src, _ = corpus_paths(name)
    return src.is_relative_to(ROOT)


def collect(name):
    """[(id, path)] for a corpus, deterministic order."""
    _, src, _ = corpus_paths(name)
    out = []
    if name == "fastr":
        snips = CACHE / "snips/fastr"
        snips.mkdir(parents=True, exist_ok=True)
        for line in open(ROOT / "tests/corpora/fastr/cases.jsonl"):
            case = json.loads(line)
            cid = case["id"]
            p = snips / f"{cid}.R"
            data = case["code"].encode()
            if not p.exists() or p.read_bytes() != data:
                p.parent.mkdir(parents=True, exist_ok=True)
                p.write_bytes(data)
            out.append((cid, p))
        return out
    pattern = "*.R"
    for p in sorted(src.rglob(pattern)):
        rel = p.relative_to(src).as_posix()
        out.append((rel, p))
    return out


def cmd_list(names, _args):
    for name in names:
        entries = []
        for id_, p in collect(name):
            data = p.read_bytes()
            enc = "u" if not data or is_utf8(data) else "l"
            try:  # in-repo sources keep repo-relative paths in the manifest
                shown = p.relative_to(ROOT).as_posix()
            except ValueError:
                shown = str(p.resolve())
            entries.append((id_, shown, enc, hashlib.sha256(data).hexdigest()[:16]))
        path = CACHE / f"manifests/{name}.tsv"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("".join(f"{id_}\t{p}\t{enc}\t{sha}\n" for id_, p, enc, sha in entries))
        latin1 = sum(1 for e in entries if e[2] == "l")
        print(f"{name}: {len(entries)} sources ({latin1} Latin-1) -> {path}")


def abs_manifest(name):
    """Path to a manifest whose paths are absolute: the committed one for
    external corpora, or a generated copy with ROOT joined to relative
    paths for in-repo corpora."""
    path = CACHE / f"manifests/{name}.tsv"
    if not path.exists():
        cmd_list([name], None)
    if not in_repo(name):
        return path
    lines = []
    for line in path.read_text().splitlines():
        id_, rest = line.split("\t", 1)
        parts = rest.split("\t")
        p = parts[0]
        if not p.startswith("/"):
            p = str(ROOT / p)
        lines.append("\t".join([id_, p] + parts[1:]))
    out = CACHE / f"manifests/{name}.abs.tsv"
    out.write_text("".join(l + "\n" for l in lines))
    return out


def is_utf8(data):
    try:
        data.decode("utf-8")
        return True
    except UnicodeDecodeError:
        return False


def record_corpus(name, force):
    """Record GNU R's dumps for a corpus; returns the records dict."""
    _, _, committed = corpus_paths(name)
    mpath = abs_manifest(name)
    stamp = CACHE / f"records/{name}.tsv"
    oracle = ROOT / "scripts/oracle-parse-batch.R"
    # The cache key hashes the canonical manifest (relative paths for
    # in-repo corpora), so it is the same in every worktree.
    canon = CACHE / f"manifests/{name}.tsv"
    key = hashlib.sha256(canon.read_bytes() + oracle.read_bytes() + (ROOT / "scripts/oracle-parse.R").read_bytes()).hexdigest()[:16]
    if not force and stamp.exists() and (CACHE / f"records/{name}.key").read_text() == key:
        return read_records(stamp)
    passes = []
    stamp.parent.mkdir(parents=True, exist_ok=True)
    for run in (1, 2):
        out = stamp.with_suffix(f".tsv.{run}")
        env = dict(os.environ, **R_ENV, R_DEFAULT_PACKAGES="NULL")
        subprocess.run(
            ["Rscript", "--vanilla", str(oracle), str(mpath), str(out)],
            check=True, env=env, cwd="/tmp",
        )
        passes.append(read_records(out))
    # The `\u{xxxx}` lexer error prints an uninitialized value; sources whose
    # records are not reproducible are unstable and not part of the corpus.
    unstable = [k for k in passes[0] if passes[0][k] != passes[1].get(k)]
    records = {k: v for k, v in passes[0].items() if k not in unstable}
    stamp.parent.mkdir(parents=True, exist_ok=True)
    with open(stamp, "wb") as f:
        for k in sorted(records):
            f.write(f"{k}\t".encode() + records[k] + b"\n")
        f.flush()
        os.fsync(f.fileno())
    (CACHE / f"records/{name}.key").write_text(key)
    stamp.with_suffix(".tsv.1").unlink(missing_ok=True)
    stamp.with_suffix(".tsv.2").unlink(missing_ok=True)
    manifest_ids = {line.split("\t", 1)[0] for line in mpath.read_text().splitlines()}
    missing = manifest_ids - set(records)
    if unstable or missing:
        print(f"{name}: {len(unstable)} unstable, {len(missing)} unreadable (excluded)")
    if committed:
        with gzip.open(committed, "wb") as f:
            for k in sorted(records):
                f.write(f"{k}\t".encode() + records[k] + b"\n")
        print(f"{name}: {len(records)} records -> {committed.relative_to(ROOT)}")
    else:
        print(f"{name}: {len(records)} records -> {stamp}")
    return records


def read_records(path):
    records = {}
    with open(path, "rb") as f:
        for line in f:
            id_, dump = line.rstrip(b"\n").split(b"\t", 1)
            records[id_.decode()] = dump
    return records


def build_parse_batch():
    subprocess.run(
        ["cargo", "build", "--release", "-p", "robigo-syntax", "--bin", "parse-batch"],
        check=True, cwd=ROOT,
    )
    target = os.environ.get("CARGO_TARGET_DIR", str(Path.home() / ".cache/cargo-target"))
    binp = Path(target) / "release/parse-batch"
    if not binp.exists():
        raise SystemExit(f"parse-batch not found at {binp}")
    return binp


def cmd_check(names, args):
    binp = build_parse_batch()
    overall_ok = True
    for name in names:
        _, _, committed = corpus_paths(name)
        mpath = abs_manifest(name)
        records = None
        if committed and committed.exists():
            records = read_records_gzip(committed)
        else:
            stamp = CACHE / f"records/{name}.tsv"
            if stamp.exists():
                records = read_records(stamp)
        if records is None:
            print(f"{name}: no records (run `record` first); skipped")
            continue
        ours_path = CACHE / f"ours/{name}.tsv"
        ours_path.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run([str(binp), str(mpath), str(ours_path)], check=True, cwd=ROOT)
        ours = read_records(ours_path)
        ids = sorted(records)
        passing = [k for k in ids if records[k] == ours.get(k)]
        failed = [(k, records[k], ours.get(k)) for k in ids if records[k] != ours.get(k)]
        ratchet_path = RATCHET_DIR / f"{name}.txt"
        listed = set(ratchet_path.read_text().split()) if ratchet_path.exists() else set()
        lost = sorted(listed - set(passing))
        gained = sorted(set(passing) - listed)
        total = len(ids)
        print(f"{name}: {len(passing)}/{total} match (ratchet {len(listed)}; "
              f"+{len(gained)} new, -{len(lost)} lost)")
        if args.verbose:
            for k, want, got in failed[: args.show]:
                print(f"  {k}\n    R:      {want[:300].decode('utf-8', 'replace')}\n    robigo: {(got or b'<missing>')[:300].decode('utf-8', 'replace')}")
        if lost:
            overall_ok = False
            print(f"  REGRESSED: {', '.join(lost[:20])}{' ...' if len(lost) > 20 else ''}")
        if args.update:
            ratchet_path.parent.mkdir(parents=True, exist_ok=True)
            ratchet_path.write_text("".join(k + "\n" for k in sorted(passing)))
            print(f"  ratchet -> {ratchet_path.relative_to(ROOT)} ({len(passing)})")
        if failed and not args.verbose:
            print(f"  ({len(failed)} mismatches; use --verbose)")
    sys.exit(0 if overall_ok else 1)


def read_records_gzip(path):
    records = {}
    with gzip.open(path, "rb") as f:
        for line in f:
            id_, dump = line.rstrip(b"\n").split(b"\t", 1)
            records[id_.decode()] = dump
    return records


def cmd_show(name, id_):
    stamp = CACHE / f"records/{name}.tsv"
    ours = CACHE / f"ours/{name}.tsv"
    for label, path in (("R", stamp), ("robigo", ours)):
        if path.exists():
            for line in open(path, "rb"):
                if line.startswith(id_.encode() + b"\t"):
                    print(f"{label}: {line.split(chr(9).encode(), 1)[1].decode('utf-8', 'replace')}")


def cmd_fetch_cran(_names, _args):
    """Download the source tarballs of the installed CRAN packages."""
    if not R_LIBS.exists():
        raise SystemExit(f"no installed library at {R_LIBS}")
    CRAN_SRC.mkdir(parents=True, exist_ok=True)
    script = ROOT / "scripts/parse-cran-download.R"
    env = dict(os.environ, **R_ENV)
    subprocess.run(["Rscript", "--vanilla", str(script), str(CRAN_SRC), str(R_LIBS)], check=True, env=env)
    n = 0
    for tar in sorted(CRAN_SRC.glob("*.tar.gz")):
        pkg = tar.name.split("_")[0]
        dest = CRAN_SRC / pkg
        if not dest.exists():
            with tarfile.open(tar) as t:
                inner = t.getmembers()[0].name.split("/")[0]
                t.extractall(CRAN_SRC, filter="data")
            (CRAN_SRC / inner).rename(dest)
        n += 1
    print(f"cran: {n} packages in {CRAN_SRC}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("command", choices=["list", "record", "check", "show", "fetch-cran"])
    ap.add_argument("corpora", nargs="*", help=f"one of {sorted(CORPUS_NAMES)} (default: all local)")
    ap.add_argument("--force", action="store_true", help="re-record even if cached")
    ap.add_argument("--update", action="store_true", help="add new passes to the ratchet")
    ap.add_argument("--verbose", "-v", action="store_true", help="show mismatching dumps")
    ap.add_argument("--show", type=int, default=10, help="how many mismatches --verbose shows")
    args = ap.parse_args()
    names = args.corpora or [n for n in CORPUS_NAMES if n != "cran" or CRAN_SRC.exists()]
    if args.command == "list":
        cmd_list(names, args)
    elif args.command == "record":
        for name in names:
            record_corpus(name, args.force)
    elif args.command == "check":
        cmd_check(names, args)
    elif args.command == "show":
        cmd_show(names[0], args.corpora[1] if len(args.corpora) > 1 else None)
    elif args.command == "fetch-cran":
        cmd_fetch_cran(names, args)


CORPUS_NAMES = ["rlib", "rtests", "fastr", "gnur-tests", "gnur-examples", "cran"]

if __name__ == "__main__":
    main()
