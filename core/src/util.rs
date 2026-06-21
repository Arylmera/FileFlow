//! Small filename predicates shared by the ingest and Photos scanners.

use std::path::Path;

/// True if `path`'s extension matches any of `extensions` (case-insensitive).
/// An empty list matches everything.
pub fn ext_matches(path: &Path, extensions: &[String]) -> bool {
    if extensions.is_empty() {
        return true;
    }
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => extensions.iter().any(|w| w.eq_ignore_ascii_case(ext)),
        None => false,
    }
}

/// True for dotfiles (e.g. `.DS_Store`), which we never ingest.
pub fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with('.'))
        .unwrap_or(false)
}

/// Move a file, creating the destination's parent dirs and falling back to
/// copy+remove across filesystems.
pub fn move_file(src: &Path, dest: &Path) -> std::io::Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if std::fs::rename(src, dest).is_ok() {
        return Ok(());
    }
    std::fs::copy(src, dest)?;
    std::fs::remove_file(src)?;
    Ok(())
}
