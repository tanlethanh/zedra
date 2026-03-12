import type { Env } from "./types";
import { errorResponse, jsonResponse } from "./utils";

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
  if (
    method === "GET" &&
    (segments.length === 0 ||
      (segments.length === 1 && segments[0] === ""))
  ) {
    return jsonResponse({ ok: true });
  }

  // GET /ping — HTTPS probe for iroh net_report (latency measurement)
  if (method === "GET" && segments.length === 1 && segments[0] === "ping") {
    return new Response("pong", { status: 200 });
  }

  // GET /generate_204 — captive portal detection
  if (method === "GET" && segments.length === 1 && segments[0] === "generate_204") {
    return new Response(null, { status: 204 });
  }

  // GET /relay?host=<hostEndpointIdHex> — WebSocket upgrade to RelayRoom DO.
  //
  // Both host and clients use the same URL (?host=<hex>), landing in the same
  // DO. CF places the DO nearest to whoever connects first (the host).
  if (method === "GET" && segments.length === 1 && segments[0] === "relay") {
    if (request.headers.get("Upgrade") !== "websocket") {
      return errorResponse("Expected WebSocket upgrade", 426);
    }

    const host = url.searchParams.get("host");
    // Accept base64url-encoded 32-byte key (43 chars, no padding).
    if (!host || !/^[A-Za-z0-9_-]{43}$/.test(host)) {
      return errorResponse("Missing or invalid ?host parameter (expected 43-char base64url)", 400);
    }

    const stub = env.ZEDRA_RELAY_ROOM.get(
      env.ZEDRA_RELAY_ROOM.idFromName(`room:${host}`),
    );
    return stub.fetch(request);
  }

  return errorResponse("Not found", 404);
}

export { RelayRoom } from "./relay-room";

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    // Handle CORS preflight
    if (request.method === "OPTIONS") {
      return new Response(null, { status: 204, headers: corsHeaders() });
    }

    try {
      const response = await handleRequest(request, env);
      // WebSocket upgrade responses (101) must be returned directly —
      // withCors creates a new Response which strips the webSocket property.
      if (response.status === 101) {
        return response;
      }
      return withCors(response);
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : "Internal server error";
      return withCors(errorResponse(msg, 500));
    }
  },
};
