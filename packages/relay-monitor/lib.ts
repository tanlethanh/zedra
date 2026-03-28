// Shared types for relay monitoring — imported by relay-check via workspace dep.

export interface IrohMetrics {
  connectedClients: number;
  acceptedTotal: number;
  bytesSent: number;
  bytesRecv: number;
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
