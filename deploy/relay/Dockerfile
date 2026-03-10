FROM rust:1.89-slim-bookworm AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

RUN cargo install iroh-relay --version 0.96.1 --features server --locked

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/local/cargo/bin/iroh-relay /usr/local/bin/iroh-relay
COPY relay.toml /etc/iroh-relay/relay.toml

# HTTP (ACME challenge + relay upgrade), HTTPS (relay), QUIC (addr discovery)
EXPOSE 80 443 7842/udp

CMD ["iroh-relay", "--config-path", "/etc/iroh-relay/relay.toml"]
