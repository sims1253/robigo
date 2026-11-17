#!/bin/bash
# The R target of src/library/Makefile.in for profile, translations, base,
# tools and compiler, then mklazycomp for base and tools, in a copy of the
# build tree whose bin/R is D0c's tracer. Usage: build-steps.sh RB LOG TOOL
set -euo pipefail
RB=$1; export D0C_LOG=$2; export D0C_TOOL=$3
rm -rf "$D0C_LOG"; mkdir -p "$D0C_LOG"
rm -f "$RB"/library/base/R/base "$RB"/library/base/R/base.rd? \
      "$RB"/library/compiler/R/compiler* "$RB"/library/tools/R/tools* "$RB"/library/tools/R/sysdata.rd?
cd "$RB/src/library"
for pkg in profile translations base tools compiler; do
  echo "== $pkg all" >> "$D0C_LOG/steps.log"
  ls "$D0C_LOG" | { grep -c '\.argv$' || true; } >> "$D0C_LOG/steps.log"
  (cd $pkg && make all) < /dev/null
done
for pkg in base tools; do
  echo "== $pkg mklazycomp" >> "$D0C_LOG/steps.log"
  ls "$D0C_LOG" | { grep -c '\.argv$' || true; } >> "$D0C_LOG/steps.log"
  (cd $pkg && make mklazycomp) < /dev/null
done
echo "== end" >> "$D0C_LOG/steps.log"
