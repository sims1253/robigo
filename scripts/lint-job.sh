#!/usr/bin/env bash
# CI's `lint` job, the whole of it: the workflow's lint job runs this script
# and nothing else (scripts/test-rt-ci.py checks that), so a green run here is
# a green lint job. Run it before pushing.
#
#   scripts/lint-job.sh
#
# Every check runs, in the job's order, each in its own group; the end lists
# the ones that failed, and the exit status is 1 if any did. (scripts/lint.sh
# is one of the checks: clippy and the Sexp rule. The script suites below are
# the job's other steps, which lint.sh alone does not run.)
set -uo pipefail
cd "$(dirname "$0")/.."

CHECKS=(
  "cargo fmt --check"
  "scripts/lint.sh"
  "python3 scripts/test_corpus.py"
  "python3 scripts/test-oracle-diff.py"
  "python3 scripts/test-check-sexp-safety.py"
  "python3 scripts/test-wall13.py"
  "python3 scripts/test-pgo-build.py"
  "python3 scripts/test-prune-target.py"
  "python3 scripts/test-scratch.py"
)

failed=()
for c in "${CHECKS[@]}"; do
  echo "::group::$c"
  # (word-split on purpose: each entry is a command and its arguments)
  # shellcheck disable=SC2086
  if ! $c; then
    failed+=("$c")
  fi
  echo "::endgroup::"
done
if [ ${#failed[@]} -gt 0 ]; then
  for c in "${failed[@]}"; do
    echo "::error::lint-job: $c failed"
  done
  exit 1
fi
echo "lint-job: all ${#CHECKS[@]} checks passed"
