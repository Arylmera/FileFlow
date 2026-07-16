//! Volume helpers — listing mounted drives and resolving a volume UUID, per platform.
//!
//! UUID resolution matters: it's the key that decides which card rule fires (and thus
//! where files go and whether the card is wiped). Both platforms therefore parse
//! *structured* output — `diskutil`'s plist on macOS, `Win32_Volume` JSON on Windows —
//! so an OS format tweak can't silently mis-match a card.

pub use imp::{find_volume_by_uuid, is_boot_volume, mounted_volumes, volume_uuid};

#[cfg(target_os = "macos")]
mod imp {
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

    /// True if `path` resolves to the boot volume (`/`). The boot disk surfaces in
    /// `/Volumes` as a symlink, so we canonicalize before comparing. Never eject it.
    pub fn is_boot_volume(path: &Path) -> bool {
        std::fs::canonicalize(path).map(|p| p == Path::new("/")).unwrap_or(false)
    }

    /// Find a currently-mounted volume whose UUID matches `uuid` (case-insensitive).
    pub fn find_volume_by_uuid(uuid: &str) -> Option<PathBuf> {
        mounted_volumes()
            .into_iter()
            .find(|p| volume_uuid(p).map(|u| u.eq_ignore_ascii_case(uuid)).unwrap_or(false))
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
}

#[cfg(target_os = "windows")]
mod imp {
    use fileflow_core::windows::{parse_win32_volumes, WinVolume, LIST_VOLUMES_COMMAND};
    use std::collections::HashSet;
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// Don't flash a console window when a GUI app spawns PowerShell.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// All lettered volumes from `Win32_Volume` (letter-less recovery/EFI partitions
    /// are dropped — they can't hold a card's files). The parsing itself is pure and
    /// unit-tested in `fileflow-core`.
    fn list_volumes() -> Vec<WinVolume> {
        let out = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", LIST_VOLUMES_COMMAND])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        let Ok(out) = out else { return Vec::new() };
        if !out.status.success() {
            return Vec::new();
        }
        parse_win32_volumes(&String::from_utf8_lossy(&out.stdout)).unwrap_or_default()
    }

    /// Mount roots (`E:\`) of every lettered volume — the Windows analogue of `/Volumes`.
    pub fn mounted_volumes() -> HashSet<PathBuf> {
        list_volumes().iter().filter_map(WinVolume::mount_root).collect()
    }

    /// True for the Windows system drive (`%SystemDrive%`, normally `C:`). Never eject it.
    pub fn is_boot_volume(path: &Path) -> bool {
        let sys = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        let p = path.to_string_lossy();
        p.trim_end_matches(['\\', '/']).eq_ignore_ascii_case(&sys)
    }

    /// Find a currently-mounted volume whose GUID matches `uuid` (case-insensitive).
    pub fn find_volume_by_uuid(uuid: &str) -> Option<PathBuf> {
        list_volumes()
            .iter()
            .find(|v| v.uuid.as_deref().is_some_and(|u| u.eq_ignore_ascii_case(uuid)))
            .and_then(WinVolume::mount_root)
    }

    /// The volume GUID for a mount path (`E:\` → the `\\?\Volume{GUID}\` GUID).
    pub fn volume_uuid(path: &Path) -> Option<String> {
        let want = path.to_string_lossy().trim_end_matches(['\\', '/']).to_ascii_uppercase();
        list_volumes()
            .iter()
            .find(|v| v.drive_letter.as_deref().is_some_and(|l| l.eq_ignore_ascii_case(&want)))
            .and_then(|v| v.uuid.clone())
    }
}

// Stubs so dev builds on other platforms (e.g. Linux) still compile; no drives surface.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod imp {
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};

    pub fn mounted_volumes() -> HashSet<PathBuf> {
        HashSet::new()
    }
    pub fn is_boot_volume(_path: &Path) -> bool {
        false
    }
    pub fn find_volume_by_uuid(_uuid: &str) -> Option<PathBuf> {
        None
    }
    pub fn volume_uuid(_path: &Path) -> Option<String> {
        None
    }
}
