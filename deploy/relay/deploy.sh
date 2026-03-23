#!/usr/bin/env bash
# Deploy iroh-relay + relay-monitor to one or more instances.
#
# SSH alias required in ~/.ssh/config for each instance:
#   Host zedra-relay-<instance>
#     HostName <EC2_PUBLIC_IP>
#     User ubuntu
#     IdentityFile ~/.ssh/zedra-relay-<instance>.pem
#
# Usage:
#   ./deploy/relay/deploy.sh --instance ap1
#   ./deploy/relay/deploy.sh --instance ap1,us1,eu1
#   ./deploy/relay/deploy.sh --help

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(dirname "$(dirname "$SCRIPT_DIR")")"
MONITOR_DIR="$ROOT_DIR/packages/relay-monitor"

usage() {
  echo "Usage: $0 --instance <instance[,instance,...]>"
  echo ""
  echo "Options:"
  echo "  --instance <instances>  Comma-separated list of instances to deploy (e.g. ap1,us1,eu1)"
  echo "  --help                  Show this help message"
  echo ""
  echo "Convention: SSH alias for each instance must be 'zedra-relay-<instance>' in ~/.ssh/config"
}

INSTANCES=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --instance)
      IFS=',' read -ra INSTANCES <<< "$2"
      shift 2
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

deploy_one() {
  local instance="$1"
  local ssh_host="zedra-relay-${instance}"
  local remote_deploy="/opt/zedra/deploy/relay"

  echo "==> [$instance] Streaming images to $ssh_host..."
  docker save "zedra-relay:latest" "zedra-monitor:latest" | gzip | ssh "$ssh_host" 'docker load'

  echo "==> [$instance] Uploading compose + config..."
  ssh "$ssh_host" bash << EOF
    mkdir -p $remote_deploy
    sudo mkdir -p /var/log/zedra-relay
    sudo chown ubuntu:ubuntu /var/log/zedra-relay
    sudo tee /etc/logrotate.d/zedra-relay-metrics > /dev/null << 'LOGROTATE'
/var/log/zedra-relay/metrics.jsonl {
    daily
    rotate 30
    compress
    delaycompress
    missingok
    notifempty
    create 0644 ubuntu ubuntu
}
LOGROTATE
EOF
  scp "$SCRIPT_DIR/docker-compose.yml" "$ssh_host:$remote_deploy/docker-compose.yml"

  echo "==> [$instance] Starting with docker compose..."
  ssh "$ssh_host" bash << EOF
    { echo "REGION=$instance"; [[ -f $remote_deploy/.env.local ]] && cat $remote_deploy/.env.local; } \
      > $remote_deploy/.env
    cd $remote_deploy
    docker compose up -d --remove-orphans
    echo ""
    docker compose logs --tail=20
EOF

  echo "==> [$instance] Done. https://${instance}.relay.zedra.dev/generate_204"
}

echo "==> Building relay image..."
docker build -f "$SCRIPT_DIR/Dockerfile" -t "zedra-relay:latest" "$SCRIPT_DIR"
echo "==> Building monitor image..."
docker build -f "$MONITOR_DIR/Dockerfile" -t "zedra-monitor:latest" "$ROOT_DIR"
echo ""

if [[ ${#INSTANCES[@]} -eq 1 ]]; then
  deploy_one "${INSTANCES[0]}"
else
  PIDS=()
  for instance in "${INSTANCES[@]}"; do
    LOG="/tmp/zedra-relay-deploy-$instance.log"
    echo "==> Launching $instance in background (log: $LOG)..."
    deploy_one "$instance" > "$LOG" 2>&1 &
    PIDS+=($!)
  done

  echo ""
  FAILED=0
  for i in "${!PIDS[@]}"; do
    if wait "${PIDS[$i]}"; then
      echo "    [OK]  ${INSTANCES[$i]}"
    else
      echo "    [ERR] ${INSTANCES[$i]} — see /tmp/zedra-relay-deploy-${INSTANCES[$i]}.log"
      FAILED=$((FAILED + 1))
    fi
  done

  echo ""
  if [[ $FAILED -eq 0 ]]; then
    echo "==> All ${#INSTANCES[@]} instances deployed."
    for instance in "${INSTANCES[@]}"; do
      echo "    https://${instance}.relay.zedra.dev/generate_204"
    done
  else
    echo "==> $FAILED instance(s) failed."
    exit 1
  fi
fi
