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
# from the valgrind package (/usr/include/valgrind). The build goes into
# its own target directory (.robigo/target-vg): the shared one keeps no
# instrumented mimalloc, so plain scripts/build.sh never flip-flops, and
# the first run here builds the workspace from scratch (~15 minutes;
# incremental after). CFLAGS is appended to, not replaced.
set -euo pipefail
cd "$(dirname "$0")/.."
vg_target="$PWD/.robigo/target-vg"
mkdir -p .robigo/bin "$vg_target"
# Same convention as scripts/build.sh: one vg build at a time.
exec 9>"$vg_target/.robigo-build.lock"
if ! flock -n 9; then
  echo "build-vg: waiting for the vg build lock (another build-vg.sh is running)..."
  flock 9
fi
CFLAGS="${CFLAGS:-} -DMI_TRACK_VALGRIND=1" \
  cargo build --release --locked -p robigo-cli --target-dir "$vg_target"
cp "$vg_target/release/robigo" .robigo/bin/robigo-vg.tmp
mv .robigo/bin/robigo-vg.tmp .robigo/bin/robigo-vg
echo ".robigo/bin/robigo-vg"
