export interface Env {
  ZEDRA_RELAY_KV: KVNamespace;
  ZEDRA_RELAY_ENDPOINT: DurableObjectNamespace;
}

export interface ErrorResponse {
  error: string;
}

// --- Relay Endpoint Routing ---

export interface EndpointRegistration {
  do_name: string;
}
