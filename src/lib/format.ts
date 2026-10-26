const UNITS = ["B", "KB", "MB", "GB", "TB"];

/** 1536 -> "1.5 KB". Uses 1024 steps like File Explorer does. */
export function formatBytes(bytes: number, digits?: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const exp = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), UNITS.length - 1);
  const value = bytes / 1024 ** exp;
  const places = digits ?? (exp === 0 ? 0 : value >= 100 ? 0 : value >= 10 ? 1 : 2);
  return `${value.toFixed(places)} ${UNITS[exp]}`;
}

/** Splits "12.3 GB" into value and unit for typographic treatment. */
export function bytesParts(bytes: number): { value: string; unit: string } {
  const [value, unit] = formatBytes(bytes).split(" ");
  return { value, unit };
}

export function formatCount(n: number): string {
  return n.toLocaleString("en-US");
}

export function formatPercent(part: number, whole: number): string {
  if (whole <= 0) return "0%";
  const pct = (part / whole) * 100;
  if (pct > 0 && pct < 0.1) return "<0.1%";
  return `${pct < 10 ? pct.toFixed(1) : Math.round(pct)}%`;
}

const DATE = new Intl.DateTimeFormat("en-US", { month: "short", day: "numeric", year: "numeric" });

/** "2026-10-24" or epoch ms -> "Oct 24, 2026". */
export function formatDate(input: string | number | null | undefined): string {
  if (input === null || input === undefined || input === 0) return "—";
  const date =
    typeof input === "string" ? new Date(`${input}T00:00:00`) : new Date(input);
  return Number.isNaN(date.getTime()) ? "—" : DATE.format(date);
}

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms} ms`;
  const s = ms / 1000;
  return s < 60 ? `${s.toFixed(1)} s` : `${Math.floor(s / 60)} min ${Math.round(s % 60)} s`;
}

export function plural(n: number, one: string, many = `${one}s`): string {
  return `${formatCount(n)} ${n === 1 ? one : many}`;
}
