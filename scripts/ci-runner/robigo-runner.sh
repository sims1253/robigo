#!/usr/bin/env bash
# One self-hosted GitHub Actions runner slot for robigo's CI, run by the
# systemd user unit robigo-runner@SLOT (scripts/ci-runner/install.sh).
#
# Every job gets a fresh runner container and a fresh Docker daemon
# (docker:dind) for its `container:` steps; both and all their volumes are
# deleted when the job ends. GitHub hands out a single-use just-in-time
# registration per job, so the host's gh token never enters a container.
# Job containers cannot reach private networks (the LAN). Image pulls go
# through a pull-through mirror shared by the host's slots (Docker Hub
# rate-limits anonymous pulls). See docs/verification.md, "Self-hosted
# runners".
set -euo pipefail

slot=${1:?usage: robigo-runner.sh SLOT}
conf=${ROBIGO_RUNNER_CONF:-$HOME/.config/robigo-runner/env}
# shellcheck disable=SC1090
[ -f "$conf" ] && . "$conf"
: "${REPO:=sims1253/robigo}"
: "${LABEL:=robigo-docker}"
: "${CPUS_PER_SLOT:=6}"
: "${MEMORY_PER_SLOT:=10g}"
: "${MIN_FREE_GB:=15}"

runner_image=ghcr.io/actions/actions-runner:latest
dind_image=docker:dind
mirror_image=registry:2
net=robigo-ci
mirror=robigo-ci-mirror
state=${XDG_STATE_HOME:-$HOME/.local/state}/robigo-runner
mkdir -p "$state"

host=$(hostname -s | tr '[:upper:]' '[:lower:]')
prefix="$host-$slot"
c="robigo-ci-$slot"
first=$((slot * CPUS_PER_SLOT))
cpus="$first-$((first + CPUS_PER_SLOT - 1))"
# The runner's `docker` group: dind's sockets belong to it.
docker_gid=123

log() { echo "[$prefix] $*"; }

cleanup() {
  docker rm -f "$c-runner" "$c-dind" >/dev/null 2>&1 || true
  docker volume rm -f "$c-work" "$c-ext" "$c-sock" "$c-docker" >/dev/null 2>&1 || true
}

# Network, mirror and a daily image refresh, once for all slots at a time.
# (`set -e` does not apply in functions called from `if`: every step
# checks its own status.)
shared_setup() {
  (
    flock 9 || exit 1
    docker network inspect "$net" >/dev/null 2>&1 ||
      docker network create "$net" >/dev/null || exit 1
    if ! docker container inspect "$mirror" >/dev/null 2>&1; then
      docker create --name "$mirror" --network "$net" --restart unless-stopped \
        -e REGISTRY_PROXY_REMOTEURL=https://registry-1.docker.io \
        -v "$mirror:/var/lib/registry" "$mirror_image" >/dev/null || exit 1
    fi
    docker start "$mirror" >/dev/null || exit 1
    if [ -z "$(find "$state/pulled" -mmin -1440 2>/dev/null)" ]; then
      for image in "$runner_image" "$dind_image" "$mirror_image"; do
        docker pull -q "$image" >/dev/null || exit 1
      done
      touch "$state/pulled"
    fi
  ) 9>"$state/lock"
}

# Registrations this slot left behind (a crash, a reboot mid-job).
remove_stale() {
  gh api --paginate "repos/$REPO/actions/runners?per_page=100" \
    --jq ".runners[] | select(.name | startswith(\"$prefix-\")) | .id" |
    while read -r id; do
      gh api -X DELETE "repos/$REPO/actions/runners/$id" >/dev/null || true
    done
  return "${PIPESTATUS[0]}"
}

# The job's own Docker daemon. The work directory must be the runner's;
# mounting the empty externals volume copies the image's node runtimes into
# it for dind to mount into job containers (/__e), as it mounts the work
# volume (/__w).
start_dind() {
  docker run --rm -u 0 --entrypoint chown \
    -v "$c-work:/home/runner/_work" -v "$c-ext:/home/runner/externals" \
    "$runner_image" runner:runner /home/runner/_work || return 1
  docker run -d --name "$c-dind" --privileged --network "$net" \
    --cpuset-cpus "$cpus" --memory "$MEMORY_PER_SLOT" \
    -v "$c-docker:/var/lib/docker" -v "$c-work:/home/runner/_work" \
    -v "$c-ext:/home/runner/externals:ro" -v "$c-sock:/run/ci" \
    "$dind_image" dockerd \
    --host=unix:///var/run/docker.sock --host=unix:///run/ci/docker.sock \
    --group="$docker_gid" \
    --registry-mirror="http://$mirror:5000" --insecure-registry="$mirror:5000" >/dev/null || return 1
  local ready=0
  for _ in $(seq 60); do
    if docker exec "$c-dind" docker info >/dev/null 2>&1; then ready=1; break; fi
    sleep 1
  done
  [ "$ready" = 1 ] || return 1
  # Only what leaves dind (eth0): replies to the job containers' own
  # 172.x networks pass, and so does traffic between them.
  for cidr in 10.0.0.0/8 172.16.0.0/12 192.168.0.0/16 169.254.0.0/16 100.64.0.0/10; do
    docker exec "$c-dind" iptables -I DOCKER-USER -o eth0 -d "$cidr" -j REJECT || return 1
  done
}

free_gb() { df -P --block-size=1G "$(docker info --format '{{.DockerRootDir}}')" 2>/dev/null | awk 'NR==2 {print $4}'; }

trap 'cleanup' EXIT
trap 'exit 143' TERM INT

while true; do
  cleanup
  if ! shared_setup || ! remove_stale; then
    log "setup failed; retrying in 60s"
    sleep 60 & wait $!
    continue
  fi
  # A job needs a few GB of image layers and build output.
  avail=$(free_gb || echo 0)
  if [ "${avail:-0}" -lt "$MIN_FREE_GB" ]; then
    log "only ${avail}G free (< ${MIN_FREE_GB}G); waiting"
    sleep 300 & wait $!
    continue
  fi

  if ! start_dind; then
    log "could not start the job's Docker daemon; retrying in 60s"
    docker logs --tail 20 "$c-dind" 2>&1 || true
    sleep 60 & wait $!
    continue
  fi

  name="$prefix-$(date +%s)"
  if ! jit=$(gh api -X POST "repos/$REPO/actions/runners/generate-jitconfig" \
    -f name="$name" -F runner_group_id=1 -f "labels[]=$LABEL" -f work_folder=_work \
    --jq .encoded_jit_config); then
    log "could not register; retrying in 60s"
    sleep 60 & wait $!
    continue
  fi

  log "registered $name; waiting for a job"
  docker run --rm --name "$c-runner" --cpuset-cpus "$cpus" --memory 2g \
    -v "$c-work:/home/runner/_work" -v "$c-ext:/home/runner/externals" -v "$c-sock:/run/ci" \
    -e DOCKER_HOST=unix:///run/ci/docker.sock -e JIT="$jit" \
    --entrypoint bash "$runner_image" -c 'exec ./run.sh --jitconfig "$JIT"' &
  wait $! || log "runner exited with status $?"
done
