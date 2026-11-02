//! Config schema (TOML) — the UI is the source of truth; this just (de)serializes it.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(rename = "card", default)]
    pub cards: Vec<CardRule>,
    #[serde(rename = "folder", default)]
    pub folders: Vec<FolderRule>,
    #[serde(default)]
    pub app: AppSettings,
}

/// A watched folder and what to do with new files that land in it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderRule {
    #[serde(default)]
    pub label: String,
    pub watch: String,
    /// Empty = handle all file types.
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Regex whitelist on the file name: only files whose name matches are handled.
    /// Empty = no whitelist. Applied with `exclude` after the `extensions` check.
    #[serde(default)]
    pub include: String,
    /// Regex blacklist on the file name: files whose name matches are skipped, even if
    /// they pass `include`. Empty = nothing excluded.
    #[serde(default)]
    pub exclude: String,
    /// Also handle files in subfolders of `watch` (the destination/archive folder is
    /// always skipped, so a nested destination never re-triggers itself).
    #[serde(default)]
    pub recursive: bool,
    /// Destination. Flattened so `kind` and its fields sit at the rule's top level
    /// (e.g. `kind = "photos"` beside `album_mode`), keeping each rule to only the
    /// fields its kind actually uses.
    #[serde(flatten)]
    pub target: Destination,
}

impl FolderRule {
    pub fn is_photos(&self) -> bool {
        matches!(self.target, Destination::Photos { .. })
    }
}

/// Where a folder rule's new files go. The `kind` tag (`folder` | `photos`) selects
/// the variant, and only that variant's fields are (de)serialized.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Destination {
    /// Move files into `dest/{layout}/…`.
    Folder {
        /// Destination root for the move. Any writable path.
        #[serde(default)]
        dest: String,
        #[serde(default = "default_folder_layout")]
        layout: String,
    },
    /// Import files into Apple Photos.
    Photos {
        #[serde(default)]
        album_mode: AlbumMode,
        /// Fixed album name (`Fixed`) or a date template (`Template`); ignored for `Library`.
        #[serde(default = "default_album")]
        photos_album: String,
        #[serde(default = "default_true")]
        skip_duplicates: bool,
        #[serde(default)]
        after_import: AfterImport,
        #[serde(default)]
        archive_folder: String,
        /// Ask for a name before importing (fills the `{name}` token).
        #[serde(default)]
        prompt_name: bool,
        #[serde(default)]
        name_mode: NameMode,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardRule {
    pub uuid: String,
    #[serde(default)]
    pub label: String,
    /// Source folder(s) relative to the volume root. Globs allowed (e.g. `DCIM/1*MSDCF`).
    pub sources: Vec<String>,
    /// Any writable path: local, cloud-synced, network share, or external drive.
    pub dest: String,
    #[serde(default = "default_layout")]
    pub layout: String,
    #[serde(default = "default_true")]
    pub prompt_name: bool,
    #[serde(default)]
    pub name_mode: NameMode,
    #[serde(default)]
    pub cleanup: CleanupPolicy,
    #[serde(default)]
    pub eject: EjectPolicy,
    /// Empty = all extensions.
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Filename template applied on copy (tokens `{year} {date} {name} {seq}`); the
    /// original extension is always kept. Empty = keep each file's original name.
    #[serde(default)]
    pub rename: String,
    /// Regex whitelist on the file name: only files whose name matches are copied.
    /// Empty = no whitelist. Applied with `exclude` after the `extensions` check.
    #[serde(default)]
    pub include: String,
    /// Regex blacklist on the file name: files whose name matches are skipped, even if
    /// they pass `include`. Empty = nothing excluded.
    #[serde(default)]
    pub exclude: String,
    /// Per-extension destination overrides (RAW→A, JPG→B). Tried in order, first match
    /// wins; files matching no route fall back to the rule's own `dest`/`layout`.
    /// Empty = a single destination. Must stay the LAST field: it serializes as an
    /// array-of-tables (`[[card.routes]]`) and the `toml` serializer needs every scalar
    /// key emitted before it. (No serde rename — the field name is the wire key the TS
    /// IPC layer reads, so TOML and JSON must agree on `routes`.)
    #[serde(default)]
    pub routes: Vec<Route>,
    /// Optional second destination (3-2-1 backup): every file is also copied and
    /// verified here, mirroring the folder structure. The drive is only erased once
    /// *both* copies verified. Empty = no backup.
    #[serde(default)]
    pub backup_dest: String,
    /// How RAW+JPEG pairs (same name, e.g. DSC0001.ARW + DSC0001.JPG) are handled.
    #[serde(default)]
    pub raw_jpeg: RawJpeg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RawJpeg {
    /// Treat every file on its own.
    #[default]
    Independent,
    /// The JPEG follows its RAW's route and shares its `{seq}` number.
    Together,
    /// Don't copy a JPEG that has a RAW twin (it stays on the drive).
    SkipJpeg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NotifyLevel {
    #[default]
    All,
    /// Only failures and things that need you (unreachable destination, permissions).
    Errors,
    Off,
}

/// One extension→destination route inside a [`CardRule`] (feature: split-by-extension).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Route {
    /// Extensions this route claims (case-insensitive). Empty = catch-all (the rest).
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Destination root override; empty = the rule's `dest`.
    #[serde(default)]
    pub dest: String,
    /// Layout override; empty = the rule's `layout`.
    #[serde(default)]
    pub layout: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NameMode {
    /// One name per distinct capture date.
    #[default]
    PerDate,
    /// One name applied to the whole import.
    Single,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CleanupPolicy {
    #[default]
    Ask,
    Always,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EjectPolicy {
    #[default]
    Never,
    Ask,
    Always,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AlbumMode {
    /// Import into the Photos library only — no album.
    Library,
    /// One fixed album, named by `photos_album`.
    #[default]
    Fixed,
    /// Album(s) named from `photos_album` as a date template, grouped by each
    /// file's capture date — the same token rules as a card's folder layout.
    Template,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AfterImport {
    Archive,
    Delete,
    /// Default to the non-destructive choice.
    #[default]
    Leave,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default = "default_true")]
    pub autostart: bool,
    /// Hide to the menu bar on window close instead of quitting.
    #[serde(default = "default_true")]
    pub keep_running_on_close: bool,
    /// Launch without showing the main window (stay in the tray/menu bar).
    #[serde(default)]
    pub start_minimized: bool,
    /// Show the app's icon in the macOS Dock.
    #[serde(default)]
    pub show_dock_icon: bool,
    /// Show the app's icon in the macOS menu bar.
    #[serde(default = "default_true")]
    pub show_tray_icon: bool,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default)]
    pub notify: NotifyLevel,
    /// Only start automatic runs between `run_from` and `run_until` (local `HH:MM`).
    /// Both blank = any time. `run_from > run_until` spans midnight (e.g. 22:00–07:00).
    /// Work that arrives outside the window runs when it opens.
    #[serde(default)]
    pub run_from: String,
    #[serde(default)]
    pub run_until: String,
    /// Hold automatic runs while the computer is on battery.
    #[serde(default)]
    pub pause_on_battery: bool,
}

impl AppSettings {
    /// At least one of the Dock or menu-bar icon must stay visible, else the
    /// app has no surface and becomes unreachable. If both are off, keep the tray.
    pub fn ensure_reachable(&mut self) {
        if !self.show_dock_icon && !self.show_tray_icon {
            self.show_tray_icon = true;
        }
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        AppSettings {
            autostart: true,
            keep_running_on_close: true,
            start_minimized: false,
            show_dock_icon: false,
            show_tray_icon: true,
            log_level: default_log_level(),
            notify: NotifyLevel::All,
            run_from: String::new(),
            run_until: String::new(),
            pause_on_battery: false,
        }
    }
}

/// Minutes since midnight for `HH:MM`; `None` when blank or malformed.
fn hhmm(s: &str) -> Option<u32> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

impl AppSettings {
    /// True if `now` (minutes since midnight) is inside the run window. An unset or
    /// malformed window allows any time — a typo must never silently stop every flow.
    pub fn in_run_window(&self, now: u32) -> bool {
        match (hhmm(&self.run_from), hhmm(&self.run_until)) {
            (Some(a), Some(b)) if a < b => (a..b).contains(&now),
            (Some(a), Some(b)) if a > b => now >= a || now < b, // spans midnight
            _ => true,
        }
    }

    /// Reject a half-filled or malformed window at save time.
    pub fn validate_window(&self) -> std::result::Result<(), String> {
        match (self.run_from.trim().is_empty(), self.run_until.trim().is_empty()) {
            (true, true) => Ok(()),
            _ if hhmm(&self.run_from).is_some() && hhmm(&self.run_until).is_some() => Ok(()),
            _ => Err("Schedule: enter both times as HH:MM, or leave both blank".into()),
        }
    }
}

fn default_true() -> bool {
    true
}
fn default_layout() -> String {
    "{year}/{date} {name}".into()
}
fn default_folder_layout() -> String {
    "{year}/{date}".into()
}
fn default_album() -> String {
    "Lightroom".into()
}
fn default_log_level() -> String {
    "info".into()
}

impl Config {
    /// The drive rule for a volume UUID (case-insensitive — diskutil and users disagree).
    pub fn card(&self, uuid: &str) -> Option<&CardRule> {
        self.cards.iter().find(|c| c.uuid.eq_ignore_ascii_case(uuid))
    }

    /// Load config from `path`; a missing file yields the default config (not an error).
    pub fn load(path: &Path) -> Result<Config> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(|e| Error::Config(e.to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(Error::Io(e)),
        }
    }

    /// Write config to `path`, creating parent directories as needed.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| Error::Config(e.to_string()))?;
        std::fs::write(path, text)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_rule_kinds_roundtrip_with_only_their_own_fields() {
        let toml = r#"
            [[folder]]
            label = "Downloads"
            watch = "~/Downloads/Incoming"
            kind = "folder"
            dest = "~/Pictures/Imports"
            layout = "{year}/{date}"

            [[folder]]
            label = "Import to Photos"
            watch = "~/Pictures/Exports"
            kind = "photos"
            album_mode = "library"
            archive_folder = "~/Pictures/Exports/_done"
            after_import = "archive"
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert_eq!(cfg.folders.len(), 2);
        assert!(matches!(cfg.folders[0].target, Destination::Folder { .. }));
        let Destination::Photos { album_mode, after_import, .. } = &cfg.folders[1].target else {
            panic!("second rule is Photos");
        };
        assert_eq!(*album_mode, AlbumMode::Library);
        assert_eq!(*after_import, AfterImport::Archive);

        // A folder rule serializes with no Photos fields, and vice versa.
        let out = toml::to_string_pretty(&cfg).unwrap();
        let folder_block = out.split("[[folder]]").nth(1).unwrap();
        assert!(!folder_block.contains("album_mode"), "folder rule has no photos fields:\n{out}");
        let photos_block = out.split("[[folder]]").nth(2).unwrap();
        assert!(!photos_block.contains("dest ="), "photos rule has no folder fields:\n{out}");
    }

    #[test]
    fn ensure_reachable_keeps_a_surface() {
        // Both off → tray restored; any other combination is left untouched.
        let mut hidden = AppSettings { show_dock_icon: false, show_tray_icon: false, ..Default::default() };
        hidden.ensure_reachable();
        assert!(hidden.show_tray_icon && !hidden.show_dock_icon);

        let mut dock_only = AppSettings { show_dock_icon: true, show_tray_icon: false, ..Default::default() };
        dock_only.ensure_reachable();
        assert!(dock_only.show_dock_icon && !dock_only.show_tray_icon);
    }

    #[test]
    fn run_window_handles_unset_daytime_and_midnight_spans() {
        let w = |a: &str, b: &str| AppSettings { run_from: a.into(), run_until: b.into(), ..Default::default() };
        assert!(w("", "").in_run_window(0), "unset = always");
        assert!(w("9:00", "x").in_run_window(0), "malformed = always (and rejected at save)");
        assert!(w("9:00", "x").validate_window().is_err());
        assert!(w("", "").validate_window().is_ok());
        let day = w("09:00", "17:30");
        assert!(day.in_run_window(9 * 60) && !day.in_run_window(17 * 60 + 30) && !day.in_run_window(60));
        let night = w("22:00", "07:00");
        assert!(night.in_run_window(23 * 60) && night.in_run_window(60) && !night.in_run_window(12 * 60));
    }
}
