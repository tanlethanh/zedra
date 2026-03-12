# sg1.relay.zedra.dev — iroh relay deployment

Self-hosted iroh-relay on AWS ap-southeast-1 (Singapore).

## Architecture

- **Host**: EC2 aarch64 (Ubuntu 24.04), `zedra-ec2` SSH alias
- **Runtime**: Docker container `zedra-relay` from local-built image
- **Ports**: 80 (HTTP/ACME), 443 (HTTPS/WebSocket relay), 7842/udp (QUIC addr discovery)
- **TLS**: Let's Encrypt via iroh-relay built-in ACME, certs persisted in Docker volume `zedra-relay-certs`

## Deploy

Build locally (cross-compiles for aarch64), stream to EC2, restart container:

```bash
./deploy/relay/deploy.sh
```

The script:
1. `docker build` locally (multi-stage: Rust builder -> Debian slim runtime)
2. `docker save | gzip | ssh zedra-ec2 docker load` to stream the image
3. Stops old container, starts new one with cert volume mounted

## Version

iroh-relay is pinned to a specific git revision in `Dockerfile`.
When upgrading, update the `--rev` in the `cargo install` line.

Current: iroh main @ `82e0695` (post-v0.96.1, includes TCP_NODELAY fix from PR #3995).
This version will ship as v0.98 on crates.io. Once published, switch back to:
```
cargo install iroh-relay --version 0.98 --features server --locked
```

## Config

`relay.toml` — baked into the Docker image. Edit and redeploy to change.

## Verify

```bash
curl https://sg1.relay.zedra.dev/generate_204    # should return 204
iroh-bench --relay-url https://sg1.relay.zedra.dev --tcp-baseline
```

## Logs

```bash
ssh zedra-ec2 "docker logs -f zedra-relay"
```
