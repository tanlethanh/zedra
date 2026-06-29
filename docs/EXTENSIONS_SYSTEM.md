# Agent Extensions System

Design notes for making agent support dynamic. Goal: adding support for a new agent becomes trivial — ideally shipping a manifest, not writing per-agent Rust on the client.

This is a brainstorm/design doc, not a finalized spec. It captures the problem, the chosen direction, and the open questions still to resolve.

## Problem

Today each agent is defined by a per-agent struct, with logic spread across host and client. Adding an agent means touching many places. We want:

- **Host-driven features** to be dynamic from the host. The client must not define agents by static structs; it consumes a dynamic description of agent features.
- **A dynamic agent RPC protocol** for request/response, sync subscriptions, and streaming. ACP is the reference for shape, but this is our own protocol.
- **App-driven features** (like add-to-chat) to keep their logic on the client, because they need local latency — detect which agent runs on which terminal and normalize a message to that agent.

## Feature taxonomy

Split every agent feature by where its logic *must* live.

### Host-driven (host knows the truth, client renders)

- Launch agent picker — which agents are installed/available on that host
- Agent info — name, icon, version, capabilities
- Agent sessions — list, resume, kill
- Agent config / auth / memory files (Hermes-style)
- Terminal status / icon — *hybrid*: terminal-stream driven plus host-supplied meta

### App-driven (needs local latency, runs in-process on the client)

- Add-to-chat — detect the running agent on a terminal and normalize the message to it
- Keyboard accessory actions per agent
- Input composer hints (slash commands, `@` mentions)
- Paste / image handling per agent
- Deeplink → agent routing

### Stream-driven (derived from terminal bytes)

- Agent identity latch (existing behavior — see the agent-identity-latch memory)
- Status (running / waiting / idle) parsed from output

## Core tension

App-driven features need their logic client-side for latency, so they cannot be pure RPC. Host-driven features want to be dynamic, which favors RPC or pushed config. The answer is **both**: a declarative manifest synced host→client, plus an execution runtime that runs on the client.

### Two approaches considered

**Option 2 — everything is RPC to the host.** Easy to build, but every add-to-chat keystroke normalization becomes a round trip. Latency kills the killer interaction. Acceptable for host-driven features that are already async (sessions, info). Rejected for app-driven features.

**Option 1 — extension / scripting system.** Logic ships as data and runs locally. Correct for app-driven features. The hard part is distribution: one client can connect to multiple hosts, so where does the extension live and who is trusted?

## Chosen direction: extension lives in host, syncs to client

This resolves the multi-host distribution problem and matches the repo invariant that the host owns the daemon/CLI and is the source of truth.

- Extensions are authored/installed **per host**. The host (workspace owner) knows which agents are actually real there.
- On connect, the host advertises a manifest plus any behavior bundle over RPC.
- The client downloads, caches by content hash, and runs behavior in a sandbox.
- A client connected to N hosts merges N manifests, deduped by agent id + version.

No app-store install on the client. No client-side agent definitions.

### Extension layers

| Layer | Content | Runtime |
|-------|---------|---------|
| Manifest (declarative) | id, name, icon, capabilities, detection rules (regex on command/output), session schema | none — just data |
| Behavior (imperative) | normalize message, build launch command, parse status | sandboxed script |

Most agents should be **manifest-only**. Adding an agent then means shipping a JSON/TOML file. Only unusual agents need a behavior script. This is what hits the "trivial to add" goal.

### Sketch

```
Host                          Client (app)
─────                         ────────────
agent registry                manifest cache (by hash)
 ├ manifest (TOML)    ──RPC──▶ merge per-host
 ├ wasm bundle (opt)  ──RPC──▶ sandbox (wasmi)
 └ live sessions      ◀─sub──▶ render
                              app-driven exec:
                               detect → normalize local
                               (manifest rules or wasm)
```

## Prior art

- **ACP** — JSON-RPC, capability negotiation, streaming sessions. Good shape for the *dynamic agent protocol* (host-driven sessions/streaming). It is agent↔editor, not an extension distribution system. Steal the protocol shape, not the distribution.
- **VSCode extensions** — Node sandbox, per-install, marketplace. Heavy, and distribution (per-client install) is exactly the problem we want to avoid.
- **Zed extensions** — WASM + declarative TOML, sandboxed, capability-gated. Closest to our need. WASM runs safely cross-platform (iOS/Android) and is declarative-first.
- **Neovim** — Lua everywhere. Powerful but unsafe, no sandbox, and iOS cannot JIT.
- **Figma/Sketch plugins** — QuickJS sandbox, iOS-safe (interpreter, no JIT).

### iOS constraint decides the scripting language

iOS forbids JIT. So the behavior layer must be a WASM interpreter (wasmi), an embedded JS interpreter (QuickJS/Boa), or Lua. WASM + wasmi mirrors Zed, is type-safe, and fits our Rust stack. **Lean WASM for the behavior layer.**

## Open questions

1. **Behavior layer now or later?** A manifest-only v1 (declarative detection + normalize templates) could land first to get "trivial add" fast and prove distribution, deferring WASM.
2. **Trust model.** Host pushing code to the client means the host can run code on the phone. Need a capability sandbox, possibly signed/pinned bundles, and a per-host trust prompt (pairing trust already exists). Decide how far to go.
3. **Sequencing.** The dynamic agent RPC protocol (host-driven sessions/streaming/info) is a separate track from extension distribution. Likely build the RPC protocol first, distribution second.

## Related

- `docs/MANAGED_AGENTS.md` — current managed agent design
- `docs/AI_AGENTS_CLI_INTEGRATION.md` — CLI agent integration
- `docs/PROTOCOL_SPECS.md` — canonical protocol contract (any new RPC must update this)
- Agent identity latch and ACP unified chat memories — existing detection/streaming behavior
