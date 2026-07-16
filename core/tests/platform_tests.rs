//! Cross-platform behaviour: `~` expansion, Windows volume discovery (pure parsing of
//! PowerShell output), Windows eject/reveal command building, and Windows-safe names.
//!
//! Everything here is pure, so these tests run on every platform — the Windows code
//! paths are exercised without needing a Windows host.

use fileflow_core::ingest::expand_with_home;
use fileflow_core::layout;
use fileflow_core::windows::{
    eject_script, explorer_select_arg, parse_win32_volumes, sanitize_segment, DriveKind,
};

// --- ~ expansion -----------------------------------------------------------

#[test]
fn expand_substitutes_home_on_unix_style_paths() {
    assert_eq!(
        expand_with_home("~/Pictures/Import", Some("/Users/g")),
        std::path::PathBuf::from("/Users/g/Pictures/Import")
    );
}

#[test]
fn expand_substitutes_home_on_windows_style_home() {
    // Windows accepts both separators, so `~/…` under a `C:\` home stays valid.
    assert_eq!(
        expand_with_home("~/Pictures", Some(r"C:\Users\g")),
        std::path::PathBuf::from(r"C:\Users\g/Pictures")
    );
}

#[test]
fn expand_without_home_or_tilde_is_identity() {
    assert_eq!(expand_with_home("~/x", None), std::path::PathBuf::from("~/x"));
    assert_eq!(
        expand_with_home(r"D:\Photos", Some(r"C:\Users\g")),
        std::path::PathBuf::from(r"D:\Photos")
    );
    assert_eq!(expand_with_home("/abs/path", Some("/home/g")), std::path::PathBuf::from("/abs/path"));
}

// --- Win32_Volume JSON parsing ----------------------------------------------

// Real shape of `Get-CimInstance Win32_Volume | Select-Object … | ConvertTo-Json`.
const TWO_VOLUMES: &str = r#"[
  {
    "DriveLetter": "C:",
    "DeviceID": "\\\\?\\Volume{11111111-2222-3333-4444-555555555555}\\",
    "Label": null,
    "DriveType": 3
  },
  {
    "DriveLetter": "E:",
    "DeviceID": "\\\\?\\Volume{aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee}\\",
    "Label": "SONY_SD",
    "DriveType": 2
  }
]"#;

#[test]
fn parses_a_volume_list() {
    let vols = parse_win32_volumes(TWO_VOLUMES).unwrap();
    assert_eq!(vols.len(), 2);
    let sd = &vols[1];
    assert_eq!(sd.drive_letter.as_deref(), Some("E:"));
    assert_eq!(sd.uuid.as_deref(), Some("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"));
    assert_eq!(sd.label.as_deref(), Some("SONY_SD"));
    assert_eq!(sd.kind, DriveKind::Removable);
    assert_eq!(sd.mount_root(), Some(std::path::PathBuf::from(r"E:\")));
}

#[test]
fn parses_a_single_volume_object() {
    // PowerShell's ConvertTo-Json collapses a 1-element list to a bare object.
    let one = r#"{
      "DriveLetter": "D:",
      "DeviceID": "\\\\?\\Volume{aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee}\\",
      "Label": "USB",
      "DriveType": 2
    }"#;
    let vols = parse_win32_volumes(one).unwrap();
    assert_eq!(vols.len(), 1);
    assert_eq!(vols[0].drive_letter.as_deref(), Some("D:"));
}

#[test]
fn volume_without_a_letter_has_no_mount_root() {
    // Recovery/EFI partitions surface with DriveLetter = null; they must be kept
    // parseable but unmountable, never crashing the poller.
    let json = r#"[{ "DriveLetter": null, "DeviceID": "\\\\?\\Volume{11111111-2222-3333-4444-555555555555}\\", "Label": null, "DriveType": 3 }]"#;
    let vols = parse_win32_volumes(json).unwrap();
    assert_eq!(vols[0].drive_letter, None);
    assert_eq!(vols[0].mount_root(), None);
}

#[test]
fn garbage_or_empty_json_is_an_error_not_a_panic() {
    assert!(parse_win32_volumes("not json").is_err());
    // An empty result set (no volumes) parses to an empty list.
    assert_eq!(parse_win32_volumes("[]").unwrap().len(), 0);
}

#[test]
fn unknown_drive_type_maps_to_other() {
    let json = r#"[{ "DriveLetter": "Z:", "DeviceID": "\\\\?\\Volume{11111111-2222-3333-4444-555555555555}\\", "Label": "NET", "DriveType": 4 }]"#;
    assert_eq!(parse_win32_volumes(json).unwrap()[0].kind, DriveKind::Other);
}

// --- eject / reveal command building ----------------------------------------

#[test]
fn eject_script_targets_the_drive_letter() {
    let script = eject_script("E:").unwrap();
    assert!(script.contains(r#"ParseName("E:")"#), "script: {script}");
    assert!(script.contains("InvokeVerb"), "script: {script}");
}

#[test]
fn eject_script_rejects_anything_but_a_drive_letter() {
    // The letter is interpolated into a PowerShell string — never accept free text.
    assert!(eject_script("E").is_none());
    assert!(eject_script(r"E:\").is_none());
    assert!(eject_script("EE:").is_none());
    assert!(eject_script("\"; Remove-Item -Recurse C:\\").is_none());
}

#[test]
fn explorer_select_arg_wraps_the_path() {
    assert_eq!(explorer_select_arg(r"C:\Users\g\Pictures\a.jpg"), r"/select,C:\Users\g\Pictures\a.jpg");
}

// --- Windows-safe folder / file names ----------------------------------------

#[test]
fn sanitize_replaces_windows_invalid_characters() {
    assert_eq!(sanitize_segment("Trip: day 1?"), "Trip- day 1-");
    assert_eq!(sanitize_segment(r#"a<b>c"d|e*f"#), "a-b-c-d-e-f");
    assert_eq!(sanitize_segment("plain name"), "plain name");
}

#[test]
fn sanitize_trims_trailing_dots_and_spaces() {
    // Windows silently strips trailing dots/spaces, which would desync verify paths.
    assert_eq!(sanitize_segment("name. "), "name");
    assert_eq!(sanitize_segment("name..."), "name");
}

#[test]
fn sanitize_guards_reserved_device_names() {
    assert_eq!(sanitize_segment("CON"), "CON_");
    assert_eq!(sanitize_segment("aux"), "aux_");
    assert_eq!(sanitize_segment("COM1"), "COM1_");
    assert_eq!(sanitize_segment("console"), "console"); // only exact names are reserved
}

#[test]
fn layout_render_windows_safe_keeps_separators_but_cleans_segments() {
    // The `/` between template segments must survive; invalid chars inside a
    // segment (here the user-typed name) must not.
    let got = layout::render_windows_safe("{year}/{date} {name}", "2026", "2026-06-20", "Trip: A*");
    assert_eq!(got, "2026/2026-06-20 Trip- A-");
}

// --- platform gating of the two OS-integration points -------------------------

/// Off macOS, a Photos import must fail loudly (never silently drop files).
#[cfg(not(target_os = "macos"))]
#[test]
fn photos_import_is_unsupported_off_macos() {
    use fileflow_core::photos::{import_to_photos, AlbumTarget};
    let err = import_to_photos(
        &[std::path::PathBuf::from("x.jpg")],
        &AlbumTarget::Library,
        true,
    )
    .unwrap_err();
    assert!(matches!(err, fileflow_core::Error::PhotosUnsupported));
}

/// On a platform with no eject backend (e.g. Linux dev builds), an explicit eject
/// request errors instead of pretending to succeed. Policy `Never` stays a no-op.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
#[test]
fn eject_is_unsupported_where_theres_no_backend() {
    use fileflow_core::config::EjectPolicy;
    use fileflow_core::ingest::eject;
    assert!(!eject(std::path::Path::new("/tmp"), EjectPolicy::Never).unwrap());
    assert!(eject(std::path::Path::new("/tmp"), EjectPolicy::Always).is_err());
}
