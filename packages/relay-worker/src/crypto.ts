// Cryptographic utilities for iroh relay handshake.
//
// Uses @noble/hashes (BLAKE3) and @noble/curves (Ed25519) for
// wire-compatible challenge-response authentication.

import { blake3 } from "@noble/hashes/blake3";
import { ed25519 } from "@noble/curves/ed25519";

/** Domain separation string used by iroh for relay handshake signatures. */
export const HANDSHAKE_DOMAIN =
  "iroh-relay handshake v1 challenge signature";

/**
 * BLAKE3 key derivation (KDF mode).
 *
 * Derives a 32-byte key from the given context string and input.
 * Wire-compatible with Rust's `blake3::derive_key(context, input)`.
 */
export function blake3DeriveKey(
  context: string,
  input: Uint8Array,
): Uint8Array {
  return blake3(input, { context });
}

/**
 * Verify an Ed25519 signature.
 *
 * Returns true if the signature is valid for the given public key and message.
 */
export function ed25519Verify(
  publicKey: Uint8Array,
  message: Uint8Array,
  signature: Uint8Array,
): boolean {
  try {
    return ed25519.verify(signature, message, publicKey);
  } catch {
    return false;
  }
}

/** Encode bytes to lowercase hex string. */
export function hexEncode(bytes: Uint8Array): string {
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

/** Decode a hex string to bytes. */
export function hexDecode(hex: string): Uint8Array {
  if (hex.length % 2 !== 0) throw new Error("Hex string must have even length");
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(hex.substring(i * 2, i * 2 + 2), 16);
  }
  return bytes;
}
