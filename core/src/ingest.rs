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
use std::collections::{BTreeMap, HashSet};
use std::io::Read;
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

/// Resolve `rule.dest` and pre-flight that it (and every route destination) exists,
/// is writable, and does not live inside the card being read.
///
/// The dest root must already exist — we never create it. Auto-creating it risks
/// writing a local stub where a NAS/external mount should be (locked decision §1).
/// A destination *inside* `volume_root` is refused: with cleanup=Always we would copy
/// the card's files onto the same card and then delete the originals, so those "copies"
/// vanish the moment the card is reused or formatted.
pub fn resolve_dest(rule: &CardRule, volume_root: &Path) -> Result<PathBuf> {
    let dest = expand(&rule.dest);
    check_dest(&dest, volume_root)?;
    // Each distinct route destination must also be reachable before we read the card:
    // a split that strands half the set on an unmounted drive must fail up front.
    for r in &rule.routes {
        if !r.dest.is_empty() {
            check_dest(&expand(&r.dest), volume_root)?;
        }
    }
    Ok(dest)
}

/// A destination must be an existing writable dir and must not sit inside the card.
fn check_dest(dest: &Path, volume_root: &Path) -> Result<()> {
    if !is_writable_dir(dest) {
        return Err(Error::DestUnavailable(dest.to_path_buf()));
    }
    if canon(dest).starts_with(canon(volume_root)) {
        return Err(Error::DestInsideCard(dest.to_path_buf()));
    }
    Ok(())
}

/// Canonicalize, falling back to the path as-given if it can't be resolved — so the
/// inside-the-card check compares real paths through symlinks/`..` where possible, and
/// both sides are normalized the same way (macOS `/var` → `/private/var`, etc.).
fn canon(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// True if `dir` exists and a probe file can be written + removed (touch-test).
pub fn is_writable_dir(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    let probe = dir.join(".fileflow-write-test");
    match std::fs::write(&probe, b"ok") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
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

/// Insert `-{n}` before the extension: `a.arw` → `a-1.arw`. Used to de-collide.
fn with_suffix(name: &Path, n: usize) -> PathBuf {
    let stem = name.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    match name.extension().and_then(|e| e.to_str()) {
        Some(ext) => PathBuf::from(format!("{stem}-{n}.{ext}")),
        None => PathBuf::from(format!("{stem}-{n}")),
    }
}

/// Compute the copy plan. `names` maps `YYYY-MM-DD` → folder name; missing = empty.
///
/// Pure (no disk writes): routing (`rule.routes`) picks each file's destination root +
/// layout by extension, and renaming (`rule.rename`) rewrites filenames with a per-folder
/// `{seq}`. The `dest_path` here is the *preferred* location — [`run_ingest`] de-collides
/// it against the live destination so a copy can never overwrite a different file.
// ponytail: {seq} restarts at 1 per plan (not seeded from the destination). So a second
// same-day import lands as `name-1` rather than continuing the count, and re-importing an
// un-wiped card after adding an earlier-sorting file re-copies the shifted files under their
// new number — a harmless orphan duplicate, never data loss (every source is still kept and
// only deleted once present). Seed the counter from the existing dest if continuous
// numbering / dedup-on-reimport ever matters.
pub fn plan_ingest(
    rule: &CardRule,
    volume_root: &Path,
    names: &BTreeMap<String, String>,
    dest_root: &Path,
) -> Vec<PlannedCopy> {
    let mut plan = Vec::new();
    let mut seq: BTreeMap<PathBuf, usize> = BTreeMap::new(); // dest_dir -> last seq used
    for src in scan_files(rule, volume_root) {
        let Some(mtime) = mtime_of(&src) else { continue };
        let (year, date) = layout::date_parts(mtime);
        let name = names.get(&date).cloned().unwrap_or_default();

        // First matching route overrides dest root + layout; else the rule's own.
        let route = rule.routes.iter().find(|r| ext_matches(&src, &r.extensions));
        let root = match route {
            Some(r) if !r.dest.is_empty() => expand(&r.dest),
            _ => dest_root.to_path_buf(),
        };
        let lay = match route {
            Some(r) if !r.layout.is_empty() => r.layout.as_str(),
            _ => rule.layout.as_str(),
        };

        let folder = layout::render(lay, &year, &date, &name);
        let dest_dir = root.join(&folder);

        let Some(orig) = src.file_name() else { continue };
        let file_name = if rule.rename.is_empty() {
            PathBuf::from(orig)
        } else {
            let n = seq.entry(dest_dir.clone()).or_insert(0);
            *n += 1;
            let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("");
            PathBuf::from(layout::render_filename(&rule.rename, &year, &date, &name, *n, ext))
        };
        let dest_path = dest_dir.join(&file_name);
        plan.push(PlannedCopy { src, folder, dest_dir, dest_path });
    }
    plan
}

/// Whole-file byte comparison. Callers pre-check equal length, so this only confirms
/// that two same-sized files are truly identical.
// ponytail: runs only when a dest of equal size already exists (re-imports, name
// clashes), never on a fresh import. A persisted source→dest manifest would avoid
// re-reading on repeated imports of an un-wiped card, if that ever gets slow.
fn files_equal(a: &Path, b: &Path, len: u64) -> bool {
    let (Ok(mut fa), Ok(mut fb)) = (std::fs::File::open(a), std::fs::File::open(b)) else {
        return false;
    };
    let (mut ba, mut bb) = ([0u8; 65536], [0u8; 65536]);
    let mut remaining = len;
    while remaining > 0 {
        let n = remaining.min(ba.len() as u64) as usize;
        if fa.read_exact(&mut ba[..n]).is_err() || fb.read_exact(&mut bb[..n]).is_err() {
            return false;
        }
        if ba[..n] != bb[..n] {
            return false;
        }
        remaining -= n as u64;
    }
    true
}

/// What [`resolve_target`] decided for one planned copy — both variants carry the dest
/// path so the run can record it as claimed (no later source may reuse that name).
enum Resolved {
    /// A byte-identical file is already at this path; the source is safe to delete.
    Skip(PathBuf),
    /// Copy the source to this (free) path.
    Write(PathBuf),
}

/// Resolve where a planned copy goes against the *live* destination and the names already
/// taken by earlier sources in this run. Walks the preferred path then `-N` suffixes:
/// the first name that no other source claimed this run and that is either free (→ `Write`)
/// or holds a byte-identical file (→ `Skip`). A different file — or a name another source
/// already took, even with matching bytes — is stepped over, never overwritten or shared.
fn resolve_target(pc: &PlannedCopy, src_len: u64, claimed: &HashSet<PathBuf>) -> Resolved {
    let base = pc.dest_path.file_name().map(PathBuf::from).unwrap_or_default();
    let mut k = 0usize;
    loop {
        let cand = if k == 0 {
            pc.dest_path.clone()
        } else {
            pc.dest_dir.join(with_suffix(&base, k))
        };
        // A name another source already resolved to this run belongs to that source —
        // never reuse it, even if the bytes match (two distinct sources, identical content).
        if claimed.contains(&cand) {
            k += 1;
            continue;
        }
        match std::fs::metadata(&cand) {
            Err(_) => return Resolved::Write(cand), // free slot
            Ok(m) if m.len() == src_len && files_equal(&pc.src, &cand, src_len) => {
                return Resolved::Skip(cand); // identical file already imported here
            }
            Ok(_) => k += 1, // a different file holds this name → try the next
        }
    }
}

/// Execute the plan: create dirs, copy preserving mtime, verify by byte size.
/// A vanished/unreachable destination surfaces here as failed copies → cleanup is skipped.
///
/// `on_progress(done, total)` is called before each file (done = files finished so far)
/// and once more at the end with `(total, total)`. Pass `|_, _| {}` to ignore it.
pub fn run_ingest(plan: &[PlannedCopy], mut on_progress: impl FnMut(usize, usize)) -> IngestReport {
    let mut report = IngestReport::default();
    let mut folders = std::collections::BTreeSet::new();
    let mut claimed: HashSet<PathBuf> = HashSet::new(); // dest paths taken this run
    let total = plan.len();

    for (i, pc) in plan.iter().enumerate() {
        on_progress(i, total);
        let src_len = match std::fs::metadata(&pc.src) {
            Ok(m) => m.len(),
            Err(e) => {
                report.failed.push(FailedCopy { src: pc.src.clone(), error: e.to_string() });
                continue;
            }
        };

        // Resolve against the live destination + names already taken this run: skip only if
        // *this* source's bytes are already there, else copy to the first free name. Never
        // overwrites a different file, and never lets two distinct sources share one name —
        // the safety net for {seq} renames and rollover-dir clashes (incl. identical bytes).
        let dest_path = match resolve_target(pc, src_len, &claimed) {
            Resolved::Skip(p) => {
                claimed.insert(p);
                report.skipped.push(pc.src.clone());
                folders.insert(pc.folder.clone());
                continue;
            }
            Resolved::Write(p) => {
                claimed.insert(p.clone());
                p
            }
        };

        if let Err(e) = std::fs::create_dir_all(&pc.dest_dir) {
            report.failed.push(FailedCopy { src: pc.src.clone(), error: e.to_string() });
            continue;
        }
        if let Err(e) = std::fs::copy(&pc.src, &dest_path) {
            report.failed.push(FailedCopy { src: pc.src.clone(), error: e.to_string() });
            continue;
        }

        // Verify by size; a short write leaves a bad dest, so remove it before failing.
        let verified = std::fs::metadata(&dest_path).map(|m| m.len() == src_len).unwrap_or(false);
        if !verified {
            let _ = std::fs::remove_file(&dest_path);
            report.failed.push(FailedCopy {
                src: pc.src.clone(),
                error: "size mismatch after copy".into(),
            });
            continue;
        }

        // Preserve mtime (std::fs::copy does not). Best-effort: a failure here isn't fatal.
        if let Ok(meta) = std::fs::metadata(&pc.src) {
            let mt = filetime::FileTime::from_last_modification_time(&meta);
            let _ = filetime::set_file_mtime(&dest_path, mt);
        }

        report.copied.push(pc.src.clone());
        folders.insert(pc.folder.clone());
    }

    on_progress(total, total);
    report.folders = folders.into_iter().collect();
    report
}

/// Outcome of [`cleanup`]: which sources were deleted, and which could not be.
#[derive(Debug, Default, Serialize)]
pub struct CleanupReport {
    pub deleted: Vec<PathBuf>,
    pub failed: Vec<FailedCopy>,
}

/// Delete verified source files from the card. Reports what was deleted and what
/// could not be, deleting per-file and *never* stopping at the first error — so a
/// card yanked mid-delete still reports exactly which files were already removed
/// (the old `?`-on-first-error dropped that record precisely when it mattered most).
///
/// All-or-nothing *gate*: refuses to delete anything if any copy failed. The policy
/// decision (never/ask/always) and any user confirmation belong to the caller; this is
/// the safe primitive it calls once it has decided to delete.
pub fn cleanup(report: &IngestReport) -> Result<CleanupReport> {
    if !report.is_clean() {
        return Err(Error::CleanupBlocked(report.failed.len()));
    }
    let mut out = CleanupReport::default();
    for src in report.deletable_sources() {
        match std::fs::remove_file(src) {
            Ok(()) => out.deleted.push(src.clone()),
            Err(e) => out.failed.push(FailedCopy { src: src.clone(), error: e.to_string() }),
        }
    }
    Ok(out)
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
