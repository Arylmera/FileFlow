// Shared form primitives used by every rule editor and the settings page.
import { useEffect, useState, type ReactNode } from "react";
import * as api from "../api";
import { csvToList, listToCsv, pickFolder } from "../lib/util";

/** Labelled field with an example placeholder and a one-line "how to fill it" hint. */
export function Field({
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

export function Group({ title, children }: { title: string; children: ReactNode }) {
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
export function CsvField({
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
  // Re-seed when the external value diverges from what's typed — e.g. a list row is
  // removed and React reuses this instance under an index key. Compared by content so an
  // in-progress trailing comma/space (which parses to the same list) isn't clobbered.
  const joined = value.join(",");
  useEffect(() => {
    if (csvToList(text).join(",") !== joined) setText(listToCsv(value));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [joined]);
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

/**
 * Single-pattern regex editor with an inline validity check. The authoritative
 * validation is server-side (save_config uses Rust's regex engine, which differs
 * slightly from JS); this just gives instant feedback on obvious typos.
 */
export function RegexField({
  label,
  help,
  placeholder,
  value,
  onChange,
}: {
  label: string;
  help?: string;
  placeholder?: string;
  value: string;
  onChange: (v: string) => void;
}) {
  let err: string | null = null;
  if (value) {
    try {
      new RegExp(value);
    } catch (e) {
      err = (e as Error).message;
    }
  }
  // Patterns run in Rust's regex engine, which rejects lookarounds/backreferences —
  // say so up front rather than letting a save fail mysteriously.
  const dialect = "Rust regex syntax — lookarounds and backreferences aren't supported.";
  return (
    <Field
      label={label}
      help={err ? undefined : help ? `${help} ${dialect}` : dialect}
      badge={err ? <span className="badge warn">invalid</span> : undefined}
    >
      <input placeholder={placeholder} value={value} onChange={(e) => onChange(e.target.value)} />
      {err && <span className="help err">{err}</span>}
    </Field>
  );
}

/** Destination-path field with a picker button and a live reachability badge. */
export function DestField({
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
