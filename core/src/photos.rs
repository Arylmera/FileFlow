//! Apple Photos import via `osascript`. (PhotoKit Swift sidecar is a future upgrade, §14.)

use crate::config::AfterImport;
use crate::{Error, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

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
pub fn build_import_script(files: &[PathBuf], album: &str, skip_duplicates: bool) -> String {
    let album = escape(album);
    let file_list = files
        .iter()
        .map(|p| format!("POSIX file \"{}\"", escape(&p.to_string_lossy())))
        .collect::<Vec<_>>()
        .join(", ");
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

/// Import `files` into the named Photos album (created if missing).
pub fn import_to_photos(files: &[PathBuf], album: &str, skip_duplicates: bool) -> Result<PhotosReport> {
    if files.is_empty() {
        return Ok(PhotosReport { imported: 0, album: album.to_string() });
    }
    let script = build_import_script(files, album, skip_duplicates);
    let out = Command::new("osascript").arg("-e").arg(&script).output()?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        if stderr.contains("-1743") || stderr.to_lowercase().contains("not authorized") {
            return Err(Error::PhotosNotAuthorized);
        }
        return Err(Error::Osascript(stderr.trim().to_string()));
    }
    Ok(PhotosReport { imported: files.len(), album: album.to_string() })
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
