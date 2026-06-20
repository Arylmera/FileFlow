//! Shared app state: the live config + where it persists.

use fileflow_core::config::Config;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct AppState {
    pub config: Mutex<Config>,
    // Read by the Phase 4 `save_config` command to persist edits back to disk.
    #[allow(dead_code)]
    pub config_path: PathBuf,
}

impl AppState {
    /// Snapshot the current config (clone) without holding the lock across work.
    pub fn snapshot(&self) -> Config {
        self.config.lock().unwrap().clone()
    }
}
