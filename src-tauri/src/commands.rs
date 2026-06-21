//! Thin, typed IPC surface over the core engine + watchers.

use crate::state::{ActivityEntry, AppState};
use crate::{volume, watchers};
use fileflow_core::config::Config;
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
}

#[tauri::command]
pub fn get_config(state: State<AppState>) -> Config {
    state.snapshot()
}

#[tauri::command]
pub fn save_config(state: State<AppState>, config: Config) -> Result<(), String> {
    config.save(&state.config_path).map_err(|e| e.to_string())?;
    *state.config.lock().unwrap() = config;
    Ok(())
}

#[tauri::command]
pub fn list_mounted_cards(state: State<AppState>) -> Vec<MountedCard> {
    let cfg = state.snapshot();
    let mut out: Vec<MountedCard> = volume::mounted_volumes()
        .into_iter()
        .map(|path| {
            let uuid = volume::volume_uuid(&path);
            let rule = uuid
                .as_ref()
                .and_then(|u| cfg.cards.iter().find(|c| c.uuid.eq_ignore_ascii_case(u)));
            MountedCard {
                label: path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string(),
                path,
                uuid,
                matched: rule.is_some(),
                rule_label: rule.map(|r| r.label.clone()),
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
    let cfg = state.snapshot();
    let rule = cfg
        .cards
        .iter()
        .find(|c| c.uuid.eq_ignore_ascii_case(&uuid))
        .cloned()
        .ok_or("no drive rule matches that UUID")?;
    let volume_root = volume::find_volume_by_uuid(&uuid).ok_or("drive not connected")?;
    ingest::resolve_dest(&rule).map_err(|e| e.to_string())?;
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
    let cfg = app.state::<AppState>().snapshot();
    let rule = cfg
        .cards
        .iter()
        .find(|c| c.uuid.eq_ignore_ascii_case(&uuid))
        .cloned()
        .ok_or("no drive rule matches that UUID")?;
    let volume_root = volume::find_volume_by_uuid(&uuid).ok_or("drive not connected")?;
    let dest = ingest::resolve_dest(&rule).map_err(|e| e.to_string())?;
    std::thread::spawn(move || {
        watchers::run_card_ingest(&app, &rule, &volume_root, &names, &dest);
    });
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

/// Reveal a path in Finder (`~` expanded). Used by Settings' "Open config".
#[tauri::command]
pub fn reveal_in_finder(path: String) -> Result<(), String> {
    let p = ingest::expand(&path);
    std::process::Command::new("open")
        .arg("-R")
        .arg(&p)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn log_path(app: AppHandle) -> Result<String, String> {
    let dir = app.path().app_log_dir().map_err(|e| e.to_string())?;
    Ok(dir.join("fileflow.log").to_string_lossy().to_string())
}

