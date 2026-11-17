#!/usr/bin/env bash
# What GNU R 4.6.2's own build of base, compiler and tools reaches (L4,
# m2m3-plan): each R step src/library/Makefile.in's R target runs is run by
# GNU R itself, traced by scripts/base-subset-trace.R, and the base
# functions it reaches are written as keys (closure:NAME, primitive:NAME,
# internal:NAME), one file per step. scripts/rt-build-gaps.py then asks a
# robigo-rt binary which of them it lacks.
#   scripts/rt-build-reach.sh [DIR]    # default .robigo/build-reach
# The steps, on a copy of GNU R's installation whose base, tools and
# compiler are in source form (library/PKG/R/PKG as basepkg.mk's mkRbase
# and mkR1 make it, no databases), as the build has them:
#   base      makebasedb.R (_R_COMPILE_PKGS_=1)
#   compiler  tools:::makeLazyLoading("compiler") (lazycomp.mk)
#   tools     makeLazyLoad.R and makeLazyLoading("tools")
#   desc      tools:::.install_package_description (basepkg.mk mkdesc)
#   vdesc     tools:::.vinstall_package_descriptions_as_RDS
#   vns       tools:::.vinstall_package_namespaces_as_RDS
# plus static.keys: the primitives and .Internals compiler's and tools' own
# code calls (the tracer records base's closures only).
# Base booted from source has no .C_* symbols (baseloader.R makes them for
# a lazy-loaded base); the tracer's addTaskCallback needs one, so the
# profile defines them in the global environment (base's enclosure) first.
# Needs GNU R 4.6.2: the Rscript on PATH if it is that version (in the
# oracle image), else docker's robigo-oracle:4.6.2.
set -euo pipefail
cd "$(dirname "$0")/.."
root=$PWD
out=${1:-.robigo/build-reach}
case "$out" in /*) ;; *) out=$root/$out ;; esac
# shellcheck source=scripts/oracle-pins.sh
. scripts/oracle-pins.sh
if [ "${RT_BUILD_REACH_INSIDE:-}" != 1 ]; then
  src=$(scripts/r-src.sh)
  home=$(scripts/rt-home.sh)
  rm -rf "$out"
  mkdir -p "$out/in"
  for p in base tools compiler; do cp "$home/library/$p/R/$p" "$out/in/$p-all.R"; done
  cp "$src/src/library/base/makebasedb.R" "$src/src/library/tools/R/makeLazyLoad.R" "$out/in/"
  cp "$src/src/library/tools/R/sysdata.rda" "$out/in/tools-sysdata.rda"
  sed -e "s/@VERSION@/$rt/g" "$src/src/library/compiler/DESCRIPTION.in" > "$out/in/DESCRIPTION"
  if command -v Rscript > /dev/null && [ "$(Rscript -e 'cat(format(getRversion()))' 2> /dev/null)" = "$rt" ]; then
    RT_BUILD_REACH_INSIDE=1 exec "$0" "$out"
  fi
  RT_BUILD_REACH_INSIDE=1 exec scripts/oracle-run.sh "$rt" env RT_BUILD_REACH_INSIDE=1 "$0" "$out"
fi

# inside: GNU R 4.6.2 is on PATH
rhome=$(R RHOME)
share=$(R --vanilla --no-echo -e 'cat(R.home("share"))')
include=$(R --vanilla --no-echo -e 'cat(R.home("include"))')
doc=$(R --vanilla --no-echo -e 'cat(R.home("doc"))')
H=$out/home
cp -a "$rhome" "$H"
for p in base tools compiler; do
  rm -f "$H/library/$p/R/$p.rdb" "$H/library/$p/R/$p.rdx"
  cp "$out/in/$p-all.R" "$H/library/$p/R/$p"
done
rm -f "$H/library/tools/R/sysdata.rdb" "$H/library/tools/R/sysdata.rdx"
cp "$out/in/tools-sysdata.rda" "$H/library/tools/R/sysdata.rda"
cat > "$out/profile.R" << 'EOF'
local({
    routines <- getDLLRegisteredRoutines("base")
    for (i in 1:2)
        lapply(routines[[i]], function(sym) assign(paste0(".C_", sym$name), sym, envir = globalenv()))
})
sys.source(file.path(Sys.getenv("RT_BUILD_REACH_ROOT"), "scripts", "base-subset-trace.R"), envir = globalenv())
EOF
mkdir -p "$out/work/pkg"
cp "$out/in/DESCRIPTION" "$out/work/pkg/"
echo 'tools:::makeLazyLoading("compiler")' > "$out/in/compiler-step.R"
{ cat "$out/in/makeLazyLoad.R"; echo 'makeLazyLoading("tools")'; } > "$out/in/tools-step.R"
echo "tools:::.install_package_description('$out/work/pkg', '$H/library/compiler')" > "$out/in/desc-step.R"
echo "tools:::.vinstall_package_descriptions_as_RDS(\"$H/library\", \"base tools compiler\")" > "$out/in/vdesc-step.R"
echo "tools:::.vinstall_package_namespaces_as_RDS(\"$H/library\", \"base tools compiler\")" > "$out/in/vns-step.R"
# step NAME SCRIPT [ENV...]: GNU R on the copy, the script on stdin as the
# build's R_EXE reads it, traced
step() {
  local name=$1 script=$2
  shift 2
  (cd "$out/work" && env R_HOME="$H" R_SHARE_DIR="$share" R_INCLUDE_DIR="$include" R_DOC_DIR="$doc" \
    LD_LIBRARY_PATH="$H/lib" TZ=UTC LC_ALL=C R_DEFAULT_PACKAGES=NULL \
    RT_BUILD_REACH_ROOT="$root" R_PROFILE_USER="$out/profile.R" ROBIGO_SUBSET_OUT="$out/$name.keys" "$@" \
    "$H/bin/exec/R" --no-environ --no-site-file --no-save --no-restore --no-echo \
    < "$script" > "$out/$name.out" 2> "$out/$name.err") ||
    { echo "rt-build-reach: step $name failed:" >&2; tail -5 "$out/$name.err" >&2; exit 1; }
  echo "rt-build-reach: $name: $(wc -l < "$out/$name.keys") keys"
}
step base "$out/in/makebasedb.R" _R_COMPILE_PKGS_=1 R_COMPILER_SUPPRESS_ALL=1
step compiler "$out/in/compiler-step.R" _R_COMPILE_PKGS_=1 R_COMPILER_SUPPRESS_ALL=1 R_ENABLE_JIT=0
step tools "$out/in/tools-step.R" _R_COMPILE_PKGS_=1 R_COMPILER_SUPPRESS_ALL=1 R_ENABLE_JIT=0
step desc "$out/in/desc-step.R"
step vdesc "$out/in/vdesc-step.R" R_DEFAULT_PACKAGES=tools
step vns "$out/in/vns-step.R" R_DEFAULT_PACKAGES=tools
# what compiler's and tools' own code calls directly
cat > "$out/in/static.R" << 'EOF'
heads <- character(); ints <- character()
walk <- function(e) {
    if (is.call(e)) {
        h <- e[[1L]]
        if (is.symbol(h)) {
            n <- as.character(h)
            heads <<- c(heads, n)
            if (n == ".Internal" && length(e) > 1L && is.call(e[[2L]]) && is.symbol(e[[2L]][[1L]]))
                ints <<- c(ints, as.character(e[[2L]][[1L]]))
        }
        for (a in as.list(e)) if (!missing(a)) walk(a)
    }
}
d <- Sys.getenv("RT_BUILD_REACH_IN")
for (f in c("compiler-all.R", "tools-all.R", "makeLazyLoad.R"))
    for (x in parse(file.path(d, f), keep.source = FALSE)) walk(x)
heads <- unique(heads)
prim <- heads[vapply(heads, function(n) exists(n, baseenv(), inherits = FALSE) &&
                                        is.primitive(get(n, baseenv())), NA)]
writeLines(c(paste0("primitive:", sort(prim)), paste0("internal:", sort(unique(ints)))))
EOF
RT_BUILD_REACH_IN=$out/in R_DEFAULT_PACKAGES=NULL Rscript --vanilla "$out/in/static.R" > "$out/static.keys"
echo "rt-build-reach: static: $(wc -l < "$out/static.keys") keys"
echo "$out"
