#!/usr/bin/env python3
"""Reduce a failing R script to a small one that fails the same way.

    scripts/reduce.py FILE.R [--mode auto|oracle|tiers|crash|emi] [--seconds N]
                      [--jobs N] [--robigo BIN] [--oracle RSCRIPT] [--loose] [-o OUT]
    scripts/reduce.py --corpus NAME --case ID [...]

The failure is one of scripts/rcheck.py's verdicts:

  oracle  robigo's output differs from GNU R 4.5.0's;
  tiers   robigo's tiers (interpreter, VM, VM with JIT) disagree;
  crash   robigo panics, overflows its stack or dies of a signal;
  emi     the program and the program with code that never runs
          (rast.insert_dead) print different things in robigo.

`auto` (the default) reads the mode from a scripts/fuzz-eval.py reproducer's
name (diff-, tier-, crash-, emi-) and is `oracle` otherwise. "The same way"
is the verdict's key: the stream that differs and the first differing line
on each side, the tier blamed, the crash kind. `--loose` keeps only the
stream, for a failure whose text moves as code is deleted.

The reduction is hierarchical delta debugging (Misherghi and Su, after
Zeller's ddmin) over GNU R's own parse tree (scripts/rast.py): first
statements of each block, then arguments of each call, comments, then each
expression replaced by one of its subexpressions, to a fixed point or the
time budget. Every candidate is a script GNU R parses; source text that is
kept stays byte for byte. Candidates run in parallel and their verdicts are
cached by text. The result is checked once more with the reference run twice
and written next to the input as NAME.min.R (or -o), and printed.

--corpus/--case reduce a corpus case (scripts/triage.py prints the command)
against GNU R, run the way scripts/corpus.py runs the case (its mode,
packages, support files); the oracle's bin directory goes first on PATH.
"""
import argparse
import concurrent.futures
import os
import pathlib
import re
import shutil
import sys
import tempfile
import threading
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
import corpus  # noqa: E402
import rast  # noqa: E402
import rcheck  # noqa: E402

PREFIX_MODE = {"diff": "oracle", "tier": "tiers", "crash": "crash", "emi": "emi"}


def log(*a):
    print(*a, file=sys.stderr, flush=True)


class Search:
    """The interestingness test, cached and parallel: `first(cands)` is
    the index of the first candidate (in order) that still fails the same
    way."""

    def __init__(self, test, parser, jobs):
        self.test, self.parser, self.jobs = test, parser, jobs
        self.cache, self.lock = {}, threading.Lock()
        self.pool = concurrent.futures.ThreadPoolExecutor(jobs)
        self.runs = 0

    def ok(self, text):
        with self.lock:
            if text in self.cache:
                return self.cache[text]
        r = self.parser.parse(text) is not None and self.test(text)
        with self.lock:
            self.cache[text] = r
            self.runs += 1
        return r

    def first(self, cands):
        for i in range(0, len(cands), self.jobs):
            chunk = cands[i:i + self.jobs]
            for j, r in enumerate(self.pool.map(self.ok, chunk)):
                if r:
                    return i + j
        return None


def ddmin(search, group, deadline):
    """The smallest subset of GROUP's units (that ddmin finds) whose
    rendering still fails; None when no unit can go."""
    keep = list(range(len(group.units)))
    gran, reduced = 2, None
    while keep and time.time() < deadline:
        size = max(1, len(keep) // min(gran, len(keep)))
        chunks = [keep[i:i + size] for i in range(0, len(keep), size)]
        sets = [[k for k in keep if k not in c] for c in chunks]
        if len(chunks) > 2:
            sets += chunks
        texts = [group.render(set(s)) for s in sets]
        i = search.first(texts)
        if i is not None:
            keep, reduced = sets[i], texts[i]
            gran = max(gran - 1, 2) if i < len(chunks) else 2
            continue
        if gran >= len(keep):
            break
        gran = min(len(keep), gran * 2)
    return reduced


def reduce_text(src, search, parser, deadline):
    """SRC reduced while SEARCH says it fails: passes over deletion groups
    and hoists until none makes progress."""
    cur = src
    progress = True
    while progress and time.time() < deadline:
        progress = False
        # deletion: each group once per round, outermost first; after a
        # success the tree is parsed again and the walk resumes at the same
        # group index (the groups before it were just tried)
        g = 0
        while time.time() < deadline:
            gs = rast.groups(parser.parse(cur))
            if g >= len(gs):
                break
            r = ddmin(search, gs[g], deadline)
            if r is not None and len(r) < len(cur):
                log(f"  {len(cur):6d} -> {len(r):6d} bytes (deleted in group {g + 1}/{len(gs)})")
                cur, progress = r, True
            else:
                g += 1
        # hoisting: the first subexpression that can replace its parent
        while time.time() < deadline:
            cands = [c for c in rast.hoists(parser.parse(cur)) if len(c) < len(cur)]
            i = search.first(cands)
            if i is None:
                break
            log(f"  {len(cur):6d} -> {len(cands[i]):6d} bytes (hoisted)")
            cur, progress = cands[i], True
    tidy = "\n".join(line.rstrip() for line in cur.split("\n") if line.strip()) + "\n"
    if tidy != cur and search.ok(tidy):
        cur = tidy
    return cur


class CorpusChecker(rcheck.Checker):
    """Runs a script as scripts/corpus.py runs case CASE of corpus NAME."""

    def __init__(self, name, case, robigo, **kw):
        super().__init__(robigo, oracle="gnur", **kw)
        self.name, self.case, self.conf = name, case, corpus.corpus_conf(name)

    def _run(self, path, runtime, tier=None):
        # one working directory per thread: candidates run in parallel
        cid = f"{self.case}@reduce{threading.get_ident()}"
        o, e, s = corpus.run_case(self.name, self.conf, cid, pathlib.Path(path).read_bytes(), runtime,
                                  self.robigo_bin, tier)
        return rcheck.Run(o, e, "timeout" if e.endswith(b"<robigo-corpus: timeout>\n") else s)

    def robigo(self, path, tier=None):
        return self._run(path, "robigo", tier)

    def gnur(self, path):
        return self._run(path, "gnur")


def verdict(checker, mode, path, parser):
    if mode == "oracle":
        return checker.oracle(path)
    if mode == "tiers":
        return checker.tiers(path)
    if mode == "crash":
        return checker.crash(path)
    t = parser.parse(pathlib.Path(path).read_text(encoding="utf-8", errors="surrogateescape"))
    orig = rast.strip_dead(t) if t else None
    if orig is None or orig == t.src:
        return rcheck.Verdict("ok", None, "no dead code to compare with")
    p = pathlib.Path(path).with_suffix(".orig.R")
    p.write_text(orig, encoding="utf-8", errors="surrogateescape")
    return checker.emi(p, path)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("file", nargs="?")
    ap.add_argument("--mode", default="auto", choices=["auto", "oracle", "tiers", "crash", "emi"])
    ap.add_argument("--corpus")
    ap.add_argument("--case")
    ap.add_argument("--robigo", default=str(ROOT / ".robigo/bin/robigo"))
    ap.add_argument("--oracle")
    ap.add_argument("--seconds", type=int, default=600)
    ap.add_argument("--jobs", type=int, default=os.cpu_count())
    ap.add_argument("--loose", action="store_true", help="the same stream differs (not the same lines)")
    ap.add_argument("-o", "--out")
    a = ap.parse_args()
    if bool(a.corpus) != bool(a.case) or bool(a.file) == bool(a.corpus):
        ap.error("give FILE, or --corpus and --case")
    robigo = pathlib.Path(a.robigo).resolve()
    if not robigo.exists():
        sys.exit(f"{robigo} is missing; run scripts/build.sh")
    rcheck.limit_resources()  # as scripts/fuzz-eval.py ran the input

    if a.corpus:
        srcs = dict(corpus.load_cases(a.corpus))
        if a.case not in srcs:
            sys.exit(f"{a.corpus} has no case {a.case!r}")
        src = srcs[a.case].decode("utf-8", "surrogateescape")
        mode = "oracle" if a.mode == "auto" else a.mode
        out = pathlib.Path(a.out or ROOT / ".robigo/reduce" / a.corpus / (re.sub(r"[^\w.-]", "_", a.case) + ".min.R"))
    else:
        path = pathlib.Path(a.file)
        src = path.read_text(encoding="utf-8", errors="surrogateescape")
        prefix = path.name.split("-", 1)[0]
        mode = PREFIX_MODE.get(prefix, "oracle") if a.mode == "auto" else a.mode
        out = pathlib.Path(a.out or path.with_name(path.name[:-2] + ".min.R" if path.name.endswith(".R")
                                                   else path.name + ".min.R"))

    rscript, env = None, dict(rcheck.ENV)
    if mode == "oracle":
        rscript, home = rcheck.require_oracle(a.oracle)
        env["R_HOME"] = home
    elif os.environ.get("R_HOME"):
        env["R_HOME"] = os.environ["R_HOME"]

    if a.corpus and rscript:
        # corpus.run_case runs `Rscript`/`R` from PATH with this environment
        os.environ["PATH"] = f"{pathlib.Path(rscript).parent}:{os.environ['PATH']}"
        os.environ["R_HOME"] = env["R_HOME"]

    def checker(stable):
        if a.corpus:
            return CorpusChecker(a.corpus, a.case, robigo, env=env, stable=stable, loose=a.loose)
        return rcheck.Checker(robigo, rscript, env=env, stable=stable, loose=a.loose)

    parser = rast.Parser(rscript or rcheck.find_oracle(a.oracle))  # any GNU R parses
    work = pathlib.Path(tempfile.mkdtemp(prefix="robigo-reduce-"))
    try:
        if parser.parse(src) is None:
            sys.exit("GNU R cannot parse the script: nothing to reduce over (scripts/fuzz.py parse finds parser bugs)")
        first = work / "original.R"
        first.write_text(src, encoding="utf-8", errors="surrogateescape")
        v = verdict(checker(True), mode, first, parser)
        if v.kind not in ("diff", "crash"):
            sys.exit(f"{mode}: the script does not fail ({v.kind}{': ' + str(v.key) if v.key else ''}); nothing to reduce")
        target = v.key
        log(f"{mode}: reducing {len(src)} bytes, failure {target}")

        fast = checker(False)
        tls = threading.local()

        def test(text):
            if not hasattr(tls, "dir"):
                tls.dir = pathlib.Path(tempfile.mkdtemp(dir=work))
            p = tls.dir / "case.R"
            p.write_text(text, encoding="utf-8", errors="surrogateescape")
            return verdict(fast, mode, p, parser).key == target

        search = Search(test, parser, max(1, a.jobs))
        t0 = time.time()
        small = reduce_text(src, search, parser, t0 + a.seconds)
        # candidates still running after an early return or the deadline
        search.pool.shutdown(wait=True, cancel_futures=True)
        final = work / "final.R"
        final.write_text(small, encoding="utf-8", errors="surrogateescape")
        again = verdict(checker(True), mode, final, parser)
        if again.key != target:
            log(f"the reduced script does not fail the same way when checked again ({again.kind}): "
                "the failure is flaky; keeping the original")
            small = src
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(small, encoding="utf-8", errors="surrogateescape")
        log(f"{len(src)} -> {len(small)} bytes in {time.time() - t0:.0f}s, {search.runs} candidates; wrote {out}")
        sys.stdout.buffer.write(small.encode("utf-8", "surrogateescape"))
    finally:
        parser.close()
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()
