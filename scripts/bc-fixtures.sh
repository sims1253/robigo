#!/usr/bin/env bash
# Regenerate robigo-rt's byte-code translation cases (tests/rt/bctrans/*.R)
# from scripts/bc-fixtures/*.R on GNU R 4.6.2 (scripts/bc-fixtures.R), then
# record each on the oracle.
#   scripts/bc-fixtures.sh [NAME ...]    # default: every cases file
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/oracle-pins.sh
names=("$@")
if [ ${#names[@]} -eq 0 ]; then
  for f in scripts/bc-fixtures/*.R; do names+=("$(basename "$f" .R)"); done
fi
mkdir -p tests/rt/bctrans
for n in "${names[@]}"; do
  scripts/oracle-run.sh "$rt" env LC_ALL=C.UTF-8 Rscript --vanilla scripts/bc-fixtures.R \
    "scripts/bc-fixtures/$n.R" "tests/rt/bctrans/$n.R"
  scripts/oracle-run.sh "$rt" scripts/regen-expected.sh "tests/rt/bctrans/$n.R"
done
