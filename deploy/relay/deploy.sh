#!/usr/bin/env bash
# Deploy iroh-relay to EC2 via SSH.
# Builds the Docker image locally and streams it to the server.
#
# Usage: ./deploy/relay/deploy.sh

set -euo pipefail

SSH_HOST="zedra-ec2"
IMAGE_NAME="zedra-relay"
CONTAINER_NAME="zedra-relay"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "==> Building Docker image..."
docker build -t "$IMAGE_NAME:latest" "$SCRIPT_DIR"

echo "==> Streaming image to $SSH_HOST..."
docker save "$IMAGE_NAME:latest" | gzip | ssh "$SSH_HOST" 'docker load'

echo "==> Restarting container on $SSH_HOST..."
ssh "$SSH_HOST" bash << 'EOF'
  docker stop zedra-relay 2>/dev/null || true
  docker rm   zedra-relay 2>/dev/null || true

  # /data/certs persists Let's Encrypt certs across redeploys
  docker run -d \
    --name zedra-relay \
    --restart unless-stopped \
    -p 80:80 \
    -p 443:443 \
    -p 7842:7842/udp \
    -v zedra-relay-certs:/data/certs \
    zedra-relay:latest

  echo "Container started. Waiting 5s for logs..."
  sleep 5
  docker logs zedra-relay
EOF

echo ""
echo "==> Done. Relay at https://sg1.relay.zedra.dev"
echo "    Test: curl https://sg1.relay.zedra.dev/generate_204"
