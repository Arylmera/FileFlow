<p align="center">
  <img src="docs/assets/banner.svg" alt="FileFlow — automatic file ingest & routing for macOS" width="100%">
</p>

<p align="center">
  <img src="https://img.shields.io/badge/platform-macOS-000000?logo=apple&logoColor=white" alt="Platform: macOS">
  <a href="https://tauri.app"><img src="https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white" alt="Tauri 2"></a>
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/Rust-1.77+-DEA584?logo=rust&logoColor=white" alt="Rust 1.77+"></a>
  <a href="https://react.dev"><img src="https://img.shields.io/badge/React-19-61DAFB?logo=react&logoColor=black" alt="React 19"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-3B6FE0" alt="License: MIT"></a>
</p>

<p align="center">
  A quiet macOS menu-bar app that moves your files where they belong — automatically, and safely.<br>
  Ingest a <b>drive into dated folders</b>, route <b>one folder into another</b>, or import straight <b>into Apple Photos</b>.
</p>

<p align="center">
  <a href="#what-it-does">What it does</a> ·
  <a href="#safety-model">Safety</a> ·
  <a href="#install">Install</a> ·
  <a href="#first-run-setup">Setup</a> ·
  <a href="#configure">Configure</a> ·
  <a href="#build-from-source">Build</a> ·
  <a href="#files--locations">Files</a>
</p>

---

## What it does

FileFlow watches **sources** and routes new files to **destinations** — verified on the way,
safe by default. Three flows, set up once from the control panel and then left alone:

| | Flow | What happens |
|---|---|---|
| 💾 | **Drive → folder** | Plug in a recognised external drive (SD card, USB stick, …) and FileFlow copies its files into a destination, organized by capture date, verifies every copy, then — optionally — wipes and ejects the drive. Moves any file type by default; filter by extension if you want. |
| 📁 | **Folder → folder** | Watch a folder; whatever lands in it is moved into a target folder, filtered by extension. |
| 🖼️ | **Folder → Photos** | Point a watched folder (e.g. your Lightroom exports) at an Apple Photos album, and new files are imported automatically. |

A destination is **any writable path** — a local folder, a cloud-synced folder (OneDrive,
iCloud Drive, Dropbox, Google Drive), a mounted network share (SMB/NFS, e.g. a TrueNAS
dataset), or another drive — plus Apple Photos as a built-in destination.

The goal is a tool you trust to run unattended and rarely need to open.

## Safety model

Your files are irreplaceable, so the destructive path — wiping a drive after ingest — is deliberately paranoid:

- **Verified copies** — every file is checked (byte size) at the destination before it counts as copied.
- **All-or-nothing deletion** — the drive is wiped *only* after *every* file in the set has copied and verified. Any single failure — including the destination going unreachable mid-run — aborts deletion entirely, so a partial or interrupted copy can never lose data.
- **Idempotent re-runs** — identical-size files already at the destination are skipped, so running it again is safe.

## Install

FileFlow is a personal utility, not an App Store download — you run it as a `.app` you build (or are handed).

- **You have a build already** — drag `FileFlow.app` into `/Applications` and launch it. macOS Gatekeeper warns the first time an ad-hoc / unsigned build runs: right-click the app ▸ **Open**, or clear the quarantine flag once with
  ```sh
  xattr -dr com.apple.quarantine /Applications/FileFlow.app
  ```
- **You're building it yourself** — see [Build from source](#build-from-source).

On first launch FileFlow lives in the **menu bar** (no Dock icon by default). Click the tray icon to open the control panel.

## First-run setup

Depending on which flows you use, FileFlow needs one or two macOS permissions. It detects when either is missing and surfaces guidance in the window — you don't have to hunt for them blind.

| Permission | Why it's needed | Where to grant it |
|---|---|---|
| **Full Disk Access** | Read drive contents under `/Volumes` and write into protected destinations (e.g. `~/Library/CloudStorage`). | *System Settings ▸ Privacy & Security ▸ Full Disk Access* |
| **Automation (Photos)** | Control Photos to import into an album — only for a *Folder → Photos* rule. macOS prompts on the first import. | *System Settings ▸ Privacy & Security ▸ Automation* |

> [!NOTE]
> Ad-hoc dev builds re-prompt for these on every rebuild, because the signature changes each time. To make grants stick, sign with a stable identity — see [Code signing & TCC persistence](#code-signing--tcc-persistence).

## Configure

Open the control panel from the tray icon, then add the rules you need:

1. **Drive rule** (drive → folder) — pick a drive to recognise and its destination; choose whether to verify-then-wipe-and-eject after a successful copy, and optionally restrict it to certain extensions.
2. **Folder rule** (folder → folder, or folder → Photos) — point a watched folder at a target folder or an Apple Photos album, filtered by extension.
3. Leave it running. Trigger a run manually any time, and check the **activity log** for what happened.

Settings live in a single config file, managed entirely from the UI — see [Files & locations](#files--locations).

## Build from source

**Prerequisites:** Rust ≥ 1.77, Node ≥ 18, Xcode Command Line Tools.

```sh
npm install
npm run tauri dev             # run with hot-reload
cargo test -p fileflow-core   # run the domain-logic tests
```

Produce a release build:

```sh
npm run tauri build      # → target/release/bundle/macos/FileFlow.app
npm run build:dmg        # also build the installer .dmg (GUI session only — see below)
```

The default build produces the `.app` only, which works headlessly. The **`.dmg`** step
drives Finder via AppleScript to lay out the disk-image window and only succeeds in an
interactive GUI login session — run `npm run build:dmg` from a logged-in desktop, or just
distribute the `.app`.

### Code signing & TCC persistence

The default build is **ad-hoc signed**, so its signature changes on every rebuild and
macOS re-prompts for the permissions above each time. To make grants persist across
rebuilds, sign with a stable identity (Apple Development or Developer ID) — list yours with
`security find-identity -v -p codesigning`:

```sh
export APPLE_SIGNING_IDENTITY="Apple Development: you@example.com (TEAMID)"
npm run tauri build
```

(or set `bundle.macOS.signingIdentity` in `src-tauri/tauri.conf.json`). The bundle
identifier `com.guillaumelemer.fileflow` is fixed, which TCC also keys on. The
hardened-runtime entitlement needed to control Photos lives in
[`src-tauri/entitlements.plist`](src-tauri/entitlements.plist)
(`com.apple.security.automation.apple-events`) and is applied automatically when signing;
it's inert for ad-hoc dev builds.

## Files & locations

| | Path |
|---|---|
| **Config** | `~/Library/Application Support/com.guillaumelemer.fileflow/config.toml` — managed from the UI (*Settings ▸ Open config folder*). |
| **Logs** | `~/Library/Logs/com.guillaumelemer.fileflow/fileflow.log` — *Settings ▸ Open log file*; verbosity via *Settings ▸ Log level*. |

By default the app runs as a menu-bar agent (no Dock icon), reachable from the tray.
*Settings* toggles the Dock and menu-bar icons independently — at least one stays visible so
the window is always reachable. It launches at login when enabled and enforces a single
instance.

## Architecture

Rust + Tauri 2 + React/TypeScript. Domain logic lives in a pure, Tauri-free `core` crate
(unit-tested); `src-tauri` is the shell, `src` the UI.

```
core/        pure domain logic (config, ingest, photos) — `cargo test -p fileflow-core`
src-tauri/   Tauri shell: watchers, commands, tray, state
src/         React control panel
```

## License

[MIT](LICENSE) © 2026 Lemer Guillaume
