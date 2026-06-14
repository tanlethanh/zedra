# Zedra Delta Integration

Zedra Delta is the backend service for push notifications, Live Activities, workspace sync signals. Doc cover every integration point between Zedra (host + mobile) and Delta.

## Node kinds

| Kind | Registered by | Purpose |
|------|--------------|---------|
| `host` | Mobile app on first pairing / reconnect, or host CLI sign-in | Identified by host dedicated Delta ed25519 key (`~/.config/zedra/delta.key`) |
| `ios` | iOS app on Delta sign-in | Receives push notifications and Live Activity updates |
| `android` | Android app on Delta sign-in | Receives push notifications and Live Activity updates |

Host nodes are workspace-scoped in Zedra even though the Delta identity key is global. A single app can manage multiple workspaces, and each workspace may point at a different host node id.

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

Mobile client connects, its Delta sign-in active, and the connected workspace provides the host Delta pubkey. Zedra reads the current `stack_id` and `client_node_id` from app-owned `DeltaState`, ensures the host pubkey is registered for that workspace, then sends `SetClientDeltaInfo` with `stack_id`, `client_node_id`, and `host_node_id` to the host via RPC. The host holds this info in daemon memory and constructs `DeltaClient` using only `delta.key` plus the Delta node IDs. No bearer token needed. Subsequent host-to-Delta calls use the signed-request path.

Workspace state persists the host Delta pubkey and the resolved `host_node_id`. `DeltaState` keeps a pubkey-keyed cache of host-node records so later app launch, reconnect, or late sign-in can reuse the existing host node metadata without re-registering if the mapping is already known.

On daemon startup, only signed-in host config (`delta.json`) is loaded. Mobile-assisted client info is restored when the app reconnects and sends `SetClientDeltaInfo`, or when the workspace later replays the cached host mapping after Delta sign-in becomes available.

## Host node registration flow

```text
Mobile app signs in to Delta or launches with a persisted workspace host binding
        │
        ▼
workspace connects / reconnects / emits refreshed sync state
        │
        ▼
workspace learns the host pubkey from `SyncSessionResult.delta_pubkey`
  and persists it alongside `host_node_id` on `WorkspaceState`
        │
        ▼
`DeltaState` checks its pubkey-keyed host cache
  ├─ cache hit: reuse the stored `host_node_id`
  └─ cache miss: register the host pubkey with Delta
        │
        ▼
register_paired_host_node(sync.delta_pubkey, metadata)
  → POST /v1/stacks/{stack_id}/nodes  (upsert, bearer auth)
        │
        ▼
HostNodeRegistrationResult { host_node_id }
        │
        ▼
workspace persists `{ delta_host_pubkey, host_node_id }`
and `DeltaState` caches the mapping by pubkey
        │
        ▼
app reads current client info from DeltaState
  → { delta_url, stack_id, client_node_id }
  → session_handle.set_client_delta_info(...)
  → SetClientDeltaInfo RPC → host
        │
        ▼
host updates DeltaClient in memory
```

`reconcile_delta_host_binding` is driven by workspace connect/sync state and Delta auth changes. It runs when the workspace first learns a host pubkey, when Delta sign-in becomes available later, and when persisted workspace state needs to be replayed after launch. Host registration remains idempotent because the server upserts by host public key.

Mobile registers host before host signs in → later host sign-in resolves to same node when both use same Delta stack: both paths register `delta.key`, Delta identifies active nodes by public key. Repeated registration preserves node ID and active grants; default-grant writes idempotent. Signing into another stack adds same node to that stack with separate stack-scoped grants. Re-registration currently restores any revoked default host grants. Zedra keeps the per-workspace `host_node_id` persisted so a later app launch or reconnect can reuse the same node id without needing to rediscover it.

Signed-in daemon starts → fetches its host node, compares host-owned metadata and registered public key. Changed metadata or old authorization key upserted under `delta.key`; when this returns different node ID, `delta.json` updated. Missing node logged and ignored without removing local Delta config. Anonymous per-workspace Delta clients do not run this startup reconciliation.

## Push notifications

Agent hook receivers (`ClaudeHookReceiver`, `CodexHookReceiver`, `OpenCodeHookReceiver`, `PiHookReceiver`, `HermesHookReceiver`) call `DeltaClient::send_notification_to_client` when the mobile client is backgrounded. Delivery targets the previous signed-in mobile client's persisted `client_node_id`; it never broadcasts to the stack. `DeltaClient` is read from `DaemonState.delta` (an `Arc<RwLock<...>>`) and updated when the client sends `SetClientDeltaInfo`.

Notifications sent only when `session.client_in_foreground == false` (updated by mobile app via `SetAppState`).

## Live Activities

Same agent hook receivers also call `DeltaClient::update_live_activity_for_stack` in parallel with push notification. Live Activity `activity_id` = `"zedra-agent"`. `content_state` carries agent-specific display data (agent kind, event name, deeplink).

## CLI commands

| Command | Auth required | Notes |
|---------|--------------|-------|
| `zedra auth login` | — | Browser auth, writes `delta.json` |
| `zedra auth status` | — | Reads `delta.json` if present |
| `zedra stack list` | signed-in | Lists all nodes in stack |
| `zedra send <node> --title <text> [--workdir <dir>]` | signed-in | Sends with host Delta config |
| `zedra send <node> --live-activity ... [--workdir <dir>]` | signed-in | Sends with host Delta config |

Mobile-assisted Delta access is used by the running daemon after a signed-in app connects; it is not persisted for standalone CLI sends.

## Persistent files

| File | Scope | Contents |
|------|-------|----------|
| `~/.config/zedra/delta.key` | global | Host Delta node authorization key |
| `~/.config/zedra/delta.json` | global | Signed-in DeltaConfig (stack_id, node_id, tokens) |

## Key types

```text
DeltaClient               — reusable API client; works in both signed-in and anonymous modes
DeltaConfig               — loaded from delta.json; access_token empty in anonymous mode
ClientDeltaInfo           — held in host daemon memory after SetClientDeltaInfo RPC
CurrentClientDeltaInfo     — app-owned current mobile Delta info read from DeltaState
HostNodeRegistrationResult — returned by register_paired_host_node; carries host_node_id and created
```
