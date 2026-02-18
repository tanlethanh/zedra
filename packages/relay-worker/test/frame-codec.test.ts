// Frame codec tests — ported from iroh-relay test_server_client_frames_snapshot.
//
// Test vectors use known key SecretKey::from_bytes(&[42u8; 32])
// → public key hex: 197f6b23e16c8532c6abc838facd5ea789be0c76b2920334039bfa8b3d368d61

import { describe, it, expect } from "vitest";
import {
  FrameType,
  encodeServerChallenge,
  encodeServerConfirmsAuth,
  encodeServerDeniesAuth,
  encodeRelayToClientDatagram,
  encodeRelayToClientDatagramBatch,
  encodeEndpointGone,
  encodePing,
  encodePong,
  encodeHealth,
  encodeRestarting,
  decodeFrameType,
  decodeClientAuth,
  decodeClientDatagram,
  decodeClientDatagramBatch,
  decodePing,
  decodePong,
  decodeServerChallenge,
  decodeHealth,
  decodeRestarting,
  decodeServerDeniesAuth,
} from "../src/frame-codec";

// Known test public key: SecretKey::from_bytes(&[42u8; 32]) → PublicKey
const TEST_KEY = hexToBytes(
  "197f6b23e16c8532c6abc838facd5ea789be0c76b2920334039bfa8b3d368d61",
);

function hexToBytes(hex: string): Uint8Array {
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(hex.substring(i * 2, i * 2 + 2), 16);
  }
  return bytes;
}

function bytesToHex(bytes: Uint8Array): string {
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

describe("frame-codec snapshot tests", () => {
  it("encodes Health frame", () => {
    const frame = encodeHealth("Hello? Yes this is dog.");
    const hex = bytesToHex(frame);
    expect(hex).toBe(
      "0b" +
        bytesToHex(new TextEncoder().encode("Hello? Yes this is dog.")),
    );
    expect(hex.startsWith("0b")).toBe(true);
    // Verify content
    expect(hex).toBe(
      "0b48656c6c6f3f2059657320746869732069732064"+ "6f672e",
    );
  });

  it("encodes EndpointGone frame", () => {
    const frame = encodeEndpointGone(TEST_KEY);
    const hex = bytesToHex(frame);
    expect(hex).toBe(
      "08197f6b23e16c8532c6abc838facd5ea789be0c76b2920334039bfa8b3d368d61",
    );
  });

  it("encodes Ping frame", () => {
    const payload = new Uint8Array([0x2a, 0x2a, 0x2a, 0x2a, 0x2a, 0x2a, 0x2a, 0x2a]);
    const frame = encodePing(payload);
    expect(bytesToHex(frame)).toBe("092a2a2a2a2a2a2a2a");
  });

  it("encodes Pong frame", () => {
    const payload = new Uint8Array([0x2a, 0x2a, 0x2a, 0x2a, 0x2a, 0x2a, 0x2a, 0x2a]);
    const frame = encodePong(payload);
    expect(bytesToHex(frame)).toBe("0a2a2a2a2a2a2a2a2a");
  });

  it("encodes RelayToClientDatagramBatch frame", () => {
    const data = new TextEncoder().encode("Hello World!");
    const frame = encodeRelayToClientDatagramBatch(TEST_KEY, 3, 6, data);
    const hex = bytesToHex(frame);
    // [0x07][32B key][ecn=3][seg_size=0x0006][data]
    expect(hex).toBe(
      "07" +
        "197f6b23e16c8532c6abc838facd5ea789be0c76b2920334039bfa8b3d368d61" +
        "03" +
        "0006" +
        bytesToHex(data),
    );
  });

  it("encodes RelayToClientDatagram frame", () => {
    const data = new TextEncoder().encode("Hello World!");
    const frame = encodeRelayToClientDatagram(TEST_KEY, 3, data);
    const hex = bytesToHex(frame);
    // [0x06][32B key][ecn=3][data]
    expect(hex).toBe(
      "06" +
        "197f6b23e16c8532c6abc838facd5ea789be0c76b2920334039bfa8b3d368d61" +
        "03" +
        bytesToHex(data),
    );
  });

  it("encodes Restarting frame", () => {
    const frame = encodeRestarting(10, 20);
    expect(bytesToHex(frame)).toBe("0c000000" + "0a000000" + "14");
  });
});

describe("frame-codec roundtrip tests", () => {
  it("ServerChallenge encode/decode roundtrip", () => {
    const challenge = new Uint8Array(16);
    for (let i = 0; i < 16; i++) challenge[i] = i;
    const encoded = encodeServerChallenge(challenge);
    const { type, offset } = decodeFrameType(encoded);
    expect(type).toBe(FrameType.ServerChallenge);
    const decoded = decodeServerChallenge(encoded.slice(offset));
    expect(Array.from(decoded)).toEqual(Array.from(challenge));
  });

  it("ServerConfirmsAuth encode/decode roundtrip", () => {
    const encoded = encodeServerConfirmsAuth();
    const { type } = decodeFrameType(encoded);
    expect(type).toBe(FrameType.ServerConfirmsAuth);
    expect(encoded.length).toBe(1);
  });

  it("ServerDeniesAuth encode/decode roundtrip", () => {
    const reason = "bad signature";
    const encoded = encodeServerDeniesAuth(reason);
    const { type, offset } = decodeFrameType(encoded);
    expect(type).toBe(FrameType.ServerDeniesAuth);
    const decoded = decodeServerDeniesAuth(encoded.slice(offset));
    expect(decoded).toBe(reason);
  });

  it("Ping/Pong encode/decode roundtrip", () => {
    const payload = new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]);

    const pingEncoded = encodePing(payload);
    const { type: pingType, offset: pingOff } = decodeFrameType(pingEncoded);
    expect(pingType).toBe(FrameType.Ping);
    expect(Array.from(decodePing(pingEncoded.slice(pingOff)))).toEqual(
      Array.from(payload),
    );

    const pongEncoded = encodePong(payload);
    const { type: pongType, offset: pongOff } = decodeFrameType(pongEncoded);
    expect(pongType).toBe(FrameType.Pong);
    expect(Array.from(decodePong(pongEncoded.slice(pongOff)))).toEqual(
      Array.from(payload),
    );
  });

  it("Health encode/decode roundtrip", () => {
    const problem = "overloaded";
    const encoded = encodeHealth(problem);
    const { type, offset } = decodeFrameType(encoded);
    expect(type).toBe(FrameType.Health);
    const decoded = decodeHealth(encoded.slice(offset));
    expect(decoded).toBe(problem);
  });

  it("Restarting encode/decode roundtrip", () => {
    const encoded = encodeRestarting(5000, 30000);
    const { type, offset } = decodeFrameType(encoded);
    expect(type).toBe(FrameType.Restarting);
    const decoded = decodeRestarting(encoded.slice(offset));
    expect(decoded).toEqual({ reconnectMs: 5000, tryForMs: 30000 });
  });

  it("EndpointGone encode/decode roundtrip", () => {
    const encoded = encodeEndpointGone(TEST_KEY);
    const { type, offset } = decodeFrameType(encoded);
    expect(type).toBe(FrameType.EndpointGone);
    expect(Array.from(encoded.slice(offset, offset + 32))).toEqual(
      Array.from(TEST_KEY),
    );
  });

  it("ClientAuth decode with known bytes", () => {
    // Build a valid ClientAuth body: [32B pubkey][0x40][64B signature]
    const pubkey = TEST_KEY;
    const signature = new Uint8Array(64);
    for (let i = 0; i < 64; i++) signature[i] = i;

    const body = new Uint8Array(97);
    body.set(pubkey, 0);
    body[32] = 0x40; // varint(64)
    body.set(signature, 33);

    const decoded = decodeClientAuth(body);
    expect(Array.from(decoded.publicKey)).toEqual(Array.from(pubkey));
    expect(Array.from(decoded.signature)).toEqual(Array.from(signature));
  });

  it("Datagram encode/decode roundtrip", () => {
    const data = new TextEncoder().encode("test payload");
    const encoded = encodeRelayToClientDatagram(TEST_KEY, 2, data);
    const { type, offset } = decodeFrameType(encoded);
    expect(type).toBe(FrameType.RelayToClientDatagram);
    // Use the client datagram decoder (same body format for src/dst)
    const body = encoded.slice(offset);
    expect(Array.from(body.slice(0, 32))).toEqual(Array.from(TEST_KEY));
    expect(body[32]).toBe(2);
    expect(new TextDecoder().decode(body.slice(33))).toBe("test payload");
  });

  it("DatagramBatch encode/decode roundtrip", () => {
    const data = new TextEncoder().encode("batch data");
    const encoded = encodeRelayToClientDatagramBatch(
      TEST_KEY,
      1,
      512,
      data,
    );
    const { type, offset } = decodeFrameType(encoded);
    expect(type).toBe(FrameType.RelayToClientDatagramBatch);
    const body = encoded.slice(offset);
    expect(Array.from(body.slice(0, 32))).toEqual(Array.from(TEST_KEY));
    expect(body[32]).toBe(1);
    const view = new DataView(body.buffer, body.byteOffset, body.byteLength);
    expect(view.getUint16(33, false)).toBe(512);
    expect(new TextDecoder().decode(body.slice(35))).toBe("batch data");
  });

  it("ClientDatagram decode", () => {
    // Manually build a ClientToRelayDatagram body
    const dstId = TEST_KEY;
    const ecn = 3;
    const payload = new TextEncoder().encode("hello");
    const body = new Uint8Array(32 + 1 + payload.length);
    body.set(dstId, 0);
    body[32] = ecn;
    body.set(payload, 33);

    const decoded = decodeClientDatagram(body);
    expect(Array.from(decoded.dstId)).toEqual(Array.from(dstId));
    expect(decoded.ecn).toBe(3);
    expect(new TextDecoder().decode(decoded.data)).toBe("hello");
  });

  it("ClientDatagramBatch decode", () => {
    const dstId = TEST_KEY;
    const ecn = 0;
    const segSize = 1024;
    const payload = new TextEncoder().encode("batch");
    const body = new Uint8Array(32 + 1 + 2 + payload.length);
    body.set(dstId, 0);
    body[32] = ecn;
    const dv = new DataView(body.buffer);
    dv.setUint16(33, segSize, false);
    body.set(payload, 35);

    const decoded = decodeClientDatagramBatch(body);
    expect(Array.from(decoded.dstId)).toEqual(Array.from(dstId));
    expect(decoded.ecn).toBe(0);
    expect(decoded.segmentSize).toBe(1024);
    expect(new TextDecoder().decode(decoded.data)).toBe("batch");
  });
});

describe("frame-codec validation", () => {
  it("rejects wrong-sized challenge", () => {
    expect(() => encodeServerChallenge(new Uint8Array(15))).toThrow(
      "Challenge must be 16 bytes",
    );
  });

  it("rejects wrong-sized ping payload", () => {
    expect(() => encodePing(new Uint8Array(7))).toThrow(
      "Ping payload must be 8 bytes",
    );
  });

  it("rejects wrong-sized endpoint ID", () => {
    expect(() => encodeEndpointGone(new Uint8Array(31))).toThrow(
      "endpointId must be 32 bytes",
    );
  });

  it("rejects empty frame", () => {
    expect(() => decodeFrameType(new Uint8Array(0))).toThrow("Empty frame");
  });

  it("rejects truncated ClientAuth body", () => {
    expect(() => decodeClientAuth(new Uint8Array(50))).toThrow("too short");
  });
});
