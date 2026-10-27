#!/bin/sh
export LC_ALL=C
# Instructions per element for the numeric-policy kernels (callgrind).
# Usage: scripts/ldouble-callgrind.sh
set -e
V="$HOME/.cache/rr-tools/bin/valgrind"
E="$HOME/.cache/cargo-target/release/examples/policy_bench"
if [ ! -x "$E" ]; then
    echo "build first: cargo build --release --example policy_bench" >&2
    exit 1
fi
N=8192
ITERS=300
OUT=/tmp/cg.ld.$$
printf '%-10s %14s %14s %8s\n' kernel exact fast speedup
for k in sum cumsum sqdev cross add_each; do
    ex=0
    fa=0
    for p in exact fast; do
        "$V" -q --tool=callgrind --callgrind-out-file="$OUT" "$E" "$k" "$p" >/dev/null 2>&1
        ins=$(awk '/^summary:/ {print $2}' "$OUT")
        if [ "$p" = exact ]; then ex=$ins; else fa=$ins; fi
    done
    exel=$((ex / (N * ITERS)))
    fael=$((fa / (N * ITERS)))
    printf '%-10s %14d %14d %7.2fx\n' "$k" "$exel" "$fael" "$(awk -v e=$ex -v f=$fa 'BEGIN{printf "%.2f", e/f}')"
done
rm -f "$OUT"
