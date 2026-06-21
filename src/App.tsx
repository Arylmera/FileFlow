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
  FolderRule,
  LightroomRule,
  MountedCard,
  NameMode,
  PhotosReady,
} from "./api";
import "./App.css";

type Tab = "status" | "cards" | "folders" | "lightroom" | "activity" | "settings";

// A request to name an import, from either flow — drives the shared naming modal.
type NamingReq =
  | { kind: "card"; uuid: string; label: string; dates: DateGroup[] }
  | { kind: "photos"; label: string; dates: DateGroup[] };

const TABS: Tab[] = ["status", "cards", "folders", "lightroom", "activity", "settings"];
const TAB_LABELS: Record<Tab, string> = {
  status: "Status",
  cards: "External Drive",
  folders: "Folders",
  lightroom: "Import to Photos",
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
  const [tab, setTab] = useState<Tab>("status");
  const [config, setConfig] = useState<Config>(api.emptyConfig);
  const [dirty, setDirty] = useState(false);
  const [activity, setActivity] = useState<ActivityEntry[]>([]);
  const [naming, setNaming] = useState<NamingReq | null>(null);

  useEffect(() => {
    api.getConfig().then(setConfig);
    api.getActivity(100).then(setActivity);

    const unlisten = [
      listen<ActivityEntry>("activity", (e) => setActivity((a) => [e.payload, ...a].slice(0, 200))),
      listen<CardReady>("card-ready", (e) =>
        setNaming({ kind: "card", uuid: e.payload.uuid, label: e.payload.label, dates: e.payload.dates }),
      ),
      listen<PhotosReady>("photos-ready", (e) =>
        setNaming({ kind: "photos", label: "Import to Photos", dates: e.payload.dates }),
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

      <main>
        {tab === "status" && (
          <StatusView
            config={config}
            onNeedNames={setNaming}
            onImported={() => api.getActivity(100).then(setActivity)}
          />
        )}
        {tab === "cards" && <CardsView config={config} patch={patchConfig} />}
        {tab === "folders" && <FoldersView config={config} patch={patchConfig} />}
        {tab === "lightroom" && <LightroomView config={config} patch={patchConfig} />}
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
              : config.lightroom?.name_mode ?? "per_date"
          }
          onClose={() => setNaming(null)}
        />
      )}
    </div>
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
      else await api.runPhotosImportNow(names);
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
        <div key={i} className="card-edit">
          <div className="row spread card-head">
            <div>
              <strong>{card.label || "Untitled drive"}</strong>
              {card.dest && <div className="card-sub muted">→ {card.dest}</div>}
            </div>
            <button className="danger" onClick={() => removeCard(i)}>
              Remove
            </button>
          </div>

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
          </Group>

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
        </div>
      ))}
    </section>
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
  const add = () => patch({ folder: [...config.folder, api.newFolder()] });

  return (
    <section>
      <header className="view-head">
        <div>
          <h2>Folder to Folder</h2>
          <p className="subtitle">
            Watch a folder and move whatever lands in it into a dated destination.
          </p>
        </div>
        <button onClick={add}>+ Add folder</button>
      </header>

      {config.folder.length === 0 && (
        <div className="empty">
          <p>No folder rules yet.</p>
          <p className="hint">Add a rule to auto-sort files dropped into a folder.</p>
        </div>
      )}

      {config.folder.map((rule, i) => (
        <div key={i} className="card-edit">
          <div className="row spread card-head">
            <div>
              <strong>{rule.label || "Untitled folder"}</strong>
              {rule.watch && (
                <div className="card-sub muted">
                  {rule.watch} → {rule.dest || "…"}
                </div>
              )}
            </div>
            <div className="row">
              <button onClick={() => api.runFolderNow(i)}>Move now</button>
              <button className="danger" onClick={() => remove(i)}>
                Remove
              </button>
            </div>
          </div>

          <Field label="Label" help="A name you'll recognise.">
            <input
              placeholder="Downloads sorter"
              value={rule.label}
              onChange={(e) => update(i, { label: e.target.value })}
            />
          </Field>

          <Field label="Watch folder" help="FileFlow moves new files that land here.">
            <div className="row">
              <input
                placeholder="~/Downloads/Incoming"
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

          <CsvField
            label="File types"
            help="Comma-separated extensions to move. Leave blank to move everything."
            placeholder="jpg, pdf, zip"
            value={rule.extensions}
            onChange={(v) => update(i, { extensions: v })}
          />
        </div>
      ))}
    </section>
  );
}

function LightroomView({ config, patch }: { config: Config; patch: (p: Partial<Config>) => void }) {
  const lr = config.lightroom;
  const update = (p: Partial<LightroomRule>) =>
    patch({ lightroom: { ...(lr as LightroomRule), ...p } });

  if (!lr) {
    return (
      <section>
        <h2>Import to Photos</h2>
        <div className="empty">
          <p>Not configured.</p>
          <p className="hint">Watch an export folder and import new files into Apple Photos.</p>
        </div>
        <button className="primary" onClick={() => patch({ lightroom: api.newLightroom() })}>
          Enable
        </button>
      </section>
    );
  }

  return (
    <section>
      <header className="view-head">
        <div>
          <h2>Import to Photos</h2>
          <p className="subtitle">New files in the watched folder are imported into Apple Photos.</p>
        </div>
        <div className="row">
          <button onClick={() => api.startPhotosImport()}>Import now</button>
          <button className="danger" onClick={() => patch({ lightroom: null })}>
            Disable
          </button>
        </div>
      </header>

      <Group title="Source">
        <Field
          label="Watch folder"
          help="FileFlow imports new files dropped here. Point Lightroom's export at this folder."
        >
          <div className="row">
            <input
              placeholder="~/Pictures/Lightroom Exports"
              value={lr.watch_folder}
              onChange={(e) => update({ watch_folder: e.target.value })}
            />
            <button
              onClick={async () => {
                const dir = await pickFolder();
                if (dir) update({ watch_folder: dir });
              }}
            >
              Choose…
            </button>
          </div>
        </Field>
        <CsvField
          label="File types"
          help="Comma-separated extensions to import."
          placeholder="jpg, jpeg, tiff, heif"
          value={lr.extensions}
          onChange={(v) => update({ extensions: v })}
        />
      </Group>

      <Group title="Into Photos">
        <Field label="Add to album" help="Where imported photos land in your Photos library.">
          <select
            value={lr.album_mode}
            onChange={(e) => update({ album_mode: e.target.value as AlbumMode })}
          >
            <option value="library">Library only — no album</option>
            <option value="fixed">A specific album</option>
            <option value="template">An album named by date</option>
          </select>
        </Field>
        {lr.album_mode === "fixed" && (
          <Field label="Album name" help="Created if it doesn't already exist.">
            <input
              placeholder="Lightroom"
              value={lr.photos_album}
              onChange={(e) => update({ photos_album: e.target.value })}
            />
          </Field>
        )}
        {lr.album_mode === "template" && (
          <>
            <Field
              label="Album name template"
              help="Files are grouped into albums by date — same tokens as a drive's folder structure: {year}, {date}, {name}."
            >
              <input
                placeholder="{date} {name}"
                value={lr.photos_album}
                onChange={(e) => update({ photos_album: e.target.value })}
              />
            </Field>
            <p className="preview">
              Example album:{" "}
              <code>{albumExample(lr.photos_album, lr.prompt_name ? "Holiday" : "")}</code>
            </p>
            <label className="check">
              <input
                type="checkbox"
                checked={lr.prompt_name}
                onChange={(e) => update({ prompt_name: e.target.checked })}
              />
              Ask me for a name before importing
            </label>
            <p className="help check-help">
              Fills the {"{name}"} token — e.g. “{"{date} {name}"}” becomes “2026-06-20 Holiday”.
            </p>
            {lr.prompt_name && (
              <Field
                label="Naming"
                help="One name for the whole import, or a separate name for each capture date."
              >
                <select
                  value={lr.name_mode}
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
            checked={lr.skip_duplicates}
            onChange={(e) => update({ skip_duplicates: e.target.checked })}
          />
          Skip files already in my Photos library
        </label>
      </Group>

      <Group title="After importing">
        <Field label="Exported files" help="What to do with the files once they're safely in Photos.">
          <select
            value={lr.after_import}
            onChange={(e) => update({ after_import: e.target.value as AfterImport })}
          >
            <option value="leave">Leave them in place</option>
            <option value="archive">Move to an archive folder</option>
            <option value="delete">Delete them</option>
          </select>
        </Field>
        {lr.after_import === "archive" && (
          <Field label="Archive folder" help="Where imported files are moved.">
            <div className="row">
              <input
                placeholder="~/Pictures/Lightroom Exports/_imported"
                value={lr.archive_folder}
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
    </section>
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
