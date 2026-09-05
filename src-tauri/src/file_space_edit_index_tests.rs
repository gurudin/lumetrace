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
