# Zedra Delta Integration

Zedra Delta is the backend service for push notifications, Live Activities, and workspace sync signals. This document covers every integration point between Zedra (host + mobile) and Delta.

## Node kinds

| Kind | Registered by | Purpose |
|------|--------------|---------|
| `host` | Mobile app on first pairing / reconnect | Identified by the host's global ed25519 key (`~/.config/zedra/identity.key`) |
| `ios` | iOS app on Delta sign-in | Receives push notifications and Live Activity updates |
| `android` | Android app on Delta sign-in | Receives push notifications and Live Activity updates |

## Authentication modes

### Signed-in (bearer + signed)

When the host operator runs `zedra auth login`, a full `DeltaConfig` is saved to `~/.config/zedra/delta.json`. This config holds `access_token`, `refresh_token`, `stack_id`, and the host's `node_id`. All host-to-Delta API calls use ed25519 request signing (`x-delta-node-id` + `x-delta-signature` headers); bearer tokens are not used on the host side.

### Anonymous (signed only, no sign-in)

When a mobile client connects and its Delta sign-in is active, it registers the host's global public key with its own Delta stack and then sends `SetClientDeltaInfo` to the host via RPC. The host persists this to `~/.config/zedra/workspaces/<hash>/delta_client.json` and constructs a `DeltaClient` using only the signing key plus `stack_id` / `node_id`. No bearer token is needed because all host → Delta calls use the signed-request path.

**Priority**: signed-in (`delta.json`) takes precedence over anonymous (`delta_client.json`). On daemon startup, `DeltaClient::try_load_for_workspace` tries signed-in first, then anonymous.

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
  → register_paired_host_node(host_iroh_pubkey, metadata)
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

`try_register_host_with_delta` is called on every `SyncComplete` event while `host_delta_registered` is false. The flag is reset on each new connection start so that a re-pair or post-sign-in reconnect retries registration. The server call is an upsert, so calling it multiple times is safe.

## Push notifications

Agent hook receivers (`ClaudeHookReceiver`, `CodexHookReceiver`, `OpenCodeHookReceiver`, `PiHookReceiver`, `HermesHookReceiver`) call `DeltaClient::send_notification_to_stack` when the mobile client is backgrounded. The `DeltaClient` is read from `DaemonState.delta` (an `Arc<RwLock<...>>`), which is updated when a client sends `SetClientDeltaInfo`.

Notifications are only sent when `session.client_in_foreground == false` (updated by the mobile app via `SetAppState`).

## Live Activities

The same agent hook receivers also call `DeltaClient::update_live_activity_for_stack` in parallel with the push notification. The Live Activity `activity_id` is `"zedra-agent"`. The `content_state` carries agent-specific display data (agent kind, event name, deeplink).

## CLI commands

| Command | Auth required | Notes |
|---------|--------------|-------|
| `zedra auth login` | — | Browser auth, writes `delta.json` |
| `zedra auth status` | — | Reads `delta.json` if present |
| `zedra stack nodes` | signed-in | Lists all nodes in the stack |
| `zedra send --id <node> --title <text> [--workdir <dir>]` | signed-in **or** anonymous | Falls back to `delta_client.json` for `--workdir` |
| `zedra live-activity ... [--workdir <dir>]` | signed-in **or** anonymous | Falls back to `delta_client.json` for `--workdir` |

For `zedra send` and `zedra live-activity`, pass `--workdir` pointing to the workspace directory to use the anonymous path when the host has not signed in but a mobile client has previously connected.

## Persistent files

| File | Scope | Contents |
|------|-------|----------|
| `~/.config/zedra/delta.json` | global | Signed-in DeltaConfig (stack_id, node_id, tokens) |
| `~/.config/zedra/workspaces/<hash>/delta_client.json` | per workspace | ClientDeltaInfo from last connected mobile client |

## Key types

```
DeltaClient               — reusable API client; works in both signed-in and anonymous modes
DeltaConfig               — loaded from delta.json; access_token empty in anonymous mode
ClientDeltaInfo           — saved from SetClientDeltaInfo RPC; identifies the host node in the client's stack
HostNodeRegistrationResult — returned by register_paired_host_node; carries delta_url, stack_id, host_node_id
```
