#!/usr/bin/env bash
# The translation cache carries source references (C1's round trip for
# source references S1, docs/ledger/g3/2026-11-23-srcref-s1.md): a tree built
# with R_KEEP_PKG_SOURCE=yes, whose compiler database keeps its functions'
# srcrefs (sys.source(keep.source = getOption("keep.source.pkgs")), as GNU
# R's build does), and its translation cache. MAKECLOSURE's constant then
# carries the inner function's srcref (cmp.R), and every closure
# `compiler:::make.codeBuf` makes must get the srcref that is its pool
# constant (identical()), with the cache on (hits), off, and in verify mode
# (each hit against a fresh translation, none differing).
#
#   scripts/rt-srcref-tcache.sh --robigo-rt BIN [DIR]   # default .robigo/rt-home-db-ks
#
# It builds the tree (scripts/rt-home-db.sh: minutes), so it is no per-PR
# check: a release check, for the nightly. The cache reads only the tree's
# own databases (tcache.rs: nothing a user sets points the reader at
# another file), so no smaller database can stand in for it.
set -euo pipefail
cd "$(dirname "$0")/.."
usage() { echo "usage: scripts/rt-srcref-tcache.sh --robigo-rt BIN [DIR]" >&2; exit 2; }
[ "${1:-}" = --robigo-rt ] && [ $# -ge 2 ] && [ -n "$2" ] || usage
bin=$(realpath "$2")
tree=${3:-.robigo/rt-home-db-ks}
R_KEEP_PKG_SOURCE=yes scripts/rt-home-db.sh --robigo-rt "$bin" "$tree"
# the install layout robigo-rt finds its tree by: DIR/bin/robigo-rt beside
# DIR/lib/robigo and DIR/lib/robigo-cache (an R_HOME in the environment is
# ignored, as GNU R's is)
inst=$(mktemp -d)
trap 'rm -rf "$inst"' EXIT
mkdir -p "$inst/bin" "$inst/lib"
cp "$bin" "$inst/bin/robigo-rt"
ln -s "$(realpath "$tree")" "$inst/lib/robigo"
ln -s "$(realpath "$tree")-cache" "$inst/lib/robigo-cache"
cat > "$inst/check.R" <<'EOF'
ns <- asNamespace("compiler")
f <- ns$make.codeBuf
stopifnot(!is.null(attr(f, "srcref")))
consts <- .Internal(disassemble(.Internal(bodyCode(f))))[[3]]
pool <- lapply(Filter(function(k) is.list(k) && length(k) == 3 && inherits(k[[3]], "srcref"), consts), `[[`, 3)
fns <- Filter(is.function, f(quote(x + 1)))
srs <- lapply(fns, attr, "srcref")
stopifnot(length(pool) > 0, length(fns) == length(pool),
          all(vapply(srs, function(s) any(vapply(pool, identical, NA, s)), NA)),
          identical(lapply(Filter(is.function, f(quote(y))), attr, "srcref"), srs),
          identical(attr(srs[[1]], "srcfile"), attr(attr(f, "srcref"), "srcfile")))
cat("srcref round trip ok:", length(fns), "closures\n")
EOF
fail=0
for mode in 1 0 verify; do
  err=$inst/err-$mode
  out=$(env -u R_HOME -u ROBIGO_RT_CHECKOUT ROBIGO_RT_BOOT=rdb R_DEFAULT_PACKAGES=NULL \
    ROBIGO_TRANSLATION_CACHE=$mode ROBIGO_TRANSLATION_CACHE_STATS=1 \
    "$inst/bin/robigo-rt" --vanilla --no-echo -f "$inst/check.R" 2> "$err") || true
  stats=$(grep '^translation cache:' "$err" || true)
  echo "cache $mode: ${out:-no output} | ${stats:-no statistics}"
  case $out in "srcref round trip ok:"*) ;; *) fail=1; cat "$err" >&2 ;; esac
  hits=$(sed -n 's/.*hit \([0-9]*\),.*/\1/p' <<< "$stats")
  differing=$(sed -n 's/.*differing \([0-9]*\),.*/\1/p' <<< "$stats")
  verified=$(sed -n 's/.*verified \([0-9]*\),.*/\1/p' <<< "$stats")
  case $mode in
    1) [ "${hits:-0}" -gt 0 ] || { echo "rt-srcref-tcache.sh: no cache hits" >&2; fail=1; } ;;
    0) [ "${hits:-1}" -eq 0 ] || { echo "rt-srcref-tcache.sh: hits with the cache off" >&2; fail=1; } ;;
    verify) [ "${verified:-0}" -gt 0 ] && [ "${differing:-1}" -eq 0 ] ||
      { echo "rt-srcref-tcache.sh: verify mode verified ${verified:-?}, differing ${differing:-?}" >&2; fail=1; } ;;
  esac
done
exit $fail
