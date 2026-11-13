#!/usr/bin/env bash
# Record GNU R's output for conformance tests (the oracle).
#   scripts/regen-expected.sh            # all tests missing a recording
#   scripts/regen-expected.sh --all      # all tests
#   scripts/regen-expected.sh FILE.R...  # specific tests
# Writes FILE.Rout (stdout), FILE.Rerr (stderr) and FILE.Rstatus (exit code).
#
# ROBIGO_R_VERSION selects the expected set (docs/verification.md,
# "Oracles"); the Rscript on PATH must be that version. 4.5.0, the
# default, records beside each script; another version V records into
# tests/r/R-V/ under the script's path relative to tests/r, plus FILE.Rsha,
# the script's sha256, so a script changed since is seen as stale.
set -uo pipefail
cd "$(dirname "$0")/.."
export LANGUAGE=en LC_ALL=C.UTF-8 TZ=UTC R_DEFAULT_PACKAGES=NULL
want=${ROBIGO_R_VERSION:-4.5.0}
if ! grep -qF "  R-$want.tar.gz" scripts/r-tarballs.sha256; then
  echo "error: R $want is not an oracle version (scripts/r-tarballs.sha256)" >&2
  exit 1
fi
# Only that version is the oracle: another would silently record its own
# behaviour. scripts/build-oracle.sh builds one if the system's differs.
# Ask the interpreter. The last line: R warns on stdout about a stale R_HOME.
version=$(Rscript --vanilla -e 'cat(format(getRversion()))' 2>/dev/null | tail -1)
if [ "$version" != "$want" ]; then
  echo "error: Rscript runs R ${version:-(none found)}, not $want; see scripts/build-oracle.sh" >&2
  exit 1
fi
# The recording's path without extension, for script $1.
target() {
  if [ "$want" = 4.5.0 ]; then
    printf '%s\n' "${1%.R}"
  else
    local rel=${1#tests/r/}
    printf 'tests/r/R-%s/%s\n' "$want" "${rel%.R}"
  fi
}
files=()
if [ "${1:-}" = "--all" ]; then
  mapfile -t files < <(find tests/r -name '*.R' | sort)
elif [ $# -gt 0 ]; then
  files=("$@")
else
  while IFS= read -r f; do
    [ -e "$(target "$f").Rout" ] || files+=("$f")
  done < <(find tests/r -name '*.R' | sort)
fi
for f in "${files[@]}"; do
  f=${f#./}
  if [ "$want" != 4.5.0 ] && [[ "$f" != tests/r/*.R ]]; then
    echo "error: $f is not a tests/r/**/*.R path relative to the repository" >&2
    exit 1
  fi
  base="${f%.R}"
  out=$(target "$f")
  mkdir -p "$(dirname "$out")"
  tz_env=(env -u TZDIR)
  if [ -f "$base.tzdir" ]; then
    IFS= read -r tzdir < "$base.tzdir"
    tzdir=$(cd "$(dirname "$f")/$tzdir" && pwd -P) || exit 1
    tz_env+=("TZDIR=$tzdir")
  fi
  "${tz_env[@]}" Rscript --vanilla "$f" > "$out.Rout" 2> "$out.Rerr"
  echo $? > "$out.Rstatus"
  if [ "$want" != 4.5.0 ]; then
    sha256sum < "$f" | cut -d' ' -f1 > "$out.Rsha"
  fi
done
echo "recorded ${#files[@]} test(s) with $(R --version | head -1)"
