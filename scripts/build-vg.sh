#!/usr/bin/env bash
# A valgrind-visible robigo: mimalloc compiled with MI_TRACK_VALGRIND, so
# memcheck, DHAT and massif see every block the runtime allocates (the
# normal build's heap lives inside mimalloc's mmap'd segments and is
# invisible to valgrind: 770 mallocs for a whole R script, zero leaks
# reported, nothing to find). The overhead does not matter under valgrind;
# never measure wall time or instruction counts with this binary.
#
#   scripts/build-vg.sh                    # -> .robigo/bin/robigo-vg
#   ROBIGO=.robigo/bin/robigo-vg python3 scripts/valgrind.py memcheck SCRIPT.R
#
# scripts/valgrind.py prefers robigo-vg when it exists. valgrind.h comes
# from the valgrind package (/usr/include/valgrind); the tracking define
# only affects mimalloc's own sources, so CFLAGS is safe here.
set -euo pipefail
cd "$(dirname "$0")/.."
target=$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
mkdir -p .robigo/bin "$target"
# Hold the same lock as scripts/build.sh across build and copy: the shared
# target's release/robigo belongs to whoever built last, and without the
# lock another worktree's build can land between ours and the copy.
exec 9>"$target/.robigo-build.lock"
flock 9
CFLAGS="-DMI_TRACK_VALGRIND=1" cargo build --release --locked -p robigo-cli
# atomic swap: readers (valgrind.py's pick_robigo, corpus.py) hold no lock
cp "$target/release/robigo" .robigo/bin/robigo-vg.tmp
mv .robigo/bin/robigo-vg.tmp .robigo/bin/robigo-vg
echo ".robigo/bin/robigo-vg"
