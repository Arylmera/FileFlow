# Flow graph — node-link map for the Flow view

Approved design (2026-07-16). Replaces the lane list in Flow ▸ Map with a
node-link graph; the History mode and the empty state are unchanged.

## Purpose

The lane list shows one row per automation but hides topology: two flows that
feed the same destination look unrelated. The graph deduplicates sources and
destinations into nodes and draws one edge per automation, so shared
destinations converge visually. Animated particles on healthy edges make the
view feel alive.

## Components

- `src/components/FlowGraph.tsx` (new) — pure SVG renderer. Props:
  `lanes: Lane[]`, `selectedKey: string | null`, `onSelect(key)`.
  No Tauri calls, no state beyond hover.
- `src/views/FlowView.tsx` — `Lane` type exported; `FlowMap` renders the
  summary stats, then `FlowGraph`, then the existing `LaneRow` for the
  selected edge (reused as the detail panel with Run now / Reveal).
- `src/App.css` — `.flow-graph` styles; `.lane*` styles stay (LaneRow is
  reused); `.lane-mid`/`.lane-line` become unused only if LaneRow drops its
  middle column — it does not, so nothing is removed.

## Data flow

`buildLanes()` is unchanged. `FlowGraph` projects lanes into:

- **Source nodes** — deduplicated by `lane.source`. Label: truncated source;
  sublabel: kind ("Drive"/"Folder"/"Photos"), or "N flows" when merged.
- **Destination nodes** — deduplicated by `lane.dest`. Label: truncated dest;
  sublabel: `lane.detail`, or "N flows" when merged.
- **Edges** — one per lane, source node → dest node, cubic Bézier.
  Stroke color by kind (`--flow-drive` / `--flow-photos` / `--flow-folder`);
  failed runs override to `--warn`; never-run edges are dashed and dimmed.
  A status dot sits at the destination end (`--ok` / `--warn` / `--muted`).

Layout is deterministic (no DOM measuring): fixed row height, two columns in
a computed `viewBox`, height grows with `max(sources, dests)`.

## Interaction

- Click (or Enter/Space — edges are `role="button"`, `tabIndex=0`) selects an
  edge; the matching `LaneRow` appears under the graph with health text and
  the Run now / Reveal actions. Clicking the selected edge deselects.
- Hover thickens an edge; an invisible wide stroke is the hit target.
- Particles: `animateMotion` circles ride edges whose last run succeeded
  (ok/partial), staggered; hidden under `prefers-reduced-motion`.

## Failure modes

- Long paths → middle-truncated labels, full text in an SVG `<title>`.
- Many flows → the SVG grows vertically; the view scrolls as today.
- Unset dest ("…") or duplicate sources across kinds → nodes render as-is;
  dedup by string is semantically correct.

## Testing

No UI test runner exists in the repo (Rust core only) — verification is
`npm run build` (tsc + vite build), the same gate CI runs. Visual check in
`npm run tauri dev` is macOS-only and left to the user.
