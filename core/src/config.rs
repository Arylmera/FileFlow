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
    /// Show the app's icon in the macOS Dock.
    #[serde(default)]
    pub show_dock_icon: bool,
    /// Show the app's icon in the macOS menu bar.
    #[serde(default = "default_true")]
    pub show_tray_icon: bool,
    #[serde(default = "default_log_level")]
    pub log_level: String,
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
            show_dock_icon: false,
            show_tray_icon: true,
            log_level: default_log_level(),
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
}
