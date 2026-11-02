//! Small filename predicates shared by the ingest and Photos scanners.

use regex::Regex;
use std::io::Read;
use std::path::{Path, PathBuf};

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
        // No patterns set → a true no-op: accept everything, including names that
        // aren't valid UTF-8 (which only the regex test below would need to read).
        if self.include.is_none() && self.exclude.is_none() {
            return true;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            return false;
        };
        let included = self.include.as_ref().map(|re| re.is_match(name)).unwrap_or(true);
        let excluded = self.exclude.as_ref().is_some_and(|re| re.is_match(name));
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

/// Insert `-{n}` before the extension: `a.arw` → `a-1.arw`. Used to de-collide.
pub fn with_suffix(name: &Path, n: usize) -> PathBuf {
    let stem = name.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    match name.extension().and_then(|e| e.to_str()) {
        Some(ext) => PathBuf::from(format!("{stem}-{n}.{ext}")),
        None => PathBuf::from(format!("{stem}-{n}")),
    }
}

/// Whole-file byte comparison. Callers pre-check equal length, so this only confirms
/// that two same-sized files are truly identical. Used both to prove an existing dest
/// is a prior copy (skip path) and to verify every fresh copy before it counts.
pub fn files_equal(a: &Path, b: &Path, len: u64) -> bool {
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

/// Copy `src` (of `len` bytes) to the free path `dest`, then verify size + every byte
/// against a fresh read of the source, preserving mtime. A copy that doesn't verify is
/// removed and reported as `InvalidData`, so it can never count as a safe copy.
///
/// The verify re-reads the *source* on purpose: a flaky card read during the copy
/// differs from the second read and is caught. A hash computed during the copy would
/// save that read but would bless corrupted bytes, so it is deliberately not used.
pub fn copy_verified(src: &Path, dest: &Path, len: u64) -> std::io::Result<()> {
    std::fs::copy(src, dest)?;
    let ok = std::fs::metadata(dest).map(|m| m.len() == len).unwrap_or(false)
        && files_equal(src, dest, len);
    if !ok {
        let _ = std::fs::remove_file(dest);
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "copy did not verify (size or content mismatch)",
        ));
    }
    // std::fs::copy does not keep mtime. Best-effort: a failure here isn't fatal.
    if let Ok(meta) = std::fs::metadata(src) {
        let mt = filetime::FileTime::from_last_modification_time(&meta);
        let _ = filetime::set_file_mtime(dest, mt);
    }
    Ok(())
}

/// Move `src` to `dest`, creating parent dirs, and return where the file now lives.
///
/// Never overwrites: if `dest` holds a *different* file the first free `name-N.ext`
/// is used; if it holds byte-identical content the move is already done, so only the
/// source is removed. Across filesystems (NAS, another drive) the file is copied,
/// verified byte-for-byte, and only then removed from the source.
// ponytail: free-name check then rename is not atomic; a writer racing into the same
// name in that window could still be replaced. Use renamex_np(RENAME_EXCL) if it bites.
pub fn move_file(src: &Path, dest: &Path) -> std::io::Result<PathBuf> {
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let len = std::fs::metadata(src)?.len();
    let base = PathBuf::from(dest.file_name().unwrap_or_default());
    let mut k = 0usize;
    let target = loop {
        let cand = if k == 0 { dest.to_path_buf() } else { parent.join(with_suffix(&base, k)) };
        match std::fs::metadata(&cand) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break cand,
            Err(e) => return Err(e),
            Ok(_) if same_file(src, &cand) => return Ok(cand), // already where it belongs
            Ok(m) if m.len() == len && files_equal(src, &cand, len) => {
                std::fs::remove_file(src)?; // identical copy already there
                return Ok(cand);
            }
            Ok(_) => k += 1, // a different file holds this name
        }
    };
    if std::fs::rename(src, &target).is_ok() {
        return Ok(target);
    }
    copy_verified(src, &target, len)?;
    std::fs::remove_file(src)?;
    Ok(target)
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Bytes free for the current user on the filesystem holding `path`, or `None` when
/// it can't be measured (callers then skip the space check rather than block a run).
#[cfg(unix)]
pub fn available_space(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path and `s` a writable statvfs.
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    #[allow(clippy::unnecessary_cast)] // field widths differ between macOS and Linux
    Some(s.f_bavail as u64 * s.f_frsize as u64)
}

#[cfg(windows)]
pub fn available_space(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free = 0u64;
    // SAFETY: `wide` is NUL-terminated; the two unused out-params may be null.
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(free)
}

/// `1.4 GB`-style size for messages.
pub fn fmt_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1000.0 && u < UNITS.len() - 1 {
        v /= 1000.0;
        u += 1;
    }
    if u == 0 { format!("{n} B") } else { format!("{v:.1} {}", UNITS[u]) }
}
