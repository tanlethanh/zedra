// Integration tests for RelayEndpoint Durable Object.
//
// Uses @cloudflare/vitest-pool-workers to run tests in a miniflare environment
// with real Durable Objects and KV.

import {
  describe,
  it,
  expect,
  beforeEach,
} from "vitest";
import {
  env,
  SELF,
} from "cloudflare:test";
import { ed25519 } from "@noble/curves/ed25519";
import {
  FrameType,
  decodeFrameType,
  decodeServerChallenge,
  decodePing,
  encodePong,
  encodeEndpointGone,
} from "../src/frame-codec";
import { blake3DeriveKey, hexEncode, HANDSHAKE_DOMAIN } from "../src/crypto";

const TEST_SECRET_A = new Uint8Array(32).fill(42);
const TEST_PUBKEY_A = ed25519.getPublicKey(TEST_SECRET_A);

const TEST_SECRET_B = new Uint8Array(32).fill(99);
const TEST_PUBKEY_B = ed25519.getPublicKey(TEST_SECRET_B);

/**
 * Build a ClientAuth frame (type byte + postcard body).
 */
function buildClientAuthFrame(
  publicKey: Uint8Array,
  signature: Uint8Array,
): Uint8Array {
  const frame = new Uint8Array(1 + 32 + 1 + 64);
  frame[0] = FrameType.ClientAuth;
  frame.set(publicKey, 1);
  frame[33] = 0x40;
  frame.set(signature, 34);
  return frame;
}

/**
 * Build a ClientToRelayDatagram frame.
 */
function buildClientDatagram(
  dstId: Uint8Array,
  ecn: number,
  data: Uint8Array,
): Uint8Array {
  const frame = new Uint8Array(1 + 32 + 1 + data.length);
  frame[0] = FrameType.ClientToRelayDatagram;
  frame.set(dstId, 1);
  frame[33] = ecn;
  frame.set(data, 34);
  return frame;
}

/**
 * Connect to the relay and perform the full handshake.
 * Returns the authenticated WebSocket.
 */
async function connectAndAuth(
  secretKey: Uint8Array,
): Promise<{ ws: WebSocket; publicKey: Uint8Array }> {
  const publicKey = ed25519.getPublicKey(secretKey);

  // Upgrade to WebSocket
  const resp = await SELF.fetch("https://relay.zedra.dev/relay", {
    headers: { Upgrade: "websocket" },
  });
  expect(resp.status).toBe(101);
  const ws = resp.webSocket!;
  ws.accept();

  // Wait for ServerChallenge
  const challenge = await waitForBinaryMessage(ws);
  const { type, offset } = decodeFrameType(new Uint8Array(challenge));
  expect(type).toBe(FrameType.ServerChallenge);
  const challengeBytes = decodeServerChallenge(
    new Uint8Array(challenge).slice(offset),
  );

  // Sign challenge
  const derived = blake3DeriveKey(HANDSHAKE_DOMAIN, challengeBytes);
  const signature = ed25519.sign(derived, secretKey);

  // Send ClientAuth
  ws.send(buildClientAuthFrame(publicKey, signature));

  // Wait for ServerConfirmsAuth
  const confirmMsg = await waitForBinaryMessage(ws);
  const { type: confirmType } = decodeFrameType(new Uint8Array(confirmMsg));
  expect(confirmType).toBe(FrameType.ServerConfirmsAuth);

  return { ws, publicKey };
}

/**
 * Wait for the next binary WebSocket message.
 */
function waitForBinaryMessage(ws: WebSocket): Promise<ArrayBuffer> {
  return new Promise((resolve, reject) => {
    const handler = (event: MessageEvent) => {
      ws.removeEventListener("message", handler);
      if (event.data instanceof ArrayBuffer) {
        resolve(event.data);
      } else {
        reject(new Error("Expected binary message"));
      }
    };
    ws.addEventListener("message", handler);
    // Timeout after 5 seconds
    setTimeout(
      () => reject(new Error("Timeout waiting for message")),
      5000,
    );
  });
}

describe("RelayEndpoint integration", () => {
  it("connects and completes handshake", async () => {
    const { ws } = await connectAndAuth(TEST_SECRET_A);
    ws.close();
  });

  it("registers endpoint in KV after handshake", async () => {
    const { ws, publicKey } = await connectAndAuth(TEST_SECRET_A);

    // Check KV for routing entry
    const hex = hexEncode(publicKey);
    const kvData = await env.ZEDRA_RELAY_KV.get("relay:ep:" + hex);
    expect(kvData).not.toBeNull();
    const route = JSON.parse(kvData!);
    expect(route.do_name).toBeDefined();

    ws.close();
  });

  it("responds to Ping with Pong", async () => {
    const { ws } = await connectAndAuth(TEST_SECRET_A);

    // Client sends Ping to server
    const pingPayload = new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]);
    const pingFrame = new Uint8Array(9);
    pingFrame[0] = FrameType.Ping;
    pingFrame.set(pingPayload, 1);
    ws.send(pingFrame);

    // Server should respond with Pong
    const pongMsg = await waitForBinaryMessage(ws);
    const pongBuf = new Uint8Array(pongMsg);
    const { type, offset } = decodeFrameType(pongBuf);
    expect(type).toBe(FrameType.Pong);
    const pongPayload = pongBuf.slice(offset, offset + 8);
    expect(Array.from(pongPayload)).toEqual(Array.from(pingPayload));

    ws.close();
  });

  it("rejects unauthenticated datagram", async () => {
    // Connect but don't auth — send a datagram immediately
    const resp = await SELF.fetch("https://relay.zedra.dev/relay", {
      headers: { Upgrade: "websocket" },
    });
    expect(resp.status).toBe(101);
    const ws = resp.webSocket!;
    ws.accept();

    // Wait for challenge
    await waitForBinaryMessage(ws);

    // Send datagram without auth
    const frame = buildClientDatagram(
      TEST_PUBKEY_B,
      0,
      new TextEncoder().encode("sneaky"),
    );
    ws.send(frame);

    // Should get ServerDeniesAuth
    const denyMsg = await waitForBinaryMessage(ws);
    const { type } = decodeFrameType(new Uint8Array(denyMsg));
    expect(type).toBe(FrameType.ServerDeniesAuth);

    ws.close();
  });

  it("sends EndpointGone for unknown target", async () => {
    const { ws } = await connectAndAuth(TEST_SECRET_A);

    // Send datagram to an endpoint that doesn't exist
    const unknownKey = new Uint8Array(32).fill(0xff);
    const frame = buildClientDatagram(
      unknownKey,
      0,
      new TextEncoder().encode("hello?"),
    );
    ws.send(frame);

    // Should receive EndpointGone
    const goneMsg = await waitForBinaryMessage(ws);
    const goneBuf = new Uint8Array(goneMsg);
    const { type, offset } = decodeFrameType(goneBuf);
    expect(type).toBe(FrameType.EndpointGone);
    const goneId = goneBuf.slice(offset, offset + 32);
    expect(Array.from(goneId)).toEqual(Array.from(unknownKey));

    ws.close();
  });

  it("denies auth with invalid signature", async () => {
    const resp = await SELF.fetch("https://relay.zedra.dev/relay", {
      headers: { Upgrade: "websocket" },
    });
    expect(resp.status).toBe(101);
    const ws = resp.webSocket!;
    ws.accept();

    // Get challenge
    const challengeMsg = await waitForBinaryMessage(ws);
    const challengeBuf = new Uint8Array(challengeMsg);
    const { offset } = decodeFrameType(challengeBuf);
    const challenge = decodeServerChallenge(challengeBuf.slice(offset));

    // Sign with wrong key
    const wrongSecret = new Uint8Array(32).fill(0);
    const derived = blake3DeriveKey(HANDSHAKE_DOMAIN, challenge);
    const badSig = ed25519.sign(derived, wrongSecret);

    // Send auth with TEST_PUBKEY_A but signature from wrong key
    ws.send(buildClientAuthFrame(TEST_PUBKEY_A, badSig));

    // Should get deny
    const denyMsg = await waitForBinaryMessage(ws);
    const { type } = decodeFrameType(new Uint8Array(denyMsg));
    expect(type).toBe(FrameType.ServerDeniesAuth);

    ws.close();
  });

  it("forwards datagrams between two endpoints", async () => {
    // Connect endpoint A
    const { ws: wsA } = await connectAndAuth(TEST_SECRET_A);

    // Connect endpoint B
    const { ws: wsB } = await connectAndAuth(TEST_SECRET_B);

    // A sends datagram to B
    const payload = new TextEncoder().encode("hello from A");
    const frame = buildClientDatagram(TEST_PUBKEY_B, 1, payload);
    wsA.send(frame);

    // B should receive RelayToClientDatagram from A
    const relayMsg = await waitForBinaryMessage(wsB);
    const relayBuf = new Uint8Array(relayMsg);
    const { type, offset } = decodeFrameType(relayBuf);
    expect(type).toBe(FrameType.RelayToClientDatagram);

    // Verify source is A's public key
    const srcId = relayBuf.slice(offset, offset + 32);
    expect(Array.from(srcId)).toEqual(Array.from(TEST_PUBKEY_A));

    // Verify ECN
    expect(relayBuf[offset + 32]).toBe(1);

    // Verify payload
    const receivedPayload = relayBuf.slice(offset + 33);
    expect(new TextDecoder().decode(receivedPayload)).toBe("hello from A");

    wsA.close();
    wsB.close();
  });
});
