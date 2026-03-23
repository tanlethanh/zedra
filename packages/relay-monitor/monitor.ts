import { loadConfig } from "./config.ts";
import { type Alert, checkAlerts, sendSummary, sendWarning } from "./discord.ts";
import { fetchNodeMetrics } from "./node.ts";
import { persistMetrics } from "./store.ts";

const cfg = loadConfig();

let lastSummaryAt = 0;
let lastAlertKey = "";

async function tick(): Promise<void> {
  const all = await Promise.all(
    Object.entries(cfg.regions).map(([region, rcfg]) => fetchNodeMetrics(region, rcfg))
  );

  const alerts: Alert[] = all.flatMap((m) => checkAlerts(m, cfg.thresholds));
  const alertKey = alerts
    .map((a) => `${a.region}:${a.field}`)
    .sort()
    .join(",");
  const now = Date.now();

  try {
    if (alerts.length > 0 && alertKey !== lastAlertKey) {
      await sendWarning(cfg.discordWebhook, alerts, all);
      lastAlertKey = alertKey;
    } else if (alerts.length === 0) {
      lastAlertKey = "";
      if (now - lastSummaryAt >= cfg.summaryMs) {
        await sendSummary(cfg.discordWebhook, all);
        lastSummaryAt = now;
      }
    }
  } catch (e) {
    console.error("Discord send failed:", e);
  }

  await Promise.allSettled(
    all.flatMap((m) => {
      const rcfg = cfg.regions[m.region];
      if (!rcfg) return [];
      return [
        persistMetrics(rcfg.sshHost, m).catch((e) =>
          console.error(`[${m.region}] persist failed:`, e)
        ),
      ];
    })
  );

  const status = all
    .map((m) => `${m.region}:${m.error ? "ERR" : `${m.iroh.connectedClients}c`}`)
    .join(" ");
  console.log(`[${new Date().toISOString()}] ${status}${alertKey ? ` ALERTS(${alertKey})` : ""}`);
}

console.log(
  `relay-monitor starting — poll every ${cfg.pollMs / 1000}s, summary every ${cfg.summaryMs / 60_000}min`
);
await tick();
lastSummaryAt = Date.now();
setInterval(tick, cfg.pollMs);
