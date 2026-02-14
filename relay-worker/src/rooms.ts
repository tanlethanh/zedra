import type { Env, Room } from "./types";
import { generateRoomCode, generateSecret } from "./utils";

export async function createRoom(
  env: Env,
): Promise<{ code: string; secret: string }> {
  const code = generateRoomCode();
  const secret = generateSecret();

  const room: Room = {
    code,
    secret,
    hostId: crypto.randomUUID(),
    mobileId: null,
    state: "waiting",
    createdAt: Date.now(),
  };

  await env.RELAY_KV.put(`room:${code}`, JSON.stringify(room), {
    expirationTtl: 300, // 5 minutes until joined
  });

  return { code, secret };
}

async function getRoom(env: Env, code: string): Promise<Room | null> {
  const raw = await env.RELAY_KV.get(`room:${code}`);
  if (!raw) return null;
  return JSON.parse(raw) as Room;
}

function verifySecret(room: Room, secret: string): boolean {
  return room.secret === secret;
}

export async function joinRoom(
  env: Env,
  code: string,
  secret: string,
): Promise<Room | null> {
  const room = await getRoom(env, code);
  if (!room) return null;
  if (!verifySecret(room, secret)) return null;
  if (room.state === "joined") return null; // already joined

  room.mobileId = crypto.randomUUID();
  room.state = "joined";

  await env.RELAY_KV.put(`room:${code}`, JSON.stringify(room), {
    expirationTtl: 3600, // 1 hour once joined
  });

  return room;
}

export async function deleteRoom(
  env: Env,
  code: string,
  secret: string,
): Promise<boolean> {
  const room = await getRoom(env, code);
  if (!room) return false;
  if (!verifySecret(room, secret)) return false;

  // Delete room and associated keys
  const keysToDelete = [`room:${code}`, `seq:${code}:host`, `seq:${code}:mobile`, `signal:${code}:host`, `signal:${code}:mobile`];

  await Promise.all(keysToDelete.map((k) => env.RELAY_KV.delete(k)));

  // Note: message keys (msg:{code}:*) will expire via TTL (60s)
  return true;
}

export async function heartbeat(
  env: Env,
  code: string,
  secret: string,
): Promise<boolean> {
  const room = await getRoom(env, code);
  if (!room) return false;
  if (!verifySecret(room, secret)) return false;

  // Refresh TTL
  await env.RELAY_KV.put(`room:${code}`, JSON.stringify(room), {
    expirationTtl: 3600,
  });

  return true;
}

export async function getRoomIfAuthorized(
  env: Env,
  code: string,
  secret: string,
): Promise<Room | null> {
  const room = await getRoom(env, code);
  if (!room) return null;
  if (!verifySecret(room, secret)) return null;
  return room;
}
