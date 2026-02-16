export interface Env {
  RELAY_KV: KVNamespace;
  WS_RELAY: DurableObjectNamespace;
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

// --- v2: Host Registry (Coordination Server) ---

export interface HostAddress {
  type: "lan" | "tailscale" | "public";
  addr: string;
}

export interface HostSession {
  id: string;
  name: string;
  workdir: string;
}

export interface HostRegistration {
  device_id: string;
  public_key: string; // base64url Curve25519
  hostname: string;
  addresses: HostAddress[];
  sessions: HostSession[];
  capabilities: string[];
  version: string;
  registered_at: number; // epoch ms
  last_seen: number; // epoch ms
}

export interface RegisterRequest {
  device_id: string;
  public_key: string;
  hostname: string;
  addresses: HostAddress[];
  sessions: HostSession[];
  capabilities: string[];
  version: string;
}

export interface HeartbeatRequest {
  addresses?: HostAddress[];
  sessions?: HostSession[];
}

export interface HostLookupResponse {
  online: boolean;
  last_seen: string; // ISO 8601
  hostname: string;
  addresses: HostAddress[];
  sessions: HostSession[];
  capabilities: string[];
  relay_endpoint: string;
}

export interface SignalCandidates {
  from_device_id: string;
  candidates: ConnectionCandidate[];
  session_id?: string;
}

export interface ConnectionCandidate {
  type: "direct-lan" | "direct-tailscale" | "direct-public" | "relay-ws" | "relay-http";
  addr?: string;
  url?: string;
  priority: number;
}
