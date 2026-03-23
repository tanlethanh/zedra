FROM rust:1.89-slim-bookworm AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev git && rm -rf /var/lib/apt/lists/*

# Build from iroh main which includes the TCP_NODELAY fix (PR #3995, commit 82e0695).
# The fix will ship as v0.98 on crates.io. Once published, switch to:
#   cargo install iroh-relay --version 0.98 --features server --locked
RUN git clone --depth 1 https://github.com/n0-computer/iroh.git /build/iroh \
    && cd /build/iroh \
    && git fetch origin 82e06959335f1ddbb6c53487395294c6388563f2 --depth 1 \
    && git checkout 82e06959335f1ddbb6c53487395294c6388563f2 \
    && cargo build --release -p iroh-relay --features server \
    && cp target/release/iroh-relay /usr/local/bin/iroh-relay

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates netcat-openbsd && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/local/bin/iroh-relay /usr/local/bin/iroh-relay
COPY relay.toml /etc/iroh-relay/relay.toml.tmpl
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

# HTTP (ACME challenge + relay upgrade), HTTPS (relay), QUIC (addr discovery)
EXPOSE 80 443 7842/udp

ENTRYPOINT ["/entrypoint.sh"]
