import { useEffect, useState, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
import * as api from "./api";
import type {
  ActivityEntry,
  AfterImport,
  AlbumMode,
  CardReady,
  CardRule,
  CleanupPolicy,
  Config,
  DateGroup,
  EjectPolicy,
  FolderKind,
  FolderRule,
  MountedCard,
  NameMode,
  PhotosReady,
  Progress,
  Route,
} from "./api";
import "./App.css";

type Tab = "flow" | "status" | "cards" | "folders" | "activity" | "settings";

// A request to name an import, from either flow — drives the shared naming modal.
type NamingReq =
  | { kind: "card"; uuid: string; label: string; dates: DateGroup[] }
  | { kind: "photos"; index: number; label: string; dates: DateGroup[] };

const TABS: Tab[] = ["flow", "status", "cards", "folders", "activity", "settings"];
const TAB_LABELS: Record<Tab, string> = {
  flow: "Flow",
  status: "Devices",
  cards: "External Drive",
  folders: "Folders",
  activity: "Activity",
  settings: "Settings",
};

const csvToList = (s: string) => s.split(",").map((x) => x.trim()).filter(Boolean);
const listToCsv = (l: string[]) => l.join(", ");

async function pickFolder(): Promise<string | null> {
  try {
    const res = await openDialog({ directory: true, multiple: false });
    return typeof res === "string" ? res : null;
  } catch {
    return null;
  }
}

/** A worked example of the folder a layout template produces. */
function layoutExample(template: string): string {
  const folder = template
    .replace(/\{year\}/g, "2026")
    .replace(/\{date\}/g, "2026-06-20")
    .replace(/\{name\}/g, "Holiday")
    .split("/")
    .map((s) => s.trim())
    .filter(Boolean)
    .join("/");
  return `${folder}/DSC0001.ARW`;
}

/** A worked example of where a folder-move rule lands a file. */
function folderExample(template: string): string {
  const folder = template
    .replace(/\{year\}/g, "2026")
    .replace(/\{date\}/g, "2026-06-20")
    .replace(/\{name\}/g, "")
    .split("/")
    .map((s) => s.trim())
    .filter(Boolean)
    .join("/");
  return folder ? `${folder}/file.jpg` : "file.jpg";
}

/** A worked example of the filename a rename template produces (extension always kept). */
function filenameExample(template: string): string {
  if (!template.trim()) return "DSC0001.ARW (unchanged)";
  const stem =
    template
      .replace(/\{year\}/g, "2026")
      .replace(/\{date\}/g, "2026-06-20")
      .replace(/\{name\}/g, "Holiday")
      .replace(/\{seq\}/g, "0001")
      .replace(/\//g, "-")
      .trim() || "0001";
  return `${stem}.ARW`;
}

/** A worked example of the album name a date template produces. */
function albumExample(template: string, name = ""): string {
  const rendered = template
    .replace(/\{year\}/g, "2026")
    .replace(/\{date\}/g, "2026-06-20")
    .replace(/\{name\}/g, name)
    .split("/")
    .map((s) => s.trim())
    .filter(Boolean)
    .join("/");
  return rendered || "Imported";
}

/** Labelled field with an example placeholder and a one-line "how to fill it" hint. */
function Field({
  label,
  help,
  badge,
  children,
}: {
  label: string;
  help?: string;
  badge?: ReactNode;
  children: ReactNode;
}) {
  return (
    <label className="field">
      <span className="lbl">
        {label}
        {badge != null && <> {badge}</>}
      </span>
      {children}
      {help != null && <span className="help">{help}</span>}
    </label>
  );
}

function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="group">
      <div className="group-title">{title}</div>
      {children}
    </div>
  );
}

/**
 * Comma-separated list editor. Keeps the raw typed text in local state so a trailing
 * comma/space survives — normalizing on every keystroke would strip the separator and
 * make it impossible to start a new entry.
 */
function CsvField({
  label,
  help,
  value,
  onChange,
  placeholder,
}: {
  label: string;
  help?: string;
  value: string[];
  onChange: (v: string[]) => void;
  placeholder?: string;
}) {
  const [text, setText] = useState(() => listToCsv(value));
  // Re-seed when the external value diverges from what's typed — e.g. a list row is
  // removed and React reuses this instance under an index key. Compared by content so an
  // in-progress trailing comma/space (which parses to the same list) isn't clobbered.
  const joined = value.join(",");
  useEffect(() => {
    if (csvToList(text).join(",") !== joined) setText(listToCsv(value));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [joined]);
  return (
    <Field label={label} help={help}>
      <input
        placeholder={placeholder}
        value={text}
        onChange={(e) => {
          setText(e.target.value);
          onChange(csvToList(e.target.value));
        }}
      />
    </Field>
  );
}

export default function App() {
  const [tab, setTab] = useState<Tab>("flow");
  const [config, setConfig] = useState<Config>(api.emptyConfig);
  const [dirty, setDirty] = useState(false);
  const [activity, setActivity] = useState<ActivityEntry[]>([]);
  const [naming, setNaming] = useState<NamingReq | null>(null);
  const [progress, setProgress] = useState<Progress | null>(null);

  useEffect(() => {
    api.getConfig().then(setConfig);
    api.getActivity(100).then(setActivity);

    const unlisten = [
      // Keep the bar while files remain; the final (total, total) event clears it.
      listen<Progress>("progress", (e) =>
        setProgress(e.payload.done >= e.payload.total ? null : e.payload),
      ),
      listen<ActivityEntry>("activity", (e) => setActivity((a) => [e.payload, ...a].slice(0, 200))),
      listen<CardReady>("card-ready", (e) =>
        setNaming({ kind: "card", uuid: e.payload.uuid, label: e.payload.label, dates: e.payload.dates }),
      ),
      listen<PhotosReady>("photos-ready", (e) =>
        setNaming({ kind: "photos", index: e.payload.index, label: "Import to Photos", dates: e.payload.dates }),
      ),
    ];
    return () => {
      unlisten.forEach((u) => u.then((f) => f()));
    };
  }, []);

  async function save() {
    await api.saveConfig(config);
    setDirty(false);
  }

  const patchConfig = (patch: Partial<Config>) => {
    setConfig((c) => ({ ...c, ...patch }));
    setDirty(true);
  };

  return (
    <div className="app">
      <header className="topbar">
        <strong>FileFlow</strong>
        <nav>
          {TABS.map((t) => (
            <button key={t} className={tab === t ? "tab active" : "tab"} onClick={() => setTab(t)}>
              {TAB_LABELS[t]}
            </button>
          ))}
        </nav>
        <button className="save" disabled={!dirty} onClick={save}>
          {dirty ? "Save changes" : "✓ Saved"}
        </button>
      </header>

      {progress && (
        <div className="progress-strip">
          <span>
            {progress.label}: {progress.done}/{progress.total}
          </span>
          <progress value={progress.done} max={progress.total} />
        </div>
      )}

      <main>
        {tab === "flow" && (
          <FlowView config={config} onNeedNames={setNaming} onNavigate={setTab} />
        )}
        {tab === "status" && (
          <StatusView
            config={config}
            onNeedNames={setNaming}
            onImported={() => api.getActivity(100).then(setActivity)}
          />
        )}
        {tab === "cards" && <CardsView config={config} patch={patchConfig} />}
        {tab === "folders" && <FoldersView config={config} patch={patchConfig} />}
        {tab === "activity" && <ActivityView activity={activity} />}
        {tab === "settings" && <SettingsView config={config} patch={patchConfig} />}
      </main>

      {naming && (
        <NamingForm
          req={naming}
          mode={
            naming.kind === "card"
              ? config.card.find((c) => c.uuid.toLowerCase() === naming.uuid.toLowerCase())
                  ?.name_mode ?? "per_date"
              : config.folder[naming.index]?.name_mode ?? "per_date"
          }
          onClose={() => setNaming(null)}
        />
      )}
    </div>
  );
}

// --- Flow map --------------------------------------------------------------

type LaneKind = "drive" | "photos" | "folder";

interface Lane {
  key: string; // matches the Rust rule_key, joins a rule to its runs
  kind: LaneKind;
  label: string;
  source: string;
  filter: string;
  action: string;
  dest: string;
  detail: string; // layout token, album name, or "Library"
  revealPath: string; // a real path to open in Finder (empty = no Reveal)
  run: api.RunRecord | null; // most-recent run
  routed: number; // cumulative files routed
  cardUuid?: string;
  folderIndex?: number;
  promptName: boolean;
}

const fmtCount = (n: number) => n.toLocaleString();

/** "just now" / "5m ago" / "3h ago" / "2d ago" / a date. */
function relTime(ts: string): string {
  const t = new Date(ts).getTime();
  if (Number.isNaN(t)) return "";
  const s = Math.max(0, (Date.now() - t) / 1000);
  if (s < 45) return "just now";
  if (s < 3600) return `${Math.round(s / 60)}m ago`;
  if (s < 86400) return `${Math.round(s / 3600)}h ago`;
  if (s < 604800) return `${Math.round(s / 86400)}d ago`;
  return new Date(ts).toLocaleDateString();
}

const laneFilter = (exts: string[]) => (exts.length ? exts.join(", ") : "all types");

/** Project the config topology + run history into one lane per automation. */
function buildLanes(config: Config, runs: api.RunRecord[]): Lane[] {
  const byKey = new Map<string, api.RunRecord[]>();
  for (const r of runs) {
    const list = byKey.get(r.rule_key) ?? [];
    list.push(r); // runs arrive most-recent-first
    byKey.set(r.rule_key, list);
  }
  const attach = (key: string) => {
    const list = byKey.get(key) ?? [];
    return { run: list[0] ?? null, routed: list.reduce((n, r) => n + r.ok, 0) };
  };

  const lanes: Lane[] = [];

  for (const c of config.card) {
    const key = `card:${c.uuid}`;
    const action = ["copy", "verify"];
    if (c.cleanup !== "never") action.push("wipe");
    lanes.push({
      key,
      kind: "drive",
      label: c.label || "Untitled drive",
      source: c.sources.join(", ") || "drive",
      filter: laneFilter(c.extensions),
      action: action.join(" · "),
      dest: c.dest || "…",
      detail: c.layout,
      revealPath: c.dest,
      cardUuid: c.uuid,
      promptName: c.prompt_name,
      ...attach(key),
    });
  }

  config.folder.forEach((f, i) => {
    const key = `folder:${f.watch}`;
    if (f.kind === "photos") {
      const after =
        f.after_import === "archive" ? " · archive" : f.after_import === "delete" ? " · delete" : "";
      lanes.push({
        key,
        kind: "photos",
        label: f.label || "Untitled import",
        source: f.watch || "…",
        filter: laneFilter(f.extensions),
        action: `import${after}`,
        dest: "Apple Photos",
        detail: f.album_mode === "library" ? "Library" : f.photos_album,
        revealPath: f.watch,
        folderIndex: i,
        promptName: f.prompt_name,
        ...attach(key),
      });
    } else {
      lanes.push({
        key,
        kind: "folder",
        label: f.label || "Untitled folder",
        source: f.watch || "…",
        filter: laneFilter(f.extensions),
        action: "move",
        dest: f.dest || "…",
        detail: f.layout || "flat",
        revealPath: f.dest,
        folderIndex: i,
        promptName: f.prompt_name,
        ...attach(key),
      });
    }
  });

  return lanes;
}

function FlowView({
  config,
  onNeedNames,
  onNavigate,
}: {
  config: Config;
  onNeedNames: (r: NamingReq) => void;
  onNavigate: (t: Tab) => void;
}) {
  const [runs, setRuns] = useState<api.RunRecord[]>([]);
  const [mode, setMode] = useState<"map" | "history">("map");

  useEffect(() => {
    const refresh = () => api.getRuns(500).then(setRuns);
    refresh();
    const subs = [listen("run", refresh), listen("activity", refresh)];
    return () => subs.forEach((u) => u.then((f) => f()));
  }, []);

  const lanes = buildLanes(config, runs);
  const empty = config.card.length === 0 && config.folder.length === 0;

  async function runLane(l: Lane) {
    try {
      if (l.kind === "drive" && l.cardUuid) {
        if (l.promptName) {
          const dates = await api.prepareIngest(l.cardUuid);
          onNeedNames({ kind: "card", uuid: l.cardUuid, label: l.label, dates });
        } else {
          await api.runIngestNow(l.cardUuid, {});
        }
      } else if (l.folderIndex != null) {
        // A Photos rule that prompts for a name surfaces the form via the photos-ready event.
        await api.runFolderNow(l.folderIndex);
      }
    } catch (e) {
      alert(String(e));
    }
  }

  return (
    <section>
      <header className="view-head">
        <div>
          <h2>Flow</h2>
          <p className="subtitle">
            Every automation, from where files come from to where they land. Health and counts come
            from each rule's last run.
          </p>
        </div>
        <div className="seg" role="tablist">
          <button className={mode === "map" ? "on" : ""} onClick={() => setMode("map")}>
            Map
          </button>
          <button className={mode === "history" ? "on" : ""} onClick={() => setMode("history")}>
            History
          </button>
        </div>
      </header>

      {empty ? (
        <div className="empty">
          <p>No automations yet.</p>
          <p className="hint">Add a drive rule or a folder rule and it shows up here as a flow.</p>
          <div className="row" style={{ marginTop: "var(--s3)" }}>
            <button onClick={() => onNavigate("cards")}>+ Add drive</button>
            <button onClick={() => onNavigate("folders")}>+ Add folder</button>
          </div>
        </div>
      ) : mode === "map" ? (
        <FlowMap lanes={lanes} onRun={runLane} />
      ) : (
        <RunHistory runs={runs} />
      )}
    </section>
  );
}

function FlowMap({ lanes, onRun }: { lanes: Lane[]; onRun: (l: Lane) => void }) {
  const filesRouted = lanes.reduce((n, l) => n + l.routed, 0);
  const attention = lanes.filter((l) => l.run?.status === "failed").length;

  return (
    <>
      <div className="flow-summary">
        <div className="stat">
          <span className="n">{lanes.length}</span>
          <span className="l">Automations</span>
        </div>
        <div className="stat">
          <span className="n">{fmtCount(filesRouted)}</span>
          <span className="l">Files routed</span>
        </div>
        <div className="stat">
          <span className={attention ? "n warnN" : "n"}>{attention}</span>
          <span className="l">Needs attention</span>
        </div>
      </div>
      <div className="lanes">
        {lanes.map((l) => (
          <LaneRow key={l.key} lane={l} onRun={onRun} />
        ))}
      </div>
    </>
  );
}

function LaneRow({ lane, onRun }: { lane: Lane; onRun: (l: Lane) => void }) {
  const health = lane.run?.status ?? "idle";
  let healthText: string;
  if (!lane.run) {
    healthText = lane.kind === "drive" ? "Connect the drive to run" : "Watching · nothing routed yet";
  } else if (health === "failed") {
    healthText = lane.run.detail || "Last run failed";
  } else {
    const verb = lane.kind === "photos" ? "imported" : lane.kind === "drive" ? "synced" : "moved";
    const noun = lane.routed === 1 ? "file" : "files";
    const partial = health === "partial" ? " · last run had errors" : "";
    healthText = `${fmtCount(lane.routed)} ${noun} ${verb}${partial} · ${relTime(lane.run.ts)}`;
  }
  const kindLabel = lane.kind === "drive" ? "Drive" : lane.kind === "photos" ? "Photos" : "Folder";

  return (
    <div className={`lane ${health}`}>
      <div className="lane-node">
        <span className={`badge ${lane.kind}`}>{kindLabel}</span>
        <span className="lane-name">{lane.label}</span>
        <span className="lane-meta">
          {lane.source} · {lane.filter}
        </span>
      </div>
      <div className="lane-mid">
        <span className="lane-action">{lane.action}</span>
        <span className="lane-line" aria-hidden="true" />
      </div>
      <div className="lane-node dst">
        <span className="lane-name">{lane.dest}</span>
        <span className="lane-meta">{lane.detail}</span>
        <span className={`lane-health h-${health}`}>
          <span className={`dot d-${health}`} />
          {healthText}
        </span>
      </div>
      <div className="lane-actions">
        <button className="run" onClick={() => onRun(lane)}>
          Run now
        </button>
        {lane.revealPath && (
          <button onClick={() => api.revealInFinder(lane.revealPath).catch((e) => alert(String(e)))}>
            Reveal
          </button>
        )}
      </div>
    </div>
  );
}

function RunHistory({ runs }: { runs: api.RunRecord[] }) {
  if (runs.length === 0) {
    return (
      <div className="empty">
        <p>No runs recorded yet.</p>
        <p className="hint">Every completed import or move is logged here, with counts and outcome.</p>
      </div>
    );
  }
  return (
    <ul className="log">
      {runs.map((r, i) => {
        const verb = r.flow === "photos" ? "imported" : r.flow === "drive" ? "copied" : "moved";
        return (
          <li key={i}>
            <span className="ts">{relTime(r.ts)}</span>
            <span className={`badge ${r.flow}`}>{r.flow}</span>
            <span className="run-counts">
              {r.status === "failed" ? (
                <span className="run-fail">{r.detail}</span>
              ) : (
                <>
                  {fmtCount(r.ok)} {verb}
                  {r.skipped > 0 && ` · ${fmtCount(r.skipped)} skipped`}
                  {r.failed > 0 && ` · ${fmtCount(r.failed)} failed`}
                </>
              )}
              <span className="run-arrow"> → {r.dest}</span>
            </span>
          </li>
        );
      })}
    </ul>
  );
}

function StatusView({
  config,
  onNeedNames,
  onImported,
}: {
  config: Config;
  onNeedNames: (r: NamingReq) => void;
  onImported: () => void;
}) {
  const [cards, setCards] = useState<MountedCard[]>([]);
  const [paused, setPaused] = useState(false);

  const refresh = () => {
    api.listMountedCards().then(setCards);
    api.getPaused().then(setPaused);
  };
  useEffect(() => {
    refresh();
    const u = listen<boolean>("paused-changed", (e) => setPaused(e.payload));
    return () => {
      u.then((f) => f());
    };
  }, []);

  async function importNow(uuid: string) {
    const rule = config.card.find((c) => c.uuid.toLowerCase() === uuid.toLowerCase());
    try {
      if (rule?.prompt_name) {
        const dates = await api.prepareIngest(uuid);
        onNeedNames({ kind: "card", uuid, label: rule.label, dates });
      } else {
        await api.runIngestNow(uuid, {});
        onImported();
      }
    } catch (e) {
      alert(String(e));
    }
  }

  return (
    <section>
      <header className="view-head">
        <div>
          <h2>Status</h2>
          <p className="subtitle">
            Watchers run in the background. Connect a drive or drop a Lightroom export to start an import.
          </p>
        </div>
        <div className="row">
          <button onClick={refresh}>Refresh</button>
          <button
            onClick={async () => {
              await api.setPaused(!paused);
              setPaused(!paused);
            }}
          >
            {paused ? "Resume watchers" : "Pause watchers"}
          </button>
        </div>
      </header>

      <p className={paused ? "badge warn" : "badge ok"}>Watchers {paused ? "paused" : "active"}</p>

      <h3>Mounted volumes</h3>
      {cards.length === 0 && (
        <div className="empty">
          <p>No volumes detected.</p>
          <p className="hint">Connect a drive, or add a rule under External Drive.</p>
        </div>
      )}
      <ul className="list">
        {cards.map((c) => (
          <li key={c.path} className="row spread">
            <span>
              {c.label}{" "}
              {c.matched ? (
                <span className="badge ok">rule: {c.rule_label}</span>
              ) : (
                <span className="badge">{c.uuid ?? "no uuid"}</span>
              )}
            </span>
            {c.matched && c.uuid && <button onClick={() => importNow(c.uuid!)}>Import now</button>}
          </li>
        ))}
      </ul>
    </section>
  );
}

function NamingForm({
  req,
  mode,
  onClose,
}: {
  req: NamingReq;
  mode: NameMode;
  onClose: () => void;
}) {
  const [single, setSingle] = useState("");
  const [perDate, setPerDate] = useState<Record<string, string>>({});

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  async function confirm() {
    const names: Record<string, string> =
      mode === "single"
        ? Object.fromEntries(req.dates.map((d) => [d.date, single]))
        : perDate;
    try {
      if (req.kind === "card") await api.runIngestNow(req.uuid, names);
      else await api.runPhotosImportNow(req.index, names);
      onClose();
    } catch (e) {
      alert(String(e));
    }
  }

  const total = req.dates.reduce((n, d) => n + d.file_count, 0);
  const target = req.kind === "card" ? "destination folders" : "Photos albums";

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>Name this import</h2>
        <p className="subtitle">
          {req.label} · {total} files. These names become the {target}.
        </p>
        {mode === "single" ? (
          <Field label={`Name for all ${total} files`}>
            <input
              placeholder="e.g. Holiday"
              value={single}
              onChange={(e) => setSingle(e.target.value)}
              autoFocus
            />
          </Field>
        ) : (
          req.dates.map((d, i) => (
            <Field key={d.date} label={`${d.date} · ${d.file_count} files`}>
              <input
                placeholder="e.g. Holiday"
                value={perDate[d.date] ?? ""}
                autoFocus={i === 0}
                onChange={(e) => setPerDate((p) => ({ ...p, [d.date]: e.target.value }))}
              />
            </Field>
          ))
        )}
        <div className="row end">
          <button onClick={onClose}>Cancel</button>
          <button className="primary" onClick={confirm}>
            Import {total} files
          </button>
        </div>
      </div>
    </div>
  );
}

function CardsView({ config, patch }: { config: Config; patch: (p: Partial<Config>) => void }) {
  const updateCard = (i: number, p: Partial<CardRule>) =>
    patch({ card: config.card.map((r, j) => (j === i ? { ...r, ...p } : r)) });
  const removeCard = (i: number) => patch({ card: config.card.filter((_, j) => j !== i) });
  const addCard = () => patch({ card: [...config.card, api.newCard()] });

  return (
    <section>
      <header className="view-head">
        <div>
          <h2>External Drive</h2>
          <p className="subtitle">
            Rules that run automatically when a recognised drive is connected.
          </p>
        </div>
        <button onClick={addCard}>+ Add drive</button>
      </header>

      {config.card.length === 0 && (
        <div className="empty">
          <p>No drive rules yet.</p>
          <p className="hint">Add a rule so a drive auto-imports the moment it's connected.</p>
        </div>
      )}

      {config.card.map((card, i) => (
        <details key={i} className="card-edit" open>
          <summary className="card-head">
            <div>
              <strong>{card.label || "Untitled drive"}</strong>
              {card.dest && <div className="card-sub muted">→ {card.dest}</div>}
            </div>
            <button className="danger" onClick={(e) => { e.preventDefault(); removeCard(i); }}>
              Remove
            </button>
          </summary>

          <Group title="This drive">
            <Field label="Label" help="A name you'll recognise.">
              <input
                placeholder="Sony A7 IV"
                value={card.label}
                onChange={(e) => updateCard(i, { label: e.target.value })}
              />
            </Field>
            <Field
              label="Volume ID"
              help="The drive's unique ID. Connect the drive and click Detect to fill this in."
            >
              <div className="row">
                <input
                  placeholder="1A2B-3C4D"
                  value={card.uuid}
                  onChange={(e) => updateCard(i, { uuid: e.target.value })}
                />
                <button
                  onClick={async () => {
                    const mounted = await api.listMountedCards();
                    const cand = mounted.find((m) => !m.matched && m.uuid);
                    if (cand?.uuid) updateCard(i, { uuid: cand.uuid });
                    else alert("Connect an unconfigured drive first, then click Detect.");
                  }}
                >
                  Detect
                </button>
              </div>
            </Field>
          </Group>

          <Group title="What to copy">
            <Field
              label="Source folders"
              help="Folders on the drive to copy from, one per line. Use * to match a series — DCIM/1*MSDCF covers 100MSDCF, 101MSDCF, …"
            >
              <textarea
                rows={2}
                placeholder="DCIM/100MSDCF"
                value={card.sources.join("\n")}
                onChange={(e) =>
                  updateCard(i, {
                    sources: e.target.value.split("\n").map((x) => x.trim()).filter(Boolean),
                  })
                }
              />
            </Field>
            <CsvField
              label="File types"
              help="Comma-separated extensions to copy. Leave blank to copy everything."
              placeholder="arw, jpg, mp4"
              value={card.extensions}
              onChange={(v) => updateCard(i, { extensions: v })}
            />
          </Group>

          <Group title="Where photos go">
            <DestField value={card.dest} onChange={(v) => updateCard(i, { dest: v })} />
            <Field
              label="Folder structure"
              help="Template for the folders created at the destination. Tokens: {year}, {date}, {name}."
            >
              <input
                placeholder="{year}/{date} {name}"
                value={card.layout}
                onChange={(e) => updateCard(i, { layout: e.target.value })}
              />
            </Field>
            <p className="preview">
              Example: <code>{layoutExample(card.layout)}</code>
            </p>
            <Field
              label="Rename files (optional)"
              help="Rename each file as it's copied. Tokens: {year}, {date}, {name}, {seq}. The original extension is always kept. Blank = keep original names."
            >
              <input
                placeholder="{date}_{seq}"
                value={card.rename}
                onChange={(e) => updateCard(i, { rename: e.target.value })}
              />
            </Field>
            <p className="preview">
              Example: <code>{filenameExample(card.rename)}</code>
            </p>
          </Group>

          <RoutesEditor
            routes={card.routes}
            onChange={(routes) => updateCard(i, { routes })}
          />

          <Group title="When a drive is connected">
            <label className="check">
              <input
                type="checkbox"
                checked={card.prompt_name}
                onChange={(e) => updateCard(i, { prompt_name: e.target.checked })}
              />
              Ask me for a name before importing
            </label>
            <p className="help check-help">
              Used in the {"{name}"} token. Off = folders are just the date.
            </p>
            {card.prompt_name && (
              <Field
                label="Naming"
                help="One name for the whole import, or a separate name for each capture date."
              >
                <select
                  value={card.name_mode}
                  onChange={(e) => updateCard(i, { name_mode: e.target.value as NameMode })}
                >
                  <option value="per_date">A name per capture date</option>
                  <option value="single">One name for everything</option>
                </select>
              </Field>
            )}
          </Group>

          <Group title="After importing">
            <div className="cols">
              <Field
                label="Clean drive"
                help="Delete files from the drive once every file is copied and verified — permanent, so it only runs after the whole set is safely copied."
              >
                <select
                  value={card.cleanup}
                  onChange={(e) => updateCard(i, { cleanup: e.target.value as CleanupPolicy })}
                >
                  <option value="ask">Ask first</option>
                  <option value="always">Always</option>
                  <option value="never">Never</option>
                </select>
              </Field>
              <Field label="Eject drive" help="Unmount the drive when the import finishes successfully.">
                <select
                  value={card.eject}
                  onChange={(e) => updateCard(i, { eject: e.target.value as EjectPolicy })}
                >
                  <option value="never">Never</option>
                  <option value="ask">Ask first</option>
                  <option value="always">Always</option>
                </select>
              </Field>
            </div>
          </Group>
        </details>
      ))}
    </section>
  );
}

/** Optional per-extension destination overrides for a card rule (RAW→A, JPG→B). */
function RoutesEditor({
  routes,
  onChange,
}: {
  routes: Route[];
  onChange: (r: Route[]) => void;
}) {
  const update = (i: number, p: Partial<Route>) =>
    onChange(routes.map((r, j) => (j === i ? { ...r, ...p } : r)));
  const add = () => onChange([...routes, { extensions: [], dest: "", layout: "" }]);
  const remove = (i: number) => onChange(routes.filter((_, j) => j !== i));

  return (
    <Group title="Split by file type (optional)">
      <p className="help">
        Send some extensions to their own destination — e.g. RAW to an archive, JPG to a working
        folder. Routes are tried top to bottom; the first match wins, and anything unmatched uses
        the destination above. Leave a field blank to reuse the rule's.
      </p>
      {routes.map((r, i) => (
        <div key={i} className="route-row">
          <CsvField
            label="File types"
            placeholder="arw, raw"
            value={r.extensions}
            onChange={(v) => update(i, { extensions: v })}
          />
          <Field label="Destination" help="Blank = the destination above.">
            <input
              placeholder="~/Archive/RAW"
              value={r.dest}
              onChange={(e) => update(i, { dest: e.target.value })}
            />
          </Field>
          <Field label="Folder structure" help="Blank = the structure above.">
            <input
              placeholder="{year}/{date}"
              value={r.layout}
              onChange={(e) => update(i, { layout: e.target.value })}
            />
          </Field>
          <button
            className="danger"
            onClick={(e) => {
              e.preventDefault();
              remove(i);
            }}
          >
            Remove route
          </button>
        </div>
      ))}
      <button onClick={(e) => { e.preventDefault(); add(); }}>+ Add route</button>
    </Group>
  );
}

function DestField({
  value,
  onChange,
  help,
}: {
  value: string;
  onChange: (v: string) => void;
  help?: string;
}) {
  const [writable, setWritable] = useState<boolean | null>(null);
  useEffect(() => {
    if (!value) {
      setWritable(null);
      return;
    }
    let live = true;
    api.destWritable(value).then((w) => live && setWritable(w));
    return () => {
      live = false;
    };
  }, [value]);

  const badge =
    value &&
    (writable === null ? (
      <span className="badge">checking…</span>
    ) : writable ? (
      <span className="badge ok">reachable</span>
    ) : (
      <span className="badge warn">unreachable</span>
    ));

  return (
    <Field
      label="Destination"
      badge={badge}
      help={
        help ??
        "Where photos are copied — a folder on this Mac, a cloud folder (OneDrive, iCloud…), a network share, or an external drive."
      }
    >
      <div className="row">
        <input
          placeholder="~/Pictures/Imports"
          value={value}
          onChange={(e) => onChange(e.target.value)}
        />
        <button
          onClick={async () => {
            const dir = await pickFolder();
            if (dir) onChange(dir);
          }}
        >
          Choose…
        </button>
      </div>
    </Field>
  );
}

function FoldersView({ config, patch }: { config: Config; patch: (p: Partial<Config>) => void }) {
  const update = (i: number, p: Partial<FolderRule>) =>
    patch({ folder: config.folder.map((r, j) => (j === i ? { ...r, ...p } : r)) });
  const remove = (i: number) => patch({ folder: config.folder.filter((_, j) => j !== i) });
  const addFolder = () => patch({ folder: [...config.folder, api.newFolder()] });
  const addPhotos = () => patch({ folder: [...config.folder, api.newPhotosFolder()] });
  // A rule loaded from disk only carries its own kind's fields; flipping kind
  // backfills the other kind's defaults so its form fields are defined.
  const flipKind = (i: number, kind: FolderKind) => {
    const base = kind === "photos" ? api.newPhotosFolder() : api.newFolder();
    update(i, { ...base, ...config.folder[i], kind });
  };

  return (
    <section>
      <header className="view-head">
        <div>
          <h2>Folders</h2>
          <p className="subtitle">
            Watch a folder and route new files — move them into a dated folder, or import them into Apple Photos.
          </p>
        </div>
        <div className="row">
          <button onClick={addFolder}>+ To folder</button>
          <button onClick={addPhotos}>+ To Photos</button>
        </div>
      </header>

      {config.folder.length === 0 && (
        <div className="empty">
          <p>No folder rules yet.</p>
          <p className="hint">
            Add a rule to auto-sort a folder, or import a Lightroom export into Photos.
          </p>
        </div>
      )}

      {config.folder.map((rule, i) => (
        <details key={i} className="card-edit" open>
          <summary className="card-head">
            <div>
              <strong>
                {rule.label || (rule.kind === "photos" ? "Untitled import" : "Untitled folder")}
              </strong>
              {rule.watch && (
                <div className="card-sub muted">
                  {rule.watch} → {rule.kind === "photos" ? "Photos" : rule.dest || "…"}
                </div>
              )}
            </div>
            <div className="row">
              <button onClick={(e) => { e.preventDefault(); api.runFolderNow(i); }}>
                {rule.kind === "photos" ? "Import now" : "Move now"}
              </button>
              <button className="danger" onClick={(e) => { e.preventDefault(); remove(i); }}>
                Remove
              </button>
            </div>
          </summary>

          <Field label="Label" help="A name you'll recognise.">
            <input
              placeholder={rule.kind === "photos" ? "Lightroom exports" : "Downloads sorter"}
              value={rule.label}
              onChange={(e) => update(i, { label: e.target.value })}
            />
          </Field>

          <Field label="Watch folder" help="FileFlow handles new files that land here.">
            <div className="row">
              <input
                placeholder={
                  rule.kind === "photos" ? "~/Pictures/Lightroom Exports" : "~/Downloads/Incoming"
                }
                value={rule.watch}
                onChange={(e) => update(i, { watch: e.target.value })}
              />
              <button
                onClick={async () => {
                  const dir = await pickFolder();
                  if (dir) update(i, { watch: dir });
                }}
              >
                Choose…
              </button>
            </div>
          </Field>

          <Field label="Destination" help="What to do with new files in this folder.">
            <select value={rule.kind} onChange={(e) => flipKind(i, e.target.value as FolderKind)}>
              <option value="folder">Move to a folder</option>
              <option value="photos">Import into Apple Photos</option>
            </select>
          </Field>

          <CsvField
            label="File types"
            help={
              rule.kind === "photos"
                ? "Comma-separated extensions to import. Leave blank to import everything."
                : "Comma-separated extensions to move. Leave blank to move everything."
            }
            placeholder={rule.kind === "photos" ? "jpg, jpeg, tiff, heif" : "jpg, pdf, zip"}
            value={rule.extensions}
            onChange={(v) => update(i, { extensions: v })}
          />

          {rule.kind === "folder" ? (
            <>
              <DestField
                value={rule.dest}
                onChange={(v) => update(i, { dest: v })}
                help="Where files are moved — a folder on this Mac, a cloud folder, a network share, or an external drive."
              />
              <Field
                label="Folder structure"
                help="Subfolders created at the destination. Tokens: {year}, {date}. Leave blank to move files in flat."
              >
                <input
                  placeholder="{year}/{date}"
                  value={rule.layout}
                  onChange={(e) => update(i, { layout: e.target.value })}
                />
              </Field>
              <p className="preview">
                Example: <code>{folderExample(rule.layout)}</code>
              </p>
            </>
          ) : (
            <PhotosDest rule={rule} update={(p) => update(i, p)} />
          )}
        </details>
      ))}
    </section>
  );
}

/** Photos-destination fields for a folder rule (kind = "photos"). */
function PhotosDest({
  rule,
  update,
}: {
  rule: FolderRule;
  update: (p: Partial<FolderRule>) => void;
}) {
  return (
    <>
      <Group title="Into Photos">
        <Field label="Add to album" help="Where imported photos land in your Photos library.">
          <select
            value={rule.album_mode}
            onChange={(e) => update({ album_mode: e.target.value as AlbumMode })}
          >
            <option value="library">Library only — no album</option>
            <option value="fixed">A specific album</option>
            <option value="template">An album named by date</option>
          </select>
        </Field>
        {rule.album_mode === "fixed" && (
          <Field label="Album name" help="Created if it doesn't already exist.">
            <input
              placeholder="Lightroom"
              value={rule.photos_album}
              onChange={(e) => update({ photos_album: e.target.value })}
            />
          </Field>
        )}
        {rule.album_mode === "template" && (
          <>
            <Field
              label="Album name template"
              help="Files are grouped into albums by date — same tokens as a drive's folder structure: {year}, {date}, {name}."
            >
              <input
                placeholder="{date} {name}"
                value={rule.photos_album}
                onChange={(e) => update({ photos_album: e.target.value })}
              />
            </Field>
            <p className="preview">
              Example album:{" "}
              <code>{albumExample(rule.photos_album, rule.prompt_name ? "Holiday" : "")}</code>
            </p>
            <label className="check">
              <input
                type="checkbox"
                checked={rule.prompt_name}
                onChange={(e) => update({ prompt_name: e.target.checked })}
              />
              Ask me for a name before importing
            </label>
            <p className="help check-help">
              Fills the {"{name}"} token — e.g. “{"{date} {name}"}” becomes “2026-06-20 Holiday”.
            </p>
            {rule.prompt_name && (
              <Field
                label="Naming"
                help="One name for the whole import, or a separate name for each capture date."
              >
                <select
                  value={rule.name_mode}
                  onChange={(e) => update({ name_mode: e.target.value as NameMode })}
                >
                  <option value="per_date">A name per capture date</option>
                  <option value="single">One name for everything</option>
                </select>
              </Field>
            )}
          </>
        )}
        <label className="check">
          <input
            type="checkbox"
            checked={rule.skip_duplicates}
            onChange={(e) => update({ skip_duplicates: e.target.checked })}
          />
          Skip files already in my Photos library
        </label>
      </Group>

      <Group title="After importing">
        <Field label="Exported files" help="What to do with the files once they're safely in Photos.">
          <select
            value={rule.after_import}
            onChange={(e) => update({ after_import: e.target.value as AfterImport })}
          >
            <option value="leave">Leave them in place</option>
            <option value="archive">Move to an archive folder</option>
            <option value="delete">Delete them</option>
          </select>
        </Field>
        {rule.after_import === "archive" && (
          <Field label="Archive folder" help="Where imported files are moved.">
            <div className="row">
              <input
                placeholder="~/Pictures/Lightroom Exports/_imported"
                value={rule.archive_folder}
                onChange={(e) => update({ archive_folder: e.target.value })}
              />
              <button
                onClick={async () => {
                  const dir = await pickFolder();
                  if (dir) update({ archive_folder: dir });
                }}
              >
                Choose…
              </button>
            </div>
          </Field>
        )}
      </Group>
    </>
  );
}

function ActivityView({ activity }: { activity: ActivityEntry[] }) {
  return (
    <section>
      <header className="view-head">
        <div>
          <h2>Activity</h2>
          <p className="subtitle">A running log of imports and any problems.</p>
        </div>
      </header>
      {activity.length === 0 && (
        <div className="empty">
          <p>Nothing yet.</p>
          <p className="hint">Drive imports and Lightroom syncs will show up here.</p>
        </div>
      )}
      <ul className="log">
        {activity.map((a, i) => (
          <li key={i}>
            <span className="ts">{a.ts}</span>
            <span className={`badge ${a.flow}`}>{a.flow}</span>
            <span>{a.message}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}

function SettingsView({ config, patch }: { config: Config; patch: (p: Partial<Config>) => void }) {
  const [autostart, setAutostart] = useState<boolean | null>(null);
  useEffect(() => {
    isEnabled().then(setAutostart).catch(() => setAutostart(false));
  }, []);

  return (
    <section>
      <header className="view-head">
        <div>
          <h2>Settings</h2>
          <p className="subtitle">App-wide preferences.</p>
        </div>
      </header>

      <label className="check">
        <input
          type="checkbox"
          checked={autostart ?? false}
          disabled={autostart === null}
          onChange={async () => {
            if (autostart) await disable();
            else await enable();
            const now = await isEnabled();
            setAutostart(now);
            patch({ app: { ...config.app, autostart: now } });
          }}
        />
        Launch at login
      </label>
      <p className="help check-help">Start FileFlow automatically and keep it in the menu bar.</p>

      <label className="check">
        <input
          type="checkbox"
          checked={config.app.keep_running_on_close}
          onChange={(e) =>
            patch({ app: { ...config.app, keep_running_on_close: e.target.checked } })
          }
        />
        Keep running in the menu bar when the window is closed
      </label>
      <p className="help check-help">When off, closing the window quits FileFlow.</p>

      <label className="check">
        <input
          type="checkbox"
          checked={config.app.show_tray_icon}
          disabled={config.app.show_tray_icon && !config.app.show_dock_icon}
          onChange={(e) =>
            patch({ app: { ...config.app, show_tray_icon: e.target.checked } })
          }
        />
        Show menu-bar icon
      </label>

      <label className="check">
        <input
          type="checkbox"
          checked={config.app.show_dock_icon}
          disabled={config.app.show_dock_icon && !config.app.show_tray_icon}
          onChange={(e) =>
            patch({ app: { ...config.app, show_dock_icon: e.target.checked } })
          }
        />
        Show Dock icon
      </label>
      <p className="help check-help">Keep at least one of the menu-bar or Dock icon visible.</p>

      <Field label="Log level" help="How much detail is written to the log file. “info” is usually enough.">
        <select
          value={config.app.log_level}
          onChange={(e) => patch({ app: { ...config.app, log_level: e.target.value } })}
        >
          {["error", "warn", "info", "debug", "trace"].map((l) => (
            <option key={l} value={l}>
              {l}
            </option>
          ))}
        </select>
      </Field>

      <div className="row">
        <button
          onClick={async () => {
            try {
              const { appConfigDir } = await import("@tauri-apps/api/path");
              await api.revealInFinder(await appConfigDir());
            } catch (e) {
              alert(String(e));
            }
          }}
        >
          Open config folder
        </button>
        <button
          onClick={async () => {
            try {
              await api.revealInFinder(await api.logPath());
            } catch (e) {
              alert(String(e));
            }
          }}
        >
          Open log file
        </button>
      </div>
    </section>
  );
}
