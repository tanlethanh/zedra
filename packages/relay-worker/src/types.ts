export interface Env {
  RELAY_KV: KVNamespace;
}

export interface Room {
  code: string;
  secret: string;
  hostId: string;
  mobileId: string | null;
  state: "waiting" | "joined";
  createdAt: number;
}

export interface SendRequest {
  role: "host" | "mobile";
  messages: string[]; // base64-encoded
}

export interface RecvParams {
  role: "host" | "mobile";
  after: number; // sequence number
}

export interface SignalData {
  role: "host" | "mobile";
  data: unknown;
}

export interface ErrorResponse {
  error: string;
}
