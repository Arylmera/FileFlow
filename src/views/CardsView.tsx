// External Drive rules: what to copy off a recognised drive, where it lands,
// and what happens to the drive afterwards.
import * as api from "../api";
import type { CardRule, CleanupPolicy, Config, EjectPolicy, NameMode, RawJpeg, Route } from "../api";
import { CsvField, DestField, Field, Group, RegexField } from "../components/fields";
import { filenameExample, layoutExample } from "../lib/examples";

export function CardsView({ config, patch }: { config: Config; patch: (p: Partial<Config>) => void }) {
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
        <details key={i} className="card-edit">
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
            <RegexField
              label="Only files matching (optional)"
              help="Regex matched against the file name, extension included (e.g. IMG_2024.jpg) — copy only matches. Contains-match; anchor with ^…$. Blank = all. Case-sensitive; prefix (?i) to ignore case."
              placeholder="^IMG_\d+"
              value={card.include}
              onChange={(v) => updateCard(i, { include: v })}
            />
            <RegexField
              label="Skip files matching (optional)"
              help="Regex on the file name (extension included). Contains-match; anchor with ^…$. Skips matches even if included above. Blank = skip none."
              placeholder="_thumb|\.tmp$"
              value={card.exclude}
              onChange={(v) => updateCard(i, { exclude: v })}
            />
            <Field
              label="RAW + JPEG pairs"
              help="Shots saved twice with the same name (DSC0001.ARW + DSC0001.JPG). Skipped JPEGs stay on the drive, even after a clean."
            >
              <select
                value={card.raw_jpeg}
                onChange={(e) => updateCard(i, { raw_jpeg: e.target.value as RawJpeg })}
              >
                <option value="independent">Copy each file on its own</option>
                <option value="together">Keep pairs together (same folder and number)</option>
                <option value="skip_jpeg">Copy only the RAW</option>
              </select>
            </Field>
          </Group>

          <Group title="Where photos go">
            <DestField value={card.dest} onChange={(v) => updateCard(i, { dest: v })} />
            <Field
              label="Folder structure"
              help="Template for the folders created at the destination. Tokens: {year}, {date}, {name}, {camera}. Dates come from the photo's EXIF when it has one."
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
              help="Rename each file as it's copied. Tokens: {year}, {date}, {time}, {camera}, {orig} (original name), {name}, {seq}. The original extension is always kept. Blank = keep original names."
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
            <DestField
              label="Backup copy (optional)"
              placeholder="/Volumes/NAS/Backup"
              value={card.backup_dest}
              onChange={(v) => updateCard(i, { backup_dest: v })}
              help="Also copy every file here, with the same folders. The drive is only cleaned once both copies are verified. Blank = no backup."
            />
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
