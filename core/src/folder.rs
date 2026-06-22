//! Folder-to-folder flow: watch a folder and move whatever lands in it into a
//! dated destination — the card-ingest idea, triggered by a plain folder.

use crate::config::{Destination, FolderRule};
use crate::ingest::{expand, is_writable_dir};
use crate::photos::scan_folder;
use crate::util::move_file;
use crate::{layout, Error, Result};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
pub struct FailedMove {
    pub src: PathBuf,
    pub error: String,
}

#[derive(Debug, Default, Serialize)]
pub struct MoveReport {
    pub moved: Vec<PathBuf>,
    pub failed: Vec<FailedMove>,
}

/// Move matching top-level files from the watch folder into `dest/{layout}/…`.
///
/// The destination root must already exist — we only create dated subfolders under
/// it, never the root itself (so an unmounted network/external dest can't get a local
/// stub; same rule as card ingest). The scan is non-recursive, so a dest nested inside
/// the watch folder won't re-trigger.
///
/// `on_progress(done, total)` reports per-file progress (see [`crate::ingest::run_ingest`]).
pub fn run_folder_move(
    rule: &FolderRule,
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
    let files = scan_folder(&src, &rule.extensions);
    let total = files.len();
    for (i, f) in files.iter().enumerate() {
        on_progress(i, total);
        let Some(name) = f.file_name() else { continue };
        let dir = match std::fs::metadata(f).and_then(|m| m.modified()) {
            Ok(mtime) => {
                let (year, date) = layout::date_parts(mtime);
                dest_root.join(layout::render(layout, &year, &date, ""))
            }
            Err(_) => dest_root.clone(),
        };
        let target = dir.join(name);
        match move_file(f, &target) {
            Ok(()) => report.moved.push(target),
            Err(e) => report.failed.push(FailedMove { src: f.clone(), error: e.to_string() }),
        }
    }
    Ok(report)
}
