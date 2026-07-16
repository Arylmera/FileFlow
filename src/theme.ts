// The theme system. A theme is a named token set in theme.css, selected by a
// data-theme attribute on <html>. The choice is a per-machine UI preference, so
// it lives in localStorage rather than in the rule config that syncs as TOML —
// changing it applies instantly and never touches the Save flow.

export type ThemeId = "daylight" | "midnight" | "darkroom" | "fjord" | "paper" | "graphite";
export type ThemePref = ThemeId | "system";

export interface ThemeMeta {
  id: ThemePref;
  name: string;
  desc: string;
  /** Picker swatch colors (any CSS background value) — always the theme's own,
   *  whatever theme is currently active. */
  swatch: { bg: string; surface: string; accent: string; text: string };
}

// The split-gradient swatch for "system" previews both halves it can resolve to.
const split = (light: string, dark: string) =>
  `linear-gradient(115deg, ${light} 50%, ${dark} 50%)`;

export const THEMES: ThemeMeta[] = [
  {
    id: "system",
    name: "System",
    desc: "Follow the OS appearance — Daylight by day, Midnight in the dark.",
    swatch: {
      bg: split("#f4f5f7", "#15161a"),
      surface: split("#ffffff", "#212229"),
      accent: split("#3b6fe0", "#5b8dff"),
      text: split("#1c1d22", "#f1f2f6"),
    },
  },
  {
    id: "daylight",
    name: "Daylight",
    desc: "The classic light look.",
    swatch: { bg: "#f4f5f7", surface: "#ffffff", accent: "#3b6fe0", text: "#1c1d22" },
  },
  {
    id: "midnight",
    name: "Midnight",
    desc: "Low-glare dark for dim rooms.",
    swatch: { bg: "#15161a", surface: "#212229", accent: "#5b8dff", text: "#f1f2f6" },
  },
  {
    id: "darkroom",
    name: "Darkroom",
    desc: "Safelight red on black — easy on night vision during late culls.",
    swatch: { bg: "#100b0b", surface: "#1c1414", accent: "#e5484d", text: "#f4e9e6" },
  },
  {
    id: "fjord",
    name: "Fjord",
    desc: "Muted nordic blues, cool and calm.",
    swatch: { bg: "#242933", surface: "#2e3440", accent: "#81a1c1", text: "#eceff4" },
  },
  {
    id: "paper",
    name: "Paper",
    desc: "Warm sepia, like a print archive.",
    swatch: { bg: "#f2ecdf", surface: "#fbf7ee", accent: "#7d5a1f", text: "#38301f" },
  },
  {
    id: "graphite",
    name: "Graphite",
    desc: "Monochrome — one less color in the room.",
    swatch: { bg: "#ececee", surface: "#ffffff", accent: "#55555e", text: "#232326" },
  },
];

const STORAGE_KEY = "fileflow.theme";
const darkQuery = window.matchMedia("(prefers-color-scheme: dark)");

const isPref = (v: unknown): v is ThemePref => THEMES.some((t) => t.id === v);

export function getThemePref(): ThemePref {
  // localStorage can throw (e.g. storage disabled); fall back to following the OS.
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    return isPref(v) ? v : "system";
  } catch {
    return "system";
  }
}

function resolve(pref: ThemePref): ThemeId {
  return pref === "system" ? (darkQuery.matches ? "midnight" : "daylight") : pref;
}

function apply(pref: ThemePref) {
  document.documentElement.dataset.theme = resolve(pref);
}

export function setThemePref(pref: ThemePref) {
  try {
    localStorage.setItem(STORAGE_KEY, pref);
  } catch {
    // Not persisted, but still applied for this session.
  }
  apply(pref);
}

/** Apply the stored theme, and track OS light/dark flips while set to "system". */
export function initTheme() {
  apply(getThemePref());
  darkQuery.addEventListener("change", () => {
    if (getThemePref() === "system") apply("system");
  });
}
