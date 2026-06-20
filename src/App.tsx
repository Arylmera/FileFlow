import { useEffect, useState } from "react";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
import "./App.css";

// ponytail: Phase 0 placeholder UI — just proves the shell + autostart toggle work.
// The real Status/Cards/Lightroom/Activity screens are Phase 5.
function App() {
  const [autostart, setAutostart] = useState<boolean | null>(null);

  useEffect(() => {
    isEnabled().then(setAutostart).catch(() => setAutostart(false));
  }, []);

  async function toggle() {
    if (autostart) {
      await disable();
    } else {
      await enable();
    }
    setAutostart(await isEnabled());
  }

  return (
    <main className="container">
      <h1>FileFlow</h1>
      <p>Resident in the menu bar. Watchers and configuration land in later phases.</p>
      <label style={{ display: "flex", gap: 8, alignItems: "center" }}>
        <input
          type="checkbox"
          checked={autostart ?? false}
          disabled={autostart === null}
          onChange={toggle}
        />
        Launch at login{autostart === null ? " (checking…)" : ""}
      </label>
    </main>
  );
}

export default App;
