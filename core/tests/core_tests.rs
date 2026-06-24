use fileflow_core::config::{
    AfterImport, CardRule, CleanupPolicy, EjectPolicy, FolderRule, NameMode, Route,
};
use fileflow_core::folder::run_folder_move;
use fileflow_core::ingest::{
    cleanup, plan_ingest, run_ingest, scan_dates, scan_files, FailedCopy, IngestReport,
};
use fileflow_core::photos::{after_import, album_groups, build_import_script, scan_folder};
use fileflow_core::util::NameFilter;
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
        rename: String::new(),
        include: String::new(),
        exclude: String::new(),
        routes: vec![],
    }
}

/// Local date string (YYYY-MM-DD) for one of the fixed test mtimes, for asserting paths.
fn day_of(unix_mtime: i64) -> String {
    let (_, date) = layout::date_parts(
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(unix_mtime as u64),
    );
    date
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

    let report = run_ingest(&plan, |_, _| {});
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
    let report2 = run_ingest(&plan, |_, _| {});
    assert_eq!(report2.copied.len(), 0);
    assert_eq!(report2.skipped.len(), 2);
}

#[test]
fn render_filename_fills_seq_and_keeps_extension() {
    // {seq} is zero-padded to 4; the extension is appended, never templated.
    assert_eq!(layout::render_filename("{date}_{seq}", "2026", "2026-06-20", "", 7, "arw"), "2026-06-20_0007.arw");
    assert_eq!(layout::render_filename("{name}_{seq}", "2026", "2026-06-20", "Trip", 1, "jpg"), "Trip_0001.jpg");
    // Path separators flatten to '-' (a filename has no folders).
    assert_eq!(layout::render_filename("{year}/{seq}", "2026", "2026-06-20", "", 3, "arw"), "2026-0003.arw");
    // A template that renders to nothing — or to a traversal name — falls back to the sequence.
    assert_eq!(layout::render_filename("{name}", "2026", "2026-06-20", "", 5, "arw"), "0005.arw");
    assert_eq!(layout::render_filename("{name}", "2026", "2026-06-20", "..", 5, ""), "0005");
    assert_eq!(layout::render_filename("{name}", "2026", "2026-06-20", ".", 5, "arw"), "0005.arw");
}

#[test]
fn render_strips_parent_traversal_segments() {
    // A user-typed name with `..`/`.` cannot introduce traversal components.
    let out = layout::render("{year}/{date} {name}", "2026", "2026-06-20", "../../../Volumes/x");
    assert!(!out.split('/').any(|s| s == ".." || s == "."), "no standalone traversal segments: {out}");
    assert_eq!(layout::render("{date}/{name}", "2026", "2026-06-20", "../.."), "2026-06-20");
}

#[test]
fn name_token_cannot_escape_dest_root() {
    // The naming form is free text; a `..`-laden name must still land under the dest root.
    let card = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let root = card.path();
    write_file(&root.join("DCIM/100MSDCF/a.arw"), b"x", DAY_A);

    let mut r = rule(&["DCIM/100MSDCF"], dest.path().to_str().unwrap(), &["arw"]);
    r.layout = "{date} {name}".into();
    let mut names = BTreeMap::new();
    names.insert(day_of(DAY_A), "../../../../escape".to_string());
    let plan = plan_ingest(&r, root, &names, dest.path());

    let dd = &plan[0].dest_dir;
    assert!(
        !dd.components().any(|c| matches!(c, std::path::Component::ParentDir)),
        "dest_dir has no `..` components: {dd:?}"
    );
    assert!(dd.starts_with(dest.path()), "dest_dir stays under the configured root: {dd:?}");
}

#[test]
fn plan_routes_split_by_extension() {
    let card = tempfile::tempdir().unwrap();
    let archive = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let root = card.path();
    write_file(&root.join("DCIM/100MSDCF/a.arw"), b"raw", DAY_A);
    write_file(&root.join("DCIM/100MSDCF/b.jpg"), b"jpg", DAY_A);

    let mut r = rule(&["DCIM/100MSDCF"], dest.path().to_str().unwrap(), &["arw", "jpg"]);
    r.layout = "{date}".into();
    r.routes = vec![
        Route { extensions: vec!["arw".into()], dest: archive.path().to_string_lossy().into(), layout: "RAW".into() },
        Route { extensions: vec!["jpg".into()], dest: String::new(), layout: "jpg".into() },
    ];
    let plan = plan_ingest(&r, root, &BTreeMap::new(), dest.path());

    let arw = plan.iter().find(|p| p.src.extension().unwrap() == "arw").unwrap();
    let jpg = plan.iter().find(|p| p.src.extension().unwrap() == "jpg").unwrap();
    assert_eq!(arw.dest_path, archive.path().join("RAW/a.arw"), "RAW routed to its own root + layout");
    assert_eq!(jpg.dest_path, dest.path().join("jpg/b.jpg"), "JPG kept default root, own subfolder");
}

#[test]
fn plan_routes_fall_back_to_rule_default_when_unmatched() {
    let card = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let root = card.path();
    write_file(&root.join("DCIM/100MSDCF/c.png"), b"png", DAY_A);

    let mut r = rule(&["DCIM/100MSDCF"], dest.path().to_str().unwrap(), &[]); // all extensions
    r.layout = "{date}".into();
    r.routes = vec![Route { extensions: vec!["arw".into()], dest: String::new(), layout: "RAW".into() }];
    let plan = plan_ingest(&r, root, &BTreeMap::new(), dest.path());

    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].dest_path, dest.path().join(day_of(DAY_A)).join("c.png"), "no route matched → rule default");
}

#[test]
fn rename_sequences_per_folder_and_copies() {
    let card = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let root = card.path();
    write_file(&root.join("DCIM/100MSDCF/z.arw"), b"one", DAY_A);
    write_file(&root.join("DCIM/100MSDCF/a.arw"), b"two!!", DAY_A); // same date → same folder

    let mut r = rule(&["DCIM/100MSDCF"], dest.path().to_str().unwrap(), &["arw"]);
    r.layout = "{date}".into();
    r.rename = "{date}_{seq}".into();
    let plan = plan_ingest(&r, root, &BTreeMap::new(), dest.path());

    let day = day_of(DAY_A);
    // Files are scanned sorted, so a.arw → _0001, z.arw → _0002.
    let names: Vec<_> = plan.iter().map(|p| p.dest_path.file_name().unwrap().to_str().unwrap().to_string()).collect();
    assert_eq!(names, vec![format!("{day}_0001.arw"), format!("{day}_0002.arw")]);

    let report = run_ingest(&plan, |_, _| {});
    assert_eq!(report.copied.len(), 2, "both renamed files copied");
    assert!(dest.path().join(&day).join(format!("{day}_0001.arw")).exists());
    assert!(dest.path().join(&day).join(format!("{day}_0002.arw")).exists());
}

#[test]
fn rename_with_seq_never_loses_a_file_when_set_changes() {
    // Regression: with `{seq}` the dest name comes from scan position, not the source. A
    // different, equal-sized file added before the first must be COPIED (its bytes land at
    // the destination), never skipped against the first file's bytes (which cleanup would
    // then delete from the card).
    let card = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let root = card.path();
    write_file(&root.join("DCIM/100MSDCF/m_a.arw"), b"AAAAA", DAY_A); // 5 bytes

    let mut r = rule(&["DCIM/100MSDCF"], dest.path().to_str().unwrap(), &["arw"]);
    r.layout = "{date}".into();
    r.rename = "{seq}".into();
    let day = day_of(DAY_A);

    run_ingest(&plan_ingest(&r, root, &BTreeMap::new(), dest.path()), |_, _| {});
    assert_eq!(std::fs::read(dest.path().join(&day).join("0001.arw")).unwrap(), b"AAAAA");

    write_file(&root.join("DCIM/100MSDCF/a_x.arw"), b"BBBBB", DAY_A); // same size, sorts earlier
    let report = run_ingest(&plan_ingest(&r, root, &BTreeMap::new(), dest.path()), |_, _| {});

    assert!(
        report.copied.iter().any(|p| p.ends_with("DCIM/100MSDCF/a_x.arw")),
        "the newcomer is copied, not silently skipped against the other file's bytes"
    );
    let contents: Vec<Vec<u8>> = std::fs::read_dir(dest.path().join(&day))
        .unwrap().flatten().map(|e| std::fs::read(e.path()).unwrap()).collect();
    assert!(contents.iter().any(|c| c == b"BBBBB"), "newcomer's bytes are physically at the destination");
    assert!(contents.iter().any(|c| c == b"AAAAA"), "the first file is still present too");
}

#[test]
fn rename_reimport_is_idempotent() {
    // Re-importing the same card with a rename template skips everything (content match),
    // never minting fresh {seq} numbers that duplicate the files.
    let card = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let root = card.path();
    write_file(&root.join("DCIM/100MSDCF/f_a.arw"), b"hello", DAY_A);
    write_file(&root.join("DCIM/100MSDCF/f_b.arw"), b"world!!", DAY_A);

    let mut r = rule(&["DCIM/100MSDCF"], dest.path().to_str().unwrap(), &["arw"]);
    r.layout = "{date}".into();
    r.rename = "{seq}".into();
    let day = day_of(DAY_A);

    let report1 = run_ingest(&plan_ingest(&r, root, &BTreeMap::new(), dest.path()), |_, _| {});
    assert_eq!(report1.copied.len(), 2);

    let report2 = run_ingest(&plan_ingest(&r, root, &BTreeMap::new(), dest.path()), |_, _| {});
    assert_eq!(report2.copied.len(), 0, "re-import copies nothing");
    assert_eq!(report2.skipped.len(), 2, "both recognised as already present");
    assert_eq!(std::fs::read_dir(dest.path().join(&day)).unwrap().count(), 2, "no duplicate copies");
}

#[test]
fn colliding_source_names_both_survive() {
    // Two rollover dirs hold a same-named, different file. Both must land (one de-collided),
    // and a re-run must recognise both as already present.
    let card = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let root = card.path();
    write_file(&root.join("DCIM/100MSDCF/x.arw"), b"first", DAY_A);
    write_file(&root.join("DCIM/101MSDCF/x.arw"), b"second!!", DAY_A); // different content + size

    let mut r = rule(&["DCIM/1*MSDCF"], dest.path().to_str().unwrap(), &["arw"]);
    r.layout = "{date}".into();
    let day = day_of(DAY_A);

    let report = run_ingest(&plan_ingest(&r, root, &BTreeMap::new(), dest.path()), |_, _| {});
    assert_eq!(report.copied.len(), 2, "both rollover files copied");
    let contents: Vec<Vec<u8>> = std::fs::read_dir(dest.path().join(&day))
        .unwrap().flatten().map(|e| std::fs::read(e.path()).unwrap()).collect();
    assert!(contents.iter().any(|c| c == b"first") && contents.iter().any(|c| c == b"second!!"), "both present");

    let report2 = run_ingest(&plan_ingest(&r, root, &BTreeMap::new(), dest.path()), |_, _| {});
    assert_eq!(report2.skipped.len(), 2, "re-run recognises both via the same de-collision chain");
}

#[test]
fn identical_content_distinct_sources_both_kept() {
    // Two distinct card files (rollover dirs) with byte-IDENTICAL content must each get their
    // own copy, not collapse to one — else cleanup deletes a source whose only dest copy
    // belongs to a different source. (Content match alone is not source identity.)
    let card = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let root = card.path();
    write_file(&root.join("DCIM/100MSDCF/x.arw"), b"IDENTICAL", DAY_A);
    write_file(&root.join("DCIM/101MSDCF/x.arw"), b"IDENTICAL", DAY_A);

    let mut r = rule(&["DCIM/1*MSDCF"], dest.path().to_str().unwrap(), &["arw"]);
    r.layout = "{date}".into();

    let report = run_ingest(&plan_ingest(&r, root, &BTreeMap::new(), dest.path()), |_, _| {});
    assert_eq!(report.copied.len(), 2, "both distinct sources copied, not deduped");
    assert_eq!(std::fs::read_dir(dest.path().join(day_of(DAY_A))).unwrap().count(), 2, "two files for two sources");
    assert!(report.is_clean(), "cleanup would delete both sources — both have their own copy");
}

#[test]
fn identical_content_not_collapsed_against_preexisting_copy() {
    // Dest already holds the file (a prior import). The card now has TWO identical-content
    // sources with that name: one matches the pre-existing copy (skip), the other gets its
    // own fresh copy rather than also skipping against that single pre-existing file.
    let card = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let root = card.path();
    let day = day_of(DAY_A);
    write_file(&dest.path().join(&day).join("x.arw"), b"SAME", DAY_A); // pre-existing
    write_file(&root.join("DCIM/100MSDCF/x.arw"), b"SAME", DAY_A);
    write_file(&root.join("DCIM/101MSDCF/x.arw"), b"SAME", DAY_A);

    let mut r = rule(&["DCIM/1*MSDCF"], dest.path().to_str().unwrap(), &["arw"]);
    r.layout = "{date}".into();
    let report = run_ingest(&plan_ingest(&r, root, &BTreeMap::new(), dest.path()), |_, _| {});
    assert_eq!(report.copied.len() + report.skipped.len(), 2, "both sources accounted for");
    assert_eq!(std::fs::read_dir(dest.path().join(&day)).unwrap().count(), 2, "pre-existing + one fresh = 2");
}

#[test]
fn cleanup_deletes_only_when_fully_clean() {
    let card = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let root = card.path();
    write_file(&root.join("DCIM/100MSDCF/a.arw"), b"hello", DAY_A);

    let r = rule(&["DCIM/100MSDCF"], dest.path().to_str().unwrap(), &["arw"]);
    let plan = plan_ingest(&r, root, &BTreeMap::new(), dest.path());
    let report = run_ingest(&plan, |_, _| {});
    assert!(report.is_clean());

    let deleted = cleanup(&report).unwrap();
    assert_eq!(deleted.deleted.len(), 1);
    assert!(deleted.failed.is_empty());
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

    let by_date = album_groups(&[a.clone(), b.clone(), c.clone()], "{date}", &BTreeMap::new());
    assert_eq!(by_date.len(), 2, "two distinct capture dates → two albums");
    assert_eq!(by_date.values().map(|v| v.len()).sum::<usize>(), 3);

    let by_year = album_groups(&[a, b, c], "{year}", &BTreeMap::new());
    assert_eq!(by_year.len(), 1, "same year → one album");
}

#[test]
fn album_groups_fills_name_token() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.jpg");
    write_file(&a, b"x", DAY_A);
    let (_, date) = fileflow_core::layout::date_parts(
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(DAY_A as u64),
    );
    let mut names = BTreeMap::new();
    names.insert(date.clone(), "Holiday".to_string());

    let groups = album_groups(&[a], "{date} {name}", &names);
    assert!(
        groups.contains_key(&format!("{date} Holiday")),
        "album name uses the {{date}} {{name}} pattern"
    );
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

fn folder_rule(watch: &str, dest: &str) -> FolderRule {
    FolderRule {
        label: "t".into(),
        watch: watch.into(),
        extensions: vec![],
        include: String::new(),
        exclude: String::new(),
        target: fileflow_core::config::Destination::Folder {
            dest: dest.into(),
            layout: "{date}".into(),
        },
    }
}

#[test]
fn folder_move_relocates_into_dated_dest() {
    let dir = tempfile::tempdir().unwrap();
    let watch = dir.path().join("incoming");
    let dest = dir.path().join("sorted");
    std::fs::create_dir_all(&watch).unwrap();
    std::fs::create_dir_all(&dest).unwrap();
    let a = watch.join("a.jpg");
    write_file(&a, b"x", DAY_A);

    let report =
        run_folder_move(&folder_rule(watch.to_str().unwrap(), dest.to_str().unwrap()), |_, _| {})
            .unwrap();
    assert_eq!(report.moved.len(), 1);
    assert!(report.failed.is_empty());
    assert!(!a.exists(), "source file moved out");
    let (_, date) = fileflow_core::layout::date_parts(
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(DAY_A as u64),
    );
    assert!(dest.join(&date).join("a.jpg").exists(), "moved into a dated subfolder");
}

#[test]
fn folder_move_errors_when_dest_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let watch = dir.path().join("incoming");
    std::fs::create_dir_all(&watch).unwrap();
    write_file(&watch.join("a.jpg"), b"x", DAY_A);

    let missing = dir.path().join("does-not-exist");
    let r = run_folder_move(&folder_rule(watch.to_str().unwrap(), missing.to_str().unwrap()), |_, _| {});
    assert!(r.is_err(), "missing dest root → error");
    assert!(watch.join("a.jpg").exists(), "source untouched when dest unavailable");
}

#[test]
fn config_roundtrips_through_toml() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let mut cfg = Config::default();
    let mut card = rule(&["DCIM/100MSDCF"], "~/dest", &["arw", "jpg"]);
    card.rename = "{date}_{seq}".into();
    card.routes = vec![Route { extensions: vec!["arw".into()], dest: "~/raw".into(), layout: "RAW".into() }];
    cfg.cards.push(card);
    cfg.save(&path).unwrap();

    // Routes nest as `[[card.routes]]` — the field name must match the TS IPC key (no rename).
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("[[card.routes]]"), "routes serialize under the `routes` key:\n{written}");
    assert!(!written.contains("[[card.route]]"), "no legacy `route` key");

    let back = Config::load(&path).unwrap();
    assert_eq!(back.cards.len(), 1);
    assert_eq!(back.cards[0].uuid, "TEST-UUID");
    assert_eq!(back.cards[0].extensions, vec!["arw", "jpg"]);
    assert_eq!(back.cards[0].rename, "{date}_{seq}");
    assert_eq!(back.cards[0].routes.len(), 1);
    assert_eq!(back.cards[0].routes[0].dest, "~/raw");

    // Missing file → default, not an error.
    let absent = Config::load(&dir.path().join("nope.toml")).unwrap();
    assert!(absent.cards.is_empty());

    // A config written before this feature (no `rename`/`routes`) still loads.
    let legacy = dir.path().join("legacy.toml");
    std::fs::write(&legacy, "[[card]]\nuuid = \"U\"\nsources = [\"DCIM/100MSDCF\"]\ndest = \"~/d\"\n").unwrap();
    let old = Config::load(&legacy).unwrap();
    assert_eq!(old.cards.len(), 1);
    assert!(old.cards[0].rename.is_empty() && old.cards[0].routes.is_empty());
}

#[test]
fn scan_folder_skips_files_still_being_written() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_file(&root.join("settled.jpg"), b"x", DAY_A); // old mtime → settled
    std::fs::write(root.join("writing.jpg"), b"x").unwrap(); // fresh mtime → skipped

    let files = scan_folder(root, &["jpg".into()], &NameFilter::default());
    assert_eq!(files.len(), 1, "only the settled file is returned");
    assert_eq!(files[0].file_name().unwrap(), "settled.jpg");
}

#[test]
fn scan_folder_skips_a_file_that_grows_during_the_recheck() {
    // A large/slow write that's quiet for >2s but still growing must be skipped, so a
    // half-written file is never handed to Photos (the >100MB metadata-warning case).
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let export = root.join("export.jpg");
    write_file(&export, b"partial", DAY_A); // old mtime → passes the quiet gate

    // A writer appends more bytes shortly after the scan samples the size.
    let target = export.clone();
    let writer = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(250));
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(&target).unwrap();
        f.write_all(b"...the rest of a big file...").unwrap();
    });

    let files = scan_folder(&root, &["jpg".into()], &NameFilter::default());
    writer.join().unwrap();
    assert!(files.is_empty(), "a file still growing across the re-check is skipped");
}

#[test]
fn name_filter_include_exclude_and_fail_closed() {
    let f = NameFilter::compile("^IMG_", "_thumb").unwrap();
    assert!(f.accepts(Path::new("/x/IMG_1.jpg")));
    assert!(!f.accepts(Path::new("/x/other.jpg")), "not matched by include");
    assert!(!f.accepts(Path::new("/x/IMG_1_thumb.jpg")), "matched by exclude wins");

    // Unset filter (and Default) accept everything.
    assert!(NameFilter::default().accepts(Path::new("/x/anything.bin")));

    // Invalid regex: compile errors, and compile_or_deny denies everything (fail closed).
    assert!(NameFilter::compile("(", "").is_err());
    assert!(!NameFilter::compile_or_deny("(", "").accepts(Path::new("/x/anything.bin")));
}

#[test]
fn scan_files_whitelists_and_blacklists_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_file(&root.join("DCIM/100MSDCF/IMG_001.jpg"), b"a", DAY_A);
    write_file(&root.join("DCIM/100MSDCF/IMG_001_thumb.jpg"), b"b", DAY_A);
    write_file(&root.join("DCIM/100MSDCF/scratch.jpg"), b"c", DAY_A);

    let mut r = rule(&["DCIM/100MSDCF"], "/tmp/out", &["jpg"]);
    r.include = "^IMG_".into(); // only IMG_*
    r.exclude = "_thumb".into(); // …but not thumbnails
    let names: Vec<_> = scan_files(&r, root)
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap().to_string())
        .collect();
    assert_eq!(names, vec!["IMG_001.jpg"], "whitelist ∩ not-blacklist");
}

#[test]
fn scan_files_bad_regex_moves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_file(&root.join("DCIM/100MSDCF/IMG_001.jpg"), b"a", DAY_A);
    let mut r = rule(&["DCIM/100MSDCF"], "/tmp/out", &["jpg"]);
    r.include = "(".into(); // invalid → deny all (fail closed)
    assert!(scan_files(&r, root).is_empty());
}

#[test]
fn scan_folder_applies_name_filter() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_file(&root.join("keep.jpg"), b"x", DAY_A);
    write_file(&root.join("draft_skip.jpg"), b"x", DAY_A);

    let filter = NameFilter::compile("", "^draft_").unwrap();
    let files = scan_folder(root, &["jpg".into()], &filter);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].file_name().unwrap(), "keep.jpg");
}
