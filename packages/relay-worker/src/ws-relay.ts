// WebSocket Relay Durable Object
//
// Manages a persistent WebSocket relay room. Two peers (host + mobile)
// connect via WebSocket, and the DO forwards binary messages between them
// in real-time. No polling required.
//
// Each room maps to a single DO instance identified by the room ID.

import type { Env } from "./types";

interface WsConnection {
  socket: WebSocket;
  role: string;
  connectedAt: number;
}

export class WsRelay {
  private state: DurableObjectState;
  private connections: Map<string, WsConnection> = new Map();

  constructor(state: DurableObjectState, _env: Env) {
    this.state = state;
  }

  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    const role = url.searchParams.get("role");
    const secret = url.searchParams.get("secret");

    if (!role || (role !== "host" && role !== "mobile")) {
      return new Response("Invalid role parameter", { status: 400 });
    }

    // Validate the room exists and secret matches
    const roomSecret = await this.state.storage.get<string>("secret");
    if (roomSecret && secret !== roomSecret) {
      return new Response("Unauthorized", { status: 401 });
    }

    // If no secret stored yet, this is the first connection — store it
    if (!roomSecret && secret) {
      await this.state.storage.put("secret", secret);
    }

    // Check if this role slot is already taken
    const existing = this.connections.get(role);
    if (existing) {
      // Close the old connection (reconnection scenario)
      try {
        existing.socket.close(1000, "replaced by new connection");
      } catch {
        // Already closed
      }
      this.connections.delete(role);
    }

    // Create WebSocket pair
    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);

    // Accept the server side
    this.state.acceptWebSocket(server, [role]);

    this.connections.set(role, {
      socket: server,
      role,
      connectedAt: Date.now(),
    });

    return new Response(null, {
      status: 101,
      webSocket: client,
    });
  }

  async webSocketMessage(ws: WebSocket, message: ArrayBuffer | string): Promise<void> {
    // Find the sender's role
    const tags = this.state.getTags(ws);
    const senderRole = tags[0];
    if (!senderRole) return;

    // Forward to the peer
    const peerRole = senderRole === "host" ? "mobile" : "host";
    const peer = this.connections.get(peerRole);

    if (peer) {
      try {
        peer.socket.send(message);
      } catch {
        // Peer disconnected
        this.connections.delete(peerRole);
      }
    }
    // If no peer connected yet, message is dropped (caller should retry)
  }

  async webSocketClose(ws: WebSocket, code: number, _reason: string, _wasClean: boolean): Promise<void> {
    const tags = this.state.getTags(ws);
    const role = tags[0];
    if (role) {
      this.connections.delete(role);
    }

    // If both peers disconnected, clean up after a grace period
    if (this.connections.size === 0) {
      // Schedule cleanup (DO will be evicted by the runtime after inactivity)
    }
  }

  async webSocketError(ws: WebSocket, _error: unknown): Promise<void> {
    const tags = this.state.getTags(ws);
    const role = tags[0];
    if (role) {
      this.connections.delete(role);
    }
  }
}
