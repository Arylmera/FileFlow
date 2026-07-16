// Small shared helpers: list parsing, display formatting, and the folder picker.
import { open as openDialog } from "@tauri-apps/plugin-dialog";

export const csvToList = (s: string) => s.split(",").map((x) => x.trim()).filter(Boolean);
export const listToCsv = (l: string[]) => l.join(", ");

export const fmtCount = (n: number) => n.toLocaleString();

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
