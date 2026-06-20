# FileFlow

A macOS menu-bar app that automates two photo chores:

1. **Card ingest** — when a recognised SD card is inserted, copy its photos into a
   per-card destination foldered by capture date, verify every copy, then
   (optionally) wipe and eject the card.
2. **Lightroom → Photos** — watch a Lightroom export folder and import new files
   into an Apple Photos album.

The destination is per-card: a local folder, a cloud-synced folder (OneDrive,
iCloud Drive, Dropbox, Google Drive), a mounted network share (SMB/NFS, e.g. a
TrueNAS dataset), or an external drive — anything that resolves to a writable path.

## Safety model

Copy is **verified** (byte size) and deletion is **all-or-nothing**: the card is
wiped only after *every* file in the set has copied and verified. A single failure
— including the destination going unreachable mid-run — aborts deletion entirely,
so a partial or interrupted copy can never lose data. Re-runs are idempotent
(identical-size files at the destination are skipped).

## Stack

Rust + Tauri 2 + React/TypeScript. Domain logic lives in a pure, Tauri-free
`core` crate (unit-tested); `src-tauri` is the shell, `src` the UI.

```
core/        pure domain logic (config, ingest, photos) — `cargo test -p fileflow-core`
src-tauri/   Tauri shell: watchers, commands, tray, state
src/         React control panel
```

## Develop

Prerequisites: Rust ≥ 1.77, Node ≥ 18, Xcode Command Line Tools.

```sh
npm install
npm run tauri dev      # run with hot-reload
cargo test -p fileflow-core
```

## Build

```sh
npm run tauri build
# → target/release/bundle/macos/FileFlow.app
```

The `.app` builds headlessly. The **`.dmg`** step drives Finder via AppleScript to
lay out the disk-image window and only succeeds in an interactive GUI login
session — build it from a logged-in desktop, or distribute the `.app` directly.

## One-time macOS permissions

- **Full Disk Access** — required to read card contents under `/Volumes` and to
  write into protected destinations (e.g. `~/Library/CloudStorage`). Grant under
  *System Settings ▸ Privacy & Security ▸ Full Disk Access*. FileFlow detects the
  blocked case and notifies you.
- **Automation (Photos)** — controlling Photos triggers a prompt on the first
  Lightroom import. If denied, FileFlow surfaces guidance; re-enable under
  *System Settings ▸ Privacy & Security ▸ Automation*.

## Code signing & TCC persistence

The default build is **ad-hoc signed**, so its signature changes on every rebuild
and macOS re-prompts for the permissions above each time. To make grants persist
across rebuilds, sign with a stable identity (Apple Development or Developer ID):

```sh
export APPLE_SIGNING_IDENTITY="Apple Development: you@example.com (TEAMID)"
npm run tauri build
```

or set `bundle.macOS.signingIdentity` in `src-tauri/tauri.conf.json`. The bundle
identifier (`com.guillaumelemer.fileflow`) is fixed, which TCC also keys on.

## Files

- **Config** — `~/Library/Application Support/com.guillaumelemer.fileflow/config.toml`
  (managed entirely from the UI; *Settings ▸ Open config folder*).
- **Logs** — `~/Library/Logs/com.guillaumelemer.fileflow/fileflow.log`
  (*Settings ▸ Open log file*; verbosity via *Settings ▸ Log level*).

The app runs as a menu-bar agent (no Dock icon); the window is reachable from the
tray. It launches at login when enabled and enforces a single instance.
