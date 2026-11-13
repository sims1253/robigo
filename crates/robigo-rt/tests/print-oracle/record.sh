#!/usr/bin/env bash
# Records cases.Rout from GNU R 4.6.2: prelude.R, then cases.R.
#   scripts/oracle-run.sh 4.6.2 crates/robigo-rt/tests/print-oracle/record.sh
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
export LANGUAGE=en LC_ALL=C.UTF-8 TZ=UTC R_DEFAULT_PACKAGES=NULL
cat "$here/prelude.R" "$here/cases.R" | Rscript - >"$here/cases.Rout" 2>"$here/cases.Rerr"
