#!/bin/sh
# Record the oracle cases' output with GNU R 4.6.2 (the oracle image's
# wrapper, never the host's R): each NAME.R, after helpers.R, to NAME.out.
#   crates/robigo-rtbuiltins/tests/oracle/record.sh [NAME...]
set -eu
cd "$(dirname "$0")"
rscript=${RSCRIPT:-$HOME/.cache/robigo-oracle/4.6.2/bin/Rscript}
[ $# -gt 0 ] || set -- $(ls *.R | grep -v '^helpers\.R$' | sed 's/\.R$//')
for n in "$@"; do
  cat helpers.R "$n.R" | LANGUAGE=en LC_ALL=C.UTF-8 TZ=UTC R_DEFAULT_PACKAGES=NULL "$rscript" - > "$n.out"
  echo "$n: $(grep -c ' = \| E : ' "$n.out") cases"
done
