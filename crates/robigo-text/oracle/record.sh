#!/usr/bin/env bash
# Records robigo-text's expected values from GNU R 4.6.2 (see cases.R and
# collate.R). RSCRIPT: the Rscript of a GNU R 4.6.2 build for cases.R
# (default: the local research build); collate.R runs in the oracle image.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
rscript=${RSCRIPT:-$HOME/.cache/robigo-research/R-4.6.2-build/bin/Rscript}
data=$here/../tests/data
export LANGUAGE=en TZ=UTC
LC_ALL=C.UTF-8 "$rscript" "$here/cases.R" numbers >"$data/numbers.tsv"
LC_ALL=C.UTF-8 "$rscript" "$here/cases.R" strings >"$data/strings-utf8.tsv"
LC_ALL=C "$rscript" "$here/cases.R" strings >"$data/strings-c.tsv"
# Collation is ICU's, so it is recorded in the oracle image (ICU 76.1), over
# the strings tests/collate.rs generates.
strings=$(mktemp)
isw=$(mktemp)
trap 'rm -f "$isw" "$strings"' EXIT
root=$here/../../..
(cd "$root" && ROBIGO_COLLATE_STRINGS=$strings cargo test -q -p robigo-text --test collate >/dev/null)
(cd "$root" && scripts/oracle-run.sh 4.6.2 env LANGUAGE=en LC_ALL=C.UTF-8 TZ=UTC R_DEFAULT_PACKAGES=NULL \
  Rscript crates/robigo-text/oracle/collate.R <"$strings") >"$data/collate.tsv"
LC_ALL=C.UTF-8 "$rscript" "$here/iswprint.R" >"$isw"
rsrc=${R_SRC:-$HOME/.cache/robigo-research/rsrc/R-4.6.2}
python3 "$here/gen-tables.py" "$isw" "$rsrc/src/main/rlocale_widths.h" >"$here/../src/tables.rs"
# glibc's wide-character classes and case mappings, for the TRE port.
ctype=$(mktemp)
(cd "$root" && scripts/oracle-run.sh 4.6.2 env LANGUAGE=en LC_ALL=C.UTF-8 TZ=UTC R_DEFAULT_PACKAGES=NULL \
  Rscript crates/robigo-text/oracle/ctype.R) >"$ctype"
python3 "$here/gen-ctype.py" "$ctype" >"$here/../src/tre/tables.rs"
rm -f "$ctype"
# TRE through regexec(), in both locales: hand-picked, fuzzed, and fuzzed
# with approximate matching.
for loc in utf8:C.UTF-8 c:C; do
  for mode in "" fuzz afuzz; do
    (cd "$root" && scripts/oracle-run.sh 4.6.2 env LANGUAGE=en LANG="${loc#*:}" LC_ALL="${loc#*:}" TZ=UTC \
      R_DEFAULT_PACKAGES=NULL Rscript crates/robigo-text/oracle/regex.R $mode) \
      >"$data/regex-${mode:+$mode-}${loc%%:*}.tsv"
  done
done
