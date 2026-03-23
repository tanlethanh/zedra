import type { Thresholds } from "./config.ts";
import { fmtMB, fmtPct, pct } from "./format.ts";
import type { NodeMetrics } from "./node.ts";

export interface Alert {
  region: string;
  field: string;
  value: string;
  threshold: string;
}

export function checkAlerts(m: NodeMetrics, t: Thresholds): Alert[] {
  if (m.error) {
    return [
      { region: m.region, field: "reachability", value: "unreachable", threshold: "must be up" },
    ];
  }

  const alerts: Alert[] = [];
  const { os } = m;

  if (os) {
    if (os.cpuPct > t.maxCpuPct) {
      alerts.push({
        region: m.region,
        field: "cpu",
        value: `${os.cpuPct.toFixed(0)}%`,
        threshold: `>${t.maxCpuPct}%`,
      });
    }
    if (os.load1 > t.maxLoad1) {
      alerts.push({
        region: m.region,
        field: "load",
        value: os.load1.toFixed(2),
        threshold: `>${t.maxLoad1}`,
      });
    }
    const memPct = pct(os.memUsedBytes, os.memTotalBytes);
    if (memPct > t.maxMemPct) {
      alerts.push({
        region: m.region,
        field: "memory",
        value: fmtPct(os.memUsedBytes, os.memTotalBytes),
        threshold: `>${t.maxMemPct}%`,
      });
    }
    const diskPct = pct(os.diskUsedBytes, os.diskTotalBytes);
    if (diskPct > t.maxDiskPct) {
      alerts.push({
        region: m.region,
        field: "disk",
        value: fmtPct(os.diskUsedBytes, os.diskTotalBytes),
        threshold: `>${t.maxDiskPct}%`,
      });
    }
  }

  return alerts;
}

function nodeField(m: NodeMetrics): object {
  if (m.error) {
    return { name: m.region.toUpperCase(), value: "❌ unreachable", inline: true };
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

  return { name: m.region.toUpperCase(), value: lines.join("\n"), inline: false };
}

async function post(webhook: string, body: object): Promise<void> {
  const res = await fetch(webhook, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`Discord ${res.status}: ${await res.text()}`);
}

export async function sendSummary(webhook: string, all: NodeMetrics[]): Promise<void> {
  await post(webhook, {
    embeds: [
      {
        title: "Zedra Relay — Hourly Health",
        color: 0x57f287,
        fields: all.map(nodeField),
        footer: { text: new Date().toUTCString() },
      },
    ],
  });
}

export async function sendWarning(
  webhook: string,
  alerts: Alert[],
  all: NodeMetrics[]
): Promise<void> {
  const description = alerts
    .map((a) => `**${a.region.toUpperCase()}** ${a.field}: ${a.value} (threshold ${a.threshold})`)
    .join("\n");

  await post(webhook, {
    embeds: [
      {
        title: "⚠️ Zedra Relay — Alert",
        color: 0xed4245,
        description,
        fields: all.map(nodeField),
        footer: { text: new Date().toUTCString() },
      },
    ],
  });
}
