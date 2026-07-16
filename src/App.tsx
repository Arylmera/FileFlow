// The app shell: top bar with tabs and Save, the progress strip, live event
// subscriptions, and the shared naming modal. Each tab's content lives in views/.
import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import * as api from "./api";
import type { ActivityEntry, CardReady, Config, PhotosReady, Progress } from "./api";
import { NamingForm, type NamingReq } from "./components/NamingForm";
import { ActivityView } from "./views/ActivityView";
import { CardsView } from "./views/CardsView";
import { FlowView } from "./views/FlowView";
import { FoldersView } from "./views/FoldersView";
import { SettingsView } from "./views/SettingsView";
import { StatusView } from "./views/StatusView";
import "./App.css";

type Tab = "flow" | "status" | "cards" | "folders" | "activity" | "settings";

const TABS: Tab[] = ["flow", "status", "cards", "folders", "activity", "settings"];
const TAB_LABELS: Record<Tab, string> = {
  flow: "Flow",
  status: "Devices",
  cards: "External Drive",
  folders: "Folders",
  activity: "Activity",
  settings: "Settings",
};

export default function App() {
  const [tab, setTab] = useState<Tab>("flow");
  const [config, setConfig] = useState<Config>(api.emptyConfig);
  const [dirty, setDirty] = useState(false);
  const [activity, setActivity] = useState<ActivityEntry[]>([]);
  const [naming, setNaming] = useState<NamingReq | null>(null);
  const [progress, setProgress] = useState<Progress | null>(null);
  const [platform, setPlatform] = useState("macos");

  useEffect(() => {
    api.getConfig().then(setConfig);
    api.getActivity(100).then(setActivity);
    api.getPlatform().then(setPlatform);

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
    try {
      await api.saveConfig(config);
      setDirty(false);
    } catch (e) {
      // e.g. an invalid include/exclude regex rejected by save_config.
      alert(`Couldn't save: ${e}`);
    }
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
        {tab === "folders" && (
          <FoldersView config={config} patch={patchConfig} isMac={platform === "macos"} />
        )}
        {tab === "activity" && <ActivityView activity={activity} />}
        {tab === "settings" && (
          <SettingsView config={config} patch={patchConfig} isMac={platform === "macos"} />
        )}
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
