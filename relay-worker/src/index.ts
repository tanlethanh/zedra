import type { Env, SendRequest, SignalData } from "./types";
import { createRoom, joinRoom, deleteRoom, heartbeat, getRoomIfAuthorized } from "./rooms";
import { sendMessages, recvMessages } from "./messaging";
import { setSignal, getSignal } from "./signaling";
import { errorResponse, jsonResponse, rateLimit, validateRoomCode } from "./utils";

function corsHeaders(): Record<string, string> {
  return {
    "Access-Control-Allow-Origin": "*",
    "Access-Control-Allow-Methods": "GET, POST, DELETE, OPTIONS",
    "Access-Control-Allow-Headers": "Content-Type, Authorization",
    "Access-Control-Max-Age": "86400",
  };
}

function withCors(response: Response): Response {
  const headers = new Headers(response.headers);
  for (const [k, v] of Object.entries(corsHeaders())) {
    headers.set(k, v);
  }
  return new Response(response.body, {
    status: response.status,
    statusText: response.statusText,
    headers,
  });
}

function extractSecret(request: Request): string | null {
  const auth = request.headers.get("Authorization");
  if (!auth) return null;
  const parts = auth.split(" ");
  if (parts.length !== 2 || parts[0] !== "Bearer") return null;
  return parts[1];
}

function parseRoute(url: URL): { segments: string[]; code: string | null } {
  const path = url.pathname.replace(/^\/+|\/+$/g, "");
  const segments = path.split("/");

  // Expected: rooms, rooms/:code, rooms/:code/:action
  let code: string | null = null;
  if (segments.length >= 2 && segments[0] === "rooms") {
    code = segments[1];
  }

  return { segments, code };
}

async function handleRequest(request: Request, env: Env): Promise<Response> {
  const url = new URL(request.url);
  const method = request.method;
  const { segments, code } = parseRoute(url);

  // POST /rooms - Create room (no auth required)
  if (method === "POST" && segments.length === 1 && segments[0] === "rooms") {
    const result = await createRoom(env);
    return jsonResponse(result, 201);
  }

  // All other routes require a valid room code
  if (!code || !validateRoomCode(code)) {
    return errorResponse("Invalid room code", 400);
  }

  const action = segments.length >= 3 ? segments[2] : null;

  // POST /rooms/:code/join - Join room
  if (method === "POST" && action === "join") {
    const secret = extractSecret(request);
    if (!secret) return errorResponse("Authorization required", 401);

    // Rate limit join attempts
    const clientIp = request.headers.get("CF-Connecting-IP") || "unknown";
    const allowed = await rateLimit(env, clientIp, 5, 60);
    if (!allowed) return errorResponse("Rate limited", 429);

    const room = await joinRoom(env, code, secret);
    if (!room) return errorResponse("Room not found or already joined", 404);

    return jsonResponse({ joined: true, mobileId: room.mobileId });
  }

  // All remaining routes require auth
  const secret = extractSecret(request);
  if (!secret) return errorResponse("Authorization required", 401);

  // POST /rooms/:code/send - Send messages
  if (method === "POST" && action === "send") {
    const room = await getRoomIfAuthorized(env, code, secret);
    if (!room) return errorResponse("Room not found or unauthorized", 404);

    const body = (await request.json()) as SendRequest;
    if (!body.role || !body.messages || !Array.isArray(body.messages)) {
      return errorResponse("Invalid request body", 400);
    }
    if (body.role !== "host" && body.role !== "mobile") {
      return errorResponse("Invalid role", 400);
    }

    try {
      const result = await sendMessages(env, code, body.role, body.messages);
      return jsonResponse(result);
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : "Send failed";
      return errorResponse(msg, 400);
    }
  }

  // GET /rooms/:code/recv?role=X&after=N - Receive messages
  if (method === "GET" && action === "recv") {
    const room = await getRoomIfAuthorized(env, code, secret);
    if (!room) return errorResponse("Room not found or unauthorized", 404);

    const role = url.searchParams.get("role");
    const afterStr = url.searchParams.get("after");

    if (!role || (role !== "host" && role !== "mobile")) {
      return errorResponse("Invalid role parameter", 400);
    }

    const after = afterStr ? parseInt(afterStr, 10) : 0;
    if (isNaN(after) || after < 0) {
      return errorResponse("Invalid after parameter", 400);
    }

    // Caller specifies their own role; we read messages from the peer
    const peerRole = role === "host" ? "mobile" : "host";
    const result = await recvMessages(env, code, peerRole, after);
    return jsonResponse(result);
  }

  // POST /rooms/:code/signal - Set signaling data
  if (method === "POST" && action === "signal") {
    const room = await getRoomIfAuthorized(env, code, secret);
    if (!room) return errorResponse("Room not found or unauthorized", 404);

    const body = (await request.json()) as SignalData;
    if (!body.role || (body.role !== "host" && body.role !== "mobile")) {
      return errorResponse("Invalid role", 400);
    }

    await setSignal(env, code, body.role, body.data);
    return jsonResponse({ ok: true });
  }

  // GET /rooms/:code/signal?role=X - Get peer's signaling data
  if (method === "GET" && action === "signal") {
    const room = await getRoomIfAuthorized(env, code, secret);
    if (!room) return errorResponse("Room not found or unauthorized", 404);

    const role = url.searchParams.get("role");
    if (!role || (role !== "host" && role !== "mobile")) {
      return errorResponse("Invalid role parameter", 400);
    }

    // Caller specifies their own role; we return peer's signal
    const peerRole = role === "host" ? "mobile" : "host";
    const data = await getSignal(env, code, peerRole);
    if (data === null) return jsonResponse({ data: null });
    return jsonResponse({ data });
  }

  // POST /rooms/:code/heartbeat - Keep room alive
  if (method === "POST" && action === "heartbeat") {
    const ok = await heartbeat(env, code, secret);
    if (!ok) return errorResponse("Room not found or unauthorized", 404);
    return jsonResponse({ ok: true });
  }

  // DELETE /rooms/:code - Delete room
  if (method === "DELETE" && !action) {
    const ok = await deleteRoom(env, code, secret);
    if (!ok) return errorResponse("Room not found or unauthorized", 404);
    return jsonResponse({ deleted: true });
  }

  return errorResponse("Not found", 404);
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    // Handle CORS preflight
    if (request.method === "OPTIONS") {
      return new Response(null, { status: 204, headers: corsHeaders() });
    }

    try {
      const response = await handleRequest(request, env);
      return withCors(response);
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : "Internal server error";
      return withCors(errorResponse(msg, 500));
    }
  },
};
