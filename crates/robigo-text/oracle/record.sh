#!/usr/bin/env bash
# Records robigo-text's expected values from GNU R 4.6.2 (see cases.R).
# R: the Rscript of a GNU R 4.6.2 build (default: the local research build).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
rscript=${RSCRIPT:-$HOME/.cache/robigo-research/R-4.6.2-build/bin/Rscript}
data=$here/../tests/data
export LANGUAGE=en TZ=UTC
LC_ALL=C.UTF-8 "$rscript" "$here/cases.R" numbers >"$data/numbers.tsv"
LC_ALL=C.UTF-8 "$rscript" "$here/cases.R" strings >"$data/strings-utf8.tsv"
LC_ALL=C "$rscript" "$here/cases.R" strings >"$data/strings-c.tsv"
isw=$(mktemp)
trap 'rm -f "$isw"' EXIT
LC_ALL=C.UTF-8 "$rscript" "$here/iswprint.R" >"$isw"
rsrc=${R_SRC:-$HOME/.cache/robigo-research/rsrc/R-4.6.2}
python3 "$here/gen-tables.py" "$isw" "$rsrc/src/main/rlocale_widths.h" >"$here/../src/tables.rs"
