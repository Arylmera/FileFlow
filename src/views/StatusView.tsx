// Devices: watcher status, mounted volumes, and manual import/eject actions.
import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import * as api from "../api";
import type { Config, MountedCard } from "../api";
import type { NamingReq } from "../components/NamingForm";

export function StatusView({
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

  async function ejectNow(path: string) {
    try {
      await api.ejectNow(path);
      refresh();
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
            <span className="row">
              {c.matched && c.uuid && <button onClick={() => importNow(c.uuid!)}>Import now</button>}
              {c.ejectable && <button onClick={() => ejectNow(c.path)}>Eject</button>}
            </span>
          </li>
        ))}
      </ul>
    </section>
  );
}
