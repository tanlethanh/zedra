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

export async function sshRead(host: string, cmd: string): Promise<string> {
  const result = await $`ssh ${SSH_OPTS} ${host} ${cmd}`;
  return result.stdout.trim();
}

export async function sshPipe(host: string, cmd: string, input: string): Promise<void> {
  const proc = $`ssh ${SSH_OPTS} ${host} ${cmd}`;
  proc.stdin.write(input);
  proc.stdin.end();
  await proc;
}
