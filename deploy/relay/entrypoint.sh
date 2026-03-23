#!/bin/sh
set -e
: "${RELAY_HOSTNAME:?RELAY_HOSTNAME env var is required}"
sed "s|__HOSTNAME__|${RELAY_HOSTNAME}|g" \
  /etc/iroh-relay/relay.toml.tmpl > /etc/iroh-relay/relay.toml
exec iroh-relay --config-path /etc/iroh-relay/relay.toml
