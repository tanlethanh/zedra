// RelayEndpoint Durable Object — one instance per connected iroh Endpoint.
//
// Uses the Hibernation API so the DO can sleep between alarm ticks.
// Each endpoint authenticates via iroh's challenge-response handshake,
// then sends/receives datagrams via the relay.
//
// Reference: docs/RELAY.md

import type { Env } from "./types";
import {
  FrameType,
  decodeFrameType,
  decodeClientAuth,
  decodeClientDatagram,
  decodeClientDatagramBatch,
  decodePong,
  encodeServerChallenge,
  encodeServerConfirmsAuth,
  encodeServerDeniesAuth,
  encodeRelayToClientDatagram,
  encodeRelayToClientDatagramBatch,
  encodeEndpointGone,
  encodePing,
  encodePong,
} from "./frame-codec";
import {
  blake3DeriveKey,
  ed25519Verify,
  hexEncode,
  HANDSHAKE_DOMAIN,
} from "./crypto";

/** KV key prefix for endpoint routing entries. */
const RELAY_EP_PREFIX = "relay:ep:";

/** KV TTL for routing entries (seconds). Refreshed by alarm. */
const RELAY_EP_TTL_SECS = 90;

/** Base interval between alarms (ms). Jitter added on top. */
const ALARM_INTERVAL_MS = 15_000;

/** Max jitter added to alarm interval (ms). */
const ALARM_JITTER_MS = 5_000;

export class RelayEndpoint {
  private state: DurableObjectState;
  private env: Env;

  constructor(state: DurableObjectState, env: Env) {
    this.state = state;
    this.env = env;
  }

  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);

    // DO-to-DO forward path
    if (url.pathname === "/forward") {
      return this.handleForward(request);
    }

    // WebSocket upgrade
    const upgradeHeader = request.headers.get("Upgrade");
    if (upgradeHeader !== "websocket") {
      return new Response("Expected WebSocket upgrade", { status: 426 });
    }

    // Close any existing connection (reconnection scenario)
    const existing = this.state.getWebSockets("endpoint");
    for (const ws of existing) {
      try {
        ws.close(1000, "replaced by new connection");
      } catch {
        // Already closed
      }
    }

    // Create WebSocket pair
    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);

    // Accept with hibernation tag
    this.state.acceptWebSocket(server, ["endpoint"]);

    // Store DO name from URL (set by router)
    const doName = url.searchParams.get("do_name");
    if (doName) {
      await this.state.storage.put("do_name", doName);
    }

    // Generate challenge
    const challenge = new Uint8Array(16);
    crypto.getRandomValues(challenge);

    // Store challenge and handshake state
    await this.state.storage.put("challenge", Array.from(challenge));
    await this.state.storage.put("authenticated", false);

    // Send ServerChallenge
    server.send(encodeServerChallenge(challenge));

    // Schedule handshake timeout alarm (15s)
    await this.state.storage.setAlarm(Date.now() + ALARM_INTERVAL_MS);

    return new Response(null, {
      status: 101,
      webSocket: client,
    });
  }

  async webSocketMessage(
    ws: WebSocket,
    message: ArrayBuffer | string,
  ): Promise<void> {
    if (typeof message === "string") {
      // Binary protocol only
      return;
    }

    const buf = new Uint8Array(message);
    if (buf.length < 1) return;

    const { type, offset } = decodeFrameType(buf);
    const body = buf.slice(offset);

    const authenticated =
      (await this.state.storage.get<boolean>("authenticated")) ?? false;

    if (!authenticated) {
      await this.handleUnauthenticated(ws, type, body);
    } else {
      await this.handleAuthenticated(ws, type, body);
    }
  }

  async webSocketClose(
    _ws: WebSocket,
    _code: number,
    _reason: string,
    _wasClean: boolean,
  ): Promise<void> {
    await this.cleanup();
  }

  async webSocketError(_ws: WebSocket, _error: unknown): Promise<void> {
    await this.cleanup();
  }

  async alarm(): Promise<void> {
    const authenticated =
      (await this.state.storage.get<boolean>("authenticated")) ?? false;
    const sockets = this.state.getWebSockets("endpoint");

    if (sockets.length === 0) {
      // No client connected, clean up
      await this.cleanup();
      return;
    }

    if (!authenticated) {
      // Handshake timeout — close connection
      for (const ws of sockets) {
        try {
          ws.close(1008, "Handshake timeout");
        } catch {
          // Already closed
        }
      }
      await this.cleanup();
      return;
    }

    // Send keepalive Ping
    const pingPayload = new Uint8Array(8);
    crypto.getRandomValues(pingPayload);
    const pingFrame = encodePing(pingPayload);
    for (const ws of sockets) {
      try {
        ws.send(pingFrame);
      } catch {
        // WebSocket broken, will be cleaned up
      }
    }

    // Refresh KV routing entry
    const endpointIdHex = await this.state.storage.get<string>(
      "endpoint_id_hex",
    );
    const doName = await this.state.storage.get<string>("do_name");
    if (endpointIdHex && doName) {
      await this.env.ZEDRA_RELAY_KV.put(
        RELAY_EP_PREFIX + endpointIdHex,
        JSON.stringify({ do_name: doName }),
        { expirationTtl: RELAY_EP_TTL_SECS },
      );
    }

    // Schedule next alarm with jitter
    const jitter = Math.floor(Math.random() * ALARM_JITTER_MS);
    await this.state.storage.setAlarm(
      Date.now() + ALARM_INTERVAL_MS + jitter,
    );
  }

  // --- Internal handlers ---

  private async handleUnauthenticated(
    ws: WebSocket,
    type: number,
    body: Uint8Array,
  ): Promise<void> {
    if (type !== FrameType.ClientAuth) {
      // Only ClientAuth accepted before authentication
      ws.send(encodeServerDeniesAuth("Expected ClientAuth frame"));
      ws.close(1008, "Unexpected frame before auth");
      return;
    }

    // Decode ClientAuth (postcard format)
    let auth;
    try {
      auth = decodeClientAuth(body);
    } catch (e) {
      const msg = e instanceof Error ? e.message : "Decode error";
      ws.send(encodeServerDeniesAuth(msg));
      ws.close(1008, "Invalid ClientAuth");
      return;
    }

    // Retrieve stored challenge
    const challengeArr = await this.state.storage.get<number[]>("challenge");
    if (!challengeArr) {
      ws.send(encodeServerDeniesAuth("No pending challenge"));
      ws.close(1008, "No challenge");
      return;
    }
    const challenge = new Uint8Array(challengeArr);

    // Verify: derive key from challenge, then verify signature
    const derived = blake3DeriveKey(HANDSHAKE_DOMAIN, challenge);
    const valid = ed25519Verify(auth.publicKey, derived, auth.signature);

    if (!valid) {
      ws.send(encodeServerDeniesAuth("Invalid signature"));
      ws.close(1008, "Auth failed");
      return;
    }

    // Authentication successful
    const endpointIdHex = hexEncode(auth.publicKey);

    await this.state.storage.put("authenticated", true);
    await this.state.storage.put(
      "endpoint_id",
      Array.from(auth.publicKey),
    );
    await this.state.storage.put("endpoint_id_hex", endpointIdHex);

    // do_name was set during WebSocket upgrade from the router's URL param.
    // Read it back to register in KV.
    const doName = await this.state.storage.get<string>("do_name");

    // Register in KV routing table
    if (doName) {
      await this.env.ZEDRA_RELAY_KV.put(
        RELAY_EP_PREFIX + endpointIdHex,
        JSON.stringify({ do_name: doName }),
        { expirationTtl: RELAY_EP_TTL_SECS },
      );
    }

    // Clear challenge
    await this.state.storage.delete("challenge");

    // Send confirmation
    ws.send(encodeServerConfirmsAuth());

    // Schedule keepalive alarm
    const jitter = Math.floor(Math.random() * ALARM_JITTER_MS);
    await this.state.storage.setAlarm(
      Date.now() + ALARM_INTERVAL_MS + jitter,
    );
  }

  private async handleAuthenticated(
    ws: WebSocket,
    type: number,
    body: Uint8Array,
  ): Promise<void> {
    switch (type) {
      case FrameType.ClientToRelayDatagram: {
        const { dstId, ecn, data } = decodeClientDatagram(body);
        const srcIdArr =
          await this.state.storage.get<number[]>("endpoint_id");
        if (!srcIdArr) return;
        const srcId = new Uint8Array(srcIdArr);
        const frame = encodeRelayToClientDatagram(srcId, ecn, data);
        await this.forwardToEndpoint(ws, dstId, frame);
        break;
      }

      case FrameType.ClientToRelayDatagramBatch: {
        const { dstId, ecn, segmentSize, data } =
          decodeClientDatagramBatch(body);
        const srcIdArr =
          await this.state.storage.get<number[]>("endpoint_id");
        if (!srcIdArr) return;
        const srcId = new Uint8Array(srcIdArr);
        const frame = encodeRelayToClientDatagramBatch(
          srcId,
          ecn,
          segmentSize,
          data,
        );
        await this.forwardToEndpoint(ws, dstId, frame);
        break;
      }

      case FrameType.Ping: {
        // Respond with Pong echoing the payload
        const payload = body.slice(0, 8);
        ws.send(encodePong(payload));
        break;
      }

      case FrameType.Pong: {
        // Client responding to our keepalive Ping — no-op
        break;
      }

      default:
        // Unknown frame type after auth — ignore
        break;
    }
  }

  /**
   * Forward a pre-encoded frame to a target endpoint via DO-to-DO fetch.
   * If the target is not found or not connected, send EndpointGone to sender.
   */
  private async forwardToEndpoint(
    senderWs: WebSocket,
    dstId: Uint8Array,
    frame: Uint8Array,
  ): Promise<void> {
    const dstHex = hexEncode(dstId);

    // Look up target in KV
    const routeData = await this.env.ZEDRA_RELAY_KV.get(
      RELAY_EP_PREFIX + dstHex,
    );
    if (!routeData) {
      // Target not registered
      senderWs.send(encodeEndpointGone(dstId));
      return;
    }

    const route = JSON.parse(routeData) as { do_name: string };

    // Forward via DO-to-DO fetch
    const targetDoId =
      this.env.ZEDRA_RELAY_ENDPOINT.idFromName(route.do_name);
    const targetStub = this.env.ZEDRA_RELAY_ENDPOINT.get(targetDoId);

    try {
      const resp = await targetStub.fetch("https://do/forward", {
        method: "POST",
        body: frame,
      });

      if (resp.status === 410) {
        // Target endpoint has no connected client
        senderWs.send(encodeEndpointGone(dstId));
      }
    } catch {
      // DO fetch failed — target gone
      senderWs.send(encodeEndpointGone(dstId));
    }
  }

  /**
   * Handle incoming DO-to-DO forward request.
   * Push the pre-encoded frame directly to the connected WebSocket.
   */
  private async handleForward(request: Request): Promise<Response> {
    const sockets = this.state.getWebSockets("endpoint");
    if (sockets.length === 0) {
      return new Response("No client connected", { status: 410 });
    }

    const body = await request.arrayBuffer();
    for (const ws of sockets) {
      try {
        ws.send(body);
      } catch {
        // WebSocket broken
      }
    }

    return new Response("OK", { status: 200 });
  }

  /** Clean up KV registration and stored state. */
  private async cleanup(): Promise<void> {
    const endpointIdHex = await this.state.storage.get<string>(
      "endpoint_id_hex",
    );
    if (endpointIdHex) {
      await this.env.ZEDRA_RELAY_KV.delete(
        RELAY_EP_PREFIX + endpointIdHex,
      );
    }
    await this.state.storage.deleteAll();
  }
}
