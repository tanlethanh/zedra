// Docker sidecar: polls relay + host stats, persists metrics, Discord alerts, serves REST API.
import { appendFileSync, mkdirSync, readFileSync, statfsSync } from "node:fs";
import { type MetricRecord, type NodeMetrics, fmtMB, pct } from "./lib.ts";

// ── Format ───────────────────────────────────────────────────────────────────

function fmtPct(used: number, total: number): string {
  return `${pct(used, total).toFixed(0)}%`;
}

// ── Config ───────────────────────────────────────────────────────────────────

interface Thresholds {
  maxCpuPct: number;
  maxLoad1: number;
  maxMemPct: number;
  maxDiskPct: number;
}

interface Config {
  discordWebhook: string;
  instance: string;
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

function loadConfig(): Config {
  const webhook = process.env.DISCORD_WEBHOOK;
  if (!webhook) throw new Error("DISCORD_WEBHOOK env var is required");

  const instance = process.env.INSTANCE?.trim();
  if (!instance) {
    throw new Error("INSTANCE env var is required (e.g. ap1 — set by deploy/relay)");
  }

  return {
    discordWebhook: webhook,
    instance,
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

// ── Metrics collection ───────────────────────────────────────────────────────

const METRIC_RE: Record<string, RegExp> = {};
function parseMetric(text: string, name: string): number {
  METRIC_RE[name] ??= new RegExp(`^${name}(?:\\{[^}]*\\})?\\s+([\\d.e+\\-]+)`, "m");
  const m = text.match(METRIC_RE[name]);
  return m ? Number.parseFloat(m[1]) : 0;
}

function parseCpuStat(): number[] {
  const line = readFileSync("/host/proc/stat", "utf-8").split("\n")[0];
  return line.split(/\s+/).slice(1).map(Number);
}

function sampleCpu(): Promise<number> {
  const before = parseCpuStat();
  return new Promise((resolve) => {
    setTimeout(() => {
      const after = parseCpuStat();
      const diffs = after.map((v, i) => v - before[i]);
      const total = diffs.reduce((a, b) => a + b, 0);
      const idle = diffs[3] + (diffs[4] ?? 0); // idle + iowait
      resolve(total > 0 ? ((total - idle) / total) * 100 : 0);
    }, 1000);
  });
}

async function collectMetrics(instance: string): Promise<NodeMetrics> {
  const result: NodeMetrics = {
    instance,
    iroh: { connectedClients: 0, acceptedTotal: 0, bytesSent: 0, bytesRecv: 0 },
    os: null,
    fetchedAt: new Date().toISOString(),
  };

  try {
    const [prom, cpuPct] = await Promise.all([
      fetch("http://relay:9090/metrics").then((r) => r.text()),
      sampleCpu(),
    ]);

    const accepts = parseMetric(prom, "relayserver_accepts_total");
    const disconnects = parseMetric(prom, "relayserver_disconnects_total");
    result.iroh = {
      connectedClients: Math.max(0, accepts - disconnects),
      acceptedTotal: accepts,
      disconnectsTotal: disconnects,
      uniqueClientKeys: parseMetric(prom, "relayserver_unique_client_keys_total"),
      bytesSent: parseMetric(prom, "relayserver_bytes_sent_total"),
      bytesRecv: parseMetric(prom, "relayserver_bytes_recv_total"),
      bytesRxRatelimited: parseMetric(prom, "relayserver_bytes_rx_ratelimited_total_total"),
      sendPacketsSent: parseMetric(prom, "relayserver_send_packets_sent_total"),
      sendPacketsRecv: parseMetric(prom, "relayserver_send_packets_recv_total"),
      sendPacketsDropped: parseMetric(prom, "relayserver_send_packets_dropped_total"),
      otherPacketsSent: parseMetric(prom, "relayserver_other_packets_sent_total"),
      otherPacketsRecv: parseMetric(prom, "relayserver_other_packets_recv_total"),
      otherPacketsDropped: parseMetric(prom, "relayserver_other_packets_dropped_total"),
      gotPing: parseMetric(prom, "relayserver_got_ping_total"),
      sentPong: parseMetric(prom, "relayserver_sent_pong_total"),
      unknownFrames: parseMetric(prom, "relayserver_unknown_frames_total"),
      connsRxRatelimited: parseMetric(prom, "relayserver_conns_rx_ratelimited_total_total"),
    };

    const loadavg = readFileSync("/host/proc/loadavg", "utf-8");
    const [load1, load5, load15] = loadavg.trim().split(" ").map(Number);

    const meminfo = readFileSync("/host/proc/meminfo", "utf-8");
    const memTotal = Number.parseInt(meminfo.match(/MemTotal:\s+(\d+)/)?.[1] ?? "0") * 1024;
    const memAvailable = Number.parseInt(meminfo.match(/MemAvailable:\s+(\d+)/)?.[1] ?? "0") * 1024;

    const diskStat = statfsSync("/host/root");
    const diskTotal = diskStat.blocks * diskStat.bsize;

    result.os = {
      cpuPct,
      load1,
      load5,
      load15,
      memTotalBytes: memTotal,
      memUsedBytes: memTotal - memAvailable,
      diskTotalBytes: diskTotal,
      diskUsedBytes: diskTotal - diskStat.bfree * diskStat.bsize,
    };
  } catch (e) {
    result.error = String(e);
  }

  return result;
}

// ── Persistence ──────────────────────────────────────────────────────────────

const LOG_DIR = "/data/metrics";
const LOG_FILE = `${LOG_DIR}/metrics.jsonl`;

function persistMetrics(m: NodeMetrics): void {
  const record: MetricRecord = {
    ts: m.fetchedAt,
    instance: m.instance,
    iroh: m.iroh,
    os: m.os,
    error: m.error,
  };
  appendFileSync(LOG_FILE, `${JSON.stringify(record)}\n`);
}

function readHistory(hours: number): MetricRecord[] {
  let text: string;
  try {
    text = readFileSync(LOG_FILE, "utf-8");
  } catch {
    return [];
  }
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

// ── Discord ──────────────────────────────────────────────────────────────────

interface Alert {
  instance: string;
  field: string;
  value: string;
  threshold: string;
}

function checkAlerts(m: NodeMetrics, t: Thresholds): Alert[] {
  if (m.error) {
    return [
      {
        instance: m.instance,
        field: "reachability",
        value: "unreachable",
        threshold: "must be up",
      },
    ];
  }
  const alerts: Alert[] = [];
  const { os } = m;
  if (!os) return alerts;

  if (os.cpuPct > t.maxCpuPct)
    alerts.push({
      instance: m.instance,
      field: "cpu",
      value: `${os.cpuPct.toFixed(0)}%`,
      threshold: `>${t.maxCpuPct}%`,
    });
  if (os.load1 > t.maxLoad1)
    alerts.push({
      instance: m.instance,
      field: "load",
      value: os.load1.toFixed(2),
      threshold: `>${t.maxLoad1}`,
    });
  if (pct(os.memUsedBytes, os.memTotalBytes) > t.maxMemPct)
    alerts.push({
      instance: m.instance,
      field: "memory",
      value: fmtPct(os.memUsedBytes, os.memTotalBytes),
      threshold: `>${t.maxMemPct}%`,
    });
  if (pct(os.diskUsedBytes, os.diskTotalBytes) > t.maxDiskPct)
    alerts.push({
      instance: m.instance,
      field: "disk",
      value: fmtPct(os.diskUsedBytes, os.diskTotalBytes),
      threshold: `>${t.maxDiskPct}%`,
    });
  return alerts;
}

function nodeField(m: NodeMetrics): object {
  if (m.error) {
    return { name: m.instance.toUpperCase(), value: "❌ unreachable", inline: true };
  }
  const lines = [
    `👥 ${m.iroh.connectedClients} clients | ${m.iroh.acceptedTotal} total`,
    `↑ ${fmtMB(m.iroh.bytesSent)}  ↓ ${fmtMB(m.iroh.bytesRecv)}`,
  ];
  if (m.os) {
    lines.push(
      `CPU ${m.os.cpuPct.toFixed(0)}% | load ${m.os.load1.toFixed(2)} | RAM ${fmtPct(m.os.memUsedBytes, m.os.memTotalBytes)} | Disk ${fmtPct(m.os.diskUsedBytes, m.os.diskTotalBytes)}`
    );
  }
  return { name: m.instance.toUpperCase(), value: lines.join("\n"), inline: false };
}

async function postDiscord(webhook: string, body: object): Promise<void> {
  const res = await fetch(webhook, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`Discord ${res.status}: ${await res.text()}`);
}

async function sendSummary(webhook: string, m: NodeMetrics): Promise<void> {
  await postDiscord(webhook, {
    embeds: [
      {
        title: `[${m.instance.toUpperCase()}] Zedra Relay — Hourly Health`,
        color: 0x57f287,
        fields: [nodeField(m)],
        footer: { text: new Date().toUTCString() },
      },
    ],
  });
}

async function sendWarning(webhook: string, alerts: Alert[], m: NodeMetrics): Promise<void> {
  const description = alerts
    .map((a) => `**${a.instance.toUpperCase()}** ${a.field}: ${a.value} (threshold ${a.threshold})`)
    .join("\n");
  await postDiscord(webhook, {
    embeds: [
      {
        title: `⚠️ [${m.instance.toUpperCase()}] Zedra Relay — Alert`,
        color: 0xed4245,
        description,
        fields: [nodeField(m)],
        footer: { text: new Date().toUTCString() },
      },
    ],
  });
}

// ── REST API ─────────────────────────────────────────────────────────────────

let latestMetrics: NodeMetrics | null = null;

Bun.serve({
  port: 9091,
  async fetch(req) {
    const url = new URL(req.url);
    if (url.pathname === "/metrics") {
      if (!latestMetrics) return Response.json({ error: "no data yet" }, { status: 503 });
      return Response.json(latestMetrics);
    }
    if (url.pathname === "/metrics/live") {
      const m = await collectMetrics(cfg.instance);
      return Response.json(m);
    }
    if (url.pathname === "/history") {
      const hours = Number(url.searchParams.get("hours") ?? 24);
      return Response.json(readHistory(hours));
    }
    return new Response("Not found", { status: 404 });
  },
});

// ── Main loop ────────────────────────────────────────────────────────────────

const cfg = loadConfig();
mkdirSync(LOG_DIR, { recursive: true });

let lastSummaryAt = 0;
let lastAlertKey = "";

async function tick(): Promise<void> {
  const m = await collectMetrics(cfg.instance);
  latestMetrics = m;

  const alerts = checkAlerts(m, cfg.thresholds);
  const alertKey = alerts
    .map((a) => `${a.instance}:${a.field}`)
    .sort()
    .join(",");
  const now = Date.now();

  const discordWork = (async () => {
    if (alerts.length > 0 && alertKey !== lastAlertKey) {
      await sendWarning(cfg.discordWebhook, alerts, m);
      lastAlertKey = alertKey;
    } else if (alerts.length === 0) {
      lastAlertKey = "";
      if (now - lastSummaryAt >= cfg.summaryMs) {
        await sendSummary(cfg.discordWebhook, m);
        lastSummaryAt = now;
      }
    }
  })().catch((e) => console.error("Discord send failed:", e));

  try {
    persistMetrics(m);
  } catch (e) {
    console.error(`[${cfg.instance}] persist failed:`, e);
  }

  await discordWork;

  const status = `${m.instance}:${m.error ? "ERR" : `${m.iroh.connectedClients}c`}`;
  console.log(`[${new Date().toISOString()}] ${status}${alertKey ? ` ALERTS(${alertKey})` : ""}`);
}

console.log(
  `relay-monitor starting — poll every ${cfg.pollMs / 1000}s, summary every ${cfg.summaryMs / 60_000}min, API on :9091`
);
await tick();
lastSummaryAt = Date.now();
setInterval(tick, cfg.pollMs);
