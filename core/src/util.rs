//! Small filename predicates shared by the ingest and Photos scanners.

use regex::Regex;
use std::path::Path;

/// Compiled include/exclude regex filter on a file *name* (feature: whitelist/blacklist).
///
/// A file passes when it matches `include` (or `include` is unset) AND does not match
/// `exclude`. Both patterns test the file name only (not the path). Empty pattern = unset.
///
/// Build with [`NameFilter::compile`] (fallible) at a trust boundary, or
/// [`NameFilter::compile_or_deny`] inside a scanner — a pattern that fails to compile
/// becomes a *deny-all* filter so a broken config moves nothing rather than silently
/// ignoring the filter and moving files it can't correctly judge (fail closed).
#[derive(Default)]
pub struct NameFilter {
    include: Option<Regex>,
    exclude: Option<Regex>,
    deny_all: bool,
}

impl NameFilter {
    /// Compile both patterns. Empty string = that side is unset. Errors on bad regex.
    pub fn compile(include: &str, exclude: &str) -> Result<Self, regex::Error> {
        let opt = |p: &str| -> Result<Option<Regex>, regex::Error> {
            if p.is_empty() {
                Ok(None)
            } else {
                Ok(Some(Regex::new(p)?))
            }
        };
        Ok(Self { include: opt(include)?, exclude: opt(exclude)?, deny_all: false })
    }

    /// Compile, falling back to a deny-all filter if either pattern is invalid.
    /// Scanners use this so a hand-edited bad pattern fails closed (moves nothing).
    pub fn compile_or_deny(include: &str, exclude: &str) -> Self {
        Self::compile(include, exclude).unwrap_or(Self {
            include: None,
            exclude: None,
            deny_all: true,
        })
    }

    /// True if `path`'s file name passes the filter.
    pub fn accepts(&self, path: &Path) -> bool {
        if self.deny_all {
            return false;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            return false;
        };
        let included = self.include.as_ref().map_or(true, |re| re.is_match(name));
        let excluded = self.exclude.as_ref().map_or(false, |re| re.is_match(name));
        included && !excluded
    }
}

/// Validate a user-entered regex for the config trust boundary. Empty = ok (unset).
/// Returns a human-readable error so `save_config` can reject a bad pattern.
pub fn validate_regex(pattern: &str) -> Result<(), String> {
    if pattern.is_empty() {
        return Ok(());
    }
    Regex::new(pattern).map(|_| ()).map_err(|e| e.to_string())
}

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
