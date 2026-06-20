//! macOS volume helpers — listing mounts and resolving a volume UUID.
//!
//! UUID resolution matters: it's the key that decides which card rule fires (and thus
//! where files go and whether the card is wiped). We parse `diskutil`'s plist output
//! rather than scraping text, so a macOS format tweak can't silently mis-match a card.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Top-level entries under `/Volumes` that are directories (mounted volumes).
pub fn mounted_volumes() -> HashSet<PathBuf> {
    let mut out = HashSet::new();
    if let Ok(rd) = std::fs::read_dir("/Volumes") {
        for e in rd.flatten() {
            let p = e.path();
            // `/Volumes` entries are mount points (incl. a symlink to the boot volume).
            if p.is_dir() {
                out.insert(p);
            }
        }
    }
    out
}

/// Resolve the volume UUID for a mount path via `diskutil info -plist`.
pub fn volume_uuid(path: &Path) -> Option<String> {
    let out = Command::new("diskutil")
        .arg("info")
        .arg("-plist")
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let value: plist::Value = plist::from_bytes(&out.stdout).ok()?;
    value
        .as_dictionary()?
        .get("VolumeUUID")?
        .as_string()
        .map(|s| s.to_string())
}
