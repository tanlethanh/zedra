// Crypto module tests — BLAKE3 KDF + Ed25519 verification.

import { describe, it, expect } from "vitest";
import { ed25519 } from "@noble/curves/ed25519";
import {
  blake3DeriveKey,
  ed25519Verify,
  hexEncode,
  hexDecode,
  HANDSHAKE_DOMAIN,
} from "../src/crypto";

// Known test key: [42u8; 32]
const TEST_SECRET = new Uint8Array(32).fill(42);
const TEST_PUBKEY = ed25519.getPublicKey(TEST_SECRET);
const TEST_PUBKEY_HEX =
  "197f6b23e16c8532c6abc838facd5ea789be0c76b2920334039bfa8b3d368d61";

describe("hexEncode / hexDecode", () => {
  it("roundtrips bytes", () => {
    const bytes = new Uint8Array([0, 1, 15, 16, 255]);
    expect(hexEncode(bytes)).toBe("00010f10ff");
    expect(Array.from(hexDecode("00010f10ff"))).toEqual(
      Array.from(bytes),
    );
  });

  it("encodes test public key", () => {
    expect(hexEncode(TEST_PUBKEY)).toBe(TEST_PUBKEY_HEX);
  });

  it("rejects odd-length hex", () => {
    expect(() => hexDecode("abc")).toThrow("even length");
  });
});

describe("blake3DeriveKey", () => {
  it("produces 32-byte output", () => {
    const result = blake3DeriveKey("test context", new Uint8Array(16));
    expect(result.length).toBe(32);
  });

  it("is deterministic", () => {
    const input = new Uint8Array([1, 2, 3, 4]);
    const a = blake3DeriveKey("ctx", input);
    const b = blake3DeriveKey("ctx", input);
    expect(hexEncode(a)).toBe(hexEncode(b));
  });

  it("different contexts produce different output", () => {
    const input = new Uint8Array([1, 2, 3, 4]);
    const a = blake3DeriveKey("context-a", input);
    const b = blake3DeriveKey("context-b", input);
    expect(hexEncode(a)).not.toBe(hexEncode(b));
  });
});

describe("ed25519Verify", () => {
  it("verifies a valid signature", () => {
    const message = new Uint8Array([1, 2, 3, 4, 5]);
    const signature = ed25519.sign(message, TEST_SECRET);
    expect(ed25519Verify(TEST_PUBKEY, message, signature)).toBe(true);
  });

  it("rejects an invalid signature", () => {
    const message = new Uint8Array([1, 2, 3, 4, 5]);
    const badSig = new Uint8Array(64); // zeros
    expect(ed25519Verify(TEST_PUBKEY, message, badSig)).toBe(false);
  });

  it("rejects wrong message", () => {
    const message = new Uint8Array([1, 2, 3, 4, 5]);
    const signature = ed25519.sign(message, TEST_SECRET);
    const wrongMessage = new Uint8Array([5, 4, 3, 2, 1]);
    expect(ed25519Verify(TEST_PUBKEY, wrongMessage, signature)).toBe(
      false,
    );
  });

  it("rejects wrong public key", () => {
    const message = new Uint8Array([1, 2, 3, 4, 5]);
    const signature = ed25519.sign(message, TEST_SECRET);
    const otherSecret = new Uint8Array(32).fill(99);
    const otherPubkey = ed25519.getPublicKey(otherSecret);
    expect(ed25519Verify(otherPubkey, message, signature)).toBe(false);
  });
});

describe("challenge-response flow", () => {
  it("full handshake: generate challenge → sign → verify", () => {
    // Server generates challenge
    const challenge = new Uint8Array(16);
    for (let i = 0; i < 16; i++) challenge[i] = i;

    // Client derives key and signs
    const derived = blake3DeriveKey(HANDSHAKE_DOMAIN, challenge);
    const signature = ed25519.sign(derived, TEST_SECRET);

    // Server verifies
    const serverDerived = blake3DeriveKey(HANDSHAKE_DOMAIN, challenge);
    expect(ed25519Verify(TEST_PUBKEY, serverDerived, signature)).toBe(true);
  });

  it("wrong domain separation fails", () => {
    const challenge = new Uint8Array(16).fill(0xaa);
    const derived = blake3DeriveKey(HANDSHAKE_DOMAIN, challenge);
    const signature = ed25519.sign(derived, TEST_SECRET);

    // Verify with wrong domain
    const wrongDerived = blake3DeriveKey("wrong domain", challenge);
    expect(ed25519Verify(TEST_PUBKEY, wrongDerived, signature)).toBe(
      false,
    );
  });
});
