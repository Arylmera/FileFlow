//! Shared app state: the live config, where it persists, and recent activity.

use fileflow_core::config::Config;
use serde::Serialize;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;

const ACTIVITY_CAP: usize = 200;

#[derive(Clone, Serialize)]
pub struct ActivityEntry {
    pub flow: String,    // "card" | "photos"
    pub message: String,
    pub ts: String,      // local HH:MM:SS
}

pub struct AppState {
    pub config: Mutex<Config>,
    pub config_path: PathBuf,
    pub activity: Mutex<VecDeque<ActivityEntry>>,
}

impl AppState {
    pub fn new(config: Config, config_path: PathBuf) -> Self {
        AppState {
            config: Mutex::new(config),
            config_path,
            activity: Mutex::new(VecDeque::new()),
        }
    }

    /// Snapshot the current config (clone) so callers don't hold the lock across work.
    pub fn snapshot(&self) -> Config {
        self.config.lock().unwrap().clone()
    }

    pub fn push_activity(&self, entry: ActivityEntry) {
        let mut a = self.activity.lock().unwrap();
        a.push_back(entry);
        while a.len() > ACTIVITY_CAP {
            a.pop_front();
        }
    }

    /// Most-recent-first, capped at `limit`.
    pub fn recent_activity(&self, limit: usize) -> Vec<ActivityEntry> {
        let a = self.activity.lock().unwrap();
        a.iter().rev().take(limit).cloned().collect()
    }
}
