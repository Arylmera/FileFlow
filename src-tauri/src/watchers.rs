//! FSEvents watchers for the two flows, plus the orchestration they trigger.
//!
//! Each flow runs on a single dedicated worker thread fed by an mpsc channel, which
//! gives re-entrancy safety for free: one event can't start an overlapping run.

use crate::state::{ActivityEntry, AppState, RunRecord};
use crate::volume;
use fileflow_core::config::{AlbumMode, CardRule, CleanupPolicy, Destination, EjectPolicy, FolderRule};
use fileflow_core::folder;
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
    _folders: Vec<notify::RecommendedWatcher>,
}

#[derive(Clone, Serialize)]
struct CardReady {
    uuid: String,
    label: String,
    volume_root: PathBuf,
    dates: Vec<DateGroup>,
}

#[derive(Clone, Serialize)]
struct PhotosReady {
    index: usize,
    dates: Vec<DateGroup>,
}

/// Per-file progress for the copy (drive) and move (folder) flows.
#[derive(Clone, Serialize)]
struct Progress {
    flow: &'static str,
    label: String,
    done: usize,
    total: usize,
}

/// Emit a `progress` event. (ponytail: one event per file; add throttling only if a
/// huge card visibly floods the UI.)
fn emit_progress(app: &AppHandle, flow: &'static str, label: &str, done: usize, total: usize) {
    let _ = app.emit("progress", Progress { flow, label: label.to_string(), done, total });
}

/// Set up the watchers and their worker threads. Call once, after [`AppState`] is managed.
///
/// Note: folder watchers bind to their configured paths at startup; adding, removing,
/// or re-pointing a folder rule needs an app restart to re-bind.
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

    // --- Folder watchers (one per configured rule, either kind) ---
    let mut folder_watchers = Vec::new();
    for (i, rule) in app.state::<AppState>().snapshot().folders.iter().enumerate() {
        let folder = ingest::expand(&rule.watch);
        if !folder.is_dir() {
            continue;
        }
        let (ftx, frx) = mpsc::channel::<()>();
        let mut w = notify::recommended_watcher(move |_res| {
            let _ = ftx.send(());
        })?;
        w.watch(&folder, RecursiveMode::NonRecursive)?;
        let h = handle.clone();
        std::thread::spawn(move || folder_worker(h, i, frx));
        folder_watchers.push(w);
    }

    app.manage(Mutex::new(WatcherHandles {
        _volumes: vol_watcher,
        _folders: folder_watchers,
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

/// Handle the folder rule at `index` after 3s of quiet (a Lightroom/Finder write burst).
fn folder_worker(app: AppHandle, index: usize, rx: mpsc::Receiver<()>) {
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
        run_now(&app, index);
    }
}

/// Run the folder rule at `index`, dispatching on its destination kind. Public for
/// the manual "Run now" trigger; a Photos rule with name-prompting surfaces the UI form.
pub fn run_now(app: &AppHandle, index: usize) {
    let Some(rule) = app.state::<AppState>().snapshot().folders.get(index).cloned() else {
        return;
    };
    match rule.target {
        Destination::Folder { .. } => run_folder_move(app, &rule),
        Destination::Photos { .. } => run_photos_flow(app, index, &rule),
    }
}

/// Move the watched folder's contents into its dated destination.
fn run_folder_move(app: &AppHandle, rule: &FolderRule) {
    let Destination::Folder { dest, .. } = &rule.target else {
        return;
    };
    let label = if rule.label.is_empty() {
        "Folder".to_string()
    } else {
        rule.label.clone()
    };
    match folder::run_folder_move(rule, |done, total| emit_progress(app, "folder", &label, done, total)) {
        Ok(report) => {
            if report.moved.is_empty() && report.failed.is_empty() {
                return;
            }
            let msg = format!(
                "{}: moved {} file(s), {} failed → {}",
                label,
                report.moved.len(),
                report.failed.len(),
                dest
            );
            notify(app, "FileFlow — Folder move", &msg);
            emit_activity(app, "folder", &msg);
            record_run(
                app,
                "folder",
                format!("folder:{}", rule.watch),
                &label,
                rule.watch.clone(),
                dest.clone(),
                report.moved.len(),
                0,
                report.failed.len(),
                run_status(report.moved.len(), report.failed.len()),
                msg,
            );
        }
        Err(e) => {
            notify(app, "FileFlow — destination unavailable", &e.to_string());
            emit_activity(app, "folder", &format!("{label}: {e}"));
            record_run(
                app,
                "folder",
                format!("folder:{}", rule.watch),
                &label,
                rule.watch.clone(),
                dest.clone(),
                0,
                0,
                0,
                "failed",
                e.to_string(),
            );
        }
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
    let dest = match ingest::resolve_dest(&rule, volume_root) {
        Ok(d) => d,
        Err(e) => {
            notify(app, "FileFlow — destination unavailable", &e.to_string());
            emit_activity(app, "drive", &format!("{}: {e}", rule.label));
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
        // Distinguish "empty card" from "macOS blocked us reading the card".
        if fda_blocked(volume_root) {
            notify(
                app,
                "FileFlow — Full Disk Access needed",
                "Grant access in System Settings ▸ Privacy & Security ▸ Full Disk Access, then reconnect the drive.",
            );
            emit_activity(app, "drive", "blocked: needs Full Disk Access");
        }
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
        crate::show_main(app);
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
    let report = ingest::run_ingest(&plan, |done, total| {
        emit_progress(app, "drive", &rule.label, done, total)
    });
    let (c, s, f) = (report.copied.len(), report.skipped.len(), report.failed.len());
    let summary = format!("{c} copied, {s} skipped, {f} failed → {}", rule.dest);
    notify(app, &format!("FileFlow — {}", rule.label), &summary);
    emit_activity(app, "drive", &summary);
    record_run(
        app,
        "drive",
        format!("card:{}", rule.uuid),
        &rule.label,
        rule.sources.join(", "),
        rule.dest.clone(),
        c,
        s,
        f,
        run_status(c + s, f),
        summary.clone(),
    );

    if !report.is_clean() {
        // All-or-nothing: any failure aborts both cleanup and eject. Card untouched.
        return;
    }

    match rule.cleanup {
        CleanupPolicy::Always => match ingest::cleanup(&report) {
            Ok(c) => {
                emit_activity(app, "drive", &format!("deleted {} file(s) from drive", c.deleted.len()));
                if !c.failed.is_empty() {
                    emit_activity(app, "drive", &format!("{} file(s) could not be removed from drive", c.failed.len()));
                }
            }
            Err(e) => emit_activity(app, "drive", &format!("cleanup error: {e}")),
        },
        CleanupPolicy::Never => {}
        CleanupPolicy::Ask => {
            // Confirmation dialog is Phase 5; until then keep the card intact and stop
            // before ejecting (the user may still want the card mounted).
            let _ = app.emit("cleanup-pending", &summary);
            emit_activity(app, "drive", "cleanup needs confirmation (drive kept intact)");
            return;
        }
    }

    match rule.eject {
        EjectPolicy::Always => match ingest::eject(volume_root, EjectPolicy::Always) {
            Ok(_) => emit_activity(app, "drive", "drive ejected"),
            Err(e) => notify(app, "FileFlow — eject failed", &e.to_string()),
        },
        EjectPolicy::Never => {}
        EjectPolicy::Ask => {
            let _ = app.emit("eject-pending", ());
        }
    }
}

/// Photos-flow entry point: prompt for names if configured, otherwise import now.
fn run_photos_flow(app: &AppHandle, index: usize, rule: &FolderRule) {
    let Destination::Photos { album_mode, prompt_name, .. } = &rule.target else {
        return;
    };
    let files = photos::scan_folder(&ingest::expand(&rule.watch), &rule.extensions);
    if files.is_empty() {
        return;
    }
    // By-date album with a name prompt → hand the dates to the UI naming form.
    if *album_mode == AlbumMode::Template && *prompt_name {
        let dates = photos::date_groups(&files);
        if dates.is_empty() {
            return;
        }
        let _ = app.emit("photos-ready", PhotosReady { index, dates });
        crate::show_main(app);
        return;
    }
    do_photos_import(app, rule, &files, &BTreeMap::new());
}

/// Import the watched folder at `index` with a date→name map (the confirmed naming form).
pub fn run_photos_import_named(app: &AppHandle, index: usize, names: &BTreeMap<String, String>) {
    let Some(rule) = app.state::<AppState>().snapshot().folders.get(index).cloned() else {
        return;
    };
    let files = photos::scan_folder(&ingest::expand(&rule.watch), &rule.extensions);
    if !files.is_empty() {
        do_photos_import(app, &rule, &files, names);
    }
}

fn do_photos_import(
    app: &AppHandle,
    rule: &FolderRule,
    files: &[PathBuf],
    names: &BTreeMap<String, String>,
) {
    let Destination::Photos {
        album_mode,
        photos_album,
        skip_duplicates,
        after_import,
        archive_folder,
        ..
    } = &rule.target
    else {
        return;
    };
    let target = match album_mode {
        AlbumMode::Library => photos::AlbumTarget::Library,
        AlbumMode::Fixed => photos::AlbumTarget::Fixed(photos_album.clone()),
        AlbumMode::Template => photos::AlbumTarget::Template {
            template: photos_album.clone(),
            names: names.clone(),
        },
    };
    match photos::import_to_photos(files, &target, *skip_duplicates) {
        Ok(rep) => {
            let msg = format!("imported {} file(s) → {}", rep.imported, rep.album);
            notify(app, "FileFlow — Photos", &msg);
            emit_activity(app, "photos", &msg);
            record_run(
                app,
                "photos",
                format!("folder:{}", rule.watch),
                &rule.label,
                rule.watch.clone(),
                "Apple Photos".to_string(),
                rep.imported,
                0,
                0,
                run_status(rep.imported, 0),
                msg,
            );
            let archive = ingest::expand(archive_folder);
            if let Err(e) = photos::after_import(files, *after_import, &archive) {
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
            record_run(
                app,
                "photos",
                format!("folder:{}", rule.watch),
                &rule.label,
                rule.watch.clone(),
                "Apple Photos".to_string(),
                0,
                0,
                files.len(),
                "failed",
                "Photos not authorized (Automation)".to_string(),
            );
        }
        Err(e) => {
            notify(app, "FileFlow — Photos import failed", &e.to_string());
            emit_activity(app, "photos", &format!("error: {e}"));
            record_run(
                app,
                "photos",
                format!("folder:{}", rule.watch),
                &rule.label,
                rule.watch.clone(),
                "Apple Photos".to_string(),
                0,
                0,
                files.len(),
                "failed",
                e.to_string(),
            );
        }
    }
}

/// Heuristic: a `PermissionDenied` reading the card root or its DCIM folder means
/// macOS is withholding Full Disk Access, not that the card is empty.
fn fda_blocked(volume_root: &Path) -> bool {
    [volume_root.to_path_buf(), volume_root.join("DCIM")]
        .iter()
        .any(|p| {
            std::fs::read_dir(p)
                .err()
                .map(|e| e.kind() == std::io::ErrorKind::PermissionDenied)
                .unwrap_or(false)
        })
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}

/// `ok` when nothing failed, `failed` when nothing got through, else `partial`.
fn run_status(progressed: usize, failed: usize) -> &'static str {
    if failed > 0 && progressed == 0 {
        "failed"
    } else if failed > 0 {
        "partial"
    } else {
        "ok"
    }
}

/// Persist a completed run and emit a `run` event so an open Flow map refreshes live.
#[allow(clippy::too_many_arguments)]
fn record_run(
    app: &AppHandle,
    flow: &str,
    rule_key: String,
    label: &str,
    source: String,
    dest: String,
    ok: usize,
    skipped: usize,
    failed: usize,
    status: &str,
    detail: String,
) {
    let rec = RunRecord {
        ts: chrono::Local::now().to_rfc3339(),
        flow: flow.to_string(),
        rule_key,
        label: label.to_string(),
        source,
        dest,
        ok,
        skipped,
        failed,
        status: status.to_string(),
        detail,
    };
    app.state::<AppState>().push_run(rec.clone());
    let _ = app.emit("run", rec);
}

fn emit_activity(app: &AppHandle, flow: &str, message: &str) {
    tracing::info!(flow, "{message}");
    let entry = ActivityEntry {
        flow: flow.to_string(),
        message: message.to_string(),
        ts: chrono::Local::now().format("%H:%M:%S").to_string(),
    };
    app.state::<AppState>().push_activity(entry.clone());
    let _ = app.emit("activity", entry);
}
