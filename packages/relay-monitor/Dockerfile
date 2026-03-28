FROM oven/bun:1-alpine

WORKDIR /app
COPY packages/relay-monitor/monitor.ts packages/relay-monitor/lib.ts ./

CMD ["bun", "monitor.ts"]
