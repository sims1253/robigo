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
inputs=(.github/ci-image/Dockerfile .github/ci-image/apt.txt tests/packages/apt.txt rust-toolchain.toml)
targets=(base rust packages)
owner=${GITHUB_REPOSITORY_OWNER:-sims1253}
repo=${CI_IMAGE_REPO:-ghcr.io/${owner,,}/robigo-ci}
hash=$(for f in "${inputs[@]}"; do printf '%s\0' "$f"; sha256sum < "$f"; done | sha256sum | cut -c1-16)

ref() { printf '%s:%s-%s\n' "$repo" "$1" "$hash"; }

context() {
  ctx=$(mktemp -d)
  cp .github/ci-image/Dockerfile .github/ci-image/apt.txt rust-toolchain.toml "$ctx/"
  cp tests/packages/apt.txt "$ctx/packages-apt.txt"
  printf '%s\n' "$ctx"
}

build() {
  local ctx t
  ctx=$(context)
  for t in "$@"; do
    echo "building $(ref "$t")" >&2
    docker build -q --target "$t" -t "$(ref "$t")" "$ctx" >&2
  done
  rm -rf "$ctx"
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
    sed -n '2,12p' "$0" >&2
    exit 2
    ;;
esac
