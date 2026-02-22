import { describe, it, expect } from "vitest";
import {
  FrameType,
  encodeServerChallenge,
  encodeServerConfirmsAuth,
  encodeServerDeniesAuth,
  encodeRelayToClientDatagram,
  encodeRelayToClientDatagramBatch,
  encodePing,
  encodePong,
  encodeHealth,
  encodeRestarting,
  decodeFrameType,
  decodeClientAuth,
  decodeClientDatagram,
  decodeClientDatagramBatch,
  decodeServerChallenge,
  decodePing,
  decodePong,
  decodeHealth,
  decodeRestarting,
  decodeServerDeniesAuth,
} from "./frame-codec";

describe("frame-codec", () => {
  describe("server → client encoding", () => {
    it("encodeServerChallenge produces 17-byte frame", () => {
      const challenge = new Uint8Array(16).fill(0x42);
      const frame = encodeServerChallenge(challenge);
      expect(frame.length).toBe(17);
      expect(frame[0]).toBe(FrameType.ServerChallenge);
      expect(frame.slice(1)).toEqual(challenge);
    });

    it("encodeServerConfirmsAuth produces single-byte frame", () => {
      const frame = encodeServerConfirmsAuth();
      expect(frame.length).toBe(1);
      expect(frame[0]).toBe(FrameType.ServerConfirmsAuth);
    });

    it("encodeServerDeniesAuth encodes reason with varint length", () => {
      const frame = encodeServerDeniesAuth("bad key");
      expect(frame[0]).toBe(FrameType.ServerDeniesAuth);
      // "bad key" = 7 bytes, varint for 7 = single byte 0x07
      expect(frame[1]).toBe(7);
      const reason = new TextDecoder().decode(frame.slice(2));
      expect(reason).toBe("bad key");
    });

    it("encodeRelayToClientDatagram includes srcId + ECN + payload", () => {
      const srcId = new Uint8Array(32).fill(0xaa);
      const payload = new Uint8Array([1, 2, 3, 4]);
      const ecn = 0x01;
      const frame = encodeRelayToClientDatagram(srcId, ecn, payload);
      expect(frame.length).toBe(1 + 32 + 1 + 4);
      expect(frame[0]).toBe(FrameType.RelayToClientDatagram);
      expect(frame.slice(1, 33)).toEqual(srcId);
      expect(frame[33]).toBe(ecn);
      expect(frame.slice(34)).toEqual(payload);
    });

    it("encodeRelayToClientDatagramBatch includes segment size", () => {
      const srcId = new Uint8Array(32).fill(0xbb);
      const payload = new Uint8Array([5, 6, 7, 8]);
      const frame = encodeRelayToClientDatagramBatch(srcId, 0x00, 1200, payload);
      expect(frame.length).toBe(1 + 32 + 1 + 2 + 4);
      expect(frame[0]).toBe(FrameType.RelayToClientDatagramBatch);
      // segment size = 1200 in big-endian at offset 34-35
      const view = new DataView(frame.buffer, frame.byteOffset, frame.byteLength);
      expect(view.getUint16(34, false)).toBe(1200);
    });

    it("encodePing produces 9-byte frame", () => {
      const payload = new Uint8Array(8).fill(0x55);
      const frame = encodePing(payload);
      expect(frame.length).toBe(9);
      expect(frame[0]).toBe(FrameType.Ping);
      expect(frame.slice(1)).toEqual(payload);
    });

    it("encodePong produces 9-byte frame", () => {
      const payload = new Uint8Array(8).fill(0x66);
      const frame = encodePong(payload);
      expect(frame.length).toBe(9);
      expect(frame[0]).toBe(FrameType.Pong);
      expect(frame.slice(1)).toEqual(payload);
    });
  });

  describe("client → server decoding", () => {
    it("decodeClientAuth extracts pubkey and signature", () => {
      const pubkey = new Uint8Array(32).fill(0xbb);
      const sig = new Uint8Array(64).fill(0xcc);
      // Body layout: [32B pubkey][varint(64)=0x40][64B signature]
      const body = new Uint8Array(32 + 1 + 64);
      body.set(pubkey, 0);
      body[32] = 0x40; // varint encoding of 64
      body.set(sig, 33);

      const result = decodeClientAuth(body);
      expect(result.publicKey).toEqual(pubkey);
      expect(result.signature).toEqual(sig);
    });

    it("decodeClientDatagram extracts dstId + ECN + data", () => {
      const dstId = new Uint8Array(32).fill(0xdd);
      const data = new Uint8Array([10, 20, 30]);
      const ecn = 0x02;
      const body = new Uint8Array(32 + 1 + 3);
      body.set(dstId, 0);
      body[32] = ecn;
      body.set(data, 33);

      const result = decodeClientDatagram(body);
      expect(result.dstId).toEqual(dstId);
      expect(result.ecn).toBe(ecn);
      expect(result.data).toEqual(data);
    });

    it("decodeClientDatagramBatch extracts segment size", () => {
      const dstId = new Uint8Array(32).fill(0xee);
      const data = new Uint8Array([1, 2, 3]);
      const body = new Uint8Array(32 + 1 + 2 + 3);
      body.set(dstId, 0);
      body[32] = 0x01; // ECN
      const view = new DataView(body.buffer);
      view.setUint16(33, 512, false); // segment size big-endian
      body.set(data, 35);

      const result = decodeClientDatagramBatch(body);
      expect(result.dstId).toEqual(dstId);
      expect(result.ecn).toBe(0x01);
      expect(result.segmentSize).toBe(512);
      expect(result.data).toEqual(data);
    });
  });

  describe("roundtrip: encode → decode", () => {
    it("Ping roundtrip", () => {
      const original = new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]);
      const frame = encodePing(original);
      const { type: frameType, offset } = decodeFrameType(frame);
      expect(frameType).toBe(FrameType.Ping);
      const decoded = decodePing(frame.slice(offset));
      expect(decoded).toEqual(original);
    });

    it("Pong roundtrip", () => {
      const original = new Uint8Array([8, 7, 6, 5, 4, 3, 2, 1]);
      const frame = encodePong(original);
      const { type: frameType, offset } = decodeFrameType(frame);
      expect(frameType).toBe(FrameType.Pong);
      const decoded = decodePong(frame.slice(offset));
      expect(decoded).toEqual(original);
    });

    it("ServerChallenge roundtrip", () => {
      const original = new Uint8Array(16);
      for (let i = 0; i < 16; i++) original[i] = i;
      const frame = encodeServerChallenge(original);
      const { type: frameType, offset } = decodeFrameType(frame);
      expect(frameType).toBe(FrameType.ServerChallenge);
      const decoded = decodeServerChallenge(frame.slice(offset));
      expect(decoded).toEqual(original);
    });

    it("ServerDeniesAuth roundtrip", () => {
      const reason = "invalid signature provided";
      const frame = encodeServerDeniesAuth(reason);
      const { type: frameType, offset } = decodeFrameType(frame);
      expect(frameType).toBe(FrameType.ServerDeniesAuth);
      const decoded = decodeServerDeniesAuth(frame.slice(offset));
      expect(decoded).toBe(reason);
    });

    it("Health roundtrip", () => {
      const problem = "memory pressure detected";
      const frame = encodeHealth(problem);
      const { type: frameType, offset } = decodeFrameType(frame);
      expect(frameType).toBe(FrameType.Health);
      const decoded = decodeHealth(frame.slice(offset));
      expect(decoded).toBe(problem);
    });

    it("Restarting roundtrip", () => {
      const frame = encodeRestarting(5000, 30000);
      const { type: frameType, offset } = decodeFrameType(frame);
      expect(frameType).toBe(FrameType.Restarting);
      const decoded = decodeRestarting(frame.slice(offset));
      expect(decoded.reconnectMs).toBe(5000);
      expect(decoded.tryForMs).toBe(30000);
    });
  });

  describe("edge cases", () => {
    it("ServerChallenge rejects wrong-size input", () => {
      expect(() => encodeServerChallenge(new Uint8Array(15))).toThrow();
      expect(() => encodeServerChallenge(new Uint8Array(17))).toThrow();
    });

    it("Ping rejects wrong-size payload", () => {
      expect(() => encodePing(new Uint8Array(7))).toThrow();
      expect(() => encodePing(new Uint8Array(9))).toThrow();
    });

    it("Pong rejects wrong-size payload", () => {
      expect(() => encodePong(new Uint8Array(7))).toThrow();
    });

    it("decodeFrameType rejects empty buffer", () => {
      expect(() => decodeFrameType(new Uint8Array(0))).toThrow();
    });

    it("decodeClientAuth rejects truncated body", () => {
      expect(() => decodeClientAuth(new Uint8Array(50))).toThrow();
    });

    it("encodeRelayToClientDatagram rejects wrong-size srcId", () => {
      expect(() =>
        encodeRelayToClientDatagram(
          new Uint8Array(31),
          0,
          new Uint8Array(10),
        ),
      ).toThrow();
    });

    it("ServerDeniesAuth with long reason string", () => {
      const longReason = "x".repeat(300);
      const frame = encodeServerDeniesAuth(longReason);
      const { offset } = decodeFrameType(frame);
      const decoded = decodeServerDeniesAuth(frame.slice(offset));
      expect(decoded).toBe(longReason);
    });
  });
});
