#!/bin/sh
# GC at scale, GNU R (oracle image) vs robigo. Usage: scale.sh [robigo binary]
set -e
here=$(cd "$(dirname "$0")" && pwd)
robigo=${1:-$here/../../../.robigo/bin/robigo}
d=$(mktemp -d /tmp/robigo-scale-XXXXXX)
cp "$here/scale.c" "$here/scale.R" "$d"
cd "$d"
docker run --rm -u "$(id -u):$(id -g)" -e HOME=/tmp -v "$d:$d" -w "$d" robigo-oracle:4.5.0 R CMD SHLIB scale.c > /dev/null
echo "== GNU R 4.5.0"
docker run --rm -u "$(id -u):$(id -g)" -e HOME=/tmp -e R_DEFAULT_PACKAGES=NULL -v "$d:$d" -w "$d" robigo-oracle:4.5.0 Rscript scale.R
echo "== robigo"
R_DEFAULT_PACKAGES=NULL "$robigo" scale.R
