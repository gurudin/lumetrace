//! Isolated regressions: no live workspace, CLI, cloud service or user documents.
use super::*;

struct Fixture {
    root: PathBuf,
    storage: PathBuf,
    versions: PathBuf,
    database: Database,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("lumetrace-edit-index-{}", Uuid::new_v4()));
        let storage = root.join("storage");
        let versions = root.join("versions");
        fs::create_dir_all(&storage).unwrap();
        let database = crate::database::open_for_test(&root.join("test.sqlite3")).unwrap();
        configure_storage_root_record(&database, storage.to_str().unwrap()).unwrap();
        Self {
            root,
            storage,
            versions,
            database,
        }
    }
    fn create(&self, name: &str, format: &str) -> String {
        create_text_file_record(&self.database, &self.versions, None, name, format)
            .unwrap()
            .file
            .id
    }
    fn save(&self, id: &str, content: &str) {
        save_markdown_file_record(&self.database, &self.versions, id, content).unwrap();
    }
    fn drain(&self) {
        for _ in 0..256 {
            if !process_next_content_extraction(&self.database).unwrap() {
                return;
            }
        }
        panic!("fixture extraction queue failed to drain within its bound");
    }
    fn state(&self, id: &str) -> (String, String, i64, String, i64) {
        self.database.0.lock().unwrap().query_row(
            "SELECT d.extraction_status, d.body_text, d.indexed_at, j.status, j.requested_document_indexed_at
             FROM file_space_search_documents d JOIN file_space_index_jobs j ON j.file_id = d.file_id WHERE d.file_id = ?1",
            [id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
        ).unwrap()
    }
    fn find(&self, query: &str, scope: &str) -> Vec<String> {
        search_file_space_records(
            &self.database,
            None,
            &FileSpaceSearchRequest {
                query: query.into(),
                scopes: vec![scope.into()],
            },
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn slow_extraction_does_not_hold_the_edit_lock_or_publish_a_superseded_version() {
    let fixture = Fixture::new();
    let id = fixture.create("concurrent", "md");
    fixture.save(&id, "old_content");
    assert!(
        process_next_content_extraction_with(&fixture.database, |_, _, _| {
            let started = std::time::Instant::now();
            let guard = loop {
                if let Ok(guard) = FILE_SPACE_OPERATION_LOCK.get().unwrap().try_lock() {
                    break guard;
                }
                assert!(
                    started.elapsed() < Duration::from_secs(3),
                    "OCR must not block edits"
                );
                std::thread::sleep(Duration::from_millis(10));
            };
            fixture.save(&id, "new_content");
            drop(guard);
            ContentExtraction {
                visual_json: None,
                text: "stale OCR".into(),
                status: ExtractionStatus::Extracted,
                error: None,
            }
        })
        .unwrap()
    );
    assert_eq!(fixture.state(&id).0, "pending");
    assert!(fixture.find("stale", "content").is_empty());
    fixture.drain();
    assert_eq!(fixture.find("new_content", "content"), vec![id]);
}

#[test]
fn extraction_cannot_write_into_a_workspace_selected_while_it_was_running() {
    let first = Fixture::new();
    let second = Fixture::new();
    let id = first.create("first", "md");
    first.save(&id, "first_content");
    let second_id = second.create("second", "md");
    let original = first.database.location().unwrap();
    assert!(
        process_next_content_extraction_with(&first.database, |_, _, _| {
            let _guard = lock_file_space_operations().unwrap();
            first
                .database
                .switch_workspace(second.database.location().unwrap())
                .unwrap();
            ContentExtraction {
                visual_json: None,
                text: "wrong_workspace".into(),
                status: ExtractionStatus::Extracted,
                error: None,
            }
        })
        .unwrap()
    );
    assert!(second.find("wrong_workspace", "content").is_empty());
    assert_eq!(second.state(&second_id).0, "pending");
    first.database.switch_workspace(original).unwrap();
    assert_eq!(first.state(&id).0, "pending");
    first.drain();
    assert_eq!(first.find("first_content", "content"), vec![id]);
}

#[test]
#[cfg(target_os = "macos")]
fn recognition_upgrade_requeues_only_images_and_pdf_not_unchanged_text() {
    let fixture = Fixture::new();
    let text = fixture.create("keep", "md");
    fixture.save(&text, "cached text");
    fixture.drain();
    let original = fixture.state(&text);
    let incoming = fixture.root.join("old-image.png");
    fs::write(&incoming, "synthetic not decoded in this migration test").unwrap();
    let image = import_files_record(
        &fixture.database,
        None,
        &[incoming.to_string_lossy().into_owned()],
    )
    .unwrap()
    .files
    .iter()
    .find(|f| f.name == "old-image.png")
    .unwrap()
    .id
    .clone();
    synchronize_file_search_index_scope(&fixture.database, Some(&fixture.storage), None).unwrap();
    fixture.database.0.lock().unwrap().execute("UPDATE file_space_search_documents SET extraction_status='extracted',extraction_version=2,body_text='old OCR without coordinates' WHERE file_id=?1", [&image]).unwrap();
    while repair_missed_edit_indexes_batch(&fixture.database).unwrap() {}
    assert_eq!(fixture.state(&text), original);
    assert_eq!(fixture.state(&image).0, "pending");
    assert_eq!(
        fixture
            .database
            .0
            .lock()
            .unwrap()
            .query_row(
                "SELECT extraction_version FROM file_space_search_documents WHERE file_id=?1",
                [&image],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        extraction_version("old-image.png")
    );
}

#[test]
#[cfg(target_os = "macos")]
fn extraction_upgrade_restarts_after_previous_repair_cursor_was_done() {
    let fixture = Fixture::new();
    let image_path = fixture.root.join("old-image.png");
    fs::write(&image_path, "synthetic not decoded in this migration test").unwrap();
    let image = import_files_record(
        &fixture.database,
        None,
        &[image_path.to_string_lossy().into_owned()],
    )
    .unwrap()
    .files
    .iter()
    .find(|file| file.name == "old-image.png")
    .unwrap()
    .id
    .clone();
    synchronize_file_search_index_scope(&fixture.database, Some(&fixture.storage), None).unwrap();

    // Simulate a database that completed the previous repair pass before the
    // image classification extraction version was released.
    fixture
        .database
        .0
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO app_settings (key, value, updated_at) VALUES ('file_space.edit_index_repair.v3', 'done', 1)",
            [],
        )
        .unwrap();
    fixture
        .database
        .0
        .lock()
        .unwrap()
        .execute(
            "UPDATE file_space_search_documents
             SET extraction_status='extracted', extraction_version=3,
                 body_text='old OCR without categories'
             WHERE file_id=?1",
            [&image],
        )
        .unwrap();

    assert!(repair_missed_edit_indexes_batch(&fixture.database).unwrap());
    assert_eq!(fixture.state(&image).0, "pending");
    assert_eq!(
        fixture
            .database
            .0
            .lock()
            .unwrap()
            .query_row(
                "SELECT extraction_version FROM file_space_search_documents WHERE file_id=?1",
                [&image],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        extraction_version("old-image.png")
    );
}

#[test]
#[cfg(target_os = "macos")]
#[ignore = "requires explicitly generated synthetic LUMETRACE_TEST_VISION_DIR fixtures; uses real Apple Vision"]
fn native_vision_local_index_is_searchable_persistent_and_incremental() {
    let fixture = Fixture::new();
    let generated = PathBuf::from(
        std::env::var_os("LUMETRACE_TEST_VISION_DIR")
            .expect("synthetic fixture directory required"),
    );
    let mut ids = Vec::new();
    for name in [
        "document.png",
        "scan.pdf",
        "mixed.pdf",
        "embedded.pdf",
        "rotated.pdf",
    ] {
        let result = import_files_record(
            &fixture.database,
            None,
            &[generated.join(name).to_string_lossy().into_owned()],
        )
        .unwrap();
        ids.push(
            result
                .files
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .id
                .clone(),
        );
    }
    while repair_missed_edit_indexes_batch(&fixture.database).unwrap() {}
    fixture.drain();
    for word in ["ORCHID", "北京"] {
        let found = fixture.find(word, "content");
        let required = if word == "ORCHID" {
            &ids[..]
        } else {
            &ids[..4]
        };
        // Rotated OCR may join adjacent Chinese words. The existing unicode61
        // tokenizer does not split that joined token; rotation tests assert
        // its recognized text/geometry separately, not artificial spacing.
        assert!(
            required.iter().all(|id| found.contains(id)),
            "{word}: {found:?}"
        );
    }
    let states: Vec<_> = ids.iter().map(|id| fixture.state(id)).collect();
    synchronize_file_search_index_scope(&fixture.database, Some(&fixture.storage), None).unwrap();
    assert!(!process_next_content_extraction(&fixture.database).unwrap());
    assert_eq!(
        states,
        ids.iter().map(|id| fixture.state(id)).collect::<Vec<_>>()
    );
    let reopened =
        crate::database::open_for_test(&fixture.database.location().unwrap().database_path)
            .unwrap();
    assert_eq!(
        search_file_space_records(
            &reopened,
            None,
            &FileSpaceSearchRequest {
                query: "ORCHID".into(),
                scopes: vec!["content".into()]
            }
        )
        .unwrap()
        .len(),
        5
    );
    assert!(!process_next_content_extraction(&reopened).unwrap());
    for id in &ids {
        let file = load_snapshot_record(&reopened)
            .unwrap()
            .files
            .into_iter()
            .find(|f| &f.id == id)
            .unwrap();
        let preview = search_preview_record(
            &reopened,
            &FileSpaceSearchPreviewRequest {
                file_id: id.clone(),
                query: "ORCHID".into(),
                scopes: vec!["content".into()],
            },
        )
        .unwrap();
        assert!(
            preview.sections.iter().any(|s| !s.rectangles.is_empty()),
            "{} has no persisted geometry",
            file.name
        );
        assert_eq!(preview.source_updated_at, file.updated_at);
        assert_eq!(preview.source_size_bytes, file.size_bytes);
        let matches = search_file_space_matches(
            &reopened,
            None,
            &FileSpaceSearchRequest {
                query: "ORCHID".into(),
                scopes: vec!["content".into()],
            },
        )
        .unwrap();
        assert!(matches.iter().all(|m| !m
            .snippet
            .as_deref()
            .unwrap_or_default()
            .contains("Image categories")));
        if std::env::var_os("LUMETRACE_TEST_WRITE_GEOMETRY").is_some() {
            use std::io::Write;
            let mut output = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(generated.join(format!("{}.search.json", file.name)))
                .unwrap();
            output
                .write_all(
                    serde_json::to_string(&serde_json::json!({"file":file,"preview":preview}))
                        .unwrap()
                        .as_bytes(),
                )
                .unwrap();
        }
    }
    drop(reopened);
}

#[test]
fn manual_creation_indexes_names_immediately_but_extracts_content_in_background() {
    let fixture = Fixture::new();
    for format in ["md", "txt"] {
        let name = format!("created-{format}");
        let id = fixture.create(&name, format);
        let state = fixture.state(&id);
        assert_eq!(
            (&*state.0, &*state.1, &*state.3),
            ("pending", "", "pending")
        );
        assert_eq!(state.2, state.4);
        assert_eq!(fixture.find(&name, "name"), vec![id.clone()]);
        assert_eq!(
            load_task_file_timeline_record(&fixture.database, &id)
                .unwrap()
                .versions
                .len(),
            1
        );
        fixture.drain();
        assert_eq!(fixture.state(&id).0, "empty");
    }
}

#[test]
fn in_app_edits_replace_fulltext_and_queue_only_the_changed_file() {
    let fixture = Fixture::new();
    let target = fixture.create("target", "md");
    let other = fixture.create("untouched", "md");
    fixture.save(&target, "oldword");
    fixture.save(&other, "otherword");
    fixture.drain();
    let untouched = fixture.state(&other);
    let previous = fixture.state(&target);
    assert_eq!(fixture.find("oldword", "content"), vec![target.clone()]);

    // Same-sized, rapid edit still gets a strictly new document generation.
    fixture.save(&target, "newword");
    let pending = fixture.state(&target);
    assert_eq!(
        (&*pending.0, &*pending.1, &*pending.3),
        ("pending", "", "pending")
    );
    assert!(pending.2 > previous.2);
    assert_eq!(pending.2, pending.4);
    assert!(fixture.find("oldword", "content").is_empty());
    fixture.drain();
    assert_eq!(fixture.find("newword", "content"), vec![target.clone()]);
    assert_eq!(fixture.state(&other), untouched);
    assert_eq!(
        load_task_file_timeline_record(&fixture.database, &target)
            .unwrap()
            .versions
            .len(),
        3
    );

    let unchanged = fixture.state(&target);
    fixture.save(&target, "newword");
    assert_eq!(fixture.state(&target), unchanged);
    assert_eq!(
        load_task_file_timeline_record(&fixture.database, &target)
            .unwrap()
            .versions
            .len(),
        3
    );
}

#[test]
fn external_events_and_focus_scan_capture_versions_and_refresh_fulltext() {
    let fixture = Fixture::new();
    let id = fixture.create("external", "md");
    fixture.save(&id, "beforeedit");
    fixture.drain();
    let working = fixture.storage.join("external.md");
    let old_mtime = fs::metadata(&working).unwrap().modified().unwrap();
    fs::write(&working, "after_edit").unwrap();
    fs::File::options()
        .write(true)
        .open(&working)
        .unwrap()
        .set_modified(old_mtime)
        .unwrap();
    let paths = HashSet::from([working.clone()]);
    let events =
        capture_external_version_changes(&fixture.database, &fixture.versions, Some(&paths))
            .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].version_number, 3);
    assert_eq!(fixture.state(&id).0, "pending");
    assert!(fixture.find("beforeedit", "content").is_empty());
    fixture.drain();
    assert_eq!(fixture.find("after_edit", "content"), vec![id.clone()]);
    let unchanged = fixture.state(&id);
    assert!(
        capture_external_version_changes(&fixture.database, &fixture.versions, Some(&paths))
            .unwrap()
            .is_empty()
    );
    assert_eq!(fixture.state(&id), unchanged);

    fs::write(&working, "focusfallbackword with a different size").unwrap();
    let events =
        capture_external_version_changes(&fixture.database, &fixture.versions, None).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].version_number, 4);
    fixture.drain();
    assert_eq!(fixture.find("focusfallbackword", "content"), vec![id]);
}

#[test]
fn saving_a_legacy_unversioned_file_also_invalidates_its_index() {
    let fixture = Fixture::new();
    let incoming = fixture.root.join("legacy.md");
    fs::write(&incoming, "legacyword").unwrap();
    let id = import_files_record(
        &fixture.database,
        None,
        &[incoming.to_string_lossy().into_owned()],
    )
    .unwrap()
    .files[0]
        .id
        .clone();
    assert!(!task_artifact_exists(&fixture.database, &id).unwrap());
    fixture.drain();
    fixture.save(&id, "replacementword");
    assert_eq!(fixture.state(&id).0, "pending");
    fixture.drain();
    assert!(fixture.find("legacyword", "content").is_empty());
    assert_eq!(fixture.find("replacementword", "content"), vec![id]);
}

#[test]
fn failed_index_queue_write_rolls_back_the_edit_and_does_not_create_a_version() {
    let fixture = Fixture::new();
    let id = fixture.create("rollback", "md");
    fixture.save(&id, "originalword");
    fixture.drain();
    let before = fixture.state(&id);
    fixture
        .database
        .0
        .lock()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER reject_test_index_update BEFORE UPDATE ON file_space_search_documents
         BEGIN SELECT RAISE(ABORT, 'synthetic index failure'); END;",
        )
        .unwrap();
    assert!(
        save_markdown_file_record(&fixture.database, &fixture.versions, &id, "rejectedword")
            .is_err()
    );
    assert_eq!(
        fs::read_to_string(fixture.storage.join("rollback.md")).unwrap(),
        "originalword"
    );
    assert_eq!(
        load_task_file_timeline_record(&fixture.database, &id)
            .unwrap()
            .versions
            .len(),
        2
    );
    assert_eq!(fixture.state(&id), before);
}

#[test]
fn edit_index_repair_is_bounded_resumable_and_does_not_read_any_body() {
    let fixture = Fixture::new();
    let healthy = fixture.create("healthy", "md");
    fixture.drain();
    let healthy_before = fixture.state(&healthy);
    {
        let mut connection = fixture.database.0.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        for index in 0..EDIT_INDEX_REPAIR_BATCH_SIZE + 5 {
            transaction
                .execute(
                    "INSERT INTO files (id, original_name, storage_path, updated_at, created_at)
                 VALUES (?1, ?2, ?2, 1, 1)",
                    params![
                        format!("repair-{index:04}"),
                        format!("not-on-disk-{index}.md")
                    ],
                )
                .unwrap();
        }
        transaction.commit().unwrap();
    }
    assert!(repair_missed_edit_indexes_batch(&fixture.database).unwrap());
    let (cursor, repaired): (String, i64) = {
        let connection = fixture.database.0.lock().unwrap();
        (connection.query_row("SELECT value FROM app_settings WHERE key = ?1", [EDIT_INDEX_REPAIR_KEY], |row| row.get(0)).unwrap(),
         connection.query_row("SELECT COUNT(*) FROM file_space_search_documents WHERE file_id LIKE 'repair-%'", [], |row| row.get(0)).unwrap())
    };
    assert_ne!(cursor, "done");
    assert!(repaired <= EDIT_INDEX_REPAIR_BATCH_SIZE as i64);
    let reopened = crate::database::open_for_test(&fixture.root.join("test.sqlite3")).unwrap();
    assert!(repair_missed_edit_indexes_batch(&reopened).unwrap());
    assert!(!repair_missed_edit_indexes_batch(&reopened).unwrap());
    assert_eq!(fixture.state(&healthy), healthy_before);
    let (total, bodies): (i64, i64) = reopened.0.lock().unwrap().query_row(
        "SELECT COUNT(*), SUM(LENGTH(body_text)) FROM file_space_search_documents WHERE file_id LIKE 'repair-%'", [],
        |row| Ok((row.get(0)?, row.get(1)?))
    ).unwrap();
    assert_eq!(total, (EDIT_INDEX_REPAIR_BATCH_SIZE + 5) as i64);
    assert_eq!(bodies, 0);
}

#[test]
fn repair_finds_missing_and_outdated_documents_but_keeps_workspaces_isolated() {
    let first = Fixture::new();
    let second = Fixture::new();
    let missing = first.create("missing", "md");
    let stale = first.create("stale", "md");
    let other = second.create("other", "md");
    first.save(&stale, "currentword");
    first.drain();
    let other_before = second.state(&other);
    first
        .database
        .0
        .lock()
        .unwrap()
        .execute(
            "DELETE FROM file_space_search_documents WHERE file_id = ?1",
            [&missing],
        )
        .unwrap();
    first
        .database
        .0
        .lock()
        .unwrap()
        .execute(
            "UPDATE file_space_search_documents SET file_updated_at = 0 WHERE file_id = ?1",
            [&stale],
        )
        .unwrap();
    assert!(repair_missed_edit_indexes_batch(&first.database).unwrap());
    assert_eq!(first.state(&missing).0, "pending");
    assert_eq!(first.state(&stale).0, "pending");
    assert_eq!(second.state(&other), other_before);
    first.drain();
    assert_eq!(first.find("currentword", "content"), vec![stale]);
}

#[test]
#[ignore = "requires an explicitly supplied local E5 model asset directory; never downloads"]
fn real_local_semantic_index_tracks_both_in_app_and_external_edits() {
    use crate::semantic_search::{
        index_one_document_with_installed_test_model, SemanticSearchRuntime,
    };
    let model_data = std::env::var_os("LUMETRACE_TEST_MODEL_DATA_DIR")
        .expect("Set LUMETRACE_TEST_MODEL_DATA_DIR to local installed model assets");
    let runtime = SemanticSearchRuntime::new(Path::new(&model_data));
    let fixture = Fixture::new();
    let id = fixture.create("semantic-test", "md");
    fixture.save(&id, "version one: synthetic oldword");
    fixture.drain();
    assert!(index_one_document_with_installed_test_model(&fixture.database, &runtime).unwrap());
    assert_eq!(fixture.state(&id).3, "ready");
    fixture.save(&id, "version two: synthetic newword");
    assert_eq!(fixture.state(&id).3, "pending");
    fixture.drain();
    assert!(index_one_document_with_installed_test_model(&fixture.database, &runtime).unwrap());

    let path = fixture.storage.join("semantic-test.md");
    fs::write(&path, "version three: synthetic externalword").unwrap();
    capture_external_version_changes(
        &fixture.database,
        &fixture.versions,
        Some(&HashSet::from([path])),
    )
    .unwrap();
    fixture.drain();
    assert!(index_one_document_with_installed_test_model(&fixture.database, &runtime).unwrap());
    assert_eq!(fixture.state(&id).3, "ready");
    let connection = fixture.database.0.lock().unwrap();
    let (current, stale, embeddings): (i64, i64, i64) = connection.query_row(
        "SELECT
         (SELECT COUNT(*) FROM file_space_search_chunks c JOIN file_space_artifacts a ON a.file_id = c.file_id WHERE c.file_id = ?1 AND c.version_id = a.current_version_id AND c.body_text LIKE '%externalword%'),
         (SELECT COUNT(*) FROM file_space_search_chunks WHERE file_id = ?1 AND (body_text LIKE '%oldword%' OR body_text LIKE '%newword%')),
         (SELECT COUNT(*) FROM file_space_semantic_embeddings e JOIN file_space_search_chunks c ON c.id = e.chunk_id WHERE c.file_id = ?1)",
        [&id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    ).unwrap();
    assert!(current > 0);
    assert_eq!(stale, 0);
    assert_eq!(embeddings, current);
    assert_eq!(connection.query_row("SELECT version_number FROM file_space_artifact_versions v JOIN file_space_artifacts a ON a.current_version_id = v.id WHERE a.file_id = ?1", [&id], |row| row.get::<_, i64>(0)).unwrap(), 4);
}
