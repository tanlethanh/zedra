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

  // GET /relay — WebSocket upgrade to RelayEndpoint DO
  if (method === "GET" && segments.length === 1 && segments[0] === "relay") {
    const upgradeHeader = request.headers.get("Upgrade");
    if (upgradeHeader !== "websocket") {
      return errorResponse("Expected WebSocket upgrade", 426);
    }

    // Each connection gets a unique DO (keyed by a random name).
    // After handshake, the DO registers itself in KV by endpoint public key.
    const doName = "conn:" + crypto.randomUUID();
    const doId = env.ZEDRA_RELAY_ENDPOINT.idFromName(doName);
    const stub = env.ZEDRA_RELAY_ENDPOINT.get(doId);

    // Pass the DO name so the DO can register itself in KV
    const doUrl = new URL(request.url);
    doUrl.searchParams.set("do_name", doName);

    return stub.fetch(
      new Request(doUrl.toString(), {
        headers: request.headers,
      }),
    );
  }

  return errorResponse("Not found", 404);
}

export { RelayEndpoint } from "./relay-endpoint";

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
