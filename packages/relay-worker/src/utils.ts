import type { Env } from "./types";

export const MAX_MESSAGE_SIZE = 1048576; // 1MB
export const MAX_BATCH = 10;
export const MAX_RECV = 50;

const ROOM_CODE_CHARS = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789"; // no O/0/I/1
const ROOM_CODE_LENGTH = 6;

export function generateRoomCode(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(ROOM_CODE_LENGTH));
  let code = "";
  for (let i = 0; i < ROOM_CODE_LENGTH; i++) {
    code += ROOM_CODE_CHARS[bytes[i] % ROOM_CODE_CHARS.length];
  }
  return code;
}

export function generateSecret(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(32));
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

export async function rateLimit(
  env: Env,
  ip: string,
  limit: number,
  windowSecs: number,
): Promise<boolean> {
  const key = `rl:${ip}`;
  const current = await env.RELAY_KV.get(key);
  const count = current ? parseInt(current, 10) : 0;

  if (count >= limit) {
    return false; // rate limited
  }

  await env.RELAY_KV.put(key, String(count + 1), {
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

export function validateRoomCode(code: string): boolean {
  return /^[A-Z2-9]{6}$/.test(code);
}
