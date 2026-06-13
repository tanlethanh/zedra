# Zedra Delta Integration

Zedra Delta is the backend service for push notifications, Live Activities, workspace sync signals. Doc cover every integration point between Zedra (host + mobile) and Delta.

## Node kinds

| Kind | Registered by | Purpose |
|------|--------------|---------|
| `host` | Mobile app on first pairing / reconnect, or host CLI sign-in | Identified by host dedicated Delta ed25519 key (`~/.config/zedra/delta.key`) |
| `ios` | iOS app on Delta sign-in | Receives push notifications and Live Activity updates |
| `android` | Android app on Delta sign-in | Receives push notifications and Live Activity updates |

## Identity boundaries

Zedra transport PKI and Delta node authorization = separate systems. No reuse keys across boundaries:

- Per-workspace host `identity.key` files identify iroh transport endpoints, sign Zedra transport auth challenges.
- Mobile `client.key` identify mobile device to Zedra host, sign Zedra transport auth challenges.
- Host `delta.key` = global host Delta node authorization key. Direct host sign-in and mobile-assisted host registration must register its public key; host-to-Delta node requests must sign with its private key.
- iOS and Android Delta ops use signed-in user JWT. Mobile registration, metadata reconciliation, push-token registration, Live Activity token registration do not use mobile Delta node signing key.

Delta node identity and authorization identity = same public key. Delta upserts nodes by public key, verifies signed node requests against public key stored for supplied node ID. Transport key, telemetry identifier, or other stable host identifier cannot replace `delta.key`.

## Authentication modes

### Signed-in (bearer + signed)

Host operator runs `zedra auth login` → full `DeltaConfig` saved to `~/.config/zedra/delta.json`. Config holds `access_token`, `refresh_token`, `stack_id`, host `node_id`. Host-to-Delta node calls use `delta.key` for ed25519 request signing (`x-delta-node-id` + `x-delta-signature` headers). Bearer tokens used for user-authorized registration and reconciliation ops.

### Anonymous (signed only, no sign-in)

Mobile client connects, its Delta sign-in active → registers host `delta.key` public key with own Delta stack, then sends `SetClientDeltaInfo` to host via RPC. Host persists to `~/.config/zedra/workspaces/<hash>/delta_client.json`, constructs `DeltaClient` using only `delta.key` plus `stack_id` / `node_id`. No bearer token needed — subsequent host-to-Delta calls use signed-request path.

**Priority**: signed-in (`delta.json`) beats anonymous (`delta_client.json`). On daemon startup, `DeltaClient::try_load_for_workspace` tries signed-in first, then anonymous.

## Host node registration flow

```
Mobile app signs in to Delta
        │
        ▼
workspace connects / reconnects
  (host_delta_registered == false)
        │
        ▼
try_register_host_with_delta()
  → register_paired_host_node(sync.delta_pubkey, metadata)
  → POST /v1/stacks/{stack_id}/nodes  (upsert, bearer auth)
        │
        ▼
 HostNodeRegistrationResult { delta_url, stack_id, host_node_id }
        │
        ▼
session_handle.set_client_delta_info(...)
  → SetClientDeltaInfo RPC → host
        │
        ▼
host saves delta_client.json, updates DeltaClient in memory
  host_delta_registered = true  (mobile side, per connection)
```

`try_register_host_with_delta` called on every `SyncComplete` event while `host_delta_registered` false. Flag reset on each new connection start so re-pair or post-sign-in reconnect retries registration. Server call = upsert, so calling multiple times safe.

Mobile registers host before host signs in → later host sign-in resolves to same node when both use same Delta stack: both paths register `delta.key`, Delta identifies active nodes by public key. Repeated registration preserves node ID and active grants; default-grant writes idempotent. Signing into another stack adds same node to that stack with separate stack-scoped grants. Re-registration currently restores any revoked default host grants.

Signed-in daemon starts → fetches its host node, compares host-owned metadata and registered public key. Changed metadata or old authorization key upserted under `delta.key`; when this returns different node ID, `delta.json` updated. Missing node logged and ignored without removing local Delta config. Anonymous per-workspace Delta clients do not run this startup reconciliation.

## Push notifications

Agent hook receivers (`ClaudeHookReceiver`, `CodexHookReceiver`, `OpenCodeHookReceiver`, `PiHookReceiver`, `HermesHookReceiver`) call `DeltaClient::send_notification_to_stack` when mobile client backgrounded. `DeltaClient` read from `DaemonState.delta` (an `Arc<RwLock<...>>`), updated when client sends `SetClientDeltaInfo`.

Notifications sent only when `session.client_in_foreground == false` (updated by mobile app via `SetAppState`).

## Live Activities

Same agent hook receivers also call `DeltaClient::update_live_activity_for_stack` in parallel with push notification. Live Activity `activity_id` = `"zedra-agent"`. `content_state` carries agent-specific display data (agent kind, event name, deeplink).

## CLI commands

| Command | Auth required | Notes |
|---------|--------------|-------|
| `zedra auth login` | — | Browser auth, writes `delta.json` |
| `zedra auth status` | — | Reads `delta.json` if present |
| `zedra stack list` | signed-in | Lists all nodes in stack |
| `zedra send <node> --title <text> [--workdir <dir>]` | signed-in **or** anonymous | Falls back to `delta_client.json` for `--workdir` |
| `zedra send <node> --live-activity ... [--workdir <dir>]` | signed-in **or** anonymous | Falls back to `delta_client.json` for `--workdir` |

For `zedra send` (notification or `--live-activity`), pass `--workdir` pointing to workspace directory to use anonymous path when host has not signed in but mobile client previously connected.

## Persistent files

| File | Scope | Contents |
|------|-------|----------|
| `~/.config/zedra/delta.key` | global | Host Delta node authorization key |
| `~/.config/zedra/delta.json` | global | Signed-in DeltaConfig (stack_id, node_id, tokens) |
| `~/.config/zedra/workspaces/<hash>/delta_client.json` | per workspace | ClientDeltaInfo from last connected mobile client |

## Key types

```
DeltaClient               — reusable API client; works in both signed-in and anonymous modes
DeltaConfig               — loaded from delta.json; access_token empty in anonymous mode
ClientDeltaInfo           — saved from SetClientDeltaInfo RPC; identifies the host node in the client's stack
HostNodeRegistrationResult — returned by register_paired_host_node; carries delta_url, stack_id, host_node_id
```
