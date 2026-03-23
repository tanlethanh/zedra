FROM oven/bun:1-alpine

RUN apk add --no-cache openssh-client procps

WORKDIR /app
COPY package.json bun.lock ./
COPY packages/relay-monitor/package.json ./packages/relay-monitor/
RUN bun install --frozen-lockfile

COPY packages/relay-monitor/*.ts ./packages/relay-monitor/

WORKDIR /app/packages/relay-monitor
CMD ["bun", "monitor.ts"]
