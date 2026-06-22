//! Shared app state: the live config, where it persists, recent activity, and the
//! durable run history that powers the Flow map.

use fileflow_core::config::Config;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

const ACTIVITY_CAP: usize = 200;
const RUN_CAP: usize = 500;

#[derive(Clone, Serialize)]
pub struct ActivityEntry {
    pub flow: String,    // "card" | "photos"
    pub message: String,
    pub ts: String,      // local HH:MM:SS
}

/// One completed run of one rule. Persisted as a JSONL line beside the config so the
/// Flow map can show real counts + last-run health across restarts.
#[derive(Clone, Serialize, Deserialize)]
pub struct RunRecord {
    pub ts: String,       // RFC3339 (chrono::Local)
    pub flow: String,     // "drive" | "folder" | "photos"
    pub rule_key: String, // "card:{uuid}" | "folder:{watch}" — joins a run back to its rule
    pub label: String,
    pub source: String,
    pub dest: String,
    pub ok: usize,        // copied / moved / imported
    pub skipped: usize,   // ingest skips; 0 elsewhere
    pub failed: usize,
    pub status: String,   // "ok" | "partial" | "failed"
    pub detail: String,
}

pub struct AppState {
    pub config: Mutex<Config>,
    pub config_path: PathBuf,
    pub activity: Mutex<VecDeque<ActivityEntry>>,
    pub runs: Mutex<VecDeque<RunRecord>>,
    pub runs_path: PathBuf,
    /// When true, automatic (watcher-initiated) flows are skipped. Manual triggers ignore it.
    pub paused: AtomicBool,
}

impl AppState {
    pub fn new(config: Config, config_path: PathBuf) -> Self {
        // runs.jsonl sits beside config.toml; seed memory from it so history survives restarts.
        let runs_path = config_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("runs.jsonl");
        let runs = load_runs(&runs_path);
        AppState {
            config: Mutex::new(config),
            config_path,
            activity: Mutex::new(VecDeque::new()),
            runs: Mutex::new(runs),
            runs_path,
            paused: AtomicBool::new(false),
        }
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
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

    /// Record a completed run: append one JSONL line and keep the capped in-memory copy.
    /// A file-append failure is logged, not fatal — the in-memory copy still updates.
    pub fn push_run(&self, rec: RunRecord) {
        if let Ok(line) = serde_json::to_string(&rec) {
            match std::fs::OpenOptions::new().create(true).append(true).open(&self.runs_path) {
                Ok(mut f) => {
                    let _ = writeln!(f, "{line}");
                }
                Err(e) => tracing::warn!("could not append run history: {e}"),
            }
        }
        let mut runs = self.runs.lock().unwrap();
        runs.push_back(rec);
        while runs.len() > RUN_CAP {
            runs.pop_front();
        }
    }

    /// Most-recent-first, capped at `limit`.
    pub fn recent_runs(&self, limit: usize) -> Vec<RunRecord> {
        let r = self.runs.lock().unwrap();
        r.iter().rev().take(limit).cloned().collect()
    }
}

/// Read `runs.jsonl`, skipping any unparseable line, keeping the most recent `RUN_CAP`.
/// A missing file is the normal first-run case and yields an empty history.
fn load_runs(path: &Path) -> VecDeque<RunRecord> {
    let Ok(file) = std::fs::File::open(path) else {
        return VecDeque::new();
    };
    let mut runs: VecDeque<RunRecord> = std::io::BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| serde_json::from_str(&l).ok())
        .collect();
    while runs.len() > RUN_CAP {
        runs.pop_front();
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(i: usize) -> RunRecord {
        RunRecord {
            ts: format!("2026-06-22T10:00:{i:02}+02:00"),
            flow: "folder".into(),
            rule_key: format!("folder:/w/{i}"),
            label: format!("rule {i}"),
            source: "~/in".into(),
            dest: "~/out".into(),
            ok: i,
            skipped: 0,
            failed: 0,
            status: "ok".into(),
            detail: "moved".into(),
        }
    }

    #[test]
    fn runs_round_trip_through_jsonl_and_cap() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("config.toml");
        let state = AppState::new(Config::default(), cfg_path);

        // Append more than the cap; memory keeps the last RUN_CAP.
        let total = RUN_CAP + 50;
        for i in 0..total {
            state.push_run(rec(i));
        }
        assert_eq!(state.runs.lock().unwrap().len(), RUN_CAP);
        let recent = state.recent_runs(3);
        assert_eq!(recent[0].ok, total - 1, "most-recent first");

        // Reload from disk: same cap, oldest dropped (the file holds all lines, loader caps).
        let reloaded = load_runs(&state.runs_path);
        assert_eq!(reloaded.len(), RUN_CAP);
        assert_eq!(reloaded.back().unwrap().ok, total - 1);
        assert_eq!(reloaded.front().unwrap().ok, total - RUN_CAP);
    }
}
