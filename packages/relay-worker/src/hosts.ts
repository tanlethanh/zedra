// Host Registry — v2 Coordination Server
//
// Stores host registrations in KV with TTL-based expiration.
// Hosts register on startup and heartbeat every 30s.
// Clients look up hosts by device_id to discover current addresses.

import type {
  Env,
  HostRegistration,
  RegisterRequest,
  HeartbeatRequest,
  HostLookupResponse,
  SignalCandidates,
} from "./types";

// KV key prefixes
const HOST_PREFIX = "host:";
const SIGNAL_PREFIX = "signal:";

// TTL for host registration (seconds). Host must heartbeat within this window.
const HOST_TTL_SECS = 90;

// TTL for signal messages (seconds). Short-lived since they're consumed quickly.
const SIGNAL_TTL_SECS = 60;

// Relay endpoint returned in registration responses.
const RELAY_ENDPOINT = "wss://relay.zedra.dev";

/**
 * Register a host. Creates or updates the host's entry in KV.
 */
export async function registerHost(
  env: Env,
  body: RegisterRequest,
): Promise<{ ttl: number; relay_endpoint: string }> {
  const now = Date.now();

  const reg: HostRegistration = {
    device_id: body.device_id,
    public_key: body.public_key,
    hostname: body.hostname,
    addresses: body.addresses || [],
    sessions: body.sessions || [],
    capabilities: body.capabilities || [],
    version: body.version || "unknown",
    registered_at: now,
    last_seen: now,
  };

  await env.ZEDRA_RELAY_KV.put(
    HOST_PREFIX + body.device_id,
    JSON.stringify(reg),
    { expirationTtl: HOST_TTL_SECS },
  );

  return { ttl: HOST_TTL_SECS, relay_endpoint: RELAY_ENDPOINT };
}

/**
 * Heartbeat: update a host's addresses/sessions and refresh TTL.
 */
export async function heartbeatHost(
  env: Env,
  deviceId: string,
  body: HeartbeatRequest,
): Promise<{ ttl: number } | null> {
  const key = HOST_PREFIX + deviceId;
  const existing = await env.ZEDRA_RELAY_KV.get(key);
  if (!existing) return null;

  const reg: HostRegistration = JSON.parse(existing);
  reg.last_seen = Date.now();

  if (body.addresses) {
    reg.addresses = body.addresses;
  }
  if (body.sessions) {
    reg.sessions = body.sessions;
  }

  await env.ZEDRA_RELAY_KV.put(key, JSON.stringify(reg), {
    expirationTtl: HOST_TTL_SECS,
  });

  return { ttl: HOST_TTL_SECS };
}

/**
 * Look up a host by device_id. Returns current info if online.
 */
export async function lookupHost(
  env: Env,
  deviceId: string,
): Promise<HostLookupResponse | null> {
  const key = HOST_PREFIX + deviceId;
  const data = await env.ZEDRA_RELAY_KV.get(key);

  if (!data) {
    // Host not registered or TTL expired = offline
    return {
      online: false,
      last_seen: new Date(0).toISOString(),
      hostname: "",
      addresses: [],
      sessions: [],
      capabilities: [],
      relay_endpoint: RELAY_ENDPOINT,
    };
  }

  const reg: HostRegistration = JSON.parse(data);
  return {
    online: true,
    last_seen: new Date(reg.last_seen).toISOString(),
    hostname: reg.hostname,
    addresses: reg.addresses,
    sessions: reg.sessions,
    capabilities: reg.capabilities,
    relay_endpoint: RELAY_ENDPOINT,
  };
}

/**
 * Store a signal message for a target device.
 * The target device polls for signals on its next heartbeat or lookup.
 */
export async function storeSignal(
  env: Env,
  targetDeviceId: string,
  signal: SignalCandidates,
): Promise<void> {
  // Store signal as a list — multiple clients may signal the same host
  const key = SIGNAL_PREFIX + targetDeviceId;
  const existing = await env.ZEDRA_RELAY_KV.get(key);
  const signals: SignalCandidates[] = existing ? JSON.parse(existing) : [];

  // Replace existing signal from same sender, or append
  const idx = signals.findIndex(
    (s) => s.from_device_id === signal.from_device_id,
  );
  if (idx >= 0) {
    signals[idx] = signal;
  } else {
    signals.push(signal);
  }

  // Cap at 10 pending signals per device
  while (signals.length > 10) {
    signals.shift();
  }

  await env.ZEDRA_RELAY_KV.put(key, JSON.stringify(signals), {
    expirationTtl: SIGNAL_TTL_SECS,
  });
}

/**
 * Retrieve and clear pending signals for a device.
 */
export async function drainSignals(
  env: Env,
  deviceId: string,
): Promise<SignalCandidates[]> {
  const key = SIGNAL_PREFIX + deviceId;
  const data = await env.ZEDRA_RELAY_KV.get(key);
  if (!data) return [];

  // Delete after reading (consumed)
  await env.ZEDRA_RELAY_KV.delete(key);
  return JSON.parse(data);
}
