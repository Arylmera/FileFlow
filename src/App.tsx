import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
import * as api from "./api";
import type {
  ActivityEntry,
  AfterImport,
  CardReady,
  CardRule,
  CleanupPolicy,
  Config,
  EjectPolicy,
  LightroomRule,
  MountedCard,
  NameMode,
} from "./api";
import "./App.css";

type Tab = "status" | "cards" | "lightroom" | "activity" | "settings";

const csvToList = (s: string) =>
  s.split(",").map((x) => x.trim()).filter(Boolean);
const listToCsv = (l: string[]) => l.join(", ");

async function pickFolder(): Promise<string | null> {
  try {
    const res = await openDialog({ directory: true, multiple: false });
    return typeof res === "string" ? res : null;
  } catch {
    return null;
  }
}

export default function App() {
  const [tab, setTab] = useState<Tab>("status");
  const [config, setConfig] = useState<Config>(api.emptyConfig);
  const [dirty, setDirty] = useState(false);
  const [activity, setActivity] = useState<ActivityEntry[]>([]);
  const [naming, setNaming] = useState<CardReady | null>(null);

  useEffect(() => {
    api.getConfig().then(setConfig);
    api.getActivity(100).then(setActivity);

    const unlisten = [
      listen<ActivityEntry>("activity", (e) =>
        setActivity((a) => [e.payload, ...a].slice(0, 200)),
      ),
      listen<CardReady>("card-ready", (e) => setNaming(e.payload)),
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
          {(["status", "cards", "lightroom", "activity", "settings"] as Tab[]).map((t) => (
            <button
              key={t}
              className={tab === t ? "tab active" : "tab"}
              onClick={() => setTab(t)}
            >
              {t}
            </button>
          ))}
        </nav>
        <button className="save" disabled={!dirty} onClick={save}>
          {dirty ? "Save changes" : "Saved"}
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
        {tab === "lightroom" && <LightroomView config={config} patch={patchConfig} />}
        {tab === "activity" && <ActivityView activity={activity} />}
        {tab === "settings" && <SettingsView config={config} patch={patchConfig} />}
      </main>

      {naming && (
        <NamingForm
          card={naming}
          mode={config.card.find((c) => c.uuid.toLowerCase() === naming.uuid.toLowerCase())?.name_mode ?? "per_date"}
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
  onNeedNames: (c: CardReady) => void;
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
        onNeedNames({ uuid, label: rule.label, volume_root: "", dates });
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
      <div className="row spread">
        <h2>Status</h2>
        <div>
          <button onClick={refresh}>Refresh</button>{" "}
          <button
            onClick={async () => {
              await api.setPaused(!paused);
              setPaused(!paused);
            }}
          >
            {paused ? "Resume watchers" : "Pause watchers"}
          </button>
        </div>
      </div>
      <p className={paused ? "badge warn" : "badge ok"}>
        Watchers {paused ? "paused" : "active"}
      </p>

      <h3>Mounted volumes</h3>
      {cards.length === 0 && <p className="muted">No volumes detected.</p>}
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
            {c.matched && c.uuid && (
              <button onClick={() => importNow(c.uuid!)}>Import now</button>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}

function NamingForm({
  card,
  mode,
  onClose,
}: {
  card: CardReady;
  mode: NameMode;
  onClose: () => void;
}) {
  const [single, setSingle] = useState("");
  const [perDate, setPerDate] = useState<Record<string, string>>({});

  async function confirm() {
    const names: Record<string, string> =
      mode === "single"
        ? Object.fromEntries(card.dates.map((d) => [d.date, single]))
        : perDate;
    try {
      await api.runIngestNow(card.uuid, names);
      onClose();
    } catch (e) {
      alert(String(e));
    }
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>Name this import — {card.label}</h2>
        {mode === "single" ? (
          <label className="field">
            Name for all {card.dates.reduce((n, d) => n + d.file_count, 0)} files
            <input value={single} onChange={(e) => setSingle(e.target.value)} autoFocus />
          </label>
        ) : (
          card.dates.map((d) => (
            <label key={d.date} className="field">
              {d.date} · {d.file_count} files
              <input
                value={perDate[d.date] ?? ""}
                onChange={(e) =>
                  setPerDate((p) => ({ ...p, [d.date]: e.target.value }))
                }
              />
            </label>
          ))
        )}
        <div className="row end">
          <button onClick={onClose}>Cancel</button>
          <button className="primary" onClick={confirm}>
            Import
          </button>
        </div>
      </div>
    </div>
  );
}

function CardsView({
  config,
  patch,
}: {
  config: Config;
  patch: (p: Partial<Config>) => void;
}) {
  const updateCard = (i: number, p: Partial<CardRule>) =>
    patch({ card: config.card.map((r, j) => (j === i ? { ...r, ...p } : r)) });
  const removeCard = (i: number) =>
    patch({ card: config.card.filter((_, j) => j !== i) });
  const addCard = () => patch({ card: [...config.card, api.newCard()] });

  return (
    <section>
      <div className="row spread">
        <h2>Cards</h2>
        <button onClick={addCard}>+ Add card</button>
      </div>
      {config.card.length === 0 && <p className="muted">No card rules yet.</p>}
      {config.card.map((card, i) => (
        <div key={i} className="card-edit">
          <div className="row spread">
            <strong>{card.label || "Untitled card"}</strong>
            <button className="danger" onClick={() => removeCard(i)}>
              Remove
            </button>
          </div>
          <label className="field">
            Label
            <input value={card.label} onChange={(e) => updateCard(i, { label: e.target.value })} />
          </label>
          <label className="field">
            Volume UUID
            <div className="row">
              <input value={card.uuid} onChange={(e) => updateCard(i, { uuid: e.target.value })} />
              <button
                onClick={async () => {
                  const mounted = await api.listMountedCards();
                  const cand = mounted.find((m) => !m.matched && m.uuid);
                  if (cand?.uuid) updateCard(i, { uuid: cand.uuid });
                  else alert("Insert an unconfigured card first.");
                }}
              >
                Detect
              </button>
            </div>
          </label>
          <label className="field">
            Source folders (one per line, globs OK)
            <textarea
              rows={2}
              value={card.sources.join("\n")}
              onChange={(e) =>
                updateCard(i, { sources: e.target.value.split("\n").map((x) => x.trim()).filter(Boolean) })
              }
            />
          </label>
          <DestField value={card.dest} onChange={(v) => updateCard(i, { dest: v })} />
          <label className="field">
            Layout
            <input value={card.layout} onChange={(e) => updateCard(i, { layout: e.target.value })} />
          </label>
          <label className="field">
            Extensions (comma-separated, blank = all)
            <input
              value={listToCsv(card.extensions)}
              onChange={(e) => updateCard(i, { extensions: csvToList(e.target.value) })}
            />
          </label>
          <label className="check">
            <input
              type="checkbox"
              checked={card.prompt_name}
              onChange={(e) => updateCard(i, { prompt_name: e.target.checked })}
            />
            Prompt for a name
          </label>
          <div className="row gap">
            <label className="field">
              Name mode
              <select
                value={card.name_mode}
                onChange={(e) => updateCard(i, { name_mode: e.target.value as NameMode })}
              >
                <option value="per_date">per date</option>
                <option value="single">single</option>
              </select>
            </label>
            <label className="field">
              Cleanup
              <select
                value={card.cleanup}
                onChange={(e) => updateCard(i, { cleanup: e.target.value as CleanupPolicy })}
              >
                <option value="ask">ask</option>
                <option value="always">always</option>
                <option value="never">never</option>
              </select>
            </label>
            <label className="field">
              Eject
              <select
                value={card.eject}
                onChange={(e) => updateCard(i, { eject: e.target.value as EjectPolicy })}
              >
                <option value="never">never</option>
                <option value="ask">ask</option>
                <option value="always">always</option>
              </select>
            </label>
          </div>
        </div>
      ))}
    </section>
  );
}

function DestField({ value, onChange }: { value: string; onChange: (v: string) => void }) {
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

  return (
    <label className="field">
      Destination{" "}
      {value &&
        (writable === null ? (
          <span className="badge">checking…</span>
        ) : writable ? (
          <span className="badge ok">reachable</span>
        ) : (
          <span className="badge warn">unreachable</span>
        ))}
      <div className="row">
        <input value={value} onChange={(e) => onChange(e.target.value)} />
        <button
          onClick={async () => {
            const dir = await pickFolder();
            if (dir) onChange(dir);
          }}
        >
          Choose…
        </button>
      </div>
    </label>
  );
}

function LightroomView({
  config,
  patch,
}: {
  config: Config;
  patch: (p: Partial<Config>) => void;
}) {
  const lr = config.lightroom;
  const update = (p: Partial<LightroomRule>) =>
    patch({ lightroom: { ...(lr as LightroomRule), ...p } });

  if (!lr) {
    return (
      <section>
        <h2>Lightroom → Photos</h2>
        <p className="muted">Not configured.</p>
        <button onClick={() => patch({ lightroom: api.newLightroom() })}>Enable</button>
      </section>
    );
  }

  return (
    <section>
      <div className="row spread">
        <h2>Lightroom → Photos</h2>
        <button className="danger" onClick={() => patch({ lightroom: null })}>
          Disable
        </button>
      </div>
      <label className="field">
        Watch folder
        <div className="row">
          <input
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
      </label>
      <label className="field">
        Photos album
        <input value={lr.photos_album} onChange={(e) => update({ photos_album: e.target.value })} />
      </label>
      <label className="check">
        <input
          type="checkbox"
          checked={lr.skip_duplicates}
          onChange={(e) => update({ skip_duplicates: e.target.checked })}
        />
        Skip files already in the library
      </label>
      <label className="field">
        After import
        <select
          value={lr.after_import}
          onChange={(e) => update({ after_import: e.target.value as AfterImport })}
        >
          <option value="leave">leave</option>
          <option value="archive">archive</option>
          <option value="delete">delete</option>
        </select>
      </label>
      {lr.after_import === "archive" && (
        <label className="field">
          Archive folder
          <div className="row">
            <input
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
        </label>
      )}
      <label className="field">
        Extensions (comma-separated)
        <input
          value={listToCsv(lr.extensions)}
          onChange={(e) => update({ extensions: csvToList(e.target.value) })}
        />
      </label>
    </section>
  );
}

function ActivityView({ activity }: { activity: ActivityEntry[] }) {
  return (
    <section>
      <h2>Activity</h2>
      {activity.length === 0 && <p className="muted">Nothing yet.</p>}
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

function SettingsView({
  config,
  patch,
}: {
  config: Config;
  patch: (p: Partial<Config>) => void;
}) {
  const [autostart, setAutostart] = useState<boolean | null>(null);
  useEffect(() => {
    isEnabled().then(setAutostart).catch(() => setAutostart(false));
  }, []);

  return (
    <section>
      <h2>Settings</h2>
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
      <label className="field">
        Log level
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
      </label>
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
