import type { RegionConfig } from "./config.ts";
import { sshRead } from "./ssh.ts";

export interface IrohMetrics {
  connectedClients: number;
  acceptedTotal: number;
  bytesSent: number;
  bytesRecv: number;
}

export interface OsHealth {
  cpuPct: number; // user+sys CPU, 1s sample via vmstat
  load1: number;
  load5: number;
  load15: number;
  memTotalBytes: number;
  memUsedBytes: number; // total - available (excludes buff/cache, matches htop)
  diskTotalBytes: number;
  diskUsedBytes: number;
}

export interface NodeMetrics {
  region: string;
  iroh: IrohMetrics;
  os: OsHealth | null; // null in HTTP mode
  fetchedAt: Date;
  error?: string;
}

const METRIC_RE: Record<string, RegExp> = {};
function parseMetric(text: string, name: string): number {
  METRIC_RE[name] ??= new RegExp(`^${name}(?:\\{[^}]*\\})?\\s+([\\d.e+\\-]+)`, "m");
  const m = text.match(METRIC_RE[name]);
  return m ? Number.parseFloat(m[1]) : 0;
}

async function fetchPrometheus(sshHost: string): Promise<string> {
  return sshRead(sshHost, "docker exec zedra-relay curl -sf http://localhost:9090/metrics");
}

const OS_CMD = [
  "awk '{print $1,$2,$3}' /proc/loadavg",
  "free -b | awk '/^Mem/{print $2,$2-$7}'",
  "df -PB1 / | awk 'NR==2{print $2,$3}'",
  "vmstat 1 2 | awk 'NR==4{print 100-$15}'",
].join(" && ");

function parseOsOutput(out: string): OsHealth {
  const [loadLine, memLine, diskLine, cpuLine] = out.split("\n");
  const [load1, load5, load15] = loadLine.trim().split(" ").map(Number);
  const [memTotalBytes, memUsedBytes] = memLine.trim().split(" ").map(Number);
  const [diskTotalBytes, diskUsedBytes] = diskLine.trim().split(" ").map(Number);
  return {
    cpuPct: Number.parseFloat(cpuLine.trim()),
    load1,
    load5,
    load15,
    memTotalBytes,
    memUsedBytes,
    diskTotalBytes,
    diskUsedBytes,
  };
}

async function fetchOs(sshHost: string): Promise<OsHealth> {
  return parseOsOutput(await sshRead(sshHost, OS_CMD));
}

export async function fetchNodeMetrics(region: string, cfg: RegionConfig): Promise<NodeMetrics> {
  const result: NodeMetrics = {
    region,
    iroh: { connectedClients: 0, acceptedTotal: 0, bytesSent: 0, bytesRecv: 0 },
    os: null,
    fetchedAt: new Date(),
  };

  try {
    const [prom, os] = await Promise.all([fetchPrometheus(cfg.sshHost), fetchOs(cfg.sshHost)]);
    result.iroh = {
      connectedClients: parseMetric(prom, "iroh_relay_connected_clients"),
      acceptedTotal: parseMetric(prom, "iroh_relay_accepted_connections_total"),
      bytesSent: parseMetric(prom, "iroh_relay_bytes_sent_total"),
      bytesRecv: parseMetric(prom, "iroh_relay_bytes_recv_total"),
    };
    result.os = os;
  } catch (e) {
    result.error = String(e);
  }

  return result;
}
