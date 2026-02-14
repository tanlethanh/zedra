import type { Env } from "./types";

export async function setSignal(
  env: Env,
  code: string,
  role: "host" | "mobile",
  data: unknown,
): Promise<void> {
  const key = `signal:${code}:${role}`;
  await env.RELAY_KV.put(key, JSON.stringify(data), {
    expirationTtl: 300,
  });
}

export async function getSignal(
  env: Env,
  code: string,
  peerRole: "host" | "mobile",
): Promise<unknown | null> {
  const key = `signal:${code}:${peerRole}`;
  const raw = await env.RELAY_KV.get(key);
  if (!raw) return null;
  return JSON.parse(raw);
}
