// App-wide preferences: appearance, launch/tray behavior, and troubleshooting.
import { useEffect, useState } from "react";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import * as api from "../api";
import type { Config, NotifyLevel } from "../api";
import { Field, Group } from "../components/fields";
import { ThemePicker } from "../components/ThemePicker";

export function SettingsView({
  config,
  patch,
  isMac,
  dirty,
}: {
  config: Config;
  patch: (p: Partial<Config>) => void;
  isMac: boolean;
  dirty: boolean;
}) {
  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [version, setVersion] = useState("");
  const trayName = isMac ? "menu bar" : "system tray";
  useEffect(() => {
    isEnabled().then(setAutostart).catch(() => setAutostart(false));
    import("@tauri-apps/api/app").then((m) => m.getVersion()).then(setVersion).catch(() => {});
  }, []);

  return (
    <section>
      <header className="view-head">
        <div>
          <h2>Settings</h2>
          <p className="subtitle">App-wide preferences.</p>
        </div>
      </header>

      <Group title="Appearance">
        <p className="help">
          Applies immediately and is remembered on this machine — separate from your rules, so no
          need to save.
        </p>
        <ThemePicker />
      </Group>

      <Group title="General">
        <label className="check">
          <input
            type="checkbox"
            checked={autostart ?? false}
            disabled={autostart === null}
            onChange={async () => {
              if (autostart) await disable();
              else await enable();
              const now = await isEnabled();
              setAutostart(now);
              patch({ app: { ...config.app, autostart: now } });
            }}
          />
          Launch at login
        </label>
        <p className="help check-help">Start FileFlow automatically and keep it in the {trayName}.</p>

        <label className="check">
          <input
            type="checkbox"
            checked={config.app.keep_running_on_close}
            onChange={(e) =>
              patch({ app: { ...config.app, keep_running_on_close: e.target.checked } })
            }
          />
          Keep running in the {trayName} when the window is closed
        </label>
        <p className="help check-help">When off, closing the window quits FileFlow.</p>

        <label className="check">
          <input
            type="checkbox"
            checked={config.app.start_minimized}
            onChange={(e) =>
              patch({ app: { ...config.app, start_minimized: e.target.checked } })
            }
          />
          Start minimized
        </label>
        <p className="help check-help">
          Open in the {trayName} without showing the window at launch.
        </p>

        {/* The Dock (and hiding the tray icon in its favour) is macOS-only; elsewhere
            the tray icon is the app's one surface and always stays on. */}
        {isMac && (
          <>
            <label className="check">
              <input
                type="checkbox"
                checked={config.app.show_tray_icon}
                disabled={config.app.show_tray_icon && !config.app.show_dock_icon}
                onChange={(e) =>
                  patch({ app: { ...config.app, show_tray_icon: e.target.checked } })
                }
              />
              Show menu-bar icon
            </label>

            <label className="check">
              <input
                type="checkbox"
                checked={config.app.show_dock_icon}
                disabled={config.app.show_dock_icon && !config.app.show_tray_icon}
                onChange={(e) =>
                  patch({ app: { ...config.app, show_dock_icon: e.target.checked } })
                }
              />
              Show Dock icon
            </label>
            <p className="help check-help">Keep at least one of the menu-bar or Dock icon visible.</p>
          </>
        )}
      </Group>

      <Group title="Notifications">
        <Field label="Show notifications" help="Errors include an unreachable destination, a failed copy, or a missing permission.">
          <select
            value={config.app.notify}
            onChange={(e) => patch({ app: { ...config.app, notify: e.target.value as NotifyLevel } })}
          >
            <option value="all">For every import and move</option>
            <option value="errors">Only for errors</option>
            <option value="off">Never</option>
          </select>
        </Field>
      </Group>

      <Group title="Schedule">
        <p className="help">
          Hold automatic imports and moves outside these hours — e.g. send big transfers to a NAS
          overnight. Work that arrives meanwhile runs when the window opens. Leave both blank to run
          any time. Run now always works.
        </p>
        <div className="cols">
          <Field label="Run from">
            <input
              type="time"
              value={config.app.run_from}
              onChange={(e) => patch({ app: { ...config.app, run_from: e.target.value } })}
            />
          </Field>
          <Field label="Until" help="Earlier than the start time = overnight.">
            <input
              type="time"
              value={config.app.run_until}
              onChange={(e) => patch({ app: { ...config.app, run_until: e.target.value } })}
            />
          </Field>
        </div>
        {(config.app.run_from || config.app.run_until) && (
          <button onClick={() => patch({ app: { ...config.app, run_from: "", run_until: "" } })}>
            Clear hours
          </button>
        )}
        <label className="check">
          <input
            type="checkbox"
            checked={config.app.pause_on_battery}
            onChange={(e) => patch({ app: { ...config.app, pause_on_battery: e.target.checked } })}
          />
          Wait for power when on battery
        </label>
        <p className="help check-help">Automatic runs wait until the charger is connected.</p>
      </Group>

      <Group title="Back up your rules">
        <p className="help">
          Save every rule and setting to a file, or load one from another computer. A loaded file
          shows up as unsaved changes, so you can review it before saving.
        </p>
        <div className="row">
          <button
            disabled={dirty}
            title={dirty ? "Save your changes first — export writes the saved rules" : undefined}
            onClick={async () => {
              try {
                const path = await saveDialog({
                  defaultPath: "fileflow-rules.toml",
                  filters: [{ name: "FileFlow rules", extensions: ["toml"] }],
                });
                if (path) await api.exportConfig(path);
              } catch (e) {
                alert(String(e));
              }
            }}
          >
            Export rules…
          </button>
          <button
            onClick={async () => {
              try {
                const path = await openDialog({
                  multiple: false,
                  filters: [{ name: "FileFlow rules", extensions: ["toml"] }],
                });
                if (typeof path !== "string") return;
                const loaded = await api.importConfig(path);
                const n = loaded.card.length + loaded.folder.length;
                if (!confirm(`Replace your current rules and settings with the ${n} rule(s) in this file?`)) return;
                patch(loaded);
              } catch (e) {
                alert(`Couldn't load that file: ${e}`);
              }
            }}
          >
            Import rules…
          </button>
        </div>
      </Group>

      <Group title="Troubleshooting">
        <Field label="Log level" help="How much detail is written to the log file. “info” is usually enough.">
          <select
            value={config.app.log_level}
            onChange={(e) => patch({ app: { ...config.app, log_level: e.target.value } })}
          >
            {["error", "warn", "info", "debug", "trace"].map((l) => (
              <option key={l} value={l}>
                {l}
              </option>
            ))}
          </select>
        </Field>

        <div className="row">
          <button
            onClick={async () => {
              try {
                const { appConfigDir } = await import("@tauri-apps/api/path");
                await api.revealInFinder(await appConfigDir());
              } catch (e) {
                alert(String(e));
              }
            }}
          >
            Open config folder
          </button>
          <button
            onClick={async () => {
              try {
                await api.revealInFinder(await api.logPath());
              } catch (e) {
                alert(String(e));
              }
            }}
          >
            Open log file
          </button>
        </div>

        <p className="help">FileFlow {version && `v${version}`}</p>
      </Group>
    </section>
  );
}
