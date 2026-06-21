//! Apple Photos import via `osascript`. (PhotoKit Swift sidecar is a future upgrade, §14.)

use crate::config::AfterImport;
use crate::ingest::DateGroup;
use crate::util::{ext_matches, is_hidden};
use crate::{layout, Error, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// List top-level files in an export folder that match `extensions` (non-recursive).
/// Subdirectories (e.g. an `_imported` archive) are skipped by design.
pub fn scan_folder(folder: &Path, extensions: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(folder) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_file() && !is_hidden(&p) && ext_matches(&p, extensions) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PhotosReport {
    pub imported: usize,
    pub album: String,
}

/// Escape a string for embedding in an AppleScript double-quoted literal.
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Build the AppleScript that ensures the album exists and imports `files` into it.
///
/// `skip_duplicates` maps to Photos' `skip check duplicates` flag: `true` skips files
/// already in the library; `false` imports everything.
/// Build the AppleScript. `album: None` imports into the library only (no album);
/// `Some(name)` ensures that album exists and imports into it.
pub fn build_import_script(files: &[PathBuf], album: Option<&str>, skip_duplicates: bool) -> String {
    let file_list = files
        .iter()
        .map(|p| format!("POSIX file \"{}\"", escape(&p.to_string_lossy())))
        .collect::<Vec<_>>()
        .join(", ");
    match album {
        Some(album) => {
            let album = escape(album);
            format!(
                r#"tell application "Photos"
  if not (exists album "{album}") then
    make new album named "{album}"
  end if
  set theAlbum to album "{album}"
  import {{{file_list}}} into theAlbum skip check duplicates {skip}
end tell"#,
                album = album,
                file_list = file_list,
                skip = skip_duplicates,
            )
        }
        None => format!(
            r#"tell application "Photos"
  import {{{file_list}}} skip check duplicates {skip}
end tell"#,
            file_list = file_list,
            skip = skip_duplicates,
        ),
    }
}

/// Where imported files should land in Photos.
pub enum AlbumTarget {
    /// The library only — no album.
    Library,
    /// One fixed album.
    Fixed(String),
    /// Album name(s) rendered from a date template, grouped by each file's capture date.
    /// `names` maps `YYYY-MM-DD` → the `{name}` token (empty when not prompted).
    Template {
        template: String,
        names: BTreeMap<String, String>,
    },
}

/// Distinct capture dates (from mtime) across a flat file list, for the naming form.
pub fn date_groups(files: &[PathBuf]) -> Vec<DateGroup> {
    let mut map: BTreeMap<(String, String), usize> = BTreeMap::new(); // (date, year) -> count
    for f in files {
        if let Ok(mtime) = std::fs::metadata(f).and_then(|m| m.modified()) {
            let (year, date) = layout::date_parts(mtime);
            *map.entry((date, year)).or_default() += 1;
        }
    }
    map.into_iter()
        .map(|((date, year), file_count)| DateGroup { date, year, file_count })
        .collect()
}

/// Group files by the album name a date template renders for each file's mtime.
/// Mirrors the card folder rules: {year}/{date}, with {name} from the `names` map.
pub fn album_groups(
    files: &[PathBuf],
    template: &str,
    names: &BTreeMap<String, String>,
) -> BTreeMap<String, Vec<PathBuf>> {
    let mut groups: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for f in files {
        let album = match std::fs::metadata(f).and_then(|m| m.modified()) {
            Ok(mtime) => {
                let (year, date) = layout::date_parts(mtime);
                let name = names.get(&date).cloned().unwrap_or_default();
                layout::render(template, &year, &date, &name)
            }
            Err(_) => template.to_string(),
        };
        let album = if album.is_empty() { "Imported".to_string() } else { album };
        groups.entry(album).or_default().push(f.clone());
    }
    groups
}

fn run_osascript(script: &str) -> Result<()> {
    let out = Command::new("osascript").arg("-e").arg(script).output()?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        if stderr.contains("-1743") || stderr.to_lowercase().contains("not authorized") {
            return Err(Error::PhotosNotAuthorized);
        }
        return Err(Error::Osascript(stderr.trim().to_string()));
    }
    Ok(())
}

/// Import `files` into Photos per `target`, creating albums as needed.
pub fn import_to_photos(
    files: &[PathBuf],
    target: &AlbumTarget,
    skip_duplicates: bool,
) -> Result<PhotosReport> {
    if files.is_empty() {
        return Ok(PhotosReport { imported: 0, album: String::new() });
    }
    let album = match target {
        AlbumTarget::Library => {
            run_osascript(&build_import_script(files, None, skip_duplicates))?;
            "library".to_string()
        }
        AlbumTarget::Fixed(name) => {
            run_osascript(&build_import_script(files, Some(name), skip_duplicates))?;
            name.clone()
        }
        AlbumTarget::Template { template, names } => {
            let groups = album_groups(files, template, names);
            let albums: Vec<String> = groups.keys().cloned().collect();
            for (album, group) in &groups {
                run_osascript(&build_import_script(group, Some(album), skip_duplicates))?;
            }
            format!("{} album(s): {}", albums.len(), albums.join(", "))
        }
    };
    Ok(PhotosReport { imported: files.len(), album })
}

/// Move a file, falling back to copy+remove across filesystems.
fn move_file(src: &Path, dest: &Path) -> Result<()> {
    if std::fs::rename(src, dest).is_ok() {
        return Ok(());
    }
    std::fs::copy(src, dest)?;
    std::fs::remove_file(src)?;
    Ok(())
}

/// Apply the post-import policy to the source export files (only after a successful import).
pub fn after_import(files: &[PathBuf], action: AfterImport, archive_dir: &Path) -> Result<()> {
    match action {
        AfterImport::Leave => Ok(()),
        AfterImport::Delete => {
            for f in files {
                std::fs::remove_file(f)?;
            }
            Ok(())
        }
        AfterImport::Archive => {
            std::fs::create_dir_all(archive_dir)?;
            for f in files {
                if let Some(name) = f.file_name() {
                    move_file(f, &archive_dir.join(name))?;
                }
            }
            Ok(())
        }
    }
}
