#!/usr/bin/env bun
// Fetch live relay metrics from one or all instances.
//
// Usage:
//   bun cli.ts [instance...] [--history [hours]]
//
// Examples:
//   bun cli.ts                 # live metrics, all instances
//   bun cli.ts ap1             # live metrics, one instance
//   bun cli.ts --history       # last 24h table, all instances
//   bun cli.ts ap1 --history 6 # last 6h table, ap1 only

import chalk from "chalk";
import minimist from "minimist";
import { type RegionConfig, loadConfig } from "./config.ts";
import { fmtMB, pct } from "./format.ts";
import { type NodeMetrics, fetchNodeMetrics } from "./node.ts";
import { type MetricRecord, fetchHistory } from "./store.ts";

const cfg = loadConfig();

function fmtPct(used: number, total: number): { str: string; n: number } {
  const n = pct(used, total);
  return { str: `${n.toFixed(0)}%`, n };
}

function colorPct(n: number, s: string): string {
  if (n > 85) return chalk.red(s);
  if (n > 70) return chalk.yellow(s);
  return chalk.green(s);
}

function printNode(m: NodeMetrics): void {
  const label = chalk.bold.cyan(`[${m.region.toUpperCase()}]`);

  if (m.error) {
    console.log(`${label}  ${chalk.red("unreachable")}  ${chalk.dim(m.error)}\n`);
    return;
  }

  const lines: string[] = [
    label,
    `  clients    ${chalk.bold(m.iroh.connectedClients)}  ${chalk.dim(`(${m.iroh.acceptedTotal} total)`)}`,
    `  bandwidth  ↑ ${fmtMB(m.iroh.bytesSent)}  ↓ ${fmtMB(m.iroh.bytesRecv)}`,
  ];

  if (m.os) {
    const mem = fmtPct(m.os.memUsedBytes, m.os.memTotalBytes);
    const disk = fmtPct(m.os.diskUsedBytes, m.os.diskTotalBytes);
    const loadColor = m.os.load1 > 2 ? chalk.red : m.os.load1 > 1 ? chalk.yellow : chalk.green;
    lines.push(
      `  cpu        ${colorPct(m.os.cpuPct, `${m.os.cpuPct.toFixed(0)}%`)}`,
      `  load       ${loadColor(`${m.os.load1.toFixed(2)} ${m.os.load5.toFixed(2)} ${m.os.load15.toFixed(2)}`)}`,
      `  memory     ${colorPct(mem.n, mem.str)}  ${chalk.dim(`(${fmtMB(m.os.memUsedBytes)} / ${fmtMB(m.os.memTotalBytes)})`)}`,
      `  disk       ${colorPct(disk.n, disk.str)}`
    );
  }

  lines.push(`  ${chalk.dim(m.fetchedAt.toISOString())}`);
  console.log(`${lines.join("\n")}\n`);
}

function printHistory(records: MetricRecord[], region: string): void {
  const label = chalk.bold.cyan(`[${region.toUpperCase()}]`);
  if (records.length === 0) {
    console.log(`${label}  ${chalk.dim("no history")}\n`);
    return;
  }
  console.log(`${label}  ${chalk.dim(`${records.length} entries`)}`);
  console.log(
    `  ${"timestamp".padEnd(26)} \
    ${"clients".padStart(7)} \
    ${"↑ sent".padStart(10)} \
    ${"↓ recv".padStart(10)} \
    ${"cpu".padStart(5)} \
    ${"ram".padStart(5)} \
    ${"disk".padStart(5)}`
  );
  console.log(`  ${"-".repeat(72)}`);
  for (const r of records) {
    const ts = new Date(r.ts).toISOString().replace("T", " ").slice(0, 19);
    const clients = String(r.iroh.connectedClients).padStart(7);
    const sent = fmtMB(r.iroh.bytesSent).padStart(10);
    const recv = fmtMB(r.iroh.bytesRecv).padStart(10);
    const cpu = r.os ? `${r.os.cpuPct.toFixed(0)}%`.padStart(5) : "  n/a";
    const ram = r.os
      ? `${pct(r.os.memUsedBytes, r.os.memTotalBytes).toFixed(0)}%`.padStart(5)
      : "  n/a";
    const disk = r.os
      ? `${pct(r.os.diskUsedBytes, r.os.diskTotalBytes).toFixed(0)}%`.padStart(5)
      : "  n/a";
    const err = r.error ? `  ${chalk.red("ERR")}` : "";
    console.log(`  ${chalk.dim(ts)} ${clients} ${sent} ${recv} ${cpu} ${ram} ${disk}${err}`);
  }
  console.log();
}

const args = minimist(process.argv.slice(2), { boolean: ["help", "h"] });

if (args.help || args.h) {
  console.log(`Usage: bun cli.ts [instance...] [--history [hours]]

Examples:
  bun cli.ts                 live metrics, all instances
  bun cli.ts ap1             live metrics, one instance
  bun cli.ts --history       last 24h table, all instances
  bun cli.ts ap1 --history 6 last 6h table, ap1 only`);
  process.exit(0);
}

const isHistory = "history" in args;
const historyHours = typeof args.history === "number" ? args.history : 24;
const regionArgs: string[] = args._;

const entries =
  regionArgs.length === 0
    ? Object.entries(cfg.regions)
    : regionArgs.map((a) => {
        if (!(a in cfg.regions)) {
          console.error(`Unknown region: ${a}`);
          process.exit(1);
        }
        return [a, cfg.regions[a]] as [string, RegionConfig];
      });

if (isHistory) {
  const histories = await Promise.all(
    entries.map(([, rcfg]) => fetchHistory(rcfg.sshHost, historyHours))
  );
  entries.forEach(([region], i) => printHistory(histories[i], region));
} else {
  const results = await Promise.all(
    entries.map(([region, rcfg]) => fetchNodeMetrics(region, rcfg))
  );
  results.forEach(printNode);
}
