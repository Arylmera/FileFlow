//! FSEvents watchers for the two flows, plus the orchestration they trigger.
//!
//! Each flow runs on a single dedicated worker thread fed by an mpsc channel, which
//! gives re-entrancy safety for free: one event can't start an overlapping run.

use crate::state::{ActivityEntry, AppState};
use crate::volume;
use fileflow_core::config::{CardRule, CleanupPolicy, EjectPolicy};
use fileflow_core::ingest::{self, DateGroup};
use fileflow_core::photos;
use notify::{RecursiveMode, Watcher};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

/// Kept in managed state purely to keep the watchers alive for the app's lifetime.
struct WatcherHandles {
    _volumes: notify::RecommendedWatcher,
    _export: Option<notify::RecommendedWatcher>,
}

#[derive(Clone, Serialize)]
struct CardReady {
    uuid: String,
    label: String,
    volume_root: PathBuf,
    dates: Vec<DateGroup>,
}

/// Set up both watchers and their worker threads. Call once, after [`AppState`] is managed.
///
/// Note: the export watcher binds to the configured folder at startup; changing
/// `lightroom.watch_folder` later needs an app restart to re-bind (revisited in Phase 5).
pub fn start(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();

    // --- /Volumes watcher: poke a worker that diffs the mount list ---
    let (vtx, vrx) = mpsc::channel::<()>();
    let mut vol_watcher = notify::recommended_watcher(move |_res| {
        let _ = vtx.send(());
    })?;
    vol_watcher.watch(Path::new("/Volumes"), RecursiveMode::NonRecursive)?;
    {
        let h = handle.clone();
        std::thread::spawn(move || volume_worker(h, vrx));
    }

    // --- Lightroom export watcher (only if configured and present) ---
    let export_watcher = {
        let folder = app
            .state::<AppState>()
            .snapshot()
            .lightroom
            .as_ref()
            .map(|l| ingest::expand(&l.watch_folder));
        match folder {
            Some(folder) if folder.is_dir() => {
                let (etx, erx) = mpsc::channel::<()>();
                let mut w = notify::recommended_watcher(move |_res| {
                    let _ = etx.send(());
                })?;
                w.watch(&folder, RecursiveMode::NonRecursive)?;
                let h = handle.clone();
                std::thread::spawn(move || export_worker(h, erx));
                Some(w)
            }
            _ => None,
        }
    };

    app.manage(Mutex::new(WatcherHandles {
        _volumes: vol_watcher,
        _export: export_watcher,
    }));
    Ok(())
}

/// Diff `/Volumes` on each poke; newly-mounted volumes are handled once.
fn volume_worker(app: AppHandle, rx: mpsc::Receiver<()>) {
    let mut seen = volume::mounted_volumes(); // seed: ignore already-mounted volumes
    while rx.recv().is_ok() {
        while rx.try_recv().is_ok() {} // coalesce an FSEvents burst
        std::thread::sleep(Duration::from_millis(1500)); // let the mount settle
        let now = volume::mounted_volumes();
        for v in now.difference(&seen).cloned().collect::<Vec<_>>() {
            handle_volume(&app, &v);
        }
        seen = now;
    }
}

/// Import after 3s of quiet (Lightroom writes a burst of files).
fn export_worker(app: AppHandle, rx: mpsc::Receiver<()>) {
    loop {
        if rx.recv().is_err() {
            break;
        }
        loop {
            match rx.recv_timeout(Duration::from_secs(3)) {
                Ok(()) => continue,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        if app.state::<AppState>().is_paused() {
            continue;
        }
        run_photos_flow(&app);
    }
}

fn handle_volume(app: &AppHandle, volume_root: &Path) {
    if app.state::<AppState>().is_paused() {
        return;
    }
    let Some(uuid) = volume::volume_uuid(volume_root) else {
        return;
    };
    let cfg = app.state::<AppState>().snapshot();
    let Some(rule) = cfg
        .cards
        .iter()
        .find(|c| c.uuid.eq_ignore_ascii_case(&uuid))
        .cloned()
    else {
        return; // unknown volume — ignore silently
    };

    // Pre-flight the destination before reading anything off the card.
    let dest = match ingest::resolve_dest(&rule) {
        Ok(d) => d,
        Err(e) => {
            notify(app, "FileFlow — destination unavailable", &e.to_string());
            emit_activity(app, "card", &format!("{}: {e}", rule.label));
            return;
        }
    };

    // Settle: a card's source folder can appear slightly after the mount event.
    let mut dates = Vec::new();
    for _ in 0..5 {
        dates = ingest::scan_dates(&rule, volume_root);
        if !dates.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    if dates.is_empty() {
        return;
    }

    if rule.prompt_name {
        // The naming form is the UI's job (Phase 5): hand it the dates and surface us.
        let _ = app.emit(
            "card-ready",
            CardReady {
                uuid,
                label: rule.label.clone(),
                volume_root: volume_root.to_path_buf(),
                dates,
            },
        );
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.show();
            let _ = w.set_focus();
        }
        return;
    }

    run_card_ingest(app, &rule, volume_root, &BTreeMap::new(), &dest);
}

/// Copy + verify, then cleanup/eject per policy. Public so the Phase 4 command can
/// call it once the UI has collected per-date names.
pub fn run_card_ingest(
    app: &AppHandle,
    rule: &CardRule,
    volume_root: &Path,
    names: &BTreeMap<String, String>,
    dest: &Path,
) {
    let plan = ingest::plan_ingest(rule, volume_root, names, dest);
    let report = ingest::run_ingest(&plan);
    let (c, s, f) = (report.copied.len(), report.skipped.len(), report.failed.len());
    let summary = format!("{c} copied, {s} skipped, {f} failed → {}", rule.dest);
    notify(app, &format!("FileFlow — {}", rule.label), &summary);
    emit_activity(app, "card", &summary);

    if !report.is_clean() {
        // All-or-nothing: any failure aborts both cleanup and eject. Card untouched.
        return;
    }

    match rule.cleanup {
        CleanupPolicy::Always => match ingest::cleanup(&report) {
            Ok(d) => emit_activity(app, "card", &format!("deleted {} file(s) from card", d.len())),
            Err(e) => emit_activity(app, "card", &format!("cleanup error: {e}")),
        },
        CleanupPolicy::Never => {}
        CleanupPolicy::Ask => {
            // Confirmation dialog is Phase 5; until then keep the card intact and stop
            // before ejecting (the user may still want the card mounted).
            let _ = app.emit("cleanup-pending", &summary);
            emit_activity(app, "card", "cleanup needs confirmation (card kept intact)");
            return;
        }
    }

    match rule.eject {
        EjectPolicy::Always => match ingest::eject(volume_root, EjectPolicy::Always) {
            Ok(_) => emit_activity(app, "card", "card ejected"),
            Err(e) => notify(app, "FileFlow — eject failed", &e.to_string()),
        },
        EjectPolicy::Never => {}
        EjectPolicy::Ask => {
            let _ = app.emit("eject-pending", ());
        }
    }
}

/// Scan the export folder and import new files into Photos. Public for the Phase 4 trigger.
pub fn run_photos_flow(app: &AppHandle) {
    let Some(lr) = app.state::<AppState>().snapshot().lightroom else {
        return;
    };
    let folder = ingest::expand(&lr.watch_folder);
    let files = photos::scan_folder(&folder, &lr.extensions);
    if files.is_empty() {
        return;
    }
    match photos::import_to_photos(&files, &lr.photos_album, lr.skip_duplicates) {
        Ok(rep) => {
            let msg = format!("imported {} file(s) → album \"{}\"", rep.imported, rep.album);
            notify(app, "FileFlow — Photos", &msg);
            emit_activity(app, "photos", &msg);
            let archive = ingest::expand(&lr.archive_folder);
            if let Err(e) = photos::after_import(&files, lr.after_import, &archive) {
                emit_activity(app, "photos", &format!("after-import error: {e}"));
            }
        }
        Err(fileflow_core::Error::PhotosNotAuthorized) => {
            notify(
                app,
                "FileFlow — Photos not authorized",
                "Grant access in System Settings ▸ Privacy & Security ▸ Automation.",
            );
            emit_activity(app, "photos", "not authorized (Automation)");
        }
        Err(e) => {
            notify(app, "FileFlow — Photos import failed", &e.to_string());
            emit_activity(app, "photos", &format!("error: {e}"));
        }
    }
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}

fn emit_activity(app: &AppHandle, flow: &str, message: &str) {
    let entry = ActivityEntry {
        flow: flow.to_string(),
        message: message.to_string(),
        ts: chrono::Local::now().format("%H:%M:%S").to_string(),
    };
    app.state::<AppState>().push_activity(entry.clone());
    let _ = app.emit("activity", entry);
}
