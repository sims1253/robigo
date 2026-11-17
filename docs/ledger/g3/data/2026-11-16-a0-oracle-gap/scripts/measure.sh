#!/usr/bin/env bash
# A0's measurements, every cell on this machine (m0pad): GNU R's rows twice
# (scripts/idiom-vs-r.py --no-robigo), then callgrind-diff --gnur --causes on
# s3 and fib. Container cells run in their images, host cells natively.
set -uo pipefail
cd ~/robigo-a0
R=$HOME/robigo-a0/repo
O=$HOME/robigo-a0/out
mkdir -p "$O"
ROWS='^(plain|defaults|named|env-fn|superassign|s3|trycatch|fib|binarytrees|e7-paste|e7-match|e7-unique)$'
# the oracle image, with valgrind added
if [ -z "$(docker images -q robigo-a0:oracle)" ]; then
  printf 'FROM robigo-oracle:4.6.2\nRUN apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends valgrind > /dev/null && rm -rf /var/lib/apt/lists/*\n' |
    docker build -q -t robigo-a0:oracle - > /dev/null
fi
cell_cmd() {  # cell -> a command prefix and RSCRIPT_BIN / R_HOME_DIR
  case $1 in
    oracle) echo "docker run --rm --security-opt seccomp=unconfined -v $R:/w -v $O:/o -w /w -e RSCRIPT_BIN=/usr/lib/R/bin/exec/R -e R_HOME_DIR=/usr/lib/R robigo-a0:oracle" ;;
    img-*) echo "docker run --rm --security-opt seccomp=unconfined -v $R:/w -v $O:/o -w /w -e RSCRIPT_BIN=/opt/R/lib/R/bin/exec/R -e R_HOME_DIR=/opt/R/lib/R robigo-a0:${1#img-}" ;;
    host-*) echo "env RSCRIPT_BIN=$HOME/robigo-a0/$1/lib/R/bin/exec/R R_HOME_DIR=$HOME/robigo-a0/$1/lib/R" ;;
  esac
}
for cell in oracle img-deb img-plain img-stock host-deb host-plain host-local; do
  pre=$(cell_cmd $cell)
  case $cell in host-*) out=$O; cd $R ;; *) out=/o ;; esac
  for run in 1 2; do
    [ -f "$O/$cell-run$run.json" ] && continue
    s=$(date +%s)
    $pre python3 scripts/idiom-vs-r.py --rows benchmarks/rt/calls-rows.toml --filter "$ROWS" --no-robigo \
      --json "$out/$cell-run$run.json" > "$O/$cell-run$run.log" 2>&1 || echo "!! $cell run$run"
    echo "$cell run$run $(( $(date +%s) - s )) s"
  done
  for row in s3 fib; do
    [ -f "$O/$cell-diff-$row.txt" ] && continue
    case $row in s3) n=4000 ;; fib) n=20 ;; esac
    $pre python3 scripts/callgrind-diff.py --gnur --causes --top 30 rows/$row-$((2 * n)).R rows/$row-$n.R $n \
      > "$O/$cell-diff-$row.txt" 2>&1 || echo "!! $cell diff $row"
  done
  cd ~/robigo-a0
done
echo done > measure.done
