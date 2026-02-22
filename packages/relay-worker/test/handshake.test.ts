// Handshake protocol tests — full challenge-response simulation.

import { describe, it, expect } from "vitest";
import { ed25519 } from "@noble/curves/ed25519";
import {
  FrameType,
  encodeServerChallenge,
  encodeServerConfirmsAuth,
  encodeServerDeniesAuth,
  decodeFrameType,
  decodeClientAuth,
  decodeServerChallenge,
  decodeServerDeniesAuth,
} from "../src/frame-codec";
import { blake3DeriveKey, ed25519Verify, HANDSHAKE_DOMAIN } from "../src/crypto";

const TEST_SECRET = new Uint8Array(32).fill(42);
const TEST_PUBKEY = ed25519.getPublicKey(TEST_SECRET);

/**
 * Build a ClientAuth frame body in postcard format:
 * [32B pubkey][varint(64)=0x40][64B signature]
 */
function buildClientAuthBody(
  publicKey: Uint8Array,
  signature: Uint8Array,
): Uint8Array {
  const body = new Uint8Array(1 + 32 + 1 + 64); // type + pubkey + varint + sig
  body[0] = FrameType.ClientAuth;
  body.set(publicKey, 1);
  body[33] = 0x40; // postcard varint for 64
  body.set(signature, 34);
  return body;
}

describe("handshake simulation", () => {
  it("full handshake: challenge → auth → confirm", () => {
    // Step 1: Server sends challenge
    const challenge = new Uint8Array(16);
    for (let i = 0; i < 16; i++) challenge[i] = i * 3;

    const challengeFrame = encodeServerChallenge(challenge);
    const { type: challengeType, offset: challengeOff } =
      decodeFrameType(challengeFrame);
    expect(challengeType).toBe(FrameType.ServerChallenge);
    const receivedChallenge = decodeServerChallenge(
      challengeFrame.slice(challengeOff),
    );
    expect(Array.from(receivedChallenge)).toEqual(Array.from(challenge));

    // Step 2: Client signs the challenge
    const derived = blake3DeriveKey(HANDSHAKE_DOMAIN, receivedChallenge);
    const signature = ed25519.sign(derived, TEST_SECRET);

    // Step 3: Client sends ClientAuth
    const authFrame = buildClientAuthBody(TEST_PUBKEY, signature);
    const { type: authType, offset: authOff } = decodeFrameType(authFrame);
    expect(authType).toBe(FrameType.ClientAuth);
    const authPayload = decodeClientAuth(authFrame.slice(authOff));
    expect(Array.from(authPayload.publicKey)).toEqual(
      Array.from(TEST_PUBKEY),
    );
    expect(authPayload.signature.length).toBe(64);

    // Step 4: Server verifies
    const serverDerived = blake3DeriveKey(HANDSHAKE_DOMAIN, challenge);
    const valid = ed25519Verify(
      authPayload.publicKey,
      serverDerived,
      authPayload.signature,
    );
    expect(valid).toBe(true);

    // Step 5: Server confirms
    const confirmFrame = encodeServerConfirmsAuth();
    const { type: confirmType } = decodeFrameType(confirmFrame);
    expect(confirmType).toBe(FrameType.ServerConfirmsAuth);
  });

  it("invalid signature → deny", () => {
    const challenge = new Uint8Array(16).fill(0xbb);

    // Client signs with wrong key
    const wrongSecret = new Uint8Array(32).fill(99);
    const derived = blake3DeriveKey(HANDSHAKE_DOMAIN, challenge);
    const badSignature = ed25519.sign(derived, wrongSecret);

    // Server verifies with the REAL public key → should fail
    const serverDerived = blake3DeriveKey(HANDSHAKE_DOMAIN, challenge);
    const valid = ed25519Verify(TEST_PUBKEY, serverDerived, badSignature);
    expect(valid).toBe(false);

    // Server sends deny
    const denyFrame = encodeServerDeniesAuth("Invalid signature");
    const { type, offset } = decodeFrameType(denyFrame);
    expect(type).toBe(FrameType.ServerDeniesAuth);
    const reason = decodeServerDeniesAuth(denyFrame.slice(offset));
    expect(reason).toBe("Invalid signature");
  });

  it("ClientAuth postcard decode with known bytes", () => {
    // Build exact postcard bytes
    const pubkey = TEST_PUBKEY;
    const sig = new Uint8Array(64);
    for (let i = 0; i < 64; i++) sig[i] = 0x2a; // all 42s

    // Body format: [32B pubkey][0x40][64B sig]
    const body = new Uint8Array(97);
    body.set(pubkey, 0);
    body[32] = 0x40;
    body.set(sig, 33);

    const decoded = decodeClientAuth(body);
    expect(Array.from(decoded.publicKey)).toEqual(Array.from(pubkey));
    expect(Array.from(decoded.signature)).toEqual(Array.from(sig));
    expect(decoded.signature.length).toBe(64);
  });

  it("rejects truncated ClientAuth", () => {
    // Only 50 bytes — too short
    expect(() => decodeClientAuth(new Uint8Array(50))).toThrow("too short");
  });

  it("rejects ClientAuth with wrong signature length varint", () => {
    const body = new Uint8Array(97);
    body.set(TEST_PUBKEY, 0);
    body[32] = 0x20; // varint(32) instead of varint(64)
    // This should fail because signature length != 64
    expect(() => decodeClientAuth(body)).toThrow(
      "Expected signature length 64",
    );
  });
});
