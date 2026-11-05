#!/usr/bin/env bash
# Install (or reconfigure) robigo's self-hosted CI runner slots on this
# machine as systemd user units. Needs Docker (the user in the `docker`
# group), gh logged in as a repository admin, and lingering enabled
# (`loginctl enable-linger`) so the slots run without a login session.
#
#   scripts/ci-runner/install.sh --slots 3 --cpus 6 --memory 10g
#   scripts/ci-runner/install.sh --slots 0      # stop and remove every slot
#
# Slot N is pinned to CPUs [N*cpus, (N+1)*cpus), so `nproc` in a job
# reports its share. See docs/verification.md, "Self-hosted runners".
set -euo pipefail

slots=1 cpus=6 memory=10g
while [ $# -gt 0 ]; do
  case $1 in
    --slots) slots=$2; shift 2 ;;
    --cpus) cpus=$2; shift 2 ;;
    --memory) memory=$2; shift 2 ;;
    *) echo "usage: $0 [--slots N] [--cpus N] [--memory SIZE]" >&2; exit 2 ;;
  esac
done
if [ $((slots * cpus)) -gt "$(nproc)" ]; then
  echo "$slots slots x $cpus CPUs exceed this machine's $(nproc)" >&2
  exit 1
fi

here=$(cd "$(dirname "$0")" && pwd)
share=$HOME/.local/share/robigo-runner
units=$HOME/.config/systemd/user
mkdir -p "$share" "$units" "$HOME/.config/robigo-runner"
install -m 755 "$here/robigo-runner.sh" "$share/robigo-runner.sh"
cat >"$HOME/.config/robigo-runner/env" <<EOF
CPUS_PER_SLOT=$cpus
MEMORY_PER_SLOT=$memory
EOF
cat >"$units/robigo-runner@.service" <<'EOF'
[Unit]
Description=robigo CI runner slot %i
After=network-online.target

[Service]
ExecStart=%h/.local/share/robigo-runner/robigo-runner.sh %i
Restart=always
RestartSec=10
TimeoutStopSec=60

[Install]
WantedBy=default.target
EOF
systemctl --user daemon-reload

for unit in $(systemctl --user list-units --all --plain --no-legend 'robigo-runner@*' | awk '{print $1}'); do
  n=${unit#robigo-runner@}; n=${n%.service}
  if [ "$n" -ge "$slots" ]; then systemctl --user disable --now "$unit"; fi
done
for ((n = 0; n < slots; n++)); do
  systemctl --user enable "robigo-runner@$n" >/dev/null 2>&1
  systemctl --user restart "robigo-runner@$n"
done
systemctl --user list-units --plain --no-legend 'robigo-runner@*'
