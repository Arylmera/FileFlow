//! Card ingest engine: scan → plan → copy+verify → cleanup → eject.
//!
//! The safety spine (locked decision §1): copy + verify the *entire* set first; only
//! if every file verified does deletion run. [`cleanup`] re-checks this itself, so the
//! all-or-nothing guarantee holds even if a caller forgets to.

use crate::config::{CardRule, EjectPolicy};
use crate::util::{ext_matches, is_hidden};
use crate::layout;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

/// A distinct capture date and how many files fall on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DateGroup {
    pub date: String, // YYYY-MM-DD
    pub year: String, // YYYY
    pub file_count: usize,
}

/// One file's copy intent — pure, no side effects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedCopy {
    pub src: PathBuf,
    /// Relative destination folder (rendered from the layout), for reporting.
    pub folder: String,
    pub dest_dir: PathBuf,
    pub dest_path: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct FailedCopy {
    pub src: PathBuf,
    pub error: String,
}

#[derive(Debug, Default, Serialize)]
pub struct IngestReport {
    /// Source paths copied and verified this run.
    pub copied: Vec<PathBuf>,
    /// Source paths already present at the destination (idempotent skip).
    pub skipped: Vec<PathBuf>,
    /// Distinct relative destination folders touched.
    pub folders: Vec<String>,
    pub failed: Vec<FailedCopy>,
}

impl IngestReport {
    pub fn is_clean(&self) -> bool {
        self.failed.is_empty()
    }
    /// Source paths safe to remove from the card (copied + already-present).
    pub fn deletable_sources(&self) -> impl Iterator<Item = &PathBuf> {
        self.copied.iter().chain(self.skipped.iter())
    }
}

/// Expand a leading `~` to `$HOME`. (ponytail: only `~`; add `${VAR}` if a config needs it.)
pub fn expand(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix('~') {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(format!("{home}{rest}"));
        }
    }
    PathBuf::from(path)
}

/// Resolve `rule.dest` and pre-flight that it exists and is writable (touch-test).
///
/// The dest root must already exist — we never create it. Auto-creating it risks
/// writing a local stub where a NAS/external mount should be (locked decision §1).
pub fn resolve_dest(rule: &CardRule) -> Result<PathBuf> {
    let dest = expand(&rule.dest);
    if !dest.is_dir() {
        return Err(Error::DestUnavailable(dest));
    }
    let probe = dest.join(".fileflow-write-test");
    std::fs::write(&probe, b"ok").map_err(|_| Error::DestUnavailable(dest.clone()))?;
    let _ = std::fs::remove_file(&probe);
    Ok(dest)
}

/// Enumerate matching files under the rule's source folders (globs supported).
pub fn scan_files(rule: &CardRule, volume_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for src in &rule.sources {
        let pattern = volume_root.join(src);
        let pattern = pattern.to_string_lossy();
        // glob() only matches existing paths, so brand/rollover globs resolve to real dirs.
        for entry in glob::glob(&pattern).into_iter().flatten().flatten() {
            if entry.is_dir() {
                for f in WalkDir::new(&entry).into_iter().filter_map(|e| e.ok()) {
                    let p = f.path();
                    if f.file_type().is_file() && !is_hidden(p) && ext_matches(p, &rule.extensions) {
                        out.push(p.to_path_buf());
                    }
                }
            } else if entry.is_file() && !is_hidden(&entry) && ext_matches(&entry, &rule.extensions) {
                out.push(entry);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn mtime_of(p: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).ok()?.modified().ok()
}

/// Group matching files by capture date (from mtime), sorted ascending by date.
/// Runs before prompting so the UI can ask for a name per date.
pub fn scan_dates(rule: &CardRule, volume_root: &Path) -> Vec<DateGroup> {
    let mut map: BTreeMap<(String, String), usize> = BTreeMap::new(); // (date, year) -> count
    for f in scan_files(rule, volume_root) {
        if let Some(mtime) = mtime_of(&f) {
            let (year, date) = layout::date_parts(mtime);
            *map.entry((date, year)).or_default() += 1;
        }
    }
    map.into_iter()
        .map(|((date, year), file_count)| DateGroup { date, year, file_count })
        .collect()
}

/// Compute the copy plan. `names` maps `YYYY-MM-DD` → folder name; missing = empty.
pub fn plan_ingest(
    rule: &CardRule,
    volume_root: &Path,
    names: &BTreeMap<String, String>,
    dest_root: &Path,
) -> Vec<PlannedCopy> {
    let mut plan = Vec::new();
    for src in scan_files(rule, volume_root) {
        let Some(mtime) = mtime_of(&src) else { continue };
        let (year, date) = layout::date_parts(mtime);
        let name = names.get(&date).cloned().unwrap_or_default();
        let folder = layout::render(&rule.layout, &year, &date, &name);
        let dest_dir = dest_root.join(&folder);
        let Some(file_name) = src.file_name() else { continue };
        let dest_path = dest_dir.join(file_name);
        plan.push(PlannedCopy { src, folder, dest_dir, dest_path });
    }
    plan
}

/// Execute the plan: create dirs, copy preserving mtime, verify by byte size.
/// A vanished/unreachable destination surfaces here as failed copies → cleanup is skipped.
pub fn run_ingest(plan: &[PlannedCopy]) -> IngestReport {
    let mut report = IngestReport::default();
    let mut folders = std::collections::BTreeSet::new();

    for pc in plan {
        let src_len = match std::fs::metadata(&pc.src) {
            Ok(m) => m.len(),
            Err(e) => {
                report.failed.push(FailedCopy { src: pc.src.clone(), error: e.to_string() });
                continue;
            }
        };

        // Idempotent: identical-size dest already there → skip, don't re-copy or double-count.
        if let Ok(dm) = std::fs::metadata(&pc.dest_path) {
            if dm.len() == src_len {
                report.skipped.push(pc.src.clone());
                folders.insert(pc.folder.clone());
                continue;
            }
        }

        if let Err(e) = std::fs::create_dir_all(&pc.dest_dir) {
            report.failed.push(FailedCopy { src: pc.src.clone(), error: e.to_string() });
            continue;
        }
        if let Err(e) = std::fs::copy(&pc.src, &pc.dest_path) {
            report.failed.push(FailedCopy { src: pc.src.clone(), error: e.to_string() });
            continue;
        }

        // Verify by size; a short write leaves a bad dest, so remove it before failing.
        let verified = std::fs::metadata(&pc.dest_path).map(|m| m.len() == src_len).unwrap_or(false);
        if !verified {
            let _ = std::fs::remove_file(&pc.dest_path);
            report.failed.push(FailedCopy {
                src: pc.src.clone(),
                error: "size mismatch after copy".into(),
            });
            continue;
        }

        // Preserve mtime (std::fs::copy does not). Best-effort: a failure here isn't fatal.
        if let Ok(meta) = std::fs::metadata(&pc.src) {
            let mt = filetime::FileTime::from_last_modification_time(&meta);
            let _ = filetime::set_file_mtime(&pc.dest_path, mt);
        }

        report.copied.push(pc.src.clone());
        folders.insert(pc.folder.clone());
    }

    report.folders = folders.into_iter().collect();
    report
}

/// Delete verified source files from the card. Returns the deleted paths.
///
/// All-or-nothing: refuses to delete anything if any copy failed. The policy
/// decision (never/ask/always) and any user confirmation belong to the caller; this is
/// the safe primitive it calls once it has decided to delete.
pub fn cleanup(report: &IngestReport) -> Result<Vec<PathBuf>> {
    if !report.is_clean() {
        return Err(Error::CleanupBlocked(report.failed.len()));
    }
    let mut deleted = Vec::new();
    for src in report.deletable_sources() {
        std::fs::remove_file(src).map_err(|e| Error::Delete { path: src.clone(), source: e })?;
        deleted.push(src.clone());
    }
    Ok(deleted)
}

/// Unmount/eject the card. Caller decides *whether* (policy + confirmation) and only
/// ever after a fully successful import. Eject failure is a warning, not a data risk.
pub fn eject(volume_root: &Path, policy: EjectPolicy) -> Result<bool> {
    if policy == EjectPolicy::Never {
        return Ok(false);
    }
    let status = Command::new("diskutil").arg("eject").arg(volume_root).status()?;
    if status.success() {
        Ok(true)
    } else {
        Err(Error::EjectFailed(volume_root.to_path_buf()))
    }
}
