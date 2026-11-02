// Typed IPC layer mirroring the Rust core config + commands.
import { invoke } from "@tauri-apps/api/core";

export type NameMode = "per_date" | "single";
export type CleanupPolicy = "ask" | "always" | "never";
export type EjectPolicy = "never" | "ask" | "always";
export type AfterImport = "archive" | "delete" | "leave";
export type AlbumMode = "library" | "fixed" | "template";
export type FolderKind = "folder" | "photos";
export type RawJpeg = "independent" | "together" | "skip_jpeg";
export type NotifyLevel = "all" | "errors" | "off";

// An extension→destination override within a card rule. Blank dest/layout reuse the rule's.
export interface Route {
  extensions: string[];
  dest: string;
  layout: string;
}

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
  // Regex name filters: only copy files matching `include` (blank = all), and never
  // copy files matching `exclude` (blank = none). Both test the file name.
  include: string;
  exclude: string;
  routes: Route[];
  rename: string;
  // Second destination: every file is also copied + verified here; the drive is only
  // erased once both copies verified. Blank = no backup.
  backup_dest: string;
  raw_jpeg: RawJpeg;
}

export interface AppSettings {
  autostart: boolean;
  keep_running_on_close: boolean;
  start_minimized: boolean;
  show_dock_icon: boolean;
  show_tray_icon: boolean;
  log_level: string;
  notify: NotifyLevel;
  // Automatic runs only start between these local HH:MM times (both blank = any time).
  run_from: string;
  run_until: string;
  pause_on_battery: boolean;
}

// A watched-folder rule: move new files to a folder (`kind: "folder"`) or import
// them into Photos (`kind: "photos"`). Fields not used by a kind are ignored.
export interface FolderRule {
  label: string;
  watch: string;
  kind: FolderKind;
  extensions: string[];
  // Regex name filters (see CardRule): include = whitelist, exclude = blacklist.
  include: string;
  exclude: string;
  recursive: boolean; // also handle files in subfolders
  // Folder destination
  dest: string;
  layout: string;
  // Photos destination
  album_mode: AlbumMode;
  photos_album: string;
  skip_duplicates: boolean;
  after_import: AfterImport;
  archive_folder: string;
  // Shared naming (Photos "template" album mode)
  prompt_name: boolean;
  name_mode: NameMode;
}

// Note: `card`/`folder` (singular) — serde keeps the TOML table names over IPC.
export interface Config {
  card: CardRule[];
  folder: FolderRule[];
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
  ejectable: boolean;
}

export interface ActivityEntry {
  flow: string;
  message: string;
  ts: string;
}

// One completed run of one rule — the durable history behind the Flow map.
export interface RunRecord {
  ts: string; // RFC3339
  flow: string; // "drive" | "folder" | "photos"
  rule_key: string; // "card:{uuid}" | "folder:{watch}"
  label: string;
  source: string;
  dest: string;
  ok: number;
  skipped: number;
  failed: number;
  status: string; // "ok" | "partial" | "failed"
  detail: string;
  bytes: number;
  duration_ms: number;
  errors?: string[]; // per-file failures, "name: error"
}

// Totals for one month ("YYYY-MM") of run history.
export interface MonthStats {
  month: string;
  runs: number;
  failed_runs: number;
  drives: number;
  files: number;
  bytes: number;
  duration_ms: number;
}

export interface CardReady {
  uuid: string;
  label: string;
  volume_root: string;
  dates: DateGroup[];
}

export interface PhotosReady {
  index: number;
  dates: DateGroup[];
}

/** A drive imported cleanly and its rule says "Ask first" to erase and/or eject. */
export interface DrivePending {
  uuid: string;
  label: string;
  files: number; // what an erase would remove
  ask_erase: boolean;
  ask_eject: boolean;
}

export interface Progress {
  flow: string;
  label: string;
  done: number;
  total: number;
}

// Running outside the Tauri webview (e.g. a plain browser) has no IPC bridge;
// loaders fall back to defaults so the shell still renders.
const inTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const emptyConfig: Config = {
  card: [],
  folder: [],
  app: {
    autostart: true,
    keep_running_on_close: true,
    start_minimized: false,
    show_dock_icon: false,
    show_tray_icon: true,
    log_level: "info",
    notify: "all",
    run_from: "",
    run_until: "",
    pause_on_battery: false,
  },
};

// Shared defaults for the per-kind fields a rule doesn't use.
const folderDefaults = {
  include: "",
  exclude: "",
  recursive: false,
  album_mode: "fixed" as AlbumMode,
  photos_album: "Lightroom",
  skip_duplicates: true,
  after_import: "leave" as AfterImport,
  archive_folder: "",
  prompt_name: false,
  name_mode: "per_date" as NameMode,
};

export function newFolder(): FolderRule {
  return {
    label: "",
    watch: "",
    kind: "folder",
    extensions: [],
    dest: "",
    layout: "{year}/{date}",
    ...folderDefaults,
  };
}

export function newPhotosFolder(): FolderRule {
  return {
    label: "",
    watch: "",
    kind: "photos",
    extensions: ["jpg", "jpeg", "tiff", "heif"],
    dest: "",
    layout: "{year}/{date}",
    ...folderDefaults,
    after_import: "archive",
  };
}

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
    include: "",
    exclude: "",
    routes: [],
    rename: "",
    backup_dest: "",
    raw_jpeg: "independent",
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
export const ejectNow = (path: string) => invoke<void>("eject_now", { path });
export const finishDrive = (uuid: string, erase: boolean, eject: boolean) =>
  invoke<void>("finish_drive", { uuid, erase, eject });
export const runFolderNow = (index: number) => invoke<void>("run_folder_now", { index });
export const runPhotosImportNow = (index: number, names: Record<string, string>) =>
  invoke<void>("run_photos_import_now", { index, names });
export const getActivity = (limit: number) => load<ActivityEntry[]>("get_activity", { limit }, []);
export const getRuns = (limit: number) => load<RunRecord[]>("get_runs", { limit }, []);
export const destWritable = (path: string) => load<boolean>("dest_writable", { path }, false);
export const getPaused = () => load<boolean>("get_paused", {}, false);
export const setPaused = (paused: boolean) => invoke<void>("set_paused", { paused });
export const revealInFinder = (path: string) => invoke<void>("reveal_in_finder", { path });
export const logPath = () => invoke<string>("log_path");
export const getStats = () => load<MonthStats[]>("get_stats", {}, []);
export const undoLast = (ruleKey: string) => invoke<string>("undo_last", { ruleKey });
export const undoKeys = () => load<string[]>("undo_keys", {}, []);
export const folderWatchStatus = () => load<boolean[]>("folder_watch_status", {}, []);
export const exportConfig = (path: string) => invoke<void>("export_config", { path });
export const importConfig = (path: string) => invoke<Config>("import_config", { path });

// The OS the shell was built for ("macos" | "windows" | "linux"). Cached — it can't
// change while the app runs. Drives platform-only UI (Apple Photos, Dock wording).
let platformCache: string | null = null;
export async function getPlatform(): Promise<string> {
  platformCache ??= await load<string>("get_platform", {}, "macos");
  return platformCache;
}
