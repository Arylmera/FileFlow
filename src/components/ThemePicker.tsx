// Theme chooser for Settings → Appearance. Selection applies instantly and is
// stored per-machine (see theme.ts) — it does not go through the Save flow.
import { useState } from "react";
import { THEMES, getThemePref, setThemePref, type ThemeMeta, type ThemePref } from "../theme";

function Swatch({ swatch }: { swatch: ThemeMeta["swatch"] }) {
  return (
    <span className="swatch" style={{ background: swatch.bg }} aria-hidden="true">
      <span className="swatch-card" style={{ background: swatch.surface }}>
        <span className="swatch-dot" style={{ background: swatch.accent }} />
        <span className="swatch-line" style={{ background: swatch.text }} />
      </span>
    </span>
  );
}

export function ThemePicker() {
  const [pref, setPref] = useState<ThemePref>(getThemePref);

  const choose = (id: ThemePref) => {
    setThemePref(id);
    setPref(id);
  };

  return (
    <div className="themes" role="radiogroup" aria-label="Theme">
      {THEMES.map((t) => (
        <button
          key={t.id}
          role="radio"
          aria-checked={pref === t.id}
          className={pref === t.id ? "theme-card on" : "theme-card"}
          onClick={() => choose(t.id)}
        >
          <Swatch swatch={t.swatch} />
          <span className="theme-name">{t.name}</span>
          <span className="theme-desc">{t.desc}</span>
        </button>
      ))}
    </div>
  );
}
