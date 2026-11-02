#!/usr/bin/env bash
# Record GNU R's output for conformance tests (the oracle).
#   scripts/regen-expected.sh            # all tests missing an .Rout
#   scripts/regen-expected.sh --all      # all tests
#   scripts/regen-expected.sh FILE.R...  # specific tests
# Writes FILE.Rout (stdout), FILE.Rerr (stderr) and FILE.Rstatus (exit code).
set -uo pipefail
cd "$(dirname "$0")/.."
export LANGUAGE=en LC_ALL=C.UTF-8 TZ=UTC R_DEFAULT_PACKAGES=NULL
# GNU R 4.5.0 is the oracle: another version would silently record its own
# behaviour. scripts/build-oracle.sh builds one if the system's differs.
# Ask the interpreter. The last line: R warns on stdout about a stale R_HOME.
version=$(Rscript --vanilla -e 'cat(format(getRversion()))' 2>/dev/null | tail -1)
if [ "$version" != "4.5.0" ]; then
  echo "error: Rscript runs R ${version:-(none found)}, not 4.5.0; see scripts/build-oracle.sh" >&2
  exit 1
fi
files=()
if [ "${1:-}" = "--all" ]; then
  mapfile -t files < <(find tests/r -name '*.R' | sort)
elif [ $# -gt 0 ]; then
  files=("$@")
else
  while IFS= read -r f; do
    [ -e "${f%.R}.Rout" ] || files+=("$f")
  done < <(find tests/r -name '*.R' | sort)
fi
for f in "${files[@]}"; do
  base="${f%.R}"
  tz_env=(env -u TZDIR)
  if [ -f "$base.tzdir" ]; then
    IFS= read -r tzdir < "$base.tzdir"
    tzdir=$(cd "$(dirname "$f")/$tzdir" && pwd -P) || exit 1
    tz_env+=("TZDIR=$tzdir")
  fi
  "${tz_env[@]}" Rscript --vanilla "$f" > "$base.Rout" 2> "$base.Rerr"
  echo $? > "$base.Rstatus"
done
echo "recorded ${#files[@]} test(s) with $(R --version | head -1)"
