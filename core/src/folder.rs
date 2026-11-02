//! Folder-to-folder flow: watch a folder and move whatever lands in it into a
//! dated destination — the card-ingest idea, triggered by a plain folder.

use crate::config::{Destination, FolderRule};
use crate::ingest::{expand, is_writable_dir};
use crate::photos::scan_folder_deep;
use crate::util::{move_file, NameFilter};
use crate::{layout, Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
pub struct FailedMove {
    pub src: PathBuf,
    pub error: String,
}

/// One file that moved: where it came from and where it now lives (for undo).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Moved {
    pub src: PathBuf,
    pub dest: PathBuf,
}

#[derive(Debug, Default, Serialize)]
pub struct MoveReport {
    pub moved: Vec<Moved>,
    pub failed: Vec<FailedMove>,
    /// Bytes moved this run.
    pub bytes: u64,
}

/// Folders a watched-folder scan must never descend into: the rule's own destination
/// or archive, so a nested destination can't feed back into its source.
pub fn skip_dirs(rule: &FolderRule) -> Vec<PathBuf> {
    let dir = match &rule.target {
        Destination::Folder { dest, .. } => dest,
        Destination::Photos { archive_folder, .. } => archive_folder,
    };
    if dir.trim().is_empty() { Vec::new() } else { vec![expand(dir)] }
}

/// Move matching files from the watch folder into `dest/{layout}/…`.
///
/// The destination root must already exist — we only create dated subfolders under
/// it, never the root itself (so an unmounted network/external dest can't get a local
/// stub; same rule as card ingest). The scan is top-level only unless the rule is
/// `recursive`; either way the destination folder itself is never scanned.
///
/// `ignore` holds paths to leave alone (files the user just put back with an undo).
/// `on_progress(done, total)` reports per-file progress (see [`crate::ingest::run_ingest`]).
pub fn run_folder_move(
    rule: &FolderRule,
    ignore: &HashSet<PathBuf>,
    mut on_progress: impl FnMut(usize, usize),
) -> Result<MoveReport> {
    let Destination::Folder { dest, layout } = &rule.target else {
        return Ok(MoveReport::default()); // not a folder-move rule — nothing to do
    };
    let dest_root = expand(dest);
    if !is_writable_dir(&dest_root) {
        return Err(Error::DestUnavailable(dest_root));
    }
    let src = expand(&rule.watch);
    let mut report = MoveReport::default();
    // Bad regex fails closed (deny-all); save_config rejects bad patterns up front.
    let filter = NameFilter::compile_or_deny(&rule.include, &rule.exclude);
    let files: Vec<PathBuf> = scan_folder_deep(&src, &rule.extensions, &filter, rule.recursive, &skip_dirs(rule))
        .into_iter()
        .filter(|f| !ignore.contains(f))
        .collect();
    let total = files.len();
    for (i, f) in files.iter().enumerate() {
        on_progress(i, total);
        let Some(name) = f.file_name() else { continue };
        let dir = match layout::capture_of(f) {
            Some(cap) => dest_root.join(layout::render_cap(layout, &cap, "")),
            None => dest_root.clone(),
        };
        let len = std::fs::metadata(f).map(|m| m.len()).unwrap_or(0);
        match move_file(f, &dir.join(name)) {
            Ok(dest) => {
                report.bytes += len;
                report.moved.push(Moved { src: f.clone(), dest });
            }
            Err(e) => report.failed.push(FailedMove { src: f.clone(), error: e.to_string() }),
        }
    }
    on_progress(total, total); // final tick clears the UI progress strip (see run_ingest)
    Ok(report)
}

/// Put moved files back where they came from (newest move first). Collision-safe like
/// every move: a file that now exists at the original path is never overwritten. A file
/// no longer at its destination (renamed, deleted) is reported as failed.
pub fn undo_moves(moves: &[Moved]) -> MoveReport {
    let mut report = MoveReport::default();
    for m in moves.iter().rev() {
        if !m.dest.is_file() {
            report.failed.push(FailedMove { src: m.dest.clone(), error: "no longer at its destination".into() });
            continue;
        }
        match move_file(&m.dest, &m.src) {
            Ok(back) => report.moved.push(Moved { src: m.dest.clone(), dest: back }),
            Err(e) => report.failed.push(FailedMove { src: m.dest.clone(), error: e.to_string() }),
        }
    }
    report
}
