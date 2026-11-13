#!/usr/bin/env bash
# The CI image (.github/ci-image/Dockerfile) and the source-built GNU R
# oracles (.github/ci-image/oracle.Dockerfile): print each target's
# reference, and with `ensure`, build and push the ones the registry lacks.
#
#   scripts/ci-image.sh refs            # base=ghcr.io/...:base-<hash> ...
#   scripts/ci-image.sh ensure          # also build + push missing tags
#   scripts/ci-image.sh build [TARGET]  # build locally, no push
#
# The tag is a hash of every input, so a pull request that changes one gets
# (and tests) an image of its own, and an unchanged tree never rebuilds.
# The CI targets share one hash; each oracle has its own (its Dockerfile,
# package list and tarball pin), so a CI image change never rebuilds R.
# CI_IMAGE_REPO overrides the repository (default ghcr.io/<owner>/robigo-ci).
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
# oracle-<version>: GNU R <version> built from the tarball that
# scripts/r-tarballs.sha256 pins (docs/verification.md, "Oracles").
oracles=(4.6.2)
targets=(base rust packages)
for v in "${oracles[@]}"; do targets+=("oracle-$v"); done
owner=${GITHUB_REPOSITORY_OWNER:-sims1253}
repo=${CI_IMAGE_REPO:-ghcr.io/${owner,,}/robigo-ci}

# A build context holds exactly the files its Dockerfile may COPY. The tag
# is the hash of that directory, so nothing can reach an image unhashed.
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
# LC_ALL=C: the glob's order, and so the hash, must not depend on the locale
hash_dir() { (cd "$1" && LC_ALL=C && for f in *; do printf '%s\0' "$f"; sha256sum < "$f"; done | sha256sum | cut -c1-16); }

mkdir "$tmp/ci"
cp .github/ci-image/Dockerfile .github/ci-image/apt.txt rust-toolchain.toml "$tmp/ci/"
cp tests/packages/apt.txt "$tmp/ci/packages-apt.txt"
declare -A hash=([ci]=$(hash_dir "$tmp/ci"))
for v in "${oracles[@]}"; do
  d=$tmp/oracle-$v
  mkdir "$d"
  cp .github/ci-image/oracle.Dockerfile "$d/Dockerfile"
  cp .github/ci-image/oracle-apt.txt "$d/"
  # only this version's pin: adding a version leaves the others' tags alone
  grep -F "  R-$v.tar.gz" scripts/r-tarballs.sha256 > "$d/r-tarballs.sha256" \
    || { echo "no sha256 pin for R-$v.tar.gz in scripts/r-tarballs.sha256" >&2; exit 1; }
  echo "R_VERSION=$v" > "$d/build-args"
  hash[oracle-$v]=$(hash_dir "$d")
done

ctx() { case "$1" in oracle-*) echo "$1" ;; *) echo ci ;; esac; }
ref() { printf '%s:%s-%s\n' "$repo" "$1" "${hash[$(ctx "$1")]}"; }
# The output key, an identifier: oracle-4.6.2 is oracle_4_6_2.
key() { printf "%s\n" "${1//[.-]/_}"; }

build() {
  local t c
  for t in "$@"; do
    c=$(ctx "$t")
    echo "building $(ref "$t")" >&2
    # amd64 always: every CI job runs on x86_64 runners
    if [ "$c" = ci ]; then
      docker build -q --platform linux/amd64 --target "$t" -t "$(ref "$t")" "$tmp/ci" >&2
    else
      docker build -q --platform linux/amd64 --build-arg "$(cat "$tmp/$c/build-args")" -t "$(ref "$t")" "$tmp/$c" >&2
    fi
  done
}

case "${1:-refs}" in
  refs)
    for t in "${targets[@]}"; do echo "$(key "$t")=$(ref "$t")"; done
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
    for t in "${targets[@]}"; do echo "$(key "$t")=$(ref "$t")"; done
    ;;
  build)
    shift
    for t in "$@"; do
      case " ${targets[*]} " in
        *" $t "*) ;;
        *) echo "unknown target $t (have: ${targets[*]})" >&2; exit 2 ;;
      esac
    done
    build "${@:-${targets[@]}}"
    ;;
  *)
    sed -n '2,/^set /{/^#/p}' "$0" >&2
    exit 2
    ;;
esac
