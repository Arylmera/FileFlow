// Shown after a drive imported cleanly when its rule says "Ask first" to erase and/or
// eject. Keeping the files is always the default: Esc, the backdrop and Enter keep them.
import { useEffect, useRef, useState } from "react";
import * as api from "../api";
import type { DrivePending } from "../api";

export function DriveDoneForm({ req, onClose }: { req: DrivePending; onClose: () => void }) {
  const [erase, setErase] = useState(false);
  const [eject, setEject] = useState(true);
  const [busy, setBusy] = useState(false);
  const answered = useRef(false);

  async function answer(doErase: boolean, doEject: boolean) {
    if (answered.current) return;
    answered.current = true;
    setBusy(true);
    try {
      await api.finishDrive(req.uuid, doErase, doEject);
    } catch (e) {
      alert(String(e)); // e.g. the drive was unplugged before the answer
    }
    onClose();
  }
  const keep = () => answer(false, false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") keep();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const name = req.label || "The drive";
  const files = `${req.files} file${req.files === 1 ? "" : "s"}`;
  const both = req.ask_erase && req.ask_eject;

  return (
    <div className="modal-backdrop" onClick={keep}>
      <div className="modal" role="dialog" aria-labelledby="drive-done-title" onClick={(e) => e.stopPropagation()}>
        <h2 id="drive-done-title">{name} is imported</h2>
        <p className="subtitle">
          {files} copied and verified at the destination.{" "}
          {both
            ? "Choose what to do with the drive."
            : req.ask_erase
              ? `Erase ${req.files === 1 ? "it" : "them"} from the drive now? Erasing is permanent.`
              : "Eject the drive now?"}
        </p>

        {both && (
          <>
            <label className="check">
              <input type="checkbox" checked={erase} onChange={(e) => setErase(e.target.checked)} />
              Erase the {files} from the drive (permanent)
            </label>
            <label className="check">
              <input type="checkbox" checked={eject} onChange={(e) => setEject(e.target.checked)} />
              Eject the drive
            </label>
          </>
        )}

        <div className="row end">
          {both ? (
            <>
              <button disabled={busy} onClick={keep}>
                Not now
              </button>
              <button className={erase ? "danger" : "primary"} disabled={busy} onClick={() => answer(erase, eject)}>
                {erase && eject ? "Erase and eject" : erase ? "Erase" : eject ? "Eject" : "Done"}
              </button>
            </>
          ) : req.ask_erase ? (
            <>
              <button className="danger" disabled={busy} onClick={() => answer(true, false)}>
                Erase {files}
              </button>
              <button className="primary" disabled={busy} onClick={keep} autoFocus>
                Keep files
              </button>
            </>
          ) : (
            <>
              <button disabled={busy} onClick={keep}>
                Not now
              </button>
              <button className="primary" disabled={busy} onClick={() => answer(false, true)} autoFocus>
                Eject
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
