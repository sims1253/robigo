#!/usr/bin/env bash
# Build robigo (release) and copy the binary to .robigo/bin/robigo.
#
# Worktrees share cargo's target directory (~/.cache/cargo-target), so the
# release binary there belongs to whichever worktree built last. The build
# and the copy run under one lock, so every worktree gets its own binary;
# scripts/corpus.py and scripts/perf.py use it.
#   scripts/build.sh [cargo build args...]
set -euo pipefail
cd "$(dirname "$0")/.."
target=$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
mkdir -p .robigo/bin "$target"
exec 9>"$target/.robigo-build.lock"
flock 9
cargo build --release -p robigo-cli "$@"
cp "$target/release/robigo" .robigo/bin/robigo.tmp
mv .robigo/bin/robigo.tmp .robigo/bin/robigo
