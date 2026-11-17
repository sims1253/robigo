#!/usr/bin/env bash
# A0, the bars' other side: robigo-rt (CI's binary from #746's run) on this
# machine in the oracle image plus valgrind (the perf-rt job's environment),
# the bar rows twice; then peak RSS of the bars' programs for robigo-rt and
# each image cell's GNU R (scripts/rss.py, 5 runs).
set -uo pipefail
R=$HOME/robigo-a0/repo
O=$HOME/robigo-a0/out
B=$HOME/robigo-a0/bin
ROWS='^(plain|defaults|named|env-fn|superassign|s3|fib|binarytrees|e7-paste|e7-match|e7-unique)$'
PROGS="benchmarks/suite/binarytrees.R benchmarks/rt/mem/listgrow.R benchmarks/rt/mem/churn.R"
run() { docker run --rm --security-opt seccomp=unconfined -v $R:/w -v $O:/o -v $B:/b -v $HOME/robigo-a0/rsrc:/rsrc:ro -e ROBIGO_RSRC=/rsrc/R-4.6.2-c297040f6392 -w /w "$@"; }
for n in 1 2; do
  [ -f "$O/rt-run$n.json" ] && continue
  s=$(date +%s)
  run robigo-a0:oracle python3 scripts/idiom-vs-r.py --rows benchmarks/rt/calls-rows.toml --filter "$ROWS" \
    --robigo /b/robigo-rt --no-r --json /o/rt-run$n.json > "$O/rt-run$n.log" 2>&1 || echo "!! rt run$n"
  echo "rt run$n $(( $(date +%s) - s )) s"
done
[ -f "$O/rt-rss.json" ] || run robigo-a0:oracle python3 scripts/rss.py --runs 5 --robigo /b/robigo-rt --no-r \
  --json /o/rt-rss.json $PROGS > "$O/rt-rss.log" 2>&1 || echo "!! rt rss"
for cell in oracle img-deb img-plain img-stock; do
  [ -f "$O/$cell-rss.json" ] && continue
  case $cell in
    oracle) env="-e RSCRIPT_BIN=/usr/lib/R/bin/exec/R -e R_HOME_DIR=/usr/lib/R"; img=robigo-a0:oracle ;;
    *) env="-e RSCRIPT_BIN=/opt/R/lib/R/bin/exec/R -e R_HOME_DIR=/opt/R/lib/R"; img=robigo-a0:${cell#img-} ;;
  esac
  run $env $img python3 scripts/rss.py --runs 5 --no-robigo --json /o/$cell-rss.json $PROGS \
    > "$O/$cell-rss.log" 2>&1 || echo "!! $cell rss"
  echo "$cell rss done"
done
echo done > measure-rt.done
