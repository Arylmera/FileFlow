// A running log of imports and any problems.
import type { ActivityEntry } from "../api";

export function ActivityView({ activity }: { activity: ActivityEntry[] }) {
  return (
    <section>
      <header className="view-head">
        <div>
          <h2>Activity</h2>
          <p className="subtitle">A running log of imports and any problems.</p>
        </div>
      </header>
      {activity.length === 0 && (
        <div className="empty">
          <p>Nothing yet.</p>
          <p className="hint">Drive imports and Lightroom syncs will show up here.</p>
        </div>
      )}
      <ul className="log">
        {activity.map((a, i) => (
          <li key={i}>
            <span className="ts">{a.ts}</span>
            <span className={`badge ${a.flow}`}>{a.flow}</span>
            <span>{a.message}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}
