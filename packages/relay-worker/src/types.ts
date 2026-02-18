export interface Env {
  ZEDRA_RELAY_KV: KVNamespace;
  ZEDRA_RELAY_ENDPOINT: DurableObjectNamespace;
}

export interface ErrorResponse {
  error: string;
}

// --- iroh Endpoint Discovery ---

export interface PublishRequest {
  endpoint_id: string;
  relay_url?: string;
  direct_addrs: string[];
}

export interface ResolveResponse {
  endpoint_id: string;
  relay_url?: string;
  direct_addrs: string[];
}

// --- Relay Endpoint Routing ---

export interface EndpointRegistration {
  do_name: string;
}
