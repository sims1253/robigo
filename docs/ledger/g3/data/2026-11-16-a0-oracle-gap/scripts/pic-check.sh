#!/usr/bin/env bash
# libR's code in the shared (img-plain) and static (img-stock) builds: the
# PLT calls, and the size of a few hot functions.
set -euo pipefail
f=/opt/R/lib/R/lib/libR.so
[ -f "$f" ] || f=/opt/R/lib/R/bin/exec/R
objdump -d --no-show-raw-insn "$f" > /tmp/d
echo "$f: instructions $(grep -cP '^\s+[0-9a-f]+:\t' /tmp/d), PLT calls $(grep -cP 'call\s+[0-9a-f]+ <[^>]*@plt>' /tmp/d)"
for fn in Rf_eval bcEval_loop Rf_findVarInFrame3 Rf_begincontext Rf_topenv R_LookupMethod; do
  awk -v fn="<$fn>:" '$2 == fn {p = 1; n = 0; c = 0; next}
    p && /^$/ {printf "  %-20s %5d instructions, %3d PLT calls\n", substr(fn, 2, length(fn) - 3), n, c; exit}
    p {n++; if (/@plt>/) c++}' /tmp/d
done
