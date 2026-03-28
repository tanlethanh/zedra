// Shared types for relay monitoring — imported by relay-check via workspace dep.

export interface IrohMetrics {
  // Connections
  connectedClients: number; // derived: accepts - disconnects
  acceptedTotal: number; // relayserver_accepts_total
  disconnectsTotal: number; // relayserver_disconnects_total
  uniqueClientKeys: number; // relayserver_unique_client_keys_total

  // Bytes
  bytesSent: number; // relayserver_bytes_sent_total
  bytesRecv: number; // relayserver_bytes_recv_total
  bytesRxRatelimited: number; // relayserver_bytes_rx_ratelimited_total_total

  // Send packets
  sendPacketsSent: number; // relayserver_send_packets_sent_total
  sendPacketsRecv: number; // relayserver_send_packets_recv_total
  sendPacketsDropped: number; // relayserver_send_packets_dropped_total

  // Other packets
  otherPacketsSent: number; // relayserver_other_packets_sent_total
  otherPacketsRecv: number; // relayserver_other_packets_recv_total
  otherPacketsDropped: number; // relayserver_other_packets_dropped_total

  // Ping / pong
  gotPing: number; // relayserver_got_ping_total
  sentPong: number; // relayserver_sent_pong_total

  // Misc
  unknownFrames: number; // relayserver_unknown_frames_total
  connsRxRatelimited: number; // relayserver_conns_rx_ratelimited_total_total
}

export interface OsHealth {
  cpuPct: number;
  load1: number;
  load5: number;
  load15: number;
  memTotalBytes: number;
  memUsedBytes: number;
  diskTotalBytes: number;
  diskUsedBytes: number;
}

export interface NodeMetrics {
  instance: string;
  iroh: IrohMetrics;
  os: OsHealth | null;
  fetchedAt: string; // ISO 8601
  error?: string;
}

export interface MetricRecord {
  ts: string;
  instance: string;
  iroh: IrohMetrics;
  os: OsHealth | null;
  error?: string;
}

export function fmtMB(bytes: number): string {
  return `${(bytes / 1_048_576).toFixed(1)} MB`;
}

export function pct(used: number, total: number): number {
  return total > 0 ? (used / total) * 100 : 0;
}
