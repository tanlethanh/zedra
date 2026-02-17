import type { Env } from "./types";

export async function rateLimit(
  env: Env,
  ip: string,
  limit: number,
  windowSecs: number,
): Promise<boolean> {
  const key = `rl:${ip}`;
  const current = await env.ZEDRA_RELAY_KV.get(key);
  const count = current ? parseInt(current, 10) : 0;

  if (count >= limit) {
    return false; // rate limited
  }

  await env.ZEDRA_RELAY_KV.put(key, String(count + 1), {
    expirationTtl: windowSecs,
  });
  return true; // allowed
}

export function jsonResponse(data: unknown, status = 200): Response {
  return new Response(JSON.stringify(data), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

export function errorResponse(message: string, status = 400): Response {
  return jsonResponse({ error: message }, status);
}
