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
# SUITE/R-V/ under the script's path relative to SUITE, plus FILE.Rsha,
# the script's sha256, so a script changed since is seen as stale.
# The suites are tests/r and, for robigo-rt's version only, tests/rt:
#   scripts/oracle-run.sh 4.6.2 scripts/regen-expected.sh tests/rt/smoke/one.R
set -uo pipefail
cd "$(dirname "$0")/.."
export LANGUAGE=en LC_ALL=C.UTF-8 TZ=UTC R_DEFAULT_PACKAGES=NULL
# The version the old runtime is gated on: its recordings sit beside each
# script. Keep in step with GATED_R_VERSION in scripts/corpus.py and in
# crates/robigo-conformance/src/lib.rs.
gated=4.5.0
# The version robigo-rt is gated on, the only one that records tests/rt.
# Keep in step with RT_R_VERSION in crates/robigo-conformance/src/lib.rs.
rt=4.6.2
want=${ROBIGO_R_VERSION:-$gated}
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
# The suites this version records.
suites=(tests/r)
[ "$want" = "$rt" ] && suites+=(tests/rt)
# The recording's path without extension, for script $1.
target() {
  local suite=tests/r
  case "$1" in tests/rt/*) suite=tests/rt ;; esac
  if [ "$want" = "$gated" ] && [ "$suite" = tests/r ]; then
    printf '%s\n' "${1%.R}"
  else
    local rel=${1#"$suite"/}
    printf '%s/R-%s/%s\n' "$suite" "$want" "${rel%.R}"
  fi
}
files=()
if [ "${1:-}" = "--all" ]; then
  mapfile -t files < <(find "${suites[@]}" -name '*.R' | sort)
elif [ $# -gt 0 ]; then
  files=("$@")
else
  # missing, or (another version) recorded from an older script
  while IFS= read -r f; do
    out=$(target "$f")
    if [ ! -e "$out.Rout" ] || { [ -e "$out.Rsha" ] &&
       [ "$(sha256sum < "$f" | cut -d' ' -f1)" != "$(cat "$out.Rsha")" ]; }; then
      files+=("$f")
    fi
  done < <(find "${suites[@]}" -name '*.R' | sort)
fi
files=("${files[@]#./}")
# Check every path before recording any. tests/rt is robigo-rt's alone, so
# only its version records it. Another version's recordings mirror the
# suite: refuse `.`/`..`, which would write outside R-V/.
for f in "${files[@]}"; do
  case "/$f/" in
    /tests/rt/*)
      if [ "$want" != "$rt" ]; then
        echo "error: $f: tests/rt is recorded only for GNU R $rt (robigo-rt's oracle), not $want" >&2
        exit 1
      fi ;;
  esac
  [ "$want" = "$gated" ] && continue
  case "/$f/" in
    */./*|*/../*) ;;
    /tests/r/*.R/|/tests/rt/*.R/) continue ;;
  esac
  echo "error: $f is not a SUITE/**/*.R path relative to the repository (suites: ${suites[*]})" >&2
  exit 1
done
for f in "${files[@]}"; do
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
  if [ "$want" != "$gated" ]; then
    sha256sum < "$f" | cut -d' ' -f1 > "$out.Rsha"
  fi
done
echo "recorded ${#files[@]} test(s) with $(R --version | head -1)"
