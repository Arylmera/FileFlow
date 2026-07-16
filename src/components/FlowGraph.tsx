// Node-link projection of the Flow lanes: sources and destinations deduplicate
// into nodes, each automation is an edge. Pure SVG with a deterministic layout —
// no DOM measuring, so it renders identically on first paint.
import type { Lane } from "../views/FlowView";

const W = 760;
const NODE_W = 240;
const NODE_H = 48;
const ROW_H = 68;
const PAD = 8;
const SRC_X = PAD;
const DST_X = W - NODE_W - PAD;

interface Node {
  label: string;
  sub: string;
  full: string; // untruncated, for the tooltip
  y: number;
}

/** Keep the tail of long paths — the leaf folder is the part that matters. */
const trunc = (s: string, max = 30) => (s.length > max ? "…" + s.slice(s.length - max + 1) : s);

const kindLabel = (k: Lane["kind"]) =>
  k === "drive" ? "Drive" : k === "photos" ? "Photos" : "Folder";

/** Deduplicate lanes on a key into positioned nodes, preserving lane order. */
function buildNodes(
  lanes: Lane[],
  keyOf: (l: Lane) => string,
  subOf: (l: Lane) => string,
): Map<string, Node> {
  const nodes = new Map<string, Node>();
  for (const l of lanes) {
    const key = keyOf(l);
    const prev = nodes.get(key);
    if (prev) {
      const n = lanes.filter((x) => keyOf(x) === key).length;
      prev.sub = `${n} flows`;
    } else {
      nodes.set(key, {
        label: trunc(key),
        sub: subOf(l),
        full: key,
        y: PAD + nodes.size * ROW_H,
      });
    }
  }
  return nodes;
}

export function FlowGraph({
  lanes,
  selectedKey,
  onSelect,
}: {
  lanes: Lane[];
  selectedKey: string | null;
  onSelect: (key: string | null) => void;
}) {
  const sources = buildNodes(
    lanes,
    (l) => l.source,
    (l) => kindLabel(l.kind),
  );
  const dests = buildNodes(
    lanes,
    (l) => l.dest,
    (l) => l.detail,
  );
  const height = 2 * PAD + Math.max(sources.size, dests.size) * ROW_H - (ROW_H - NODE_H);

  return (
    <div className="flow-graph">
      <svg viewBox={`0 0 ${W} ${height}`} role="group" aria-label="Flow map">
        {lanes.map((l, i) => {
          const y1 = sources.get(l.source)!.y + NODE_H / 2;
          const y2 = dests.get(l.dest)!.y + NODE_H / 2;
          const x1 = SRC_X + NODE_W;
          const mx = (x1 + DST_X) / 2;
          const d = `M ${x1},${y1} C ${mx},${y1} ${mx},${y2} ${DST_X},${y2}`;
          const status = l.run?.status ?? "idle";
          const color =
            status === "failed" || status === "partial"
              ? "var(--warn)"
              : `var(--flow-${l.kind})`;
          const selected = l.key === selectedKey;
          const select = () => onSelect(selected ? null : l.key);
          return (
            <g
              key={l.key}
              className={`fg-edge${selected ? " sel" : ""}`}
              role="button"
              tabIndex={0}
              aria-label={`${l.label}: ${l.source} → ${l.dest}`}
              onClick={select}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  select();
                }
              }}
            >
              <title>{`${l.label} — ${l.action}`}</title>
              <path
                className="fg-line"
                d={d}
                style={{ stroke: color }}
                strokeDasharray={status === "idle" ? "6 6" : undefined}
              />
              <path className="fg-hit" d={d} />
              {l.run && status !== "failed" && (
                <circle className="fg-dot" r="3.5" style={{ fill: color }}>
                  <animateMotion dur={`${2.6 + (i % 5) * 0.4}s`} repeatCount="indefinite" path={d} />
                </circle>
              )}
              <circle className={`fg-status d-${status}`} cx={DST_X - 7} cy={y2} r="4" />
              {selected && (
                <text className="fg-action" x={mx} y={(y1 + y2) / 2 - 8} textAnchor="middle">
                  {l.label} · {l.action}
                </text>
              )}
            </g>
          );
        })}
        {[...sources.values(), ...dests.values()].map((n, i) => {
          const x = i < sources.size ? SRC_X : DST_X;
          return (
            <g className="fg-node" key={`${x}:${n.full}`}>
              <title>{n.full}</title>
              <rect x={x} y={n.y} width={NODE_W} height={NODE_H} rx="10" />
              <text className="t1" x={x + 14} y={n.y + 20}>
                {n.label}
              </text>
              <text className="t2" x={x + 14} y={n.y + 37}>
                {n.sub}
              </text>
            </g>
          );
        })}
      </svg>
    </div>
  );
}
