import type { NodeMetrics } from "./node.ts";
import { sshPipe, sshRead } from "./ssh.ts";

const LOG_DIR = "/var/log/zedra-relay";
const LOG_FILE = `${LOG_DIR}/metrics.jsonl`;

export interface MetricRecord {
  ts: string;
  region: string;
  iroh: NodeMetrics["iroh"];
  os: NodeMetrics["os"];
  error?: string;
}

export async function persistMetrics(sshHost: string, m: NodeMetrics): Promise<void> {
  const record: MetricRecord = {
    ts: m.fetchedAt.toISOString(),
    region: m.region,
    iroh: m.iroh,
    os: m.os,
    error: m.error,
  };
  await sshPipe(
    sshHost,
    `mkdir -p ${LOG_DIR} && cat >> ${LOG_FILE}`,
    `${JSON.stringify(record)}\n`
  );
}

// Reads the current log file and returns entries within the last `hours` hours.
// With daily rotation + 5-min polls the file holds at most ~288 lines — always tiny.
export async function fetchHistory(sshHost: string, hours = 24): Promise<MetricRecord[]> {
  const text = await sshRead(sshHost, `cat ${LOG_FILE} 2>/dev/null || true`);
  if (!text) return [];

  const cutoff = new Date(Date.now() - hours * 3_600_000);
  return text
    .split("\n")
    .filter(Boolean)
    .flatMap((line) => {
      try {
        return [JSON.parse(line) as MetricRecord];
      } catch {
        return [];
      }
    })
    .filter((r) => new Date(r.ts) >= cutoff);
}
