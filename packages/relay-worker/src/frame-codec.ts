// Frame codec for iroh relay protocol.
//
// All 13 frame types. QUIC VarInt prefix (1 byte for types 0–12).
// Wire-compatible with iroh-relay's Rust implementation.
//
// Reference: iroh/iroh-relay/src/protos/relay.rs

// --- Frame type constants ---

export const FrameType = {
  ServerChallenge: 0x00,
  ClientAuth: 0x01,
  ServerConfirmsAuth: 0x02,
  ServerDeniesAuth: 0x03,
  ClientToRelayDatagram: 0x04,
  ClientToRelayDatagramBatch: 0x05,
  RelayToClientDatagram: 0x06,
  RelayToClientDatagramBatch: 0x07,
  EndpointGone: 0x08,
  Ping: 0x09,
  Pong: 0x0a,
  Health: 0x0b,
  Restarting: 0x0c,
} as const;

export type FrameTypeValue = (typeof FrameType)[keyof typeof FrameType];

// --- Postcard VarInt encoding ---

/** Encode a u64 as a postcard varint (LEB128-like, unsigned). */
function encodePostcardVarint(value: number): Uint8Array {
  if (value < 128) {
    return new Uint8Array([value]);
  }
  const bytes: number[] = [];
  let v = value;
  while (v >= 128) {
    bytes.push((v & 0x7f) | 0x80);
    v >>>= 7;
  }
  bytes.push(v);
  return new Uint8Array(bytes);
}

/** Decode a postcard varint from buf at offset. Returns { value, bytesRead }. */
function decodePostcardVarint(
  buf: Uint8Array,
  offset: number,
): { value: number; bytesRead: number } {
  let value = 0;
  let shift = 0;
  let bytesRead = 0;
  while (offset + bytesRead < buf.length) {
    const byte = buf[offset + bytesRead];
    value |= (byte & 0x7f) << shift;
    bytesRead++;
    if ((byte & 0x80) === 0) {
      return { value, bytesRead };
    }
    shift += 7;
    if (shift > 35) {
      throw new Error("Varint too long");
    }
  }
  throw new Error("Unexpected end of varint");
}

// --- Encoding functions (server → client) ---

export function encodeServerChallenge(challenge: Uint8Array): Uint8Array {
  if (challenge.length !== 16) throw new Error("Challenge must be 16 bytes");
  const buf = new Uint8Array(1 + 16);
  buf[0] = FrameType.ServerChallenge;
  buf.set(challenge, 1);
  return buf;
}

export function encodeServerConfirmsAuth(): Uint8Array {
  return new Uint8Array([FrameType.ServerConfirmsAuth]);
}

export function encodeServerDeniesAuth(reason: string): Uint8Array {
  const reasonBytes = new TextEncoder().encode(reason);
  const lenBytes = encodePostcardVarint(reasonBytes.length);
  const buf = new Uint8Array(1 + lenBytes.length + reasonBytes.length);
  buf[0] = FrameType.ServerDeniesAuth;
  buf.set(lenBytes, 1);
  buf.set(reasonBytes, 1 + lenBytes.length);
  return buf;
}

export function encodeRelayToClientDatagram(
  srcId: Uint8Array,
  ecn: number,
  data: Uint8Array,
): Uint8Array {
  if (srcId.length !== 32) throw new Error("srcId must be 32 bytes");
  const buf = new Uint8Array(1 + 32 + 1 + data.length);
  buf[0] = FrameType.RelayToClientDatagram;
  buf.set(srcId, 1);
  buf[33] = ecn;
  buf.set(data, 34);
  return buf;
}

export function encodeRelayToClientDatagramBatch(
  srcId: Uint8Array,
  ecn: number,
  segmentSize: number,
  data: Uint8Array,
): Uint8Array {
  if (srcId.length !== 32) throw new Error("srcId must be 32 bytes");
  const buf = new Uint8Array(1 + 32 + 1 + 2 + data.length);
  buf[0] = FrameType.RelayToClientDatagramBatch;
  buf.set(srcId, 1);
  buf[33] = ecn;
  // segment size as 2-byte big-endian
  buf[34] = (segmentSize >> 8) & 0xff;
  buf[35] = segmentSize & 0xff;
  buf.set(data, 36);
  return buf;
}

export function encodeEndpointGone(endpointId: Uint8Array): Uint8Array {
  if (endpointId.length !== 32)
    throw new Error("endpointId must be 32 bytes");
  const buf = new Uint8Array(1 + 32);
  buf[0] = FrameType.EndpointGone;
  buf.set(endpointId, 1);
  return buf;
}

export function encodePing(payload: Uint8Array): Uint8Array {
  if (payload.length !== 8) throw new Error("Ping payload must be 8 bytes");
  const buf = new Uint8Array(1 + 8);
  buf[0] = FrameType.Ping;
  buf.set(payload, 1);
  return buf;
}

export function encodePong(payload: Uint8Array): Uint8Array {
  if (payload.length !== 8) throw new Error("Pong payload must be 8 bytes");
  const buf = new Uint8Array(1 + 8);
  buf[0] = FrameType.Pong;
  buf.set(payload, 1);
  return buf;
}

export function encodeHealth(problem: string): Uint8Array {
  const problemBytes = new TextEncoder().encode(problem);
  const buf = new Uint8Array(1 + problemBytes.length);
  buf[0] = FrameType.Health;
  buf.set(problemBytes, 1);
  return buf;
}

export function encodeRestarting(
  reconnectMs: number,
  tryForMs: number,
): Uint8Array {
  const buf = new Uint8Array(1 + 4 + 4);
  buf[0] = FrameType.Restarting;
  const view = new DataView(buf.buffer);
  view.setUint32(1, reconnectMs, false); // big-endian
  view.setUint32(5, tryForMs, false);
  return buf;
}

// --- Decoding functions (client → server) ---

export interface DecodedFrameType {
  type: FrameTypeValue;
  offset: number;
}

/** Decode the frame type byte and return the type + body offset. */
export function decodeFrameType(buf: Uint8Array): DecodedFrameType {
  if (buf.length < 1) throw new Error("Empty frame");
  const type = buf[0] as FrameTypeValue;
  return { type, offset: 1 };
}

export interface ClientAuthPayload {
  publicKey: Uint8Array;
  signature: Uint8Array;
}

/**
 * Decode ClientAuth body (postcard format).
 * Body layout: [32B pubkey][varint(64)=0x40][64B signature]
 */
export function decodeClientAuth(body: Uint8Array): ClientAuthPayload {
  if (body.length < 97) {
    throw new Error(
      `ClientAuth body too short: ${body.length} bytes, need 97`,
    );
  }
  const publicKey = body.slice(0, 32);
  // Decode varint for signature length
  const { value: sigLen, bytesRead } = decodePostcardVarint(body, 32);
  if (sigLen !== 64) {
    throw new Error(`Expected signature length 64, got ${sigLen}`);
  }
  const sigOffset = 32 + bytesRead;
  const signature = body.slice(sigOffset, sigOffset + 64);
  if (signature.length !== 64) {
    throw new Error(
      `Signature truncated: ${signature.length} bytes, need 64`,
    );
  }
  return { publicKey, signature };
}

export interface DatagramPayload {
  dstId: Uint8Array;
  ecn: number;
  data: Uint8Array;
}

/** Decode ClientToRelayDatagram body. */
export function decodeClientDatagram(body: Uint8Array): DatagramPayload {
  if (body.length < 33) throw new Error("Datagram body too short");
  return {
    dstId: body.slice(0, 32),
    ecn: body[32],
    data: body.slice(33),
  };
}

export interface DatagramBatchPayload {
  dstId: Uint8Array;
  ecn: number;
  segmentSize: number;
  data: Uint8Array;
}

/** Decode ClientToRelayDatagramBatch body. */
export function decodeClientDatagramBatch(
  body: Uint8Array,
): DatagramBatchPayload {
  if (body.length < 35) throw new Error("DatagramBatch body too short");
  const view = new DataView(body.buffer, body.byteOffset, body.byteLength);
  return {
    dstId: body.slice(0, 32),
    ecn: body[32],
    segmentSize: view.getUint16(33, false), // big-endian
    data: body.slice(35),
  };
}

/** Decode Ping body (8 bytes). */
export function decodePing(body: Uint8Array): Uint8Array {
  if (body.length < 8) throw new Error("Ping body too short");
  return body.slice(0, 8);
}

/** Decode Pong body (8 bytes). */
export function decodePong(body: Uint8Array): Uint8Array {
  if (body.length < 8) throw new Error("Pong body too short");
  return body.slice(0, 8);
}

/** Decode ServerChallenge body (16 bytes). */
export function decodeServerChallenge(body: Uint8Array): Uint8Array {
  if (body.length < 16) throw new Error("ServerChallenge body too short");
  return body.slice(0, 16);
}

/** Decode Health body (raw UTF-8, no length prefix). */
export function decodeHealth(body: Uint8Array): string {
  return new TextDecoder().decode(body);
}

/** Decode Restarting body (4B + 4B big-endian). */
export function decodeRestarting(
  body: Uint8Array,
): { reconnectMs: number; tryForMs: number } {
  if (body.length < 8) throw new Error("Restarting body too short");
  const view = new DataView(body.buffer, body.byteOffset, body.byteLength);
  return {
    reconnectMs: view.getUint32(0, false),
    tryForMs: view.getUint32(4, false),
  };
}

/** Decode ServerDeniesAuth body (postcard varint len + UTF-8). */
export function decodeServerDeniesAuth(body: Uint8Array): string {
  const { value: len, bytesRead } = decodePostcardVarint(body, 0);
  return new TextDecoder().decode(body.slice(bytesRead, bytesRead + len));
}
