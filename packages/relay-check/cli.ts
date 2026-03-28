#!/usr/bin/env bun
// SSH to relay hosts, query the monitor REST API (port 9091), print metrics.
//
// Usage:
//   bun cli.ts --instance sg1,us1,eu1 [--cached] [--history [hours]]

import chalk from "chalk";
import minimist from "minimist";
import type { MetricRecord, NodeMetrics } from "relay-monitor/lib.ts";
import { fmtMB, pct } from "relay-monitor/lib.ts";
import { $ } from "zx";

$.verbose = false;

const SSH_OPTS = [
  "-o",
  "ConnectTimeout=5",
  "-o",
  "BatchMode=yes",
  "-o",
  "StrictHostKeyChecking=accept-new",
];

async function sshCurl(sshHost: string, path: string): Promise<string> {
  const result = await $`ssh ${SSH_OPTS} ${sshHost} curl -sf http://localhost:9091${path}`;
  return result.stdout.trim();
}

function colorPct(n: number, s: string): string {
  if (n > 85) return chalk.red(s);
  if (n > 70) return chalk.yellow(s);
  return chalk.green(s);
}

function printNode(m: NodeMetrics): void {
  const label = chalk.bold.cyan(`[${m.instance.toUpperCase()}]`);

  if (m.error) {
    console.log(`${label}  ${chalk.red("unreachable")}  ${chalk.dim(m.error)}\n`);
    return;
  }

  const { iroh } = m;
  const dropColor = (n: number, s: string) => n > 0 ? chalk.yellow(s) : chalk.dim(s);

  const lines: string[] = [
    label,
    `  clients    ${chalk.bold(iroh.connectedClients)} connected  ${chalk.dim(`${iroh.acceptedTotal} accepted  ${iroh.disconnectsTotal} disconnected  ${iroh.uniqueClientKeys} unique/day`)}`,
    `  bandwidth  ↑ ${fmtMB(iroh.bytesSent)}  ↓ ${fmtMB(iroh.bytesRecv)}`,
    `  packets    ↑ ${iroh.sendPacketsSent}  ↓ ${iroh.sendPacketsRecv}  ${dropColor(iroh.sendPacketsDropped, `${iroh.sendPacketsDropped} dropped`)}`,
    `  ping/pong  ${iroh.gotPing} / ${iroh.sentPong}`,
  ];

  if (iroh.otherPacketsSent > 0 || iroh.otherPacketsRecv > 0 || iroh.otherPacketsDropped > 0) {
    lines.push(`  other pkt  ↑ ${iroh.otherPacketsSent}  ↓ ${iroh.otherPacketsRecv}  ${dropColor(iroh.otherPacketsDropped, `${iroh.otherPacketsDropped} dropped`)}`);
  }
  if (iroh.connsRxRatelimited > 0 || iroh.bytesRxRatelimited > 0) {
    lines.push(`  ratelimit  ${chalk.yellow(`${iroh.connsRxRatelimited} conns  ${fmtMB(iroh.bytesRxRatelimited)} bytes`)}`);
  }
  if (iroh.unknownFrames > 0) {
    lines.push(`  unknown    ${chalk.yellow(`${iroh.unknownFrames} frames`)}`);
  }

  if (m.os) {
    const memN = pct(m.os.memUsedBytes, m.os.memTotalBytes);
    const diskN = pct(m.os.diskUsedBytes, m.os.diskTotalBytes);
    const loadColor = m.os.load1 > 2 ? chalk.red : m.os.load1 > 1 ? chalk.yellow : chalk.green;
    lines.push(
      `  cpu        ${colorPct(m.os.cpuPct, `${m.os.cpuPct.toFixed(0)}%`)}`,
      `  load       ${loadColor(`${m.os.load1.toFixed(2)} ${m.os.load5.toFixed(2)} ${m.os.load15.toFixed(2)}`)}`,
      `  memory     ${colorPct(memN, `${memN.toFixed(0)}%`)}  ${chalk.dim(`(${fmtMB(m.os.memUsedBytes)} / ${fmtMB(m.os.memTotalBytes)})`)}`,
      `  disk       ${colorPct(diskN, `${diskN.toFixed(0)}%`)}`
    );
  }

  lines.push(`  ${chalk.dim(m.fetchedAt)}`);
  console.log(`${lines.join("\n")}\n`);
}

function printHistory(records: MetricRecord[], instance: string): void {
  const label = chalk.bold.cyan(`[${instance.toUpperCase()}]`);
  if (records.length === 0) {
    console.log(`${label}  ${chalk.dim("no history")}\n`);
    return;
  }
  console.log(`${label}  ${chalk.dim(`${records.length} entries`)}`);
  console.log(
    `  ${"timestamp".padEnd(26)}` +
      `${"clients".padStart(9)}` +
      `${"↑ sent".padStart(12)}` +
      `${"↓ recv".padStart(12)}` +
      `${"cpu".padStart(6)}` +
      `${"ram".padStart(6)}` +
      `${"disk".padStart(6)}`
  );
  console.log(`  ${"-".repeat(72)}`);
  for (const r of records) {
    const ts = new Date(r.ts).toISOString().replace("T", " ").slice(0, 19);
    const clients = String(r.iroh.connectedClients).padStart(9);
    const sent = fmtMB(r.iroh.bytesSent).padStart(12);
    const recv = fmtMB(r.iroh.bytesRecv).padStart(12);
    const cpu = r.os ? `${r.os.cpuPct.toFixed(0)}%`.padStart(6) : "   n/a";
    const ram = r.os
      ? `${pct(r.os.memUsedBytes, r.os.memTotalBytes).toFixed(0)}%`.padStart(6)
      : "   n/a";
    const disk = r.os
      ? `${pct(r.os.diskUsedBytes, r.os.diskTotalBytes).toFixed(0)}%`.padStart(6)
      : "   n/a";
    const err = r.error ? `  ${chalk.red("ERR")}` : "";
    console.log(`  ${chalk.dim(ts)} ${clients} ${sent} ${recv} ${cpu} ${ram} ${disk}${err}`);
  }
  console.log();
}

// ── Main ─────────────────────────────────────────────────────────────────────

const args = minimist(process.argv.slice(2), { boolean: ["help", "h", "cached"] });

if (args.help || args.h) {
  console.log(`Usage: bun cli.ts --instance <instance[,instance,...]> [--cached] [--history [hours]]

Examples:
  bun cli.ts --instance sg1,us1,eu1         real-time metrics, all instances
  bun cli.ts --instance ap1                 real-time metrics, one instance
  bun cli.ts --instance ap1 --cached        cached snapshot
  bun cli.ts --instance sg1,us1 --history   last 24h table
  bun cli.ts --instance ap1 --history 6     last 6h table`);
  process.exit(0);
}

const instanceArg: string = args.instance ?? "";
if (!instanceArg) {
  console.error("Error: --instance is required (e.g. --instance sg1,us1,eu1)");
  process.exit(1);
}

const instances = instanceArg
  .split(",")
  .map((s) => s.trim())
  .filter(Boolean);

const isHistory = "history" in args;
const historyHours = typeof args.history === "number" ? args.history : 24;

if (isHistory) {
  const results = await Promise.all(
    instances.map(async (name) => {
      const host = `zedra-relay-${name}`;
      try {
        return {
          name,
          records: JSON.parse(
            await sshCurl(host, `/history?hours=${historyHours}`)
          ) as MetricRecord[],
        };
      } catch (e) {
        return { name, records: [] as MetricRecord[] };
      }
    })
  );
  for (const { name, records } of results) printHistory(records, name);
} else {
  const metricsPath = args.cached ? "/metrics" : "/metrics/live";
  const results = await Promise.all(
    instances.map(async (name) => {
      const host = `zedra-relay-${name}`;
      try {
        return JSON.parse(await sshCurl(host, metricsPath)) as NodeMetrics;
      } catch {
        return {
          instance: name,
          iroh: { connectedClients: 0, acceptedTotal: 0, bytesSent: 0, bytesRecv: 0 },
          os: null,
          fetchedAt: new Date().toISOString(),
          error: "monitor unreachable",
        } satisfies NodeMetrics;
      }
    })
  );
  for (const m of results) printNode(m);
}
