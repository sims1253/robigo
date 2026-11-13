#!/usr/bin/env bash
# Run a command inside the GNU R oracle image robigo-oracle:VERSION, in
# this checkout, with ROBIGO_R_VERSION=VERSION (docs/verification.md,
# "Oracles"). The way to record a corpus: one container, GNU R native in
# it, instead of a container per case through the R/Rscript wrappers.
#
#   scripts/oracle-run.sh 4.6.2 scripts/regen-expected.sh tests/r/strings/x.R
#   scripts/oracle-run.sh 4.6.2 python3 scripts/corpus.py record gnur-tests --jobs 8
#
# The image comes from scripts/build-oracle.sh --image VERSION (4.5.0's
# from tests/packages/Dockerfile). Docker's seccomp filter is off: under
# it a syscall glibc tries fails with ENOSYS, and system() then reports
# that errno where GNU R elsewhere reports none.
set -euo pipefail
[ $# -ge 2 ] || { sed -n '2,/^set /{/^#/p}' "$0" >&2; exit 2; }
version=$1
shift
root=$(cd "$(dirname "$0")/.." && pwd)
docker image inspect "robigo-oracle:$version" > /dev/null 2>&1 || {
  echo "error: no image robigo-oracle:$version; run scripts/build-oracle.sh --image $version" >&2
  exit 1
}
# the current directory when it is in the checkout, else the checkout
case "$PWD/" in "$root/"*) wd=$PWD ;; *) wd=$root ;; esac
tty=()
[ -t 0 ] && tty=(-t)
exec docker run --rm -i "${tty[@]}" --security-opt seccomp=unconfined -u "$(id -u):$(id -g)" \
  -v "$root:$root" -w "$wd" -e HOME=/tmp -e ROBIGO_R_VERSION="$version" \
  "robigo-oracle:$version" "$@"
