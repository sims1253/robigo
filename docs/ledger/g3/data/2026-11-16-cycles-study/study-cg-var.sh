#!/usr/bin/env bash
# the study's callgrind rows under fixed cache models (VALGRIND_OPTS)
set -uo pipefail
R=/home/m0hawk/.t3/worktrees/robigo/t3-0238db9f-4a1a4b353c265294
B=$R/.robigo/study/bin
O=$R/.robigo/study/cgvar
cd "$R"
declare -A MODEL=(
  [m0pad]="--I1=32768,8,64 --D1=32768,8,64 --LL=16777216,1,64"
  [ci]="--I1=32768,8,64 --D1=32768,8,64 --LL=268435456,1,64"
  [pinned]="--I1=32768,8,64 --D1=32768,8,64 --LL=33554432,16,64"
)
for c in "$@"; do
  for v in m0pad-on ci-on pinned-on pinned-off; do
    m=${v%-*}; mode=${v#*-}
    d=$O/$c/$v
    [ -f "$d/m1.json" ] && continue
    mkdir -p "$d"
    env=(); [ "$mode" = off ] && env=(--robigo-env ROBIGO_NO_INLINE=1)
    s=$(date +%s)
    VALGRIND_OPTS="${MODEL[$m]}" scripts/rt-bars.sh "$B/robigo-rt-$c" "$d" --rows 'e7-paste|e7-match|e7-unique' "${env[@]}" > "$d.log" 2>&1 ||
      echo "!! $c $v"
    echo "$c $v $(( $(date +%s) - s )) s"
  done
done
