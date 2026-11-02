//! Card ingest engine: scan → plan → copy+verify → cleanup → eject.
//!
//! The safety spine (locked decision §1): copy + verify the *entire* set first; only
//! if every file verified does deletion run. [`cleanup`] re-checks this itself, so the
//! all-or-nothing guarantee holds even if a caller forgets to.

use crate::config::{CardRule, EjectPolicy, RawJpeg};
use crate::util::{available_space, copy_verified, ext_matches, files_equal, is_hidden, with_suffix, NameFilter};
use crate::layout;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
#[cfg(any(target_os = "macos", target_os = "windows"))]
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
    /// The destination root this copy lands under (the rule's, a route's, or the backup).
    pub root: PathBuf,
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
    /// Bytes written this run (copied files only).
    pub bytes: u64,
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

/// Expand a leading `~` to the user's home directory — `$HOME` on Unix, falling back
/// to `%USERPROFILE%` on Windows. (ponytail: only `~`; add `${VAR}` if a config needs it.)
pub fn expand(path: &str) -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok();
    expand_with_home(path, home.as_deref())
}

/// Pure body of [`expand`]: substitute a leading `~` with `home` (no-op when `None`).
pub fn expand_with_home(path: &str, home: Option<&str>) -> PathBuf {
    if let (Some(rest), Some(home)) = (path.strip_prefix('~'), home) {
        return PathBuf::from(format!("{home}{rest}"));
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
    if !rule.backup_dest.is_empty() {
        check_dest(&expand(&rule.backup_dest), volume_root)?;
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
    // Bad regex fails closed (deny-all): save_config rejects bad patterns, so a broken
    // one here means a hand-edited config — copy nothing rather than misjudge files.
    let filter = NameFilter::compile_or_deny(&rule.include, &rule.exclude);
    let mut out = Vec::new();
    for src in &rule.sources {
        let pattern = volume_root.join(src);
        let pattern = pattern.to_string_lossy();
        // glob() only matches existing paths, so brand/rollover globs resolve to real dirs.
        for entry in glob::glob(&pattern).into_iter().flatten().flatten() {
            if entry.is_dir() {
                for f in WalkDir::new(&entry).into_iter().filter_map(|e| e.ok()) {
                    let p = f.path();
                    if f.file_type().is_file()
                        && !is_hidden(p)
                        && ext_matches(p, &rule.extensions)
                        && filter.accepts(p)
                    {
                        out.push(p.to_path_buf());
                    }
                }
            } else if entry.is_file()
                && !is_hidden(&entry)
                && ext_matches(&entry, &rule.extensions)
                && filter.accepts(&entry)
            {
                out.push(entry);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Group matching files by capture date (EXIF, else mtime), sorted ascending by date.
/// Runs before prompting so the UI can ask for a name per date.
pub fn scan_dates(rule: &CardRule, volume_root: &Path) -> Vec<DateGroup> {
    date_groups(&scan_files(rule, volume_root))
}

/// Distinct capture dates (EXIF, else mtime) across a file list, sorted ascending — the
/// naming form's input for both the drive and the Photos flow.
pub fn date_groups(files: &[PathBuf]) -> Vec<DateGroup> {
    let mut map: BTreeMap<(String, String), usize> = BTreeMap::new(); // (date, year) -> count
    for f in files {
        if let Some(c) = layout::capture_of(f) {
            *map.entry((c.date, c.year)).or_default() += 1;
        }
    }
    map.into_iter()
        .map(|((date, year), file_count)| DateGroup { date, year, file_count })
        .collect()
}

/// Camera RAW extensions recognised for RAW+JPEG pairing.
const RAW_EXTS: &[&str] = &[
    "arw", "srf", "sr2", "cr2", "cr3", "crw", "nef", "nrw", "raf", "orf", "rw2", "dng", "pef", "srw",
    "3fr", "iiq", "x3f", "raw", "rwl",
];
const JPEG_EXTS: &[&str] = &["jpg", "jpeg", "hif", "heif", "heic"];

fn ext_in(p: &Path, set: &[&str]) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| set.iter().any(|x| x.eq_ignore_ascii_case(e)))
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
    let files = scan_files(rule, volume_root);
    // RAW twin of each JPEG, keyed by path-without-extension (same folder, same stem).
    let raws: HashMap<PathBuf, &PathBuf> = if rule.raw_jpeg == RawJpeg::Independent {
        HashMap::new()
    } else {
        files.iter().filter(|f| ext_in(f, RAW_EXTS)).map(|f| (f.with_extension(""), f)).collect()
    };
    let twin = |f: &Path| ext_in(f, JPEG_EXTS).then(|| raws.get(&f.with_extension("")).copied()).flatten();

    let mut plan = Vec::new();
    let mut seq: BTreeMap<PathBuf, usize> = BTreeMap::new(); // dest_dir -> last seq used
    let mut pair_seq: HashMap<(PathBuf, PathBuf), usize> = HashMap::new(); // (dest_dir, stem) -> seq
    for src in files.iter().cloned() {
        let raw_twin = twin(&src);
        if rule.raw_jpeg == RawJpeg::SkipJpeg && raw_twin.is_some() {
            continue; // the RAW carries the shot; this JPEG stays on the drive
        }
        let Some(cap) = layout::capture_of(&src) else { continue };
        let name = names.get(&cap.date).cloned().unwrap_or_default();

        // First matching route overrides dest root + layout; else the rule's own.
        // A paired JPEG routes as its RAW does, so the two never get split up.
        let route_by = raw_twin.map(PathBuf::as_path).unwrap_or(&src);
        let route = rule.routes.iter().find(|r| ext_matches(route_by, &r.extensions));
        let root = match route {
            Some(r) if !r.dest.is_empty() => expand(&r.dest),
            _ => dest_root.to_path_buf(),
        };
        let lay = match route {
            Some(r) if !r.layout.is_empty() => r.layout.as_str(),
            _ => rule.layout.as_str(),
        };

        let folder = layout::render_cap(lay, &cap, &name);
        let dest_dir = root.join(&folder);

        let Some(orig) = src.file_name() else { continue };
        let file_name = if rule.rename.is_empty() {
            PathBuf::from(orig)
        } else {
            let pair_key = (dest_dir.clone(), src.with_extension(""));
            let n = match (rule.raw_jpeg, pair_seq.get(&pair_key)) {
                (RawJpeg::Together, Some(n)) => *n, // the twin already took a number
                _ => {
                    let n = seq.entry(dest_dir.clone()).or_insert(0);
                    *n += 1;
                    pair_seq.insert(pair_key, *n);
                    *n
                }
            };
            let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("");
            let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            PathBuf::from(layout::render_filename_cap(&rule.rename, &cap, &name, n, ext, stem))
        };
        let dest_path = dest_dir.join(&file_name);
        plan.push(PlannedCopy { src, root, folder, dest_dir, dest_path });
    }
    plan
}

/// The plan for a rule's backup copy: the same files and folder structure under
/// `backup_root`. Route *destinations* are ignored (everything mirrors into the one
/// backup root) but route layouts still apply, so the backup tree matches the primary.
pub fn plan_backup(
    rule: &CardRule,
    volume_root: &Path,
    names: &BTreeMap<String, String>,
    backup_root: &Path,
) -> Vec<PlannedCopy> {
    let mut mirror = rule.clone();
    for r in &mut mirror.routes {
        r.dest.clear();
    }
    plan_ingest(&mirror, volume_root, names, backup_root)
}

/// Refuse a plan whose new bytes don't fit at their destination. Files already present
/// with the same size count as zero (they'll be skipped). Destinations whose free space
/// can't be measured are not checked.
pub fn check_space(plan: &[PlannedCopy]) -> Result<()> {
    let mut need: BTreeMap<&Path, u64> = BTreeMap::new();
    for pc in plan {
        let len = std::fs::metadata(&pc.src).map(|m| m.len()).unwrap_or(0);
        let present = std::fs::metadata(&pc.dest_path).is_ok_and(|m| m.len() == len);
        if !present {
            *need.entry(&pc.root).or_default() += len;
        }
    }
    for (root, needed) in need {
        if let Some(available) = available_space(root) {
            if needed > available {
                return Err(Error::NoSpace { dest: root.to_path_buf(), needed, available });
            }
        }
    }
    Ok(())
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

/// Execute the plan: create dirs, copy preserving mtime, verify size + full byte compare
/// (callers run [`check_space`] first).
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
        // Verify every byte against a second read of the source before it counts: this
        // run's sources may be deleted afterwards (see util::copy_verified).
        if let Err(e) = copy_verified(&pc.src, &dest_path, src_len) {
            report.failed.push(FailedCopy { src: pc.src.clone(), error: e.to_string() });
            continue;
        }
        report.bytes += src_len;

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
///
/// macOS ejects via `diskutil`; Windows via the Shell.Application COM verb (the same
/// path as Explorer's Eject). Other platforms have no backend and error explicitly.
pub fn eject(volume_root: &Path, policy: EjectPolicy) -> Result<bool> {
    if policy == EjectPolicy::Never {
        return Ok(false);
    }
    eject_backend(volume_root)
}

#[cfg(target_os = "macos")]
fn eject_backend(volume_root: &Path) -> Result<bool> {
    let status = Command::new("diskutil").arg("eject").arg(volume_root).status()?;
    if status.success() {
        Ok(true)
    } else {
        Err(Error::EjectFailed(volume_root.to_path_buf()))
    }
}

#[cfg(target_os = "windows")]
fn eject_backend(volume_root: &Path) -> Result<bool> {
    // `E:\` → `E:` — the COM verb wants the bare drive letter.
    let letter = volume_root
        .to_string_lossy()
        .trim_end_matches(['\\', '/'])
        .to_string();
    let script = crate::windows::eject_script(&letter)
        .ok_or_else(|| Error::EjectFailed(volume_root.to_path_buf()))?;
    let status = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .status()?;
    if status.success() {
        Ok(true)
    } else {
        Err(Error::EjectFailed(volume_root.to_path_buf()))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn eject_backend(_volume_root: &Path) -> Result<bool> {
    Err(Error::EjectUnsupported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Fresh unique temp dir, removed on drop. std-only so core keeps zero dev-deps.
    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            static N: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "fileflow-test-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    // The public ingest/cleanup pipeline is covered by tests/core_tests.rs; this module
    // only tests what's private to this file.
    #[test]
    fn dest_inside_card_is_refused() {
        let card = TempDir::new();
        let inner = card.path().join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        assert!(matches!(
            check_dest(&inner, card.path()),
            Err(Error::DestInsideCard(_))
        ));
    }
}
