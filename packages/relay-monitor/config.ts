export interface Thresholds {
  maxCpuPct: number;
  maxLoad1: number;
  maxMemPct: number;
  maxDiskPct: number;
}

export interface RegionConfig {
  sshHost: string;
}

export interface Config {
  discordWebhook: string;
  regions: Record<string, RegionConfig>;
  thresholds: Thresholds;
  pollMs: number;
  summaryMs: number;
}

function envNum(name: string, def: number): number {
  const v = process.env[name];
  if (!v) return def;
  const n = Number(v);
  if (Number.isNaN(n)) throw new Error(`${name} must be a number, got: ${v}`);
  return n;
}

export function loadConfig(): Config {
  const webhook = process.env.DISCORD_WEBHOOK;
  if (!webhook) throw new Error("DISCORD_WEBHOOK env var is required");

  const regionsStr = process.env.INSTANCES;
  if (!regionsStr) throw new Error("INSTANCES env var is required (e.g. ap1,us1,eu1)");

  const regions: Record<string, RegionConfig> = {};
  for (const region of regionsStr
    .split(",")
    .map((r) => r.trim())
    .filter(Boolean)) {
    regions[region] = { sshHost: `zedra-relay-${region}` };
  }

  return {
    discordWebhook: webhook,
    regions,
    thresholds: {
      maxCpuPct: envNum("MAX_CPU_PCT", 80),
      maxLoad1: envNum("MAX_LOAD1", 2.0),
      maxMemPct: envNum("MAX_MEM_PCT", 85),
      maxDiskPct: envNum("MAX_DISK_PCT", 80),
    },
    pollMs: envNum("POLL_MS", 5 * 60 * 1000),
    summaryMs: envNum("SUMMARY_MS", 60 * 60 * 1000),
  };
}
