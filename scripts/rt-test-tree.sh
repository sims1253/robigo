#!/usr/bin/env bash
# The database tree a test binary's robigo-rt boots from, never the
# checkout's: the checkout's .robigo/rt-home-db is the one its scripts and
# rt-ci's plans write for the release binary, and one tree has one binary's
# stamp and translation cache. Each binary a test spawns gets a directory
# of its own shaped as a checkout, .robigo/test-trees/<KEY>, which the
# spawned robigo-rt is pointed at (ROBIGO_RT_CHECKOUT):
#   .robigo/rt-home-db (and -cache)  scripts/rt-home-db.sh's tree for BIN
#   .robigo/rt-home                  a link to the checkout's source tree
#                                    (scripts/rt-home.sh's), for
#                                    ROBIGO_RT_BOOT=m1 and stage0
# KEY is the binary's path, hashed: one directory per binary path (a
# profile of a target directory), not per build; the stamp decides when the
# tree is rebuilt. Prints the directory.
#   scripts/rt-test-tree.sh --robigo-rt BIN
#   scripts/rt-test-tree.sh --profile PROFILE   # cargo's target directory's
#                                               # PROFILE/robigo-rt
# crates/robigo-rtapi/tests/common/db_tree.rs runs it (ensure_db_tree,
# rt_command); CI's test job warms the same tree with --profile quick.
set -euo pipefail
cd "$(dirname "$0")/.."
usage() { echo "usage: scripts/rt-test-tree.sh --robigo-rt BIN | --profile PROFILE" >&2; exit 2; }
[ $# -eq 2 ] && [ -n "$2" ] || usage
case "$1" in
  --robigo-rt) bin=$2 ;;
  --profile)
    # (the target directory cargo uses here: a worktree's parent config may
    # name another than ./target)
    target=$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')
    bin=$target/$2/robigo-rt ;;
  *) usage ;;
esac
[ -x "$bin" ] || { echo "rt-test-tree.sh: no robigo-rt at $bin" >&2; exit 2; }
bin=$(realpath "$bin")
key=$(printf '%s' "$bin" | sha256sum | cut -c1-16)
dir=$PWD/.robigo/test-trees/$key
mkdir -p "$dir/.robigo"
# (the source tree first: rt-home-db.sh copies it, and m1 and stage0 boot
# from it through the link)
src_tree=$(realpath "$(scripts/rt-home.sh)")
ln -sfn "$src_tree" "$dir/.robigo/rt-home"
# (which binary, for a reader of the directory)
printf '%s\n' "$bin" > "$dir/.robigo/binary"
scripts/rt-home-db.sh --robigo-rt "$bin" "$dir/.robigo/rt-home-db" > /dev/null
echo "$dir"
