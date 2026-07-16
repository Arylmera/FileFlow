// The shared "name this import" modal, driven by either flow (card or Photos).
import { useEffect, useState } from "react";
import * as api from "../api";
import type { DateGroup, NameMode } from "../api";
import { Field } from "./fields";

// A request to name an import, from either flow — drives the shared naming modal.
export type NamingReq =
  | { kind: "card"; uuid: string; label: string; dates: DateGroup[] }
  | { kind: "photos"; index: number; label: string; dates: DateGroup[] };

export function NamingForm({
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
