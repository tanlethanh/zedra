export function fmtMB(bytes: number): string {
  return `${(bytes / 1_048_576).toFixed(1)} MB`;
}

export function pct(used: number, total: number): number {
  return total > 0 ? (used / total) * 100 : 0;
}

export function fmtPct(used: number, total: number): string {
  return `${pct(used, total).toFixed(0)}%`;
}
