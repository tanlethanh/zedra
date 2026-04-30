#!/usr/bin/env bash
# Deploy iroh-relay + relay-monitor to one or more instances.
#
# Local secrets: copy deploy/relay/.env.example → deploy/relay/.env (gitignored) and
# set DISCORD_WEBHOOK, optional thresholds, SSH_DIR. deploy.sh merges that file with
# injected INSTANCE (from --instance), uploads to each host as
# /opt/zedra/deploy/relay/.env.local, then copies to .env for docker compose.
#
# SSH alias required in ~/.ssh/config for each instance:
#   Host zedra-relay-<instance>
#     HostName <EC2_PUBLIC_IP>
#     User ubuntu
#     IdentityFile ~/.ssh/zedra-relay-<instance>.pem
#
# Usage:
#   ./deploy/relay/deploy.sh --instance ap1
#   ./deploy/relay/deploy.sh --instance sg1,us1,eu1
#   ./deploy/relay/deploy.sh --help
#
# The script detects each target host architecture, builds one local image per
# Docker platform, and streams each image only to matching hosts.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(dirname "$(dirname "$SCRIPT_DIR")")"
MONITOR_DIR="$ROOT_DIR/packages/relay-monitor"
LOCAL_ENV="$SCRIPT_DIR/.env"

usage() {
  echo "Usage: $0 --instance <instance[,instance,...]> [--service relay|monitor] [--skip-deploy]"
  echo ""
  echo "Options:"
  echo "  --instance <instances>  Comma-separated list of instances to deploy (e.g. sg1,us1,eu1)"
  echo "  --instance local        Build and run locally (no SSH, uses docker compose in-place)"
  echo "  --service relay         Rebuild and redeploy only the iroh-relay container"
  echo "  --service monitor       Rebuild and redeploy only the relay-monitor container"
  echo "  --skip-deploy           Build images and print the target plan without uploading or restarting"
  echo "  --help                  Show this help message"
  echo ""
  echo "Requires: $LOCAL_ENV (copy from .env.example). INSTANCE is injected per host."
  echo "Convention: SSH alias for each instance must be 'zedra-relay-<instance>' in ~/.ssh/config"
}

INSTANCES=()
SERVICE="all"  # relay | monitor | all
SKIP_DEPLOY=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --instance)
      IFS=',' read -ra INSTANCES <<< "$2"
      shift 2
      ;;
    --service)
      SERVICE="$2"
      if [[ "$SERVICE" != "relay" && "$SERVICE" != "monitor" ]]; then
        echo "Unknown service: $SERVICE (must be relay or monitor)" >&2
        usage >&2
        exit 1
      fi
      shift 2
      ;;
    --skip-deploy|--build-only)
      SKIP_DEPLOY=true
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ ${#INSTANCES[@]} -eq 0 ]]; then
  usage >&2
  exit 1
fi

if [[ ! -f "$LOCAL_ENV" ]]; then
  echo "deploy.sh: missing $LOCAL_ENV" >&2
  echo "Copy deploy/relay/.env.example to deploy/relay/.env and set secrets (e.g. DISCORD_WEBHOOK)." >&2
  exit 1
fi

if [[ ${#INSTANCES[@]} -gt 1 ]]; then
  for instance in "${INSTANCES[@]}"; do
    if [[ "$instance" == "local" ]]; then
      echo "deploy.sh: --instance local cannot be combined with remote instances." >&2
      exit 1
    fi
  done
fi

# Writes merged env for one instance to stdout: injected values first, then local
# .env lines. Empty image values are omitted so untouched services keep their
# Compose default or existing image setting.
render_remote_env_local() {
  local instance="$1"
  local relay_image="$2"
  local monitor_image="$3"
  echo "INSTANCE=${instance}"
  if [[ -n "$relay_image" ]]; then
    echo "RELAY_IMAGE=${relay_image}"
  fi
  if [[ -n "$monitor_image" ]]; then
    echo "MONITOR_IMAGE=${monitor_image}"
  fi
  grep -v '^\s*\(#\|$\|INSTANCE=\|INSTANCES=\|RELAY_IMAGE=\|MONITOR_IMAGE=\)' "$LOCAL_ENV" || true
}

local_env_value() {
  local key="$1"
  grep -E "^${key}=" "$SCRIPT_DIR/.env.local" 2>/dev/null | tail -n 1 | cut -d= -f2- || true
}

local_existing_image_value() {
  local key="$1"
  local image
  image="$(local_env_value "$key")"
  if [[ -n "$image" ]] && docker image inspect "$image" >/dev/null 2>&1; then
    echo "$image"
  fi
  return 0
}

remote_env_value() {
  local ssh_host="$1"
  local remote_deploy="$2"
  local key="$3"
  ssh "$ssh_host" "grep -E '^${key}=' $remote_deploy/.env 2>/dev/null | tail -n 1 | cut -d= -f2- || true"
}

remote_existing_image_value() {
  local ssh_host="$1"
  local remote_deploy="$2"
  local key="$3"
  local image
  image="$(remote_env_value "$ssh_host" "$remote_deploy" "$key")"
  if [[ -n "$image" ]] && ssh "$ssh_host" "docker image inspect '$image' >/dev/null 2>&1"; then
    echo "$image"
  fi
  return 0
}

select_relay_env_image() {
  local new_image="$1"
  local existing_image="$2"

  if [[ "$SERVICE" == "all" || "$SERVICE" == "relay" ]]; then
    echo "$new_image"
  else
    echo "$existing_image"
  fi
}

select_monitor_env_image() {
  local new_image="$1"
  local existing_image="$2"

  if [[ "$SERVICE" == "all" || "$SERVICE" == "monitor" ]]; then
    echo "$new_image"
  else
    echo "$existing_image"
  fi
}

platform_for_uname() {
  local os="$1"
  local arch="$2"

  case "$os" in
    Linux|linux)
      ;;
    *)
      echo "deploy.sh: unsupported target OS '$os' (expected Linux)" >&2
      return 1
      ;;
  esac

  case "$arch" in
    x86_64|amd64)
      echo "linux/amd64"
      ;;
    aarch64|arm64)
      echo "linux/arm64"
      ;;
    *)
      echo "deploy.sh: unsupported target architecture '$arch'" >&2
      return 1
      ;;
  esac
}

detect_local_platform() {
  local os_arch
  os_arch="$(docker version --format '{{.Server.Os}} {{.Server.Arch}}')"
  platform_for_uname "${os_arch%% *}" "${os_arch##* }"
}

detect_instance_platform() {
  local instance="$1"

  if [[ "$instance" == "local" ]]; then
    detect_local_platform
    return
  fi

  local ssh_host="zedra-relay-${instance}"
  local os_arch
  os_arch="$(ssh "$ssh_host" 'printf "%s %s\n" "$(uname -s)" "$(uname -m)"')"
  platform_for_uname "${os_arch%% *}" "${os_arch##* }"
}

add_target_platform() {
  local platform="$1"
  local existing

  for existing in "${TARGET_PLATFORMS[@]}"; do
    if [[ "$existing" == "$platform" ]]; then
      return
    fi
  done

  TARGET_PLATFORMS+=("$platform")
}

detect_target_platforms() {
  local instance
  local platform

  for instance in "${INSTANCES[@]}"; do
    platform="$(detect_instance_platform "$instance")"
    echo "==> [$instance] Detected target platform: $platform" >&2
    INSTANCE_PLATFORMS+=("$platform")
    add_target_platform "$platform"
  done
}

join_instances() {
  local joined=""
  local instance

  for instance in "$@"; do
    if [[ -z "$joined" ]]; then
      joined="$instance"
    else
      joined="$joined,$instance"
    fi
  done

  echo "$joined"
}

image_suffix_for_platform() {
  local platform="$1"
  echo "${platform##*/}"
}

relay_image_for_platform() {
  local platform="$1"
  echo "zedra-relay:$(image_suffix_for_platform "$platform")"
}

monitor_image_for_platform() {
  local platform="$1"
  echo "zedra-monitor:$(image_suffix_for_platform "$platform")"
}

print_platform_plan() {
  local platform="$1"
  shift
  local instances=("$@")
  local relay_image
  local monitor_image

  relay_image="$(relay_image_for_platform "$platform")"
  monitor_image="$(monitor_image_for_platform "$platform")"

  echo "==> Plan for $platform"
  echo "    instances: $(join_instances "${instances[@]}")"
  if [[ "$SERVICE" == "all" || "$SERVICE" == "relay" ]]; then
    echo "    relay image: $relay_image"
  fi
  if [[ "$SERVICE" == "all" || "$SERVICE" == "monitor" ]]; then
    echo "    monitor image: $monitor_image"
  fi
}

build_images() {
  local platform="$1"
  local relay_image
  local monitor_image

  relay_image="$(relay_image_for_platform "$platform")"
  monitor_image="$(monitor_image_for_platform "$platform")"

  echo "==> Using Docker platform: $platform"
  if [[ "$SERVICE" == "all" || "$SERVICE" == "relay" ]]; then
    echo "==> Building relay image: $relay_image"
    docker build --platform "$platform" -f "$SCRIPT_DIR/Dockerfile" -t "$relay_image" "$SCRIPT_DIR"
  fi
  if [[ "$SERVICE" == "all" || "$SERVICE" == "monitor" ]]; then
    echo "==> Building monitor image: $monitor_image"
    docker build --platform "$platform" -f "$MONITOR_DIR/Dockerfile" -t "$monitor_image" "$ROOT_DIR"
  fi
  echo ""
}

deploy_one() {
  local instance="$1"
  local platform="$2"
  local ssh_host="zedra-relay-${instance}"
  local remote_deploy="/opt/zedra/deploy/relay"
  local relay_image
  local monitor_image
  local relay_env_image
  local monitor_env_image

  relay_image="$(relay_image_for_platform "$platform")"
  monitor_image="$(monitor_image_for_platform "$platform")"

  echo "==> [$instance] Streaming image(s) to $ssh_host..."
  if [[ "$SERVICE" == "relay" ]]; then
    docker save "$relay_image" | gzip | ssh "$ssh_host" 'docker load'
  elif [[ "$SERVICE" == "monitor" ]]; then
    docker save "$monitor_image" | gzip | ssh "$ssh_host" 'docker load'
  else
    docker save "$relay_image" "$monitor_image" | gzip | ssh "$ssh_host" 'docker load'
  fi

  echo "==> [$instance] Uploading compose + config..."
  ssh "$ssh_host" bash << EOF
    sudo mkdir -p $remote_deploy
    sudo chown \$(id -un):\$(id -gn) $remote_deploy
    sudo mkdir -p /var/log/zedra-relay
    sudo chown \$(id -un):\$(id -gn) /var/log/zedra-relay
    sudo tee /etc/logrotate.d/zedra-relay-metrics > /dev/null << 'LOGROTATE'
/var/log/zedra-relay/metrics.jsonl {
    daily
    rotate 30
    compress
    delaycompress
    missingok
    notifempty
}
LOGROTATE
EOF
  scp "$SCRIPT_DIR/docker-compose.yml" "$ssh_host:$remote_deploy/docker-compose.yml"

  echo "==> [$instance] Uploading merged .env (local .env + injected INSTANCE)..."
  relay_env_image="$(select_relay_env_image "$relay_image" "$(remote_existing_image_value "$ssh_host" "$remote_deploy" RELAY_IMAGE)")"
  monitor_env_image="$(select_monitor_env_image "$monitor_image" "$(remote_existing_image_value "$ssh_host" "$remote_deploy" MONITOR_IMAGE)")"
  render_remote_env_local "$instance" "$relay_env_image" "$monitor_env_image" | ssh "$ssh_host" "cat > $remote_deploy/.env.local"

  echo "==> [$instance] Starting with docker compose..."
  if [[ "$SERVICE" == "all" ]]; then
    ssh "$ssh_host" bash << EOF
      cp -f $remote_deploy/.env.local $remote_deploy/.env
      cd $remote_deploy
      docker compose up -d --remove-orphans
      echo ""
      docker compose logs --tail=20
EOF
  else
    ssh "$ssh_host" bash << EOF
      cp -f $remote_deploy/.env.local $remote_deploy/.env
      cd $remote_deploy
      docker compose up -d --no-deps $SERVICE
      echo ""
      docker compose logs --tail=10 $SERVICE
EOF
  fi

  echo "==> [$instance] Done. http://${instance}.relay.zedra.dev/generate_204"
}

deploy_local() {
  local platform="$1"
  local relay_image
  local monitor_image
  local relay_env_image
  local monitor_env_image

  relay_image="$(relay_image_for_platform "$platform")"
  monitor_image="$(monitor_image_for_platform "$platform")"
  relay_env_image="$(select_relay_env_image "$relay_image" "$(local_existing_image_value RELAY_IMAGE)")"
  monitor_env_image="$(select_monitor_env_image "$monitor_image" "$(local_existing_image_value MONITOR_IMAGE)")"

  echo "==> [local] Writing merged .env..."
  render_remote_env_local "local" "$relay_env_image" "$monitor_env_image" > "$SCRIPT_DIR/.env.local"
  cp -f "$SCRIPT_DIR/.env.local" "$SCRIPT_DIR/.env"

  echo "==> [local] Starting with docker compose..."
  cd "$SCRIPT_DIR"
  if [[ "$SERVICE" == "all" ]]; then
    docker compose up -d --remove-orphans
    echo ""
    docker compose logs --tail=20
  else
    docker compose up -d --no-deps "$SERVICE"
    echo ""
    docker compose logs --tail=10 "$SERVICE"
  fi
  echo "==> [local] Done."
}

deploy_instances() {
  local platform="$1"
  shift
  local instances=("$@")
  local instance
  local i
  local failed
  local pids=()

  if [[ ${#instances[@]} -eq 1 && "${instances[0]}" == "local" ]]; then
    deploy_local "$platform"
    return
  fi

  if [[ ${#instances[@]} -eq 1 ]]; then
    deploy_one "${instances[0]}" "$platform"
    return
  fi

  for instance in "${instances[@]}"; do
    (
      set -o pipefail
      deploy_one "$instance" "$platform" 2>&1 | awk -v p="[${instance}]" '{ print p " " $0; fflush() }'
    ) &
    pids+=($!)
  done

  echo ""
  failed=0
  for i in "${!pids[@]}"; do
    if wait "${pids[$i]}"; then
      echo "    [OK]  ${instances[$i]}"
    else
      echo "    [ERR] ${instances[$i]}"
      failed=$((failed + 1))
    fi
  done

  echo ""
  if [[ $failed -eq 0 ]]; then
    echo "==> All ${#instances[@]} instances deployed."
    for instance in "${instances[@]}"; do
      echo "    http://${instance}.relay.zedra.dev/generate_204"
    done
  else
    echo "==> $failed instance(s) failed."
    exit 1
  fi
}

deploy_platform_group() {
  local platform="$1"
  local instances=()
  local i

  for i in "${!INSTANCES[@]}"; do
    if [[ "${INSTANCE_PLATFORMS[$i]}" == "$platform" ]]; then
      instances+=("${INSTANCES[$i]}")
    fi
  done

  echo "==> Deploying $(join_instances "${instances[@]}") for $platform"
  build_images "$platform"
  if [[ "$SKIP_DEPLOY" == "true" ]]; then
    print_platform_plan "$platform" "${instances[@]}"
    echo "==> Skipping deploy for $platform"
    echo ""
    return
  fi
  deploy_instances "$platform" "${instances[@]}"
}

TARGET_PLATFORMS=()
INSTANCE_PLATFORMS=()
detect_target_platforms

for platform in "${TARGET_PLATFORMS[@]}"; do
  deploy_platform_group "$platform"
done
