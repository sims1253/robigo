#!/usr/bin/env python3
"""The C API census: which libR symbols do R packages actually import?

    scripts/capi-census.py scan             # write tests/capi/symbols.tsv
    scripts/capi-census.py coverage [--top N]
    scripts/capi-census.py symbols PATTERN...   # grep the census

`scan` reads every shared library of GNU R's own packages
(`$(R RHOME)/library/*/libs/*.so`) and of the installed CRAN packages
(`$R_LIBS_USER/*/libs/*.so`), collects the symbols they import from
`$(R RHOME)/lib/libR.so` (`nm -D`, the undefined symbols a dynamic loader
would resolve against libR), and writes `tests/capi/symbols.tsv`:
symbol, function or data, how many packages import it, GNU R's own packages
that do, the header that declares it, and its API status in R 4.5
(`api`/`eapi`/`emb` from `tools:::funAPI()`, the lists of "Writing R
Extensions"; empty: not documented API).

`coverage` builds the `capi` corpus (its build hook, GNU R's toolchain),
reads from every case the shared libraries it `dyn.load`s, and reports how
much of the census they exercise, weighted by the number of packages that
import each symbol:

    coverage = (sum of n_pkgs over symbols a corpus case imports)
             / (sum of n_pkgs over all census symbols)

That is the metric of the `capi` track: G2 (`robigo-capi`) hill-climbs it
by making cases pass, new cases can only raise the total they cover.
"""
import argparse
import collections
import os
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TSV = os.path.join(ROOT, "tests", "capi", "symbols.tsv")
CORPUS = os.path.join(ROOT, "tests", "corpora", "capi")

# The oracle's environment (scripts/corpus.py).
BASE_ENV = {"LANGUAGE": "en", "LC_ALL": "C.UTF-8", "R_ENVIRON_USER": "", "R_PROFILE_USER": ""}

NM_LINE = re.compile(r"^\s*[0-9a-fA-F]*\s+(\S+)\s+(\S+)$")

FUNCTION_LETTERS = set("TtWwIi")


def r(args, **kw):
    env = dict(os.environ, **BASE_ENV)
    return subprocess.run(args, capture_output=True, text=True, check=True, env=env, **kw)


def r_home():
    return r(["R", "RHOME"]).stdout.strip()


def rscript(expr):
    return r(["Rscript", "--vanilla", "-e", expr]).stdout


def nm(path, mode):
    """`nm -D` output as {symbol: letter}."""
    out = r(["nm", "--dynamic"] + mode + [path]).stdout
    syms = {}
    for line in out.splitlines():
        m = NM_LINE.match(line)
        if m:
            syms[m.group(2).split("@")[0]] = m.group(1)
    return syms


# ------------------------------------------------------------------ scan

def package_libs(rhome):
    """[(package, "gnur"|"cran", so_path)] for every installed package .so."""
    libs = []
    for so in sorted(__import__("glob").glob(os.path.join(rhome, "library", "*", "libs", "*.so"))):
        libs.append((os.path.basename(os.path.dirname(os.path.dirname(so))), "gnur", so))
    user = os.path.expanduser(rscript("cat(Sys.getenv('R_LIBS_USER'))").strip())
    for so in sorted(__import__("glob").glob(os.path.join(user, "*", "libs", "*.so"))):
        libs.append((os.path.basename(os.path.dirname(os.path.dirname(so))), "cran", so))
    return libs


def fun_api():
    """{symbol: apitype} from tools:::funAPI() (R 4.5's API lists)."""
    out = rscript("a <- tools:::funAPI(); cat(paste(a$name, a$apitype, sep='\\t'), sep='\\n')")
    return dict(line.split("\t", 1) for line in out.splitlines() if "\t" in line)


def headers(rhome):
    """{header: text} for every header under R's include directory."""
    inc = rscript("cat(R.home('include'))").strip() or os.path.join(rhome, "include")
    out = {}
    for root, _, files in os.walk(inc):
        for fn in files:
            if fn.endswith(".h"):
                p = os.path.join(root, fn)
                with open(p, encoding="utf-8", errors="replace") as f:
                    out[os.path.relpath(p, inc)] = f.read()
    return out


def declaring_headers(sym, hdrs, cache):
    """Headers mentioning sym as a whole word (cached per sym)."""
    if sym not in cache:
        rx = re.compile(r"(?<![0-9A-Za-z_])" + re.escape(sym) + r"(?![0-9A-Za-z_])")
        cache[sym] = [h for h, text in sorted(hdrs.items()) if rx.search(text)]
    return cache[sym]


def cmd_scan(a):
    rhome = r_home()
    libR = os.path.join(rhome, "lib", "libR.so")
    exports = nm(libR, ["--defined-only"])
    libs = package_libs(rhome)
    api = fun_api()
    hdrs = headers(rhome)
    hdr_cache = {}

    # symbol -> {package: kind}; kind remembers whether an R-core package imports it
    imported = collections.defaultdict(dict)
    for pkg, kind, so in libs:
        for sym, letter in nm(so, ["--undefined-only"]).items():
            if sym in exports:
                imported[sym][pkg] = kind
    n_gnur = sum(1 for _, k, _ in libs if k == "gnur")
    n_cran = len(libs) - n_gnur

    os.makedirs(os.path.dirname(TSV), exist_ok=True)
    with open(TSV, "w", encoding="utf-8") as f:
        f.write("# libR symbols imported by installed R packages from "
                f"{libR} (nm -D), {n_gnur} of GNU R's own + {n_cran} CRAN packages.\n")
        f.write("# api: api|eapi|emb per tools:::funAPI() (Writing R Extensions); "
                "empty: not documented. Regenerate: scripts/capi-census.py scan\n")
        f.write("symbol\tkind\tn_pkgs\tgnur_pkgs\theader\tapi\n")
        rows = sorted(imported.items(), key=lambda kv: (-len(kv[1]), kv[0]))
        for sym, pkgs in rows:
            letter = exports[sym]
            kind = "function" if letter in FUNCTION_LETTERS else "data"
            gnur = ",".join(sorted(p for p, k in pkgs.items() if k == "gnur"))
            hdr = ";".join(declaring_headers(sym, hdrs, hdr_cache))
            f.write(f"{sym}\t{kind}\t{len(pkgs)}\t{gnur}\t{hdr}\t{api.get(sym, '')}\n")
    total = sum(len(p) for p in imported.values())
    print(f"{TSV}: {len(imported)} symbols, total weight {total} "
          f"(from {n_gnur} GNU R + {n_cran} CRAN packages)")


# -------------------------------------------------------------- coverage

def read_census():
    """[(symbol, n_pkgs)] from symbols.tsv."""
    if not os.path.exists(TSV):
        sys.exit(f"no census at {TSV}; run: scripts/capi-census.py scan")
    with open(TSV, encoding="utf-8") as f:
        rows = [l.split("\t") for l in f if not l.startswith("#") and l.strip()]
    if rows and rows[0][0] == "symbol":  # the column header
        rows = rows[1:]
    return {r[0]: int(r[2]) for r in rows}


def case_dynlibs(case_src):
    """Shared-library names a case dyn.loads, in order."""
    return re.findall(r"dyn\.load\(\s*[\"']([^\"']+)[\"']", case_src)


def build_corpus():
    hook = os.path.join(CORPUS, "build.sh")
    if not os.path.exists(hook):
        sys.exit(f"no build hook at {hook}")
    subprocess.run(["sh", hook], check=True)


def cmd_coverage(a):
    census = read_census()
    build_corpus()
    support = os.path.join(CORPUS, "support")
    cases_dir = os.path.join(CORPUS, "cases")

    # case -> imported census symbols, via the .so files it dyn.loads
    per_case = {}
    so_imports = {}
    for root, _, files in sorted(os.walk(cases_dir)):
        for fn in sorted(files):
            if not fn.endswith(".R"):
                continue
            cid = os.path.relpath(os.path.join(root, fn), cases_dir)[:-2]
            with open(os.path.join(root, fn), encoding="utf-8") as f:
                src = f.read()
            syms = set()
            for so in case_dynlibs(src):
                if so not in so_imports:
                    p = os.path.join(support, so)
                    if not os.path.exists(p):
                        sys.exit(f"case {cid} dyn.loads {so}, which build.sh did not build")
                    undefined = set(nm(p, ["--undefined-only"]))
                    so_imports[so] = {s for s in undefined if s in census}
                syms |= so_imports[so]
            per_case[cid] = syms

    covered = set().union(*per_case.values()) if per_case else set()
    got = sum(census[s] for s in covered)
    total = sum(census.values())
    for cid in sorted(per_case):
        w = sum(census[s] for s in per_case[cid])
        print(f"{cid:24} {len(per_case[cid]):4} symbols, weight {w}")
    print(f"\ncoverage: {len(covered)}/{len(census)} symbols, "
          f"weighted {got}/{total} = {100 * got / max(total, 1):.1f}%")
    if a.top:
        print(f"\ntop {a.top} uncovered symbols by weight:")
        for sym, n in sorted(census.items(), key=lambda kv: (-kv[1], kv[0])):
            if sym not in covered:
                print(f"  {n:4} {sym}")
                a.top -= 1
                if not a.top:
                    break


def cmd_symbols(a):
    census = read_census()
    for sym, n in sorted(census.items(), key=lambda kv: (-kv[1], kv[0])):
        if any(re.search(p, sym) for p in a.pattern):
            print(f"{n:4} {sym}")


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("scan")
    p = sub.add_parser("coverage")
    p.add_argument("--top", type=int, default=0, help="also list the N heaviest uncovered symbols")
    p = sub.add_parser("symbols")
    p.add_argument("pattern", nargs="+", help="regexes to search for")
    a = ap.parse_args()
    {"scan": cmd_scan, "coverage": cmd_coverage, "symbols": cmd_symbols}[a.cmd](a)


if __name__ == "__main__":
    main()
