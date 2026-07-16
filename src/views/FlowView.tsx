// The Flow map: a node-link graph of every automation — sources and
// destinations deduplicate into nodes, each rule is an edge (see FlowGraph).
// Selecting an edge surfaces its lane row with health, counts and actions.
// The raw run history list lives behind the History tab.
import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import * as api from "../api";
import type { Config } from "../api";
import type { NamingReq } from "../components/NamingForm";
import { FlowGraph } from "../components/FlowGraph";
import { fmtCount, relTime } from "../lib/util";

type LaneKind = "drive" | "photos" | "folder";

export interface Lane {
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

export function FlowView({
  config,
  onNeedNames,
  onNavigate,
}: {
  config: Config;
  onNeedNames: (r: NamingReq) => void;
  onNavigate: (t: "cards" | "folders") => void;
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
  const [selected, setSelected] = useState<string | null>(null);
  // A rule edited or removed while selected must not strand a stale panel.
  const selLane = lanes.find((l) => l.key === selected) ?? null;

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
      <FlowGraph lanes={lanes} selectedKey={selected} onSelect={setSelected} />
      {selLane ? (
        <div className="lanes">
          <LaneRow lane={selLane} onRun={onRun} />
        </div>
      ) : (
        <p className="fg-hint">Select a flow to inspect it and run it.</p>
      )}
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
