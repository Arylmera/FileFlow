// Typed IPC layer mirroring the Rust core config + commands.
import { invoke } from "@tauri-apps/api/core";

export type NameMode = "per_date" | "single";
export type CleanupPolicy = "ask" | "always" | "never";
export type EjectPolicy = "never" | "ask" | "always";
export type AfterImport = "archive" | "delete" | "leave";
export type AlbumMode = "library" | "fixed" | "template";

export interface CardRule {
  uuid: string;
  label: string;
  sources: string[];
  dest: string;
  layout: string;
  prompt_name: boolean;
  name_mode: NameMode;
  cleanup: CleanupPolicy;
  eject: EjectPolicy;
  extensions: string[];
}

export interface LightroomRule {
  watch_folder: string;
  album_mode: AlbumMode;
  photos_album: string;
  prompt_name: boolean;
  name_mode: NameMode;
  skip_duplicates: boolean;
  after_import: AfterImport;
  archive_folder: string;
  extensions: string[];
}

export interface AppSettings {
  autostart: boolean;
  log_level: string;
}

// Note: `card` (singular) — serde keeps the TOML `[[card]]` table name over IPC.
export interface Config {
  card: CardRule[];
  lightroom: LightroomRule | null;
  app: AppSettings;
}

export interface DateGroup {
  date: string;
  year: string;
  file_count: number;
}

export interface MountedCard {
  label: string;
  path: string;
  uuid: string | null;
  matched: boolean;
  rule_label: string | null;
}

export interface ActivityEntry {
  flow: string;
  message: string;
  ts: string;
}

export interface CardReady {
  uuid: string;
  label: string;
  volume_root: string;
  dates: DateGroup[];
}

export interface PhotosReady {
  dates: DateGroup[];
}

// Running outside the Tauri webview (e.g. a plain browser) has no IPC bridge;
// loaders fall back to defaults so the shell still renders.
const inTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const emptyConfig: Config = {
  card: [],
  lightroom: null,
  app: { autostart: true, log_level: "info" },
};

export function newCard(): CardRule {
  return {
    uuid: "",
    label: "",
    sources: ["DCIM/100MSDCF"],
    dest: "",
    layout: "{year}/{date} {name}",
    prompt_name: true,
    name_mode: "per_date",
    cleanup: "ask",
    eject: "never",
    extensions: ["arw", "jpg"],
  };
}

export function newLightroom(): LightroomRule {
  return {
    watch_folder: "",
    album_mode: "fixed",
    photos_album: "Lightroom",
    prompt_name: false,
    name_mode: "per_date",
    skip_duplicates: true,
    after_import: "archive",
    archive_folder: "",
    extensions: ["jpg", "jpeg", "tiff", "heif"],
  };
}

async function load<T>(cmd: string, args: Record<string, unknown>, fallback: T): Promise<T> {
  if (!inTauri) return fallback;
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    console.error(`${cmd} failed`, e);
    return fallback;
  }
}

export const getConfig = () => load<Config>("get_config", {}, emptyConfig);
export const saveConfig = (config: Config) => invoke<void>("save_config", { config });
export const listMountedCards = () => load<MountedCard[]>("list_mounted_cards", {}, []);
export const prepareIngest = (uuid: string) => invoke<DateGroup[]>("prepare_ingest", { uuid });
export const runIngestNow = (uuid: string, names: Record<string, string>) =>
  invoke<void>("run_ingest_now", { uuid, names });
export const startPhotosImport = () => invoke<void>("start_photos_import");
export const runPhotosImportNow = (names: Record<string, string>) =>
  invoke<void>("run_photos_import_now", { names });
export const getActivity = (limit: number) => load<ActivityEntry[]>("get_activity", { limit }, []);
export const destWritable = (path: string) => load<boolean>("dest_writable", { path }, false);
export const getPaused = () => load<boolean>("get_paused", {}, false);
export const setPaused = (paused: boolean) => invoke<void>("set_paused", { paused });
export const revealInFinder = (path: string) => invoke<void>("reveal_in_finder", { path });
export const logPath = () => invoke<string>("log_path");
