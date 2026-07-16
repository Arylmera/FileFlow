//! Guards on the Tauri bundle configuration for the Windows MSI installer.
//!
//! These live in `fileflow-core` (not `src-tauri`) deliberately: core's tests run on
//! every platform without the Tauri toolchain, so a broken installer config fails CI
//! everywhere — not only on the one job that actually builds the MSI.

use serde_json::Value;
use std::path::Path;

fn tauri_conf() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src-tauri/tauri.conf.json");
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
}

fn package_json() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../package.json");
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
}

/// The MSI's WiX upgrade code is what lets a newer installer replace an older install
/// instead of appearing beside it. It must be an explicit, stable GUID — deriving it
/// implicitly (from productName) risks silently changing if the name ever does.
#[test]
fn msi_has_a_stable_wix_upgrade_code() {
    let conf = tauri_conf();
    let code = conf["bundle"]["windows"]["wix"]["upgradeCode"]
        .as_str()
        .expect("bundle.windows.wix.upgradeCode must be set");
    let parts: Vec<&str> = code.split('-').collect();
    let lens: Vec<usize> = parts.iter().map(|p| p.len()).collect();
    assert_eq!(lens, [8, 4, 4, 4, 12], "upgradeCode must be a GUID, got {code}");
    assert!(
        code.chars().all(|c| c.is_ascii_hexdigit() || c == '-'),
        "upgradeCode must be hex, got {code}"
    );
}

/// Users without WebView2 (fresh Windows 10 installs) need the installer to fetch it,
/// or FileFlow starts to a blank window.
#[test]
fn msi_installs_webview2_when_missing() {
    let conf = tauri_conf();
    let mode = conf["bundle"]["windows"]["webviewInstallMode"]["type"]
        .as_str()
        .expect("bundle.windows.webviewInstallMode.type must be set");
    assert_eq!(mode, "downloadBootstrapper");
}

/// The Windows bundler requires an .ico icon; the config must keep listing one.
#[test]
fn bundle_icons_include_an_ico() {
    let conf = tauri_conf();
    let icons = conf["bundle"]["icon"].as_array().expect("bundle.icon must be a list");
    assert!(
        icons.iter().any(|i| i.as_str().is_some_and(|s| s.ends_with(".ico"))),
        "bundle.icon must include a .ico for the Windows installer"
    );
}

/// The MSI version comes from tauri.conf.json; keep it in lockstep with package.json
/// so a release bump can't ship two different versions.
#[test]
fn app_versions_agree() {
    assert_eq!(
        tauri_conf()["version"].as_str().unwrap(),
        package_json()["version"].as_str().unwrap(),
        "tauri.conf.json and package.json versions must match"
    );
}
