// RelayRoom Durable Object — one instance per host endpoint.
//
// Named "room:<hostEndpointIdHex>", placed at the CF edge nearest the host
// on first creation. All endpoints (host + clients) connecting with
// ?host=<hex> share the same DO, so datagram forwarding is a pure
// in-memory Map<hex, WebSocket>.send() — no DO-to-DO HTTP hop.
//
// State model (tags are immutable after acceptWebSocket):
//   accept tag:  ["pid:<uuid>"]
//   on auth:     storage["pid:<uuid>"] = endpointIdHex
//                storage["chal:<uuid>"] deleted
//   on wake:     ensureClientsMap() does one storage.list("pid:") +
//                getWebSockets() scan to rebuild _authed + _clients

import type { Env } from "./types";
import {
  FrameType,
  decodeFrameType,
  decodeClientAuth,
  decodeClientDatagram,
  decodeClientDatagramBatch,
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
  hexDecode,
  HANDSHAKE_DOMAIN,
} from "./crypto";

// Keepalive ping interval for idle connections.
const KEEPALIVE_INTERVAL_MS = 15_000;
const KEEPALIVE_JITTER_MS = 5_000;

export class RelayRoom {
  private state: DurableObjectState;
  private env: Env;

  // endpointIdHex → WebSocket  (O(1) forwarding)
  private _clients = new Map<string, WebSocket>();
  // pendingId → endpointIdHex  (identify sender on each message)
  private _authed = new Map<string, string>();
  // reset each JS context; ensureClientsMap() sets it true after rebuild
  private _clientsReady = false;
  // Tracks when to next send keepalive pings; 0 = ping on first alarm fire.
  // Reset to 0 on every hibernation wake (field is in-memory only).
  private _nextPingAt = 0;

  constructor(state: DurableObjectState, env: Env) {
    this.state = state;
    this.env = env;
  }

  // --- Hibernation recovery ---

  // Rebuild both maps from storage + live WebSockets.
  // Called at the start of every handler; no-op after first call per context.
  private async ensureClientsMap(): Promise<void> {
    if (this._clientsReady) return;

    const stored = await this.state.storage.list<string>({ prefix: "pid:" });
    for (const ws of this.state.getWebSockets()) {
      const tags = this.state.getTags(ws);
      const pidTag = tags.find((t) => t.startsWith("pid:"));
      if (!pidTag) continue;
      const pendingId = pidTag.slice(4);
      const endpointIdHex = stored.get(`pid:${pendingId}`);
      if (endpointIdHex) {
        this._authed.set(pendingId, endpointIdHex);
        this._clients.set(endpointIdHex, ws);
      }
    }

    this._clientsReady = true;
  }

  // --- WebSocket upgrade ---

  async fetch(request: Request): Promise<Response> {
    if (request.headers.get("Upgrade") !== "websocket") {
      return new Response("Expected WebSocket upgrade", { status: 426 });
    }

    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);

    const pendingId = crypto.randomUUID();
    this.state.acceptWebSocket(server, [`pid:${pendingId}`]);

    const challenge = new Uint8Array(16);
    crypto.getRandomValues(challenge);
    await this.state.storage.put(`chal:${pendingId}`, Array.from(challenge));

    server.send(encodeServerChallenge(challenge));

    // Ensure keepalive alarm is running
    if ((await this.state.storage.getAlarm()) === null) {
      const jitter = Math.floor(Math.random() * KEEPALIVE_JITTER_MS);
      await this.state.storage.setAlarm(
        Date.now() + KEEPALIVE_INTERVAL_MS + jitter,
      );
    }

    // Echo back Sec-WebSocket-Protocol if the client requested one.
    // iroh sends "iroh-relay-v1" and tokio_websockets validates it's echoed.
    const responseHeaders: Record<string, string> = {};
    const requestedProtocol = request.headers.get("Sec-WebSocket-Protocol");
    if (requestedProtocol) {
      responseHeaders["Sec-WebSocket-Protocol"] = requestedProtocol;
    }

    return new Response(null, {
      status: 101,
      webSocket: client,
      headers: responseHeaders,
    });
  }

  // --- WebSocket message dispatch ---

  async webSocketMessage(
    ws: WebSocket,
    message: ArrayBuffer | string,
  ): Promise<void> {
    if (typeof message === "string") return;

    await this.ensureClientsMap();

    const tags = this.state.getTags(ws);
    const pidTag = tags.find((t) => t.startsWith("pid:"));
    if (!pidTag) return;
    const pendingId = pidTag.slice(4);

    const buf = new Uint8Array(message);
    if (buf.length < 1) return;
    const { type, offset } = decodeFrameType(buf);
    const body = buf.slice(offset);

    const endpointIdHex = this._authed.get(pendingId);
    if (!endpointIdHex) {
      await this.handleAuth(ws, type, body, pendingId);
    } else {
      this.handleDatagram(ws, type, body, endpointIdHex);
    }
  }

  async webSocketClose(
    ws: WebSocket,
    _code: number,
    _reason: string,
    _wasClean: boolean,
  ): Promise<void> {
    await this.handleDisconnect(ws);
  }

  async webSocketError(ws: WebSocket, _error: unknown): Promise<void> {
    await this.handleDisconnect(ws);
  }

  // --- Keepalive alarm ---

  async alarm(): Promise<void> {
    await this.ensureClientsMap();

    const sockets = this.state.getWebSockets();
    if (sockets.length === 0) return;

    const now = Date.now();

    if (now >= this._nextPingAt) {
      // Keepalive ping time — send pings to all authenticated sockets.
      const pingPayload = new Uint8Array(8);
      crypto.getRandomValues(pingPayload);
      const pingFrame = encodePing(pingPayload);

      for (const ws of sockets) {
        const tags = this.state.getTags(ws);
        const pidTag = tags.find((t) => t.startsWith("pid:"));
        if (!pidTag) continue;
        if (!this._authed.has(pidTag.slice(4))) continue;
        try {
          ws.send(pingFrame);
        } catch {
          // broken — cleaned up in webSocketClose
        }
      }

      const jitter = Math.floor(Math.random() * KEEPALIVE_JITTER_MS);
      this._nextPingAt = now + KEEPALIVE_INTERVAL_MS + jitter;
    }
    // else: keep-warm wake — DO context stays alive, no ping needed.

    // Reschedule for the next keepalive ping. If webSocketMessage() receives
    // packets before then it will push the alarm forward (keep-warm), and
    // this reschedule ensures we still eventually send the keepalive ping.
    await this.state.storage.setAlarm(this._nextPingAt);
  }

  // --- Auth handler ---

  private async handleAuth(
    ws: WebSocket,
    type: number,
    body: Uint8Array,
    pendingId: string,
  ): Promise<void> {
    if (type !== FrameType.ClientAuth) {
      ws.send(encodeServerDeniesAuth("Expected ClientAuth frame"));
      ws.close(1008, "Unexpected frame before auth");
      return;
    }

    const challengeArr = await this.state.storage.get<number[]>(
      `chal:${pendingId}`,
    );
    if (!challengeArr) {
      ws.close(1008, "Challenge expired");
      return;
    }

    let auth;
    try {
      auth = decodeClientAuth(body);
    } catch (e) {
      ws.send(encodeServerDeniesAuth(e instanceof Error ? e.message : "Decode error"));
      ws.close(1008, "Invalid ClientAuth");
      return;
    }

    const derived = blake3DeriveKey(HANDSHAKE_DOMAIN, new Uint8Array(challengeArr));
    if (!ed25519Verify(auth.publicKey, derived, auth.signature)) {
      ws.send(encodeServerDeniesAuth("Invalid signature"));
      ws.close(1008, "Auth failed");
      return;
    }

    const endpointIdHex = hexEncode(auth.publicKey);

    // Close any existing connection for this endpoint (reconnection)
    const existingWs = this._clients.get(endpointIdHex);
    if (existingWs) {
      for (const [pid, hex] of this._authed.entries()) {
        if (hex === endpointIdHex) {
          this._authed.delete(pid);
          this.state.storage.delete(`pid:${pid}`);
          break;
        }
      }
      try {
        existingWs.close(1000, "replaced by new connection");
      } catch { /* already closed */ }
    }

    // Register in memory + persist for hibernation recovery
    this._authed.set(pendingId, endpointIdHex);
    this._clients.set(endpointIdHex, ws);
    await Promise.all([
      this.state.storage.put(`pid:${pendingId}`, endpointIdHex),
      this.state.storage.delete(`chal:${pendingId}`),
    ]);

    ws.send(encodeServerConfirmsAuth());
  }

  // --- Datagram forwarding (hot path — pure in-memory, no I/O) ---

  private handleDatagram(
    ws: WebSocket,
    type: number,
    body: Uint8Array,
    srcHex: string,
  ): void {
    switch (type) {
      case FrameType.ClientToRelayDatagram: {
        const { dstId, ecn, data } = decodeClientDatagram(body);
        const dstWs = this._clients.get(hexEncode(dstId));
        if (!dstWs) {
          ws.send(encodeEndpointGone(dstId));
          return;
        }
        dstWs.send(encodeRelayToClientDatagram(hexDecode(srcHex), ecn, data));
        break;
      }

      case FrameType.ClientToRelayDatagramBatch: {
        const { dstId, ecn, segmentSize, data } = decodeClientDatagramBatch(body);
        const dstWs = this._clients.get(hexEncode(dstId));
        if (!dstWs) {
          ws.send(encodeEndpointGone(dstId));
          return;
        }
        dstWs.send(
          encodeRelayToClientDatagramBatch(hexDecode(srcHex), ecn, segmentSize, data),
        );
        break;
      }

      case FrameType.Ping:
        ws.send(encodePong(body.slice(0, 8)));
        break;

      case FrameType.Pong:
        break;

      default:
        break;
    }
  }

  // --- Disconnect cleanup ---

  private async handleDisconnect(ws: WebSocket): Promise<void> {
    const tags = this.state.getTags(ws);
    const pidTag = tags.find((t) => t.startsWith("pid:"));
    if (!pidTag) return;
    const pendingId = pidTag.slice(4);

    const endpointIdHex = this._authed.get(pendingId);
    if (endpointIdHex) {
      this._clients.delete(endpointIdHex);
      this._authed.delete(pendingId);
    }

    await Promise.all([
      this.state.storage.delete(`pid:${pendingId}`),
      this.state.storage.delete(`chal:${pendingId}`),
    ]);

    if (this.state.getWebSockets().length === 0) {
      await this.state.storage.deleteAlarm();
    }
  }
}
