#!/usr/bin/env bash
# The CI image (.github/ci-image/Dockerfile): print each target's reference,
# and with `ensure`, build and push the ones the registry lacks.
#
#   scripts/ci-image.sh refs            # base=ghcr.io/...:base-<hash> ...
#   scripts/ci-image.sh ensure          # also build + push missing tags
#   scripts/ci-image.sh build [TARGET]  # build locally, no push
#
# The tag is a hash of every input, so a pull request that changes one gets
# (and tests) an image of its own, and an unchanged tree never rebuilds.
# CI_IMAGE_REPO overrides the repository (default ghcr.io/<owner>/robigo-ci).
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
targets=(base rust packages)
owner=${GITHUB_REPOSITORY_OWNER:-sims1253}
repo=${CI_IMAGE_REPO:-ghcr.io/${owner,,}/robigo-ci}

# The build context: exactly the files the Dockerfile may COPY. The tag is
# the hash of this directory, so nothing can reach the image unhashed.
ctx=$(mktemp -d)
trap 'rm -rf "$ctx"' EXIT
cp .github/ci-image/Dockerfile .github/ci-image/apt.txt rust-toolchain.toml "$ctx/"
cp tests/packages/apt.txt "$ctx/packages-apt.txt"
# LC_ALL=C: the glob's order, and so the hash, must not depend on the locale
hash=$(cd "$ctx" && LC_ALL=C && for f in *; do printf '%s\0' "$f"; sha256sum < "$f"; done | sha256sum | cut -c1-16)

ref() { printf '%s:%s-%s\n' "$repo" "$1" "$hash"; }

build() {
  local t
  for t in "$@"; do
    echo "building $(ref "$t")" >&2
    # amd64 always: every CI job runs on x86_64 runners
    docker build -q --platform linux/amd64 --target "$t" -t "$(ref "$t")" "$ctx" >&2
  done
}

case "${1:-refs}" in
  refs)
    for t in "${targets[@]}"; do echo "$t=$(ref "$t")"; done
    ;;
  ensure)
    missing=()
    for t in "${targets[@]}"; do
      if docker manifest inspect "$(ref "$t")" > /dev/null 2>&1; then
        echo "have $(ref "$t")" >&2
      else
        missing+=("$t")
      fi
    done
    if [ "${#missing[@]}" -gt 0 ]; then
      build "${missing[@]}"
      for t in "${missing[@]}"; do docker push -q "$(ref "$t")" >&2; done
    fi
    for t in "${targets[@]}"; do echo "$t=$(ref "$t")"; done
    ;;
  build)
    shift
    build "${@:-${targets[@]}}"
    ;;
  *)
    sed -n '2,/^set /{/^#/p}' "$0" >&2
    exit 2
    ;;
esac
