//! Thin, typed IPC surface over the core engine + watchers.

use crate::state::{ActivityEntry, AppState, MonthStats, RunRecord};
use crate::{volume, watchers};
use fileflow_core::config::{AfterImport, CardRule, Config, Destination, EjectPolicy};
use fileflow_core::ingest::{self, DateGroup};
use std::collections::BTreeMap;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, State};

#[derive(serde::Serialize)]
pub struct MountedCard {
    pub label: String, // volume name (the /Volumes dir name)
    pub path: PathBuf,
    pub uuid: Option<String>,
    pub matched: bool, // matches a configured card rule
    pub rule_label: Option<String>,
    pub ejectable: bool, // false for the boot disk
}

#[tauri::command]
pub fn get_config(state: State<AppState>) -> Config {
    state.snapshot()
}

/// Reject a config the flows can't run safely:
/// - an include/exclude regex that won't compile would silently turn the flow into a
///   deny-all (the scanners' fail-closed fallback);
/// - a Photos rule set to archive with no archive folder would move the exports into
///   the process's working directory.
fn validate(config: &Config) -> Result<(), String> {
    use fileflow_core::util::validate_regex;
    config.app.validate_window()?;
    let name = |label: &str| if label.is_empty() { "(unnamed)".to_string() } else { label.to_string() };
    let check = |pat: &str, which: &str, what: &str, label: &str| {
        validate_regex(pat)
            .map_err(|e| format!("{what} “{}”: {which} pattern is invalid — {e}", name(label)))
    };
    for c in &config.cards {
        check(&c.include, "include", "Drive", &c.label)?;
        check(&c.exclude, "exclude", "Drive", &c.label)?;
    }
    for f in &config.folders {
        check(&f.include, "include", "Folder", &f.label)?;
        check(&f.exclude, "exclude", "Folder", &f.label)?;
        if let Destination::Photos { after_import: AfterImport::Archive, archive_folder, .. } = &f.target {
            if archive_folder.trim().is_empty() {
                return Err(format!("Folder “{}”: choose an archive folder, or pick another after-import action", name(&f.label)));
            }
        }
    }
    Ok(())
}

/// The drive rule for `uuid`, its mounted volume, and its pre-flighted destination.
fn resolve_card(state: &AppState, uuid: &str) -> Result<(CardRule, PathBuf, PathBuf), String> {
    let rule = state.snapshot().card(uuid).cloned().ok_or("no drive rule matches that UUID")?;
    let volume_root = volume::find_volume_by_uuid(uuid).ok_or("drive not connected")?;
    let dest = ingest::resolve_dest(&rule, &volume_root).map_err(|e| e.to_string())?;
    Ok((rule, volume_root, dest))
}

#[tauri::command]
pub fn save_config(app: AppHandle, state: State<AppState>, mut config: Config) -> Result<(), String> {
    validate(&config)?;
    config.app.ensure_reachable();
    config.save(&state.config_path).map_err(|e| e.to_string())?;
    crate::apply_window_mode(&app, &config.app);
    *state.config.lock().unwrap() = config;
    watchers::rebind_folders(&app); // pick up added/removed/re-pointed folder rules without a restart
    Ok(())
}

#[tauri::command]
pub fn list_mounted_cards(state: State<AppState>) -> Vec<MountedCard> {
    let cfg = state.snapshot();
    let mut out: Vec<MountedCard> = volume::mounted_volumes()
        .into_iter()
        .map(|path| {
            let uuid = volume::volume_uuid(&path);
            let rule = uuid.as_deref().and_then(|u| cfg.card(u));
            MountedCard {
                label: path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string(),
                matched: rule.is_some(),
                rule_label: rule.map(|r| r.label.clone()),
                ejectable: !volume::is_boot_volume(&path),
                uuid,
                path,
            }
        })
        .collect();
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
}

/// Resolve + pre-flight the destination and return the distinct capture dates,
/// so the UI can show the naming form.
#[tauri::command]
pub fn prepare_ingest(state: State<AppState>, uuid: String) -> Result<Vec<DateGroup>, String> {
    let (rule, volume_root, _) = resolve_card(&state, &uuid)?;
    Ok(ingest::scan_dates(&rule, &volume_root))
}

/// Manual card ingest. `names` maps `YYYY-MM-DD` → folder name (one entry in single mode).
/// Runs on a background thread; progress streams via `activity` events.
#[tauri::command]
pub fn run_ingest_now(
    app: AppHandle,
    uuid: String,
    names: BTreeMap<String, String>,
) -> Result<(), String> {
    let (rule, volume_root, dest) = resolve_card(&app.state::<AppState>(), &uuid)?;
    std::thread::spawn(move || {
        watchers::run_card_ingest(&app, &rule, &volume_root, &names, &dest);
    });
    Ok(())
}

/// The user's answer to a `drive-pending` confirmation: erase and/or eject the drive.
#[tauri::command]
pub fn finish_drive(app: AppHandle, uuid: String, erase: bool, eject: bool) -> Result<(), String> {
    watchers::finish_drive(&app, &uuid, erase, eject)
}

/// Manually eject a mounted volume by its mount path. Independent of any rule.
#[tauri::command]
pub fn eject_now(path: PathBuf) -> Result<(), String> {
    if volume::is_boot_volume(&path) {
        return Err("refusing to eject the boot disk".into());
    }
    ingest::eject(&path, EjectPolicy::Always).map_err(|e| e.to_string())?;
    Ok(())
}

/// Run the folder rule at `index` now, dispatching on its kind. A Photos rule with
/// name-prompting emits `photos-ready`; everything else runs immediately.
#[tauri::command]
pub fn run_folder_now(app: AppHandle, index: usize) {
    std::thread::spawn(move || watchers::run_now(&app, index));
}

/// Import the Photos folder rule at `index` with a date→name map (confirmed naming form).
#[tauri::command]
pub fn run_photos_import_now(app: AppHandle, index: usize, names: BTreeMap<String, String>) {
    std::thread::spawn(move || watchers::run_photos_import_named(&app, index, &names));
}

#[tauri::command]
pub fn get_activity(state: State<AppState>, limit: usize) -> Vec<ActivityEntry> {
    state.recent_activity(limit)
}

/// Recent run records (most-recent-first), the durable history behind the Flow map.
#[tauri::command]
pub fn get_runs(state: State<AppState>, limit: usize) -> Vec<RunRecord> {
    state.recent_runs(limit)
}

/// Month-by-month totals over the full run history (newest first).
#[tauri::command]
pub fn get_stats(state: State<AppState>) -> Vec<MonthStats> {
    state.stats()
}

/// Revert a folder rule's last move (`rule_key` = `folder:{watch}`).
#[tauri::command]
pub fn undo_last(app: AppHandle, rule_key: String) -> Result<String, String> {
    watchers::undo_last(&app, &rule_key)
}

/// Rule keys whose last move can be undone.
#[tauri::command]
pub fn undo_keys(state: State<AppState>) -> Vec<String> {
    state.undo_keys()
}

/// Per folder rule (by index): is its watch folder being watched right now?
#[tauri::command]
pub fn folder_watch_status(app: AppHandle) -> Vec<bool> {
    watchers::folder_watch_status(&app)
}

/// Write the saved rules + settings to `path` as TOML (a backup or a transfer file).
#[tauri::command]
pub fn export_config(state: State<AppState>, path: PathBuf) -> Result<(), String> {
    state.snapshot().save(&path).map_err(|e| e.to_string())
}

/// Read a config file for the editor. Nothing is applied here: the UI loads it as
/// unsaved changes, so the user reviews it and Save runs the usual validation.
#[tauri::command]
pub fn import_config(path: PathBuf) -> Result<Config, String> {
    if !path.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }
    let config = Config::load(&path).map_err(|e| e.to_string())?;
    validate(&config)?;
    Ok(config)
}

/// True if `path` (after `~` expansion) is an existing, writable directory.
/// Drives the Cards form's live "reachable & writable?" indicator.
#[tauri::command]
pub fn dest_writable(path: String) -> bool {
    ingest::is_writable_dir(&ingest::expand(&path))
}

#[tauri::command]
pub fn get_paused(state: State<AppState>) -> bool {
    state.is_paused()
}

#[tauri::command]
pub fn set_paused(state: State<AppState>, paused: bool) {
    state.set_paused(paused);
}

/// Reveal a path in the platform file manager (`~` expanded) — Finder on macOS,
/// Explorer on Windows. Used by Settings' "Open config". (The command keeps its
/// historical name: it's the wire key the frontend invokes.)
#[tauri::command]
pub fn reveal_in_finder(path: String) -> Result<(), String> {
    let p = ingest::expand(&path);
    reveal(&p).map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
fn reveal(p: &std::path::Path) -> std::io::Result<std::process::Child> {
    std::process::Command::new("open").arg("-R").arg(p).spawn()
}

#[cfg(target_os = "windows")]
fn reveal(p: &std::path::Path) -> std::io::Result<std::process::Child> {
    std::process::Command::new("explorer")
        .arg(fileflow_core::windows::explorer_select_arg(&p.to_string_lossy()))
        .spawn()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn reveal(p: &std::path::Path) -> std::io::Result<std::process::Child> {
    // No "select" verb portable across Linux file managers; open the containing dir.
    let dir = p.parent().unwrap_or(p);
    std::process::Command::new("xdg-open").arg(dir).spawn()
}

/// The OS the shell was built for (`macos` | `windows` | `linux`), so the UI can hide
/// platform-only flows (Apple Photos) and use the right file-manager wording.
#[tauri::command]
pub fn get_platform() -> &'static str {
    std::env::consts::OS
}

#[tauri::command]
pub fn log_path(app: AppHandle) -> Result<String, String> {
    let dir = app.path().app_log_dir().map_err(|e| e.to_string())?;
    Ok(dir.join("fileflow.log").to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::validate;
    use fileflow_core::config::Config;
    use serde_json::json;

    fn cfg(folder: serde_json::Value) -> Config {
        serde_json::from_value(json!({ "folder": [folder] })).unwrap()
    }

    #[test]
    fn validate_rejects_archive_without_folder_and_bad_regex() {
        let photos = |archive: &str| {
            cfg(json!({ "watch": "~/in", "kind": "photos", "after_import": "archive", "archive_folder": archive }))
        };
        assert!(validate(&photos("")).is_err(), "archive with no folder is rejected");
        assert!(validate(&photos("~/done")).is_ok());
        assert!(validate(&cfg(json!({ "watch": "~/in", "kind": "folder", "include": "(" }))).is_err());
    }

    #[test]
    fn import_round_trips_an_export() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rules.toml");
        let c = cfg(json!({ "watch": "~/in", "kind": "folder", "dest": "~/out", "recursive": true }));
        c.save(&path).unwrap();
        let back = super::import_config(path).unwrap();
        assert!(back.folders[0].recursive);
        assert!(super::import_config(dir.path().join("missing.toml")).is_err());
    }
}
