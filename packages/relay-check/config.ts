export interface InstanceConfig {
  sshHost: string;
}

export interface Config {
  instances: Record<string, InstanceConfig>;
}

export function loadConfig(): Config {
  const str = process.env.INSTANCES?.trim();
  if (!str) throw new Error("INSTANCES env var is required (e.g. ap1,us1,eu1)");

  const instances: Record<string, InstanceConfig> = {};
  for (const name of str
    .split(",")
    .map((r) => r.trim())
    .filter(Boolean)) {
    instances[name] = { sshHost: `zedra-relay-${name}` };
  }

  return { instances };
}
