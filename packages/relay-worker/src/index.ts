import type {
  Env,
  RegisterRequest,
  HeartbeatRequest,
  SignalCandidates,
} from "./types";
import {
  registerHost,
  heartbeatHost,
  lookupHost,
  storeSignal,
  drainSignals,
} from "./hosts";
import { errorResponse, jsonResponse, rateLimit } from "./utils";

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

function parseRoute(url: URL): { segments: string[] } {
  const path = url.pathname.replace(/^\/+|\/+$/g, "");
  const segments = path.split("/");
  return { segments };
}

async function handleRequest(request: Request, env: Env): Promise<Response> {
  const url = new URL(request.url);
  const method = request.method;
  const { segments } = parseRoute(url);

  // GET / — health check
  if (method === "GET" && (segments.length === 0 || (segments.length === 1 && segments[0] === ""))) {
    return jsonResponse({ ok: true });
  }

  // POST /hosts/register
  if (
    method === "POST" &&
    segments.length === 2 &&
    segments[0] === "hosts" &&
    segments[1] === "register"
  ) {
    const body = (await request.json()) as RegisterRequest;
    if (!body.device_id || !body.public_key) {
      return errorResponse("device_id and public_key required", 400);
    }
    const result = await registerHost(env, body);
    return jsonResponse(result, 201);
  }

  // POST /hosts/:device_id/heartbeat
  if (
    method === "POST" &&
    segments.length === 3 &&
    segments[0] === "hosts" &&
    segments[2] === "heartbeat"
  ) {
    const deviceId = segments[1];
    const body = (await request.json()) as HeartbeatRequest;
    const result = await heartbeatHost(env, deviceId, body);
    if (!result) return errorResponse("Host not registered", 404);
    return jsonResponse(result);
  }

  // GET /hosts/:device_id
  if (method === "GET" && segments.length === 2 && segments[0] === "hosts") {
    const deviceId = segments[1];
    const result = await lookupHost(env, deviceId);
    if (!result) return errorResponse("Host not found", 404);
    return jsonResponse(result);
  }

  // POST /signal/:device_id
  if (method === "POST" && segments.length === 2 && segments[0] === "signal") {
    const targetDeviceId = segments[1];
    const body = (await request.json()) as SignalCandidates;
    if (!body.from_device_id || !body.candidates) {
      return errorResponse("from_device_id and candidates required", 400);
    }
    await storeSignal(env, targetDeviceId, body);
    return jsonResponse({ ok: true });
  }

  // GET /signal/:device_id
  if (method === "GET" && segments.length === 2 && segments[0] === "signal") {
    const deviceId = segments[1];
    const signals = await drainSignals(env, deviceId);
    return jsonResponse({ signals });
  }

  // GET /ws/:room_id - WebSocket relay (upgrade to WebSocket)
  if (method === "GET" && segments.length === 2 && segments[0] === "ws") {
    const roomId = segments[1];
    const upgradeHeader = request.headers.get("Upgrade");
    if (upgradeHeader !== "websocket") {
      return errorResponse("Expected WebSocket upgrade", 426);
    }

    // Route to Durable Object for this room
    const doId = env.ZEDRA_WS_RELAY.idFromName(roomId);
    const stub = env.ZEDRA_WS_RELAY.get(doId);
    return stub.fetch(request);
  }

  return errorResponse("Not found", 404);
}

export { WsRelay } from "./ws-relay";

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
