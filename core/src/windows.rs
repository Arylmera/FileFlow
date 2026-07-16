//! Windows platform helpers — pure functions only, so they compile and test on
//! every platform (the shell runs the actual PowerShell/Explorer commands).
//!
//! Volume discovery parses `Get-CimInstance Win32_Volume … | ConvertTo-Json` rather
//! than scraping text, mirroring the macOS choice of `diskutil -plist`: the volume
//! GUID decides which card rule fires (and thus whether a card is wiped), so it must
//! come from a structured source.

use serde::Deserialize;
use std::path::PathBuf;

/// What kind of drive a volume sits on (from Win32_Volume's `DriveType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveKind {
    /// SD cards, USB sticks (DriveType 2).
    Removable,
    /// Internal/fixed disks (DriveType 3).
    Fixed,
    /// Anything else (network, CD-ROM, RAM disk, unknown).
    Other,
}

/// One mounted (or letter-less) volume as reported by Win32_Volume.
#[derive(Debug, Clone)]
pub struct WinVolume {
    /// `E:` — absent for letter-less volumes (recovery/EFI partitions).
    pub drive_letter: Option<String>,
    /// The volume GUID from `DeviceID` (`\\?\Volume{GUID}\`), lowercased braces stripped.
    pub uuid: Option<String>,
    pub label: Option<String>,
    pub kind: DriveKind,
}

impl WinVolume {
    /// The volume's mount root (`E:\`), if it has a drive letter.
    pub fn mount_root(&self) -> Option<PathBuf> {
        self.drive_letter.as_ref().map(|l| PathBuf::from(format!("{l}\\")))
    }
}

/// Raw JSON row — `Select-Object DriveLetter, DeviceID, Label, DriveType`.
#[derive(Deserialize)]
struct RawVolume {
    #[serde(rename = "DriveLetter")]
    drive_letter: Option<String>,
    #[serde(rename = "DeviceID")]
    device_id: Option<String>,
    #[serde(rename = "Label")]
    label: Option<String>,
    #[serde(rename = "DriveType")]
    drive_type: Option<u32>,
}

/// The PowerShell command whose output [`parse_win32_volumes`] understands.
pub const LIST_VOLUMES_COMMAND: &str = "Get-CimInstance -ClassName Win32_Volume | Select-Object DriveLetter, DeviceID, Label, DriveType | ConvertTo-Json -Compress";

/// Extract the GUID from a `\\?\Volume{GUID}\` device path.
fn guid_of(device_id: &str) -> Option<String> {
    let start = device_id.find('{')? + 1;
    let end = device_id.find('}')?;
    (start < end).then(|| device_id[start..end].to_ascii_lowercase())
}

/// Parse the JSON printed by [`LIST_VOLUMES_COMMAND`]. Accepts both a JSON array and
/// the bare object `ConvertTo-Json` emits when exactly one volume matches.
pub fn parse_win32_volumes(json: &str) -> Result<Vec<WinVolume>, String> {
    let raws: Vec<RawVolume> = match serde_json::from_str::<Vec<RawVolume>>(json) {
        Ok(list) => list,
        Err(_) => vec![serde_json::from_str::<RawVolume>(json).map_err(|e| e.to_string())?],
    };
    Ok(raws
        .into_iter()
        .map(|r| WinVolume {
            uuid: r.device_id.as_deref().and_then(guid_of),
            drive_letter: r.drive_letter.filter(|l| !l.is_empty()),
            label: r.label.filter(|l| !l.is_empty()),
            kind: match r.drive_type {
                Some(2) => DriveKind::Removable,
                Some(3) => DriveKind::Fixed,
                _ => DriveKind::Other,
            },
        })
        .collect())
}

/// PowerShell script that ejects the drive at `letter` (`E:`) via the Shell.Application
/// COM object — the same mechanism as Explorer's Eject menu item, so it flushes and
/// dismounts safely. Returns `None` unless `letter` is exactly one ASCII letter + `:`,
/// because the value is interpolated into the script (never inject free text).
pub fn eject_script(letter: &str) -> Option<String> {
    let bytes = letter.as_bytes();
    if bytes.len() != 2 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' {
        return None;
    }
    Some(format!(
        r#"(New-Object -ComObject Shell.Application).Namespace(17).ParseName("{letter}").InvokeVerb("Eject")"#
    ))
}

/// The single argument Explorer expects to open a folder with `path` selected.
pub fn explorer_select_arg(path: &str) -> String {
    format!("/select,{path}")
}

/// Names Windows reserves at any extension (CON.txt is as invalid as CON).
const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Make one path segment legal on Windows/NTFS/exFAT:
/// - `< > : " | ? * \` and control chars become `-` (the `/` separator never reaches
///   here — layout rendering splits on it first);
/// - trailing dots and spaces are trimmed (Windows strips them silently, which would
///   desync the path we verify against the path that actually exists);
/// - exact reserved device names (`CON`, `COM1`, …) get a `_` suffix.
pub fn sanitize_segment(segment: &str) -> String {
    let cleaned: String = segment
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\\' => '-',
            c if (c as u32) < 0x20 => '-',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim_end_matches(['.', ' ']).to_string();
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(&cleaned)) {
        format!("{cleaned}_")
    } else {
        cleaned
    }
}
