// The Flow map: a node-link graph of every automation — sources and
// destinations deduplicate into nodes, each rule is an edge (see FlowGraph).
// Selecting an edge surfaces its lane row with health, counts and actions.
// The raw run history list lives behind the History tab, monthly totals behind Stats.
import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import * as api from "../api";
import type { Config } from "../api";
import type { NamingReq } from "../components/NamingForm";
import { FlowGraph } from "../components/FlowGraph";
import { fmtBytes, fmtCount, fmtDuration, relTime } from "../lib/util";

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
  dirty,
  onNeedNames,
  onNavigate,
}: {
  config: Config;
  dirty: boolean; // unsaved edits: lane indices may not match the saved rules the backend runs
  onNeedNames: (r: NamingReq) => void;
  onNavigate: (t: "cards" | "folders") => void;
}) {
  const [runs, setRuns] = useState<api.RunRecord[]>([]);
  const [undoable, setUndoable] = useState<string[]>([]);
  const [mode, setMode] = useState<"map" | "history" | "stats">("map");

  useEffect(() => {
    const refresh = () => {
      api.getRuns(500).then(setRuns);
      api.undoKeys().then(setUndoable);
    };
    refresh();
    const subs = [listen("run", refresh), listen("activity", refresh)];
    return () => subs.forEach((u) => u.then((f) => f()));
  }, []);

  const lanes = buildLanes(config, runs);
  const empty = config.card.length === 0 && config.folder.length === 0;

  async function runLane(l: Lane) {
    if (dirty) {
      alert("Save your changes first.");
      return;
    }
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

  async function undoLane(l: Lane) {
    if (!confirm(`Put the files from “${l.label}”'s last move back in ${l.source}?`)) return;
    try {
      alert(await api.undoLast(l.key));
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
          <button className={mode === "stats" ? "on" : ""} onClick={() => setMode("stats")}>
            Stats
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
        <FlowMap lanes={lanes} undoable={undoable} onRun={runLane} onUndo={undoLane} />
      ) : mode === "history" ? (
        <RunHistory runs={runs} />
      ) : (
        <Stats />
      )}
    </section>
  );
}

function FlowMap({
  lanes,
  undoable,
  onRun,
  onUndo,
}: {
  lanes: Lane[];
  undoable: string[];
  onRun: (l: Lane) => void;
  onUndo: (l: Lane) => void;
}) {
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
          <LaneRow
            lane={selLane}
            onRun={onRun}
            onUndo={selLane.kind === "folder" && undoable.includes(selLane.key) ? onUndo : undefined}
          />
        </div>
      ) : (
        <p className="fg-hint">Select a flow to inspect it and run it.</p>
      )}
    </>
  );
}

function LaneRow({
  lane,
  onRun,
  onUndo,
}: {
  lane: Lane;
  onRun: (l: Lane) => void;
  onUndo?: (l: Lane) => void; // set when the lane's last move can be undone
}) {
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
          {health === "failed" || health === "partial" ? "Retry" : "Run now"}
        </button>
        {onUndo && <button onClick={() => onUndo(lane)}>Undo last move</button>}
        {lane.revealPath && (
          <button onClick={() => api.revealInFinder(lane.revealPath).catch((e) => alert(String(e)))}>
            Reveal
          </button>
        )}
      </div>
      {lane.run?.errors?.length ? <ErrorList errors={lane.run.errors} failed={lane.run.failed} /> : null}
    </div>
  );
}

/** The per-file failures of one run, collapsed by default. */
function ErrorList({ errors, failed }: { errors: string[]; failed: number }) {
  const more = failed - errors.length;
  return (
    <details className="run-errors">
      <summary>
        {fmtCount(failed)} file{failed === 1 ? "" : "s"} failed — show why
      </summary>
      <ul>
        {errors.map((e, i) => (
          <li key={i}>{e}</li>
        ))}
        {more > 0 && <li className="muted">…and {fmtCount(more)} more (see the log file)</li>}
      </ul>
    </details>
  );
}

/** Month-by-month totals over the whole run history. */
function Stats() {
  const [months, setMonths] = useState<api.MonthStats[] | null>(null);
  useEffect(() => {
    api.getStats().then(setMonths);
  }, []);
  if (months === null) return null;
  if (months.length === 0) {
    return (
      <div className="empty">
        <p>No runs recorded yet.</p>
        <p className="hint">Monthly totals appear here once files start flowing.</p>
      </div>
    );
  }
  const total = months.reduce(
    (t, m) => ({ files: t.files + m.files, bytes: t.bytes + m.bytes, drives: t.drives + m.drives }),
    { files: 0, bytes: 0, drives: 0 },
  );
  return (
    <>
      <div className="flow-summary">
        <div className="stat">
          <span className="n">{fmtCount(total.files)}</span>
          <span className="l">Files routed</span>
        </div>
        <div className="stat">
          <span className="n">{fmtBytes(total.bytes)}</span>
          <span className="l">Data moved</span>
        </div>
        <div className="stat">
          <span className="n">{fmtCount(total.drives)}</span>
          <span className="l">Drive imports</span>
        </div>
      </div>
      <table className="stats-table">
        <thead>
          <tr>
            <th>Month</th>
            <th>Runs</th>
            <th>Drives</th>
            <th>Files</th>
            <th>Data</th>
            <th>Avg. run</th>
            <th>Failed</th>
          </tr>
        </thead>
        <tbody>
          {months.map((m) => (
            <tr key={m.month}>
              <td>{m.month}</td>
              <td>{fmtCount(m.runs)}</td>
              <td>{fmtCount(m.drives)}</td>
              <td>{fmtCount(m.files)}</td>
              <td>{fmtBytes(m.bytes)}</td>
              <td>{m.runs ? fmtDuration(m.duration_ms / m.runs) : "—"}</td>
              <td className={m.failed_runs ? "warnN" : undefined}>{fmtCount(m.failed_runs)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <p className="help">Runs recorded before this version have no size or duration.</p>
    </>
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
              {r.errors?.length ? <ErrorList errors={r.errors} failed={r.failed} /> : null}
            </span>
          </li>
        );
      })}
    </ul>
  );
}
