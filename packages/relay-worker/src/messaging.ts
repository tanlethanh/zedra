import type { Env } from "./types";
import { MAX_BATCH, MAX_MESSAGE_SIZE, MAX_RECV } from "./utils";

export async function sendMessages(
  env: Env,
  code: string,
  role: "host" | "mobile",
  messages: string[],
): Promise<{ sent: number; seq: number }> {
  if (messages.length > MAX_BATCH) {
    throw new Error(`Batch size exceeds maximum of ${MAX_BATCH}`);
  }

  for (const msg of messages) {
    if (msg.length > MAX_MESSAGE_SIZE) {
      throw new Error(
        `Message size exceeds maximum of ${MAX_MESSAGE_SIZE} bytes`,
      );
    }
  }

  // Get current sequence number
  const seqKey = `seq:${code}:${role}`;
  const currentSeqRaw = await env.RELAY_KV.get(seqKey);
  let seq = currentSeqRaw ? parseInt(currentSeqRaw, 10) : 0;

  // Store each message with incrementing sequence
  const puts: Promise<void>[] = [];
  for (const msg of messages) {
    seq++;
    const msgKey = `msg:${code}:${role}:${seq}`;
    puts.push(env.RELAY_KV.put(msgKey, msg, { expirationTtl: 60 }));
  }

  // Update sequence counter
  puts.push(env.RELAY_KV.put(seqKey, String(seq), { expirationTtl: 3600 }));

  await Promise.all(puts);

  return { sent: messages.length, seq };
}

export async function recvMessages(
  env: Env,
  code: string,
  peerRole: "host" | "mobile",
  afterSeq: number,
): Promise<{ messages: { seq: number; data: string }[]; lastSeq: number }> {
  // Read the peer's current sequence number
  const seqKey = `seq:${code}:${peerRole}`;
  const currentSeqRaw = await env.RELAY_KV.get(seqKey);
  const currentSeq = currentSeqRaw ? parseInt(currentSeqRaw, 10) : 0;

  if (afterSeq >= currentSeq) {
    return { messages: [], lastSeq: currentSeq };
  }

  // Fetch messages from afterSeq+1 to currentSeq, capped at MAX_RECV
  const startSeq = afterSeq + 1;
  const endSeq = Math.min(currentSeq, afterSeq + MAX_RECV);

  const fetches: Promise<{ seq: number; data: string | null }>[] = [];
  for (let s = startSeq; s <= endSeq; s++) {
    const msgKey = `msg:${code}:${peerRole}:${s}`;
    fetches.push(env.RELAY_KV.get(msgKey).then((data) => ({ seq: s, data })));
  }

  const results = await Promise.all(fetches);

  // Filter out expired/missing messages
  const messages = results
    .filter((r): r is { seq: number; data: string } => r.data !== null)
    .sort((a, b) => a.seq - b.seq);

  return { messages, lastSeq: endSeq };
}
