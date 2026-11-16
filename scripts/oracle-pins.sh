# Sourced, from the checkout's root, by the shell scripts that read the
# oracle pins: refuses a scripts/oracle-versions that is not in the format
# its header states (the checks crates/robigo-conformance and
# scripts/corpus.py make), then sources it, which sets $gated and $rt.
# POSIX sh, for /bin/sh and bash callers alike.
if LC_ALL=C grep -Evq '^(#.*|[a-z]+=[0-9]+\.[0-9]+\.[0-9]+)?$' scripts/oracle-versions ||
  [ -n "$(LC_ALL=C grep -E '^[a-z]+=' scripts/oracle-versions | cut -d= -f1 | sort | uniq -d)" ]; then
  echo "error: scripts/oracle-versions: a line that is not NAME=X.Y.Z, a # comment or empty, or a NAME on two lines" >&2
  exit 1
fi
. scripts/oracle-versions
