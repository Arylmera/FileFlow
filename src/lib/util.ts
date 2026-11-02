// Small shared helpers: list parsing, display formatting, and the folder picker.
import { open as openDialog } from "@tauri-apps/plugin-dialog";

export const csvToList = (s: string) => s.split(",").map((x) => x.trim()).filter(Boolean);
export const listToCsv = (l: string[]) => l.join(", ");

export const fmtCount = (n: number) => n.toLocaleString();

/** "1.4 GB"-style size (decimal units, like Finder). */
export function fmtBytes(n: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let v = n;
  let u = 0;
  while (v >= 1000 && u < units.length - 1) {
    v /= 1000;
    u++;
  }
  return u === 0 ? `${n} B` : `${v.toFixed(1)} ${units[u]}`;
}

/** "4.2 s" / "3 min" duration. */
export function fmtDuration(ms: number): string {
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`;
  return `${Math.round(ms / 60_000)} min`;
}

/** "just now" / "5m ago" / "3h ago" / "2d ago" / a date. */
export function relTime(ts: string): string {
  const t = new Date(ts).getTime();
  if (Number.isNaN(t)) return "";
  const s = Math.max(0, (Date.now() - t) / 1000);
  if (s < 45) return "just now";
  if (s < 3600) return `${Math.round(s / 60)}m ago`;
  if (s < 86400) return `${Math.round(s / 3600)}h ago`;
  if (s < 604800) return `${Math.round(s / 86400)}d ago`;
  return new Date(ts).toLocaleDateString();
}

/** Native directory picker; resolves to null when cancelled or unavailable. */
export async function pickFolder(): Promise<string | null> {
  try {
    const res = await openDialog({ directory: true, multiple: false });
    return typeof res === "string" ? res : null;
  } catch {
    return null;
  }
}
