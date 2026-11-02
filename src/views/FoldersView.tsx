// Watched-folder rules: move new files into a dated folder, or (on macOS)
// import them into Apple Photos.
import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import * as api from "../api";
import type { AfterImport, AlbumMode, Config, FolderKind, FolderRule, NameMode } from "../api";
import { CsvField, DestField, Field, Group, RegexField } from "../components/fields";
import { albumExample, folderExample } from "../lib/examples";
import { pickFolder } from "../lib/util";

export function FoldersView({
  config,
  patch,
  isMac,
  dirty,
}: {
  config: Config;
  patch: (p: Partial<Config>) => void;
  isMac: boolean;
  dirty: boolean; // unsaved edits: indices may not match the saved rules the backend runs
}) {
  const update = (i: number, p: Partial<FolderRule>) =>
    patch({ folder: config.folder.map((r, j) => (j === i ? { ...r, ...p } : r)) });
  const remove = (i: number) => patch({ folder: config.folder.filter((_, j) => j !== i) });
  const addFolder = () => patch({ folder: [...config.folder, api.newFolder()] });
  const addPhotos = () => patch({ folder: [...config.folder, api.newPhotosFolder()] });
  // A rule loaded from disk only carries its own kind's fields; flipping kind
  // backfills the other kind's defaults so its form fields are defined.
  // Which saved rules are actually being watched (a missing folder isn't).
  const [watched, setWatched] = useState<boolean[]>([]);
  useEffect(() => {
    api.folderWatchStatus().then(setWatched);
    const sub = listen<boolean[]>("watch-status", (e) => setWatched(e.payload));
    return () => {
      sub.then((f) => f());
    };
  }, []);
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
            {isMac
              ? "Watch a folder and route new files — move them into a dated folder, or import them into Apple Photos."
              : "Watch a folder and route new files — move them into a dated folder."}
          </p>
        </div>
        <div className="row">
          <button onClick={addFolder}>+ To folder</button>
          {isMac && <button onClick={addPhotos}>+ To Photos</button>}
        </div>
      </header>

      {config.folder.length === 0 && (
        <div className="empty">
          <p>No folder rules yet.</p>
          <p className="hint">
            {isMac
              ? "Add a rule to auto-sort a folder, or import a Lightroom export into Photos."
              : "Add a rule to auto-sort a folder."}
          </p>
        </div>
      )}

      {config.folder.map((rule, i) => (
        <details key={i} className="card-edit">
          <summary className="card-head">
            <div>
              <strong>
                {rule.label || (rule.kind === "photos" ? "Untitled import" : "Untitled folder")}
              </strong>
              {rule.watch && (
                <div className="card-sub muted">
                  {rule.watch} → {rule.kind === "photos" ? "Photos" : rule.dest || "…"}{" "}
                  {!dirty && watched[i] !== undefined && (
                    watched[i] ? (
                      <span className="badge ok">watching</span>
                    ) : (
                      <span
                        className="badge warn"
                        title="The folder doesn't exist right now (e.g. a share that isn't mounted). FileFlow starts watching it as soon as it appears."
                      >
                        not watched — folder missing
                      </span>
                    )
                  )}
                </div>
              )}
            </div>
            <div className="row">
              <button
                disabled={dirty}
                title={dirty ? "Save your changes first" : undefined}
                onClick={(e) => { e.preventDefault(); api.runFolderNow(i); }}
              >
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

          <label className="check">
            <input
              type="checkbox"
              checked={rule.recursive}
              onChange={(e) => update(i, { recursive: e.target.checked })}
            />
            Include subfolders
          </label>
          <p className="help check-help">
            Also handle files inside folders within the watch folder. The destination
            {rule.kind === "photos" ? " and archive folder are" : " folder is"} always skipped.
          </p>

          <Field label="Destination" help="What to do with new files in this folder.">
            <select value={rule.kind} onChange={(e) => flipKind(i, e.target.value as FolderKind)}>
              <option value="folder">Move to a folder</option>
              {/* Apple Photos only exists on macOS; keep the option visible for an
                  existing photos rule (e.g. a config copied from a Mac) so the select
                  doesn't render blank. */}
              {(isMac || rule.kind === "photos") && (
                <option value="photos">Import into Apple Photos</option>
              )}
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

          <RegexField
            label="Only files matching (optional)"
            help={`Regex on the file name, extension included (e.g. IMG_2024.jpg) — only ${
              rule.kind === "photos" ? "import" : "move"
            } matches. Contains-match; anchor with ^…$. Blank = all. Case-sensitive; prefix (?i) to ignore case.`}
            placeholder="^IMG_\d+"
            value={rule.include}
            onChange={(v) => update(i, { include: v })}
          />
          <RegexField
            label="Skip files matching (optional)"
            help={`Regex on the file name (extension included). Contains-match; anchor with ^…$. Skips matches even if included above. Blank = skip none.`}
            placeholder="_thumb|\.tmp$"
            value={rule.exclude}
            onChange={(v) => update(i, { exclude: v })}
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
                help="Subfolders created at the destination. Tokens: {year}, {date}, {camera}. Dates come from the photo's EXIF when it has one. Leave blank to move files in flat."
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
