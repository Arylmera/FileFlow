use fileflow_core::config::{AfterImport, CardRule, CleanupPolicy, EjectPolicy, NameMode};
use fileflow_core::ingest::{
    cleanup, plan_ingest, run_ingest, scan_dates, scan_files, FailedCopy, IngestReport,
};
use fileflow_core::photos::{after_import, album_groups, build_import_script};
use fileflow_core::{config::Config, layout};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn rule(sources: &[&str], dest: &str, exts: &[&str]) -> CardRule {
    CardRule {
        uuid: "TEST-UUID".into(),
        label: "Test".into(),
        sources: sources.iter().map(|s| s.to_string()).collect(),
        dest: dest.into(),
        layout: "{year}/{date} {name}".into(),
        prompt_name: true,
        name_mode: NameMode::PerDate,
        cleanup: CleanupPolicy::Ask,
        eject: EjectPolicy::Never,
        extensions: exts.iter().map(|s| s.to_string()).collect(),
    }
}

fn write_file(path: &Path, bytes: &[u8], unix_mtime: i64) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
    let ft = filetime::FileTime::from_unix_time(unix_mtime, 0);
    filetime::set_file_mtime(path, ft).unwrap();
}

// Two timestamps ~2 days apart → distinct local calendar dates in every timezone.
const DAY_A: i64 = 1_718_000_000; // 2024-06-10ish
const DAY_B: i64 = 1_718_200_000; // 2024-06-12ish

#[test]
fn layout_render_drops_empty_name() {
    assert_eq!(layout::render("{year}/{date} {name}", "2026", "2026-06-20", "Trip"), "2026/2026-06-20 Trip");
    assert_eq!(layout::render("{year}/{date} {name}", "2026", "2026-06-20", ""), "2026/2026-06-20");
    assert_eq!(layout::render("{date}", "2026", "2026-06-20", ""), "2026-06-20");
}

#[test]
fn scan_files_respects_globs_and_extensions() {
    let card = tempfile::tempdir().unwrap();
    let root = card.path();
    write_file(&root.join("DCIM/100MSDCF/a.arw"), b"x", DAY_A);
    write_file(&root.join("DCIM/101MSDCF/b.arw"), b"x", DAY_A);
    write_file(&root.join("DCIM/100MSDCF/c.jpg"), b"x", DAY_A); // filtered out by extensions

    let r = rule(&["DCIM/1*MSDCF"], "/tmp", &["arw"]);
    let files = scan_files(&r, root);
    assert_eq!(files.len(), 2, "glob should match both rollover dirs, jpg filtered out");
    assert!(files.iter().all(|p| p.extension().unwrap() == "arw"));
}

#[test]
fn scan_dates_groups_by_capture_date() {
    let card = tempfile::tempdir().unwrap();
    let root = card.path();
    write_file(&root.join("DCIM/100MSDCF/a.arw"), b"x", DAY_A);
    write_file(&root.join("DCIM/100MSDCF/b.arw"), b"x", DAY_A);
    write_file(&root.join("DCIM/100MSDCF/c.arw"), b"x", DAY_B);

    let groups = scan_dates(&rule(&["DCIM/100MSDCF"], "/tmp", &["arw"]), root);
    assert_eq!(groups.len(), 2, "two distinct capture dates");
    assert_eq!(groups.iter().map(|g| g.file_count).sum::<usize>(), 3);
    assert!(groups[0].date < groups[1].date, "sorted ascending by date");
    assert_eq!(groups.iter().find(|g| g.file_count == 2).map(|_| ()), Some(()));
}

#[test]
fn run_ingest_copies_verifies_and_is_idempotent() {
    let card = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let root = card.path();
    write_file(&root.join("DCIM/100MSDCF/a.arw"), b"hello", DAY_A);
    write_file(&root.join("DCIM/100MSDCF/b.arw"), b"world!!", DAY_B);

    let r = rule(&["DCIM/100MSDCF"], dest.path().to_str().unwrap(), &["arw"]);
    let names = BTreeMap::new(); // no names → folders are just date
    let plan = plan_ingest(&r, root, &names, dest.path());
    assert_eq!(plan.len(), 2);

    let report = run_ingest(&plan);
    assert_eq!(report.copied.len(), 2);
    assert!(report.failed.is_empty());
    assert_eq!(report.folders.len(), 2, "two date folders created");
    // dest files exist with matching size
    for pc in &plan {
        let s = std::fs::metadata(&pc.src).unwrap().len();
        let d = std::fs::metadata(&pc.dest_path).unwrap().len();
        assert_eq!(s, d);
    }

    // Re-run: everything already present → all skipped, nothing re-copied.
    let report2 = run_ingest(&plan);
    assert_eq!(report2.copied.len(), 0);
    assert_eq!(report2.skipped.len(), 2);
}

#[test]
fn cleanup_deletes_only_when_fully_clean() {
    let card = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let root = card.path();
    write_file(&root.join("DCIM/100MSDCF/a.arw"), b"hello", DAY_A);

    let r = rule(&["DCIM/100MSDCF"], dest.path().to_str().unwrap(), &["arw"]);
    let plan = plan_ingest(&r, root, &BTreeMap::new(), dest.path());
    let report = run_ingest(&plan);
    assert!(report.is_clean());

    let deleted = cleanup(&report).unwrap();
    assert_eq!(deleted.len(), 1);
    assert!(!root.join("DCIM/100MSDCF/a.arw").exists(), "source removed after verified copy");
}

#[test]
fn cleanup_blocked_leaves_card_untouched() {
    // A surviving copy plus a failure: cleanup must delete nothing.
    let card = tempfile::tempdir().unwrap();
    let src = card.path().join("a.arw");
    write_file(&src, b"hello", DAY_A);

    let report = IngestReport {
        copied: vec![src.clone()],
        skipped: vec![],
        folders: vec![],
        failed: vec![FailedCopy { src: PathBuf::from("/nope/b.arw"), error: "boom".into() }],
    };
    assert!(cleanup(&report).is_err(), "any failure blocks cleanup");
    assert!(src.exists(), "no source deleted when the set is not fully clean");
}

#[test]
fn photos_script_sets_skip_flag_and_escapes() {
    let files = vec![PathBuf::from("/a/b c.jpg")];
    let yes = build_import_script(&files, Some("My \"Album\""), true);
    assert!(yes.contains("skip check duplicates true"));
    assert!(yes.contains(r#"album "My \"Album\"""#), "album name escaped");
    assert!(yes.contains(r#"POSIX file "/a/b c.jpg""#));

    let no = build_import_script(&files, Some("Lightroom"), false);
    assert!(no.contains("skip check duplicates false"));
}

#[test]
fn photos_library_script_has_no_album() {
    let files = vec![PathBuf::from("/a/b.jpg")];
    let s = build_import_script(&files, None, true);
    assert!(s.contains("import {"));
    assert!(s.contains("skip check duplicates true"));
    assert!(!s.contains("into theAlbum"), "library import must not target an album");
    assert!(!s.contains("make new album"));
}

#[test]
fn album_groups_split_by_date_template() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.jpg");
    let b = dir.path().join("b.jpg");
    let c = dir.path().join("c.jpg");
    write_file(&a, b"x", DAY_A);
    write_file(&b, b"x", DAY_A);
    write_file(&c, b"x", DAY_B);

    let by_date = album_groups(&[a.clone(), b.clone(), c.clone()], "{date}");
    assert_eq!(by_date.len(), 2, "two distinct capture dates → two albums");
    assert_eq!(by_date.values().map(|v| v.len()).sum::<usize>(), 3);

    let by_year = album_groups(&[a, b, c], "{year}");
    assert_eq!(by_year.len(), 1, "same year → one album");
}

#[test]
fn after_import_archive_moves_files() {
    let dir = tempfile::tempdir().unwrap();
    let exp = dir.path().join("export/a.jpg");
    write_file(&exp, b"img", DAY_A);
    let archive = dir.path().join("export/_imported");

    after_import(&[exp.clone()], AfterImport::Archive, &archive).unwrap();
    assert!(!exp.exists(), "source moved out");
    assert!(archive.join("a.jpg").exists(), "file landed in archive");
}

#[test]
fn config_roundtrips_through_toml() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let mut cfg = Config::default();
    cfg.cards.push(rule(&["DCIM/100MSDCF"], "~/dest", &["arw", "jpg"]));
    cfg.save(&path).unwrap();
    let back = Config::load(&path).unwrap();
    assert_eq!(back.cards.len(), 1);
    assert_eq!(back.cards[0].uuid, "TEST-UUID");
    assert_eq!(back.cards[0].extensions, vec!["arw", "jpg"]);

    // Missing file → default, not an error.
    let absent = Config::load(&dir.path().join("nope.toml")).unwrap();
    assert!(absent.cards.is_empty());
}
