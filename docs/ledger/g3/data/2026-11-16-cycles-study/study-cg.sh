#!/usr/bin/env bash
# Instructions and estimated cycles per iteration of the study's rows, for
# each binary: scripts/rt-bars.sh (callgrind --cycles, N against 2N), twice
# with inlining on (run-to-run noise) and twice with it off (real calls).
#   study-cg.sh SHA ...
set -uo pipefail
R=/home/m0hawk/.t3/worktrees/robigo/t3-0238db9f-4a1a4b353c265294
B=$R/.robigo/study/bin
O=$R/.robigo/study/cg
cd "$R"
for c in "$@"; do
  for v in on1 on2 off1 off2; do
    d=$O/$c/$v
    [ -f "$d/m1.json" ] && continue
    mkdir -p "$d"
    env=()
    case $v in off*) env=(--robigo-env ROBIGO_NO_INLINE=1) ;; esac
    s=$(date +%s)
    scripts/rt-bars.sh "$B/robigo-rt-$c" "$d" --rows 'e7-paste|e7-match|e7-unique' "${env[@]}" > "$d.log" 2>&1 ||
      echo "!! $c $v (see $d.log)"
    echo "$c $v $(( $(date +%s) - s )) s"
  done
done
