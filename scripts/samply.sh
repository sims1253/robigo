#!/usr/bin/env bash
# Wall-time flamegraph with samply (perf sampling, opens the Firefox Profiler).
#
#   scripts/samply.sh SCRIPT.R [samply record options...]
#
# Instruction counts (scripts/perf.py, scripts/valgrind.py cachegrind) are the
# gate: exact and repeatable, but blind to what wall time really spends —
# cache misses, branch misses, wait and IO. Sampling sees all of it, GNU R's
# comparison runs included: profile the same script on both runtimes and
# compare where the samples go.
#
# Environment: R_DEFAULT_PACKAGES=NULL, LC_ALL=C.UTF-8, TZ=UTC (the
# conformance environment; pass your own via --env), and ROBIGO_PERF_MAP=1
# so JIT frames carry names (set ROBIGO_PERF_MAP=0 to turn it off).
# Output: .robigo/samply/NAME.json (then: samply load FILE, or upload it at
# https://profiler.firefox.com); pass --save-only/-o yourself to choose.
set -euo pipefail

usage() {
  cat >&2 <<'EOM'
usage: scripts/samply.sh SCRIPT.R [samply record options...]

Profiles SCRIPT.R with samply (perf sampling) in the conformance
environment; without --save-only/-o a profile JSON lands in
.robigo/samply/NAME.json.
EOM
  exit 2
}

[ $# -ge 1 ] || usage
# resolve the script before the cd: a relative path is the caller's.
script=$(realpath -- "$1"); shift
cd "$(dirname "$0")/.."
command -v perf >/dev/null 2>&1 || { echo "samply needs perf: sudo pacman -S perf" >&2; exit 2; }
command -v samply >/dev/null 2>&1 || { echo "samply is missing: sudo pacman -S samply" >&2; exit 2; }
# unreadable: unknown, so the perf probe below decides.
paranoid=$(cat /proc/sys/kernel/perf_event_paranoid 2>/dev/null || echo 1)
# samply needs paranoid <= 1; perf stat alone still works at 2.
if [ "$paranoid" -gt 1 ]; then
  cat >&2 <<'EOM'
/proc/sys/kernel/perf_event_paranoid is set above 1; sampling needs:
    echo '1' | sudo tee /proc/sys/kernel/perf_event_paranoid
(persistent: /etc/sysctl.d/README.sysctl -> kernel.perf_event_paranoid=1;
 on WSL2 it resets on reboot.)
EOM
  exit 2
fi
perf stat -e cycles true >/dev/null 2>&1 ||
  { echo "perf cannot read events here (perf_event_open failed); samply cannot sample" >&2; exit 2; }

ROBIGO=${ROBIGO:-.robigo/bin/robigo}
[ -x "$ROBIGO" ] || { echo "$ROBIGO is missing; run scripts/build.sh" >&2; exit 2; }

name=$(basename "$script" .R)
mkdir -p .robigo/samply
# Default to a saved profile; pass --save-only/-o yourself to override.
save_args=()
case " $* " in
  *--save-only*|*--out*|*" -o "*|*" -o="*) ;;
  *) save_args=(--save-only -o ".robigo/samply/${name}.json") ;;
esac

env LC_ALL=C.UTF-8 TZ=UTC R_DEFAULT_PACKAGES=NULL ROBIGO_PERF_MAP="${ROBIGO_PERF_MAP:-1}" \
  samply record "${save_args[@]}" "$@" -- "$ROBIGO" --vanilla "$script"
