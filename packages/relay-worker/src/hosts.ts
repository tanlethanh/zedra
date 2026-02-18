// Endpoint discovery — publish/resolve for iroh endpoints via KV.

import type { Env, PublishRequest, ResolveResponse } from "./types";

// KV key prefix for endpoint discovery entries
const ENDPOINT_PREFIX = "ep:";

// TTL for endpoint discovery entries (seconds)
const ENDPOINT_TTL_SECS = 90;

/**
 * Publish an iroh endpoint's addressing info.
 * Called by CfWorkerDiscovery on the host side.
 */
export async function publishEndpoint(
  env: Env,
  body: PublishRequest,
): Promise<{ ttl: number }> {
  const key = ENDPOINT_PREFIX + body.endpoint_id;
  await env.ZEDRA_RELAY_KV.put(key, JSON.stringify(body), {
    expirationTtl: ENDPOINT_TTL_SECS,
  });
  return { ttl: ENDPOINT_TTL_SECS };
}

/**
 * Resolve an iroh endpoint by its ID.
 * Called by CfWorkerDiscovery on the client side.
 */
export async function resolveEndpoint(
  env: Env,
  endpointId: string,
): Promise<ResolveResponse | null> {
  const key = ENDPOINT_PREFIX + endpointId;
  const data = await env.ZEDRA_RELAY_KV.get(key);
  if (!data) return null;

  const stored: PublishRequest = JSON.parse(data);
  return {
    endpoint_id: stored.endpoint_id,
    relay_url: stored.relay_url,
    direct_addrs: stored.direct_addrs,
  };
}
