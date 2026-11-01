//! FSEvents watchers for the two flows, plus the orchestration they trigger.
//!
//! Each flow runs on a single dedicated worker thread fed by an mpsc channel, which
//! gives re-entrancy safety for free: one event can't start an overlapping run.

use crate::state::{ActivityEntry, AppState, PendingDrive, RunRecord};
use crate::volume;
use fileflow_core::config::{AlbumMode, CardRule, CleanupPolicy, Destination, EjectPolicy, FolderRule};
use fileflow_core::folder;
use fileflow_core::ingest::{self, DateGroup};
use fileflow_core::photos;
use fileflow_core::util::NameFilter;
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
/// `_volumes` is `None` on Windows, where drive arrivals are polled instead of watched.
struct WatcherHandles {
    _volumes: Option<notify::RecommendedWatcher>,
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
/// Folder watchers bind to their configured paths here and are re-bound on every
/// `save_config` via [`rebind_folders`], so adding/removing/re-pointing a folder rule
/// takes effect live without an app restart.
pub fn start(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();

    // Drive-arrival detection feeds the same diffing worker on every platform; only
    // the poke source differs. macOS watches `/Volumes` (FSEvents); Windows has no
    // mount directory to watch, so a thread polls the drive list on a short interval.
    let (vtx, vrx) = mpsc::channel::<()>();

    #[cfg(target_os = "macos")]
    let vol_watcher = {
        let mut w = notify::recommended_watcher(move |_res| {
            let _ = vtx.send(());
        })?;
        w.watch(Path::new("/Volumes"), RecursiveMode::NonRecursive)?;
        Some(w)
    };

    #[cfg(not(target_os = "macos"))]
    let vol_watcher = {
        #[cfg(target_os = "windows")]
        std::thread::spawn(move || {
            // Exits when the receiver is dropped (app shutdown).
            while vtx.send(()).is_ok() {
                // Each poke shells out to PowerShell for the volume list, so poll
                // gently — a card inserted between polls is picked up a few seconds
                // later, which is invisible next to the copy itself.
                std::thread::sleep(Duration::from_secs(5));
            }
        });
        #[cfg(not(target_os = "windows"))]
        drop(vtx); // no drive source on this platform; the worker idles forever
        None
    };

    {
        let h = handle.clone();
        std::thread::spawn(move || volume_worker(h, vrx));
    }

    app.manage(Mutex::new(WatcherHandles {
        _volumes: vol_watcher,
        _folders: build_folder_watchers(&handle),
    }));
    Ok(())
}

/// Build one folder watcher + worker per configured rule whose watch path exists.
/// Dropping a returned watcher closes its worker's channel, so the worker exits —
/// which is how [`rebind_folders`] tears down the old set.
fn build_folder_watchers(app: &AppHandle) -> Vec<notify::RecommendedWatcher> {
    let mut watchers = Vec::new();
    for (i, rule) in app.state::<AppState>().snapshot().folders.iter().enumerate() {
        let folder = ingest::expand(&rule.watch);
        if !folder.is_dir() {
            continue;
        }
        let (ftx, frx) = mpsc::channel::<()>();
        let mut w = match notify::recommended_watcher(move |_res| {
            let _ = ftx.send(());
        }) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!("folder watcher create failed for {}: {e}", rule.watch);
                continue;
            }
        };
        if let Err(e) = w.watch(&folder, RecursiveMode::NonRecursive) {
            tracing::warn!("folder watch failed for {}: {e}", rule.watch);
            continue;
        }
        let h = app.clone();
        std::thread::spawn(move || folder_worker(h, i, frx));
        watchers.push(w);
    }
    watchers
}

/// Re-bind folder watchers from the current config (call after `save_config`). Replacing
/// the stored handles drops the old watchers, which stops their workers; new workers pick
/// up the updated rule list. ponytail: a poke arriving mid-swap could double-run a rule,
/// but a re-move just finds nothing — harmless, so no cross-thread coordination.
pub fn rebind_folders(app: &AppHandle) {
    let fresh = build_folder_watchers(app);
    if let Some(handles) = app.try_state::<Mutex<WatcherHandles>>() {
        handles.lock().unwrap()._folders = fresh;
    }
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
    let run = RunTarget::folder(rule);
    match folder::run_folder_move(rule, |done, total| emit_progress(app, "folder", &run.label, done, total)) {
        Ok(report) => {
            let (moved, failed) = (report.moved.len(), report.failed.len());
            if moved == 0 && failed == 0 {
                return;
            }
            let msg = format!("{}: moved {moved} file(s), {failed} failed → {}", run.label, run.dest);
            notify(app, "FileFlow — Folder move", &msg);
            emit_activity(app, "folder", &msg);
            run.finished(app, moved, 0, failed, msg);
        }
        Err(e) => {
            notify(app, "FileFlow — destination unavailable", &e.to_string());
            emit_activity(app, "folder", &format!("{}: {e}", run.label));
            run.failed(app, 0, e.to_string());
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
    let Some(rule) = app.state::<AppState>().snapshot().card(&uuid).cloned() else {
        return; // unknown volume — ignore silently
    };

    // Pre-flight the destination before reading anything off the card.
    let dest = match ingest::resolve_dest(&rule, volume_root) {
        Ok(d) => d,
        Err(e) => {
            notify(app, "FileFlow — destination unavailable", &e.to_string());
            emit_activity(app, "drive", &format!("{}: {e}", rule.label));
            RunTarget::card(&rule).failed(app, 0, e.to_string());
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
        // (Full Disk Access is a macOS-only concept; Windows reads cards directly.)
        #[cfg(target_os = "macos")]
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
        // Naming is the UI's job: hand it the dates and bring the window forward.
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

/// Copy + verify, then cleanup/eject per policy. Public so `run_ingest_now` can call it
/// once the UI has collected per-date names.
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
    RunTarget::card(rule).finished(app, c, s, f, summary.clone());

    if !report.is_clean() {
        // All-or-nothing: any failure aborts both cleanup and eject. Card untouched.
        return;
    }

    if rule.cleanup == CleanupPolicy::Always {
        erase_drive(app, &report);
    }

    let ask_erase = rule.cleanup == CleanupPolicy::Ask;
    let ask_eject = rule.eject == EjectPolicy::Ask;
    if ask_erase || ask_eject {
        // Park the verified import and ask; `finish_drive` acts on the answer.
        let pending = DrivePending {
            uuid: rule.uuid.clone(),
            label: rule.label.clone(),
            files: report.deletable_sources().count(),
            ask_erase,
            ask_eject,
        };
        app.state::<AppState>().pending.lock().unwrap().insert(
            rule.uuid.to_lowercase(),
            PendingDrive { rule: rule.clone(), volume_root: volume_root.to_path_buf(), report },
        );
        let _ = app.emit("drive-pending", pending);
        crate::show_main(app);
        return;
    }

    if rule.eject == EjectPolicy::Always {
        eject_drive(app, volume_root);
    }
}

/// Event payload: a drive imported cleanly and its rule says "ask first" to erase and/or eject.
#[derive(Clone, Serialize)]
struct DrivePending {
    uuid: String,
    label: String,
    files: usize, // sources an erase would remove (copied + already present)
    ask_erase: bool,
    ask_eject: bool,
}

/// Apply the user's answer to a pending drive. Validates synchronously (so the UI can
/// report a problem), then erases/ejects on a worker thread.
///
/// Answers only count for the questions the rule asked: `erase` is ignored unless the
/// rule's cleanup is "ask", and an "always" eject still happens whatever `eject` says.
/// Before touching the volume we re-check its UUID, so a different drive remounted at
/// the same path can never be erased with another card's file list.
pub fn finish_drive(app: &AppHandle, uuid: &str, erase: bool, eject: bool) -> Result<(), String> {
    let p = app
        .state::<AppState>()
        .pending
        .lock()
        .unwrap()
        .remove(&uuid.to_lowercase())
        .ok_or("this drive has no pending import")?;
    let (erase, eject) = effective_answers(&p.rule, erase, eject);
    if !erase && !eject {
        emit_activity(app, "drive", &format!("{}: files kept on the drive", p.rule.label));
        return Ok(());
    }
    let same_drive = volume::volume_uuid(&p.volume_root).is_some_and(|u| u.eq_ignore_ascii_case(uuid));
    if !same_drive {
        return Err("The drive is no longer connected, so nothing was erased or ejected.".into());
    }
    let app = app.clone();
    std::thread::spawn(move || {
        if erase {
            erase_drive(&app, &p.report);
        }
        if eject {
            eject_drive(&app, &p.volume_root);
        }
    });
    Ok(())
}

/// What to actually do with a pending drive: the user's answer only counts where the rule
/// asked. Erase needs cleanup = "ask" (an "always" erase already ran; "never" never does);
/// eject follows the policy unless it is "ask".
fn effective_answers(rule: &CardRule, erase: bool, eject: bool) -> (bool, bool) {
    let erase = erase && rule.cleanup == CleanupPolicy::Ask;
    let eject = match rule.eject {
        EjectPolicy::Always => true,
        EjectPolicy::Ask => eject,
        EjectPolicy::Never => false,
    };
    (erase, eject)
}

/// Delete an import's verified sources from the drive (`ingest::cleanup` re-checks the
/// all-or-nothing gate itself) and log the outcome.
fn erase_drive(app: &AppHandle, report: &ingest::IngestReport) {
    match ingest::cleanup(report) {
        Ok(c) => {
            emit_activity(app, "drive", &format!("erased {} file(s) from the drive", c.deleted.len()));
            if !c.failed.is_empty() {
                emit_activity(app, "drive", &format!("{} file(s) could not be erased from the drive", c.failed.len()));
            }
        }
        Err(e) => emit_activity(app, "drive", &format!("erase skipped: {e}")),
    }
}

fn eject_drive(app: &AppHandle, volume_root: &Path) {
    match ingest::eject(volume_root, EjectPolicy::Always) {
        Ok(_) => emit_activity(app, "drive", "drive ejected"),
        Err(e) => notify(app, "FileFlow — eject failed", &e.to_string()),
    }
}

/// Scan a Photos rule's watch folder through its filters.
fn photos_files(rule: &FolderRule) -> Vec<PathBuf> {
    let filter = NameFilter::compile_or_deny(&rule.include, &rule.exclude);
    photos::scan_folder(&ingest::expand(&rule.watch), &rule.extensions, &filter)
}

/// Photos-flow entry point: prompt for names if configured, otherwise import now.
fn run_photos_flow(app: &AppHandle, index: usize, rule: &FolderRule) {
    let Destination::Photos { album_mode, prompt_name, .. } = &rule.target else {
        return;
    };
    let files = photos_files(rule);
    if files.is_empty() {
        return;
    }
    // By-date album with a name prompt → hand the dates to the UI naming form.
    if *album_mode == AlbumMode::Template && *prompt_name {
        let dates = ingest::date_groups(&files);
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
    let files = photos_files(&rule);
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
    let run = RunTarget::folder(rule);
    match photos::import_to_photos(files, &target, *skip_duplicates) {
        Ok(rep) => {
            let msg = format!("imported {} file(s) → {}", rep.imported, rep.album);
            notify(app, "FileFlow — Photos", &msg);
            emit_activity(app, "photos", &msg);
            run.finished(app, rep.imported, 0, 0, msg);
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
            run.failed(app, files.len(), "Photos not authorized (Automation)".into());
        }
        Err(e) => {
            notify(app, "FileFlow — Photos import failed", &e.to_string());
            emit_activity(app, "photos", &format!("error: {e}"));
            run.failed(app, files.len(), e.to_string());
        }
    }
}

/// Heuristic: a `PermissionDenied` reading the card root or its DCIM folder means
/// macOS is withholding Full Disk Access, not that the card is empty.
#[cfg(target_os = "macos")]
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

/// The rule a run belongs to — the fields every [`RunRecord`] for one rule shares.
struct RunTarget {
    flow: &'static str,
    rule_key: String, // joins a run back to its rule in the Flow map
    label: String,
    source: String,
    dest: String,
}

impl RunTarget {
    fn card(rule: &CardRule) -> Self {
        RunTarget {
            flow: "drive",
            rule_key: format!("card:{}", rule.uuid),
            label: rule.label.clone(),
            source: rule.sources.join(", "),
            dest: rule.dest.clone(),
        }
    }

    fn folder(rule: &FolderRule) -> Self {
        let (flow, dest) = match &rule.target {
            Destination::Folder { dest, .. } => ("folder", dest.clone()),
            Destination::Photos { .. } => ("photos", "Apple Photos".to_string()),
        };
        let label = if rule.label.is_empty() { "Folder".to_string() } else { rule.label.clone() };
        RunTarget { flow, rule_key: format!("folder:{}", rule.watch), label, source: rule.watch.clone(), dest }
    }

    /// Record a run that went through. Status: `ok` when nothing failed, `failed` when
    /// nothing got through, else `partial`. A no-op re-scan (nothing done, nothing failed)
    /// isn't recorded: it would reset the lane's "last run" time while the count stays
    /// cumulative, so a no-op re-import would read as "44 files imported · just now".
    fn finished(&self, app: &AppHandle, ok: usize, skipped: usize, failed: usize, detail: String) {
        if ok == 0 && failed == 0 {
            return;
        }
        let status = match (ok + skipped, failed) {
            (_, 0) => "ok",
            (0, _) => "failed",
            _ => "partial",
        };
        self.record(app, ok, skipped, failed, status, detail);
    }

    /// Record a run that couldn't complete (destination unavailable, Photos refused).
    /// Always recorded, so the Flow map flags the rule as needing attention.
    fn failed(&self, app: &AppHandle, failed: usize, detail: String) {
        self.record(app, 0, 0, failed, "failed", detail);
    }

    /// Persist the run and emit a `run` event so an open Flow map refreshes live.
    fn record(&self, app: &AppHandle, ok: usize, skipped: usize, failed: usize, status: &str, detail: String) {
        let rec = RunRecord {
            ts: chrono::Local::now().to_rfc3339(),
            flow: self.flow.to_string(),
            rule_key: self.rule_key.clone(),
            label: self.label.clone(),
            source: self.source.clone(),
            dest: self.dest.clone(),
            ok,
            skipped,
            failed,
            status: status.to_string(),
            detail,
        };
        app.state::<AppState>().push_run(rec.clone());
        let _ = app.emit("run", rec);
    }
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

#[cfg(test)]
mod tests {
    use super::effective_answers;
    use fileflow_core::config::{CardRule, CleanupPolicy, EjectPolicy};

    fn rule(cleanup: CleanupPolicy, eject: EjectPolicy) -> CardRule {
        let mut r: CardRule = serde_json::from_value(serde_json::json!({
            "uuid": "U", "sources": ["DCIM"], "dest": "/tmp"
        }))
        .unwrap();
        r.cleanup = cleanup;
        r.eject = eject;
        r
    }

    #[test]
    fn answers_only_count_where_the_rule_asked() {
        use CleanupPolicy as C;
        use EjectPolicy as E;
        // Asked both: the answer decides.
        assert_eq!(effective_answers(&rule(C::Ask, E::Ask), true, false), (true, false));
        assert_eq!(effective_answers(&rule(C::Ask, E::Ask), false, true), (false, true));
        // Erase is never honoured unless asked — "never" stays never, "always" already ran.
        assert_eq!(effective_answers(&rule(C::Never, E::Ask), true, true), (false, true));
        assert_eq!(effective_answers(&rule(C::Always, E::Ask), true, false), (false, false));
        // A fixed eject policy wins over the answer.
        assert_eq!(effective_answers(&rule(C::Ask, E::Always), false, false), (false, true));
        assert_eq!(effective_answers(&rule(C::Ask, E::Never), true, true), (true, false));
    }
}
