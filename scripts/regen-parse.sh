#!/usr/bin/env bash
# Regenerate tests/parse/**/*.dump from GNU R (the oracle).
set -euo pipefail
cd "$(dirname "$0")/.."
for f in tests/parse/*.R tests/parse/errors/*.R; do
  Rscript --vanilla scripts/oracle-parse.R "$f" "${f%.R}.dump"
done
echo "regenerated $(ls tests/parse/*.dump tests/parse/errors/*.dump | wc -l) dumps with $(R --version | head -1)"
