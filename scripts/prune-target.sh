#!/usr/bin/env bash
# Delete build artifacts that no build has used for a while from the shared
# target directory (AGENTS.md, Machine). Cargo never deletes old artifacts,
# and every worktree compiles the workspace crates under its own hash, so the
# directory grows with every worktree.
#   scripts/prune-target.sh [-n] [--days N] [TARGET_DIR]
# It deletes, in every profile (debug, quick, release, cross targets, miri):
#   - incremental caches not changed for N days (default 5);
#   - builds in deps/ that a newer build of the same crate or test replaced
#     and that no build has read for N days. Every worktree builds the
#     workspace crates under its own hash; dependencies get a new hash when
#     their version or features change. The newest build of each is kept:
#     a build-time dependency (syn, a build script's crates) is read only
#     when something recompiles, and deleting it would rebuild everything
#     that depends on it.
# A deleted artifact costs only a rebuild: cargo notices that an output is
# missing. Each profile is pruned under cargo's own lock on it, so a build
# running there finishes first. -n prints what would be deleted.
set -euo pipefail
dry=0
days=5
target=
while [ $# -gt 0 ]; do
  case "$1" in
    -n) dry=1 ;;
    --days) days=$2; shift ;;
    -*) echo "usage: $0 [-n] [--days N] [TARGET_DIR]" >&2; exit 2 ;;
    *) target=$1 ;;
  esac
  shift
done
if [ -z "$target" ]; then
  cd "$(dirname "$0")/.."
  target=$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
fi
[ -d "$target" ] || { echo "no target directory: $target" >&2; exit 1; }
cut=$(date -d "-$days days" '+%F %T')
before=$(du -sb "$target" | cut -f1)

superseded() { # deps dir: files of replaced builds, NUL-separated
  python3 - "$1" "$(date -d "$cut" +%s)" <<'EOF'
import os, re, sys
deps, cut = sys.argv[1], int(sys.argv[2])
unit = re.compile(r'^(.+)-([0-9a-f]{16})(\.[a-z]+)?$')
crates = {}  # name -> hash -> (newest access, files)
for f in os.listdir(deps):
    m = unit.match(f)
    if not m:
        continue
    name, h, ext = m.groups()
    if ext in ('.rlib', '.rmeta', '.so', '.a') and name.startswith('lib'):
        name = name[3:]
    path = os.path.join(deps, f)
    st = os.stat(path)
    seen, files = crates.setdefault(name, {}).get(h, (0, []))
    crates[name][h] = (max(seen, st.st_atime, st.st_mtime), files + [path])
for builds in crates.values():
    newest = max(builds, key=lambda h: builds[h][0])
    for h, (seen, files) in builds.items():
        if h != newest and seen < cut:
            for path in files:
                sys.stdout.write(path + '\0')
EOF
}

prune() { # profile dir: list or delete its stale artifacts
  local p=$1
  {
    [ ! -d "$p/incremental" ] || find "$p/incremental" -mindepth 1 -maxdepth 1 ! -newermt "$cut" -print0
    [ ! -d "$p/deps" ] || superseded "$p/deps"
  } | if [ $dry = 1 ]; then tr '\0' '\n'; else xargs -0r rm -rf; fi
}

# A profile is a directory holding deps/ or incremental/, at most two levels
# down (release/, x86_64-unknown-linux-gnu/release/, miri/x86_64-.../debug/).
find "$target" -mindepth 2 -maxdepth 4 -type d \( -name deps -o -name incremental \) ! -path "*/build/*" -printf '%h\n' |
  sort -u | while read -r p; do
    lock="$p/.cargo-lock"
    if [ -e "$lock" ]; then
      exec 8<"$lock"
      flock 8
      prune "$p"
      exec 8<&-
    else
      prune "$p"
    fi
  done

after=$(du -sb "$target" | cut -f1)
[ $dry = 1 ] || echo "freed $(numfmt --to=iec $((before - after))) in $target ($(numfmt --to=iec "$after") left)"
