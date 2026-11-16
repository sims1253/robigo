#!/bin/sh
# Record the oracle cases' output with robigo-rt's GNU R (`rt` in
# scripts/oracle-versions; the oracle image's wrapper, never the host's R):
# each NAME.R, after helpers.R, to NAME.out.
#   crates/robigo-rtbuiltins/tests/oracle/record.sh [NAME...]
set -eu
cd "$(dirname "$0")/../../../.."
. scripts/oracle-pins.sh
cd crates/robigo-rtbuiltins/tests/oracle
rscript=${RSCRIPT:-$HOME/.cache/robigo-oracle/$rt/bin/Rscript}
[ $# -gt 0 ] || set -- $(ls *.R | grep -v '^helpers\.R$' | sed 's/\.R$//')
for n in "$@"; do
  cat helpers.R "$n.R" | LANGUAGE=en LC_ALL=C.UTF-8 TZ=UTC R_DEFAULT_PACKAGES=NULL "$rscript" - > "$n.out"
  echo "$n: $(grep -Ec ' = | E : ' "$n.out") cases"
done
