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
    pub lightroom: Option<LightroomRule>,
    #[serde(default)]
    pub app: AppSettings,
}

/// Watch a folder and move whatever lands in it into a dated destination.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderRule {
    #[serde(default)]
    pub label: String,
    pub watch: String,
    pub dest: String,
    #[serde(default = "default_folder_layout")]
    pub layout: String,
    /// Empty = move all file types.
    #[serde(default)]
    pub extensions: Vec<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LightroomRule {
    pub watch_folder: String,
    #[serde(default)]
    pub album_mode: AlbumMode,
    /// Fixed album name (`Fixed`) or a date template (`Template`); ignored for `Library`.
    #[serde(default = "default_album")]
    pub photos_album: String,
    /// Ask for a name before importing (used by the `Template` album mode's `{name}`).
    #[serde(default)]
    pub prompt_name: bool,
    #[serde(default)]
    pub name_mode: NameMode,
    #[serde(default = "default_true")]
    pub skip_duplicates: bool,
    #[serde(default)]
    pub after_import: AfterImport,
    #[serde(default)]
    pub archive_folder: String,
    #[serde(default)]
    pub extensions: Vec<String>,
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
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        AppSettings {
            autostart: true,
            keep_running_on_close: true,
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
