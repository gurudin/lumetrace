//! Indexed, metadata-only file lookup. No body reads, embeddings or model calls.
use crate::{database::Database, version_comparison::FileTarget};
use rusqlite::{params, OptionalExtension};
use tauri::Manager;

const CURSOR_KEY: &str = "file_space.file_lookup.v1";
const BATCH_SIZE: i64 = 64;
pub(crate) const CANDIDATE_LIMIT: usize = 8;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileLookupResult {
    ready: bool,
    candidates: Vec<FileTarget>,
}

/// Local-only IPC. The request stays pinned to the workspace in which it began.
#[tauri::command]
pub(crate) async fn lookup_file_space_file_names(
    app: tauri::AppHandle,
    query: String,
    by_path: bool,
) -> Result<FileLookupResult, String> {
    let query = query.trim().to_owned();
    if query.is_empty() || query.chars().count() > 1024 || query.chars().any(char::is_control) {
        return Err("Provide a valid file name or relative path".into());
    }
    let location = app.state::<Database>().location()?;
    tauri::async_runtime::spawn_blocking(move || {
        let database = crate::database::open_workspace_database(
            location.database_path,
            location.artifact_store_path,
            location.workspace_id,
        )?;
        let ready = lookup_ready(&database)?;
        // Incomplete backfill must not present one provisional candidate as unique.
        let candidates = if ready {
            find_files(&database, &query, by_path)?
        } else {
            vec![]
        };
        Ok(FileLookupResult { ready, candidates })
    })
    .await
    .map_err(|e| format!("Unable to locate the file: {e}"))?
}

/// Reads recorded metadata only, including when extraction/embedding is absent.
#[tauri::command]
pub(crate) async fn get_file_space_file_version_summary(
    app: tauri::AppHandle,
    file_id: String,
    include_recent_versions: bool,
) -> Result<VersionMetadata, String> {
    let location = app.state::<Database>().location()?;
    tauri::async_runtime::spawn_blocking(move || {
        let database = crate::database::open_workspace_database(
            location.database_path,
            location.artifact_store_path,
            location.workspace_id,
        )?;
        version_metadata(&database, &file_id, include_recent_versions)
    })
    .await
    .map_err(|e| format!("Unable to query recorded versions: {e}"))?
}

pub(crate) fn backfill_file_lookup_batch(database: &Database) -> Result<bool, String> {
    let mut connection = database.0.lock().map_err(|_| "Database unavailable")?;
    let transaction = connection.transaction().map_err(|e| e.to_string())?;
    let cursor: String = transaction
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [CURSOR_KEY],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    if cursor == "done" {
        return Ok(false);
    }
    let ids = {
        let mut statement = transaction
            .prepare("SELECT id FROM files WHERE id > ?1 ORDER BY id LIMIT ?2")
            .map_err(|e| e.to_string())?;
        statement
            .query_map(params![cursor, BATCH_SIZE], |r| r.get::<_, String>(0))
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|e| e.to_string())?
    };
    for id in &ids {
        transaction.execute(
            "INSERT INTO file_space_file_lookup SELECT id, original_name, storage_path FROM files
             WHERE id = ?1 AND trashed_at IS NULL AND storage_path IS NOT NULL
             ON CONFLICT(file_id) DO UPDATE SET file_name=excluded.file_name, relative_path=excluded.relative_path",
            [id],
        ).map_err(|e| e.to_string())?;
    }
    let next = if ids.len() < BATCH_SIZE as usize {
        "done"
    } else {
        ids.last().unwrap()
    };
    transaction
        .execute(
            "INSERT INTO app_settings(key,value,updated_at) VALUES(?1,?2,0)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![CURSOR_KEY, next],
        )
        .map_err(|e| e.to_string())?;
    transaction.commit().map_err(|e| e.to_string())?;
    Ok(!ids.is_empty())
}

pub(crate) fn lookup_ready(database: &Database) -> Result<bool, String> {
    database
        .0
        .lock()
        .map_err(|_| "Database unavailable")?
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM app_settings WHERE key=?1 AND value='done')",
            [CURSOR_KEY],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())
}

pub(crate) fn find_files(
    database: &Database,
    value: &str,
    by_path: bool,
) -> Result<Vec<FileTarget>, String> {
    let connection = database.0.lock().map_err(|_| "Database unavailable")?;
    let column = if by_path {
        "relative_path"
    } else {
        "file_name"
    };
    // Only constant column names reach SQL; all model-supplied values are bound.
    let sql = format!(
        "SELECT file_id,file_name,relative_path FROM file_space_file_lookup
        WHERE {column} = ?1 ORDER BY file_id LIMIT ?2"
    );
    let mut statement = connection.prepare(&sql).map_err(|e| e.to_string())?;
    let mut found = statement
        .query_map(params![value, CANDIDATE_LIMIT + 1], |r| {
            Ok(FileTarget {
                file_id: r.get(0)?,
                file_name: r.get(1)?,
                relative_path: r.get(2)?,
            })
        })
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .map_err(|e| e.to_string())?;
    if !found.is_empty() || by_path {
        return Ok(found);
    }
    // A missing extension is a bounded indexed prefix lookup, never a body scan
    // or an unbounded '%name%' query. Return ambiguity rather than picking one.
    let escaped = value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let pattern = if std::path::Path::new(value).extension().is_none() {
        format!("{escaped}.%")
    } else {
        return Ok(found);
    };
    let mut statement = connection
        .prepare(
            "SELECT file_id,file_name,relative_path FROM file_space_file_lookup
         WHERE file_name LIKE ?1 ESCAPE '\\' ORDER BY file_name,file_id LIMIT ?2",
        )
        .map_err(|e| e.to_string())?;
    found = statement
        .query_map(params![pattern, CANDIDATE_LIMIT + 1], |r| {
            Ok(FileTarget {
                file_id: r.get(0)?,
                file_name: r.get(1)?,
                relative_path: r.get(2)?,
            })
        })
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .map_err(|e| e.to_string())?;
    Ok(found)
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VersionMetadata {
    pub count: i64,
    pub current_version_id: Option<String>,
    pub current_version_number: Option<i64>,
    pub recent_versions: Vec<i64>,
}

pub(crate) fn version_metadata(
    database: &Database,
    file_id: &str,
    list: bool,
) -> Result<VersionMetadata, String> {
    let mut connection = database.0.lock().map_err(|_| "Database unavailable")?;
    let transaction = connection.transaction().map_err(|e| e.to_string())?;
    let (count, current_version_id, current_version_number) = transaction.query_row(
        "SELECT (SELECT COUNT(*) FROM file_space_artifact_versions WHERE file_id=f.id),v.id,v.version_number
         FROM files f LEFT JOIN file_space_artifacts a ON a.file_id=f.id
         LEFT JOIN file_space_artifact_versions v ON v.id=a.current_version_id AND v.file_id=f.id
         WHERE f.id=?1 AND f.trashed_at IS NULL", [file_id],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).map_err(|e| e.to_string())?;
    let recent_versions = if list {
        let mut statement = transaction.prepare(
            "SELECT version_number FROM file_space_artifact_versions WHERE file_id=?1 ORDER BY version_number DESC LIMIT 20",
        ).map_err(|e| e.to_string())?;
        statement
            .query_map([file_id], |r| r.get(0))
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|e| e.to_string())?
    } else {
        vec![]
    };
    transaction.commit().map_err(|e| e.to_string())?;
    Ok(VersionMetadata {
        count,
        current_version_id,
        current_version_number,
        recent_versions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    struct Fixture {
        root: std::path::PathBuf,
        database: Database,
    }
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("lumetrace-file-query-{}", Uuid::new_v4()));
            let database = crate::database::open_for_test(&root.join("test.sqlite3")).unwrap();
            Self { root, database }
        }
        fn add(&self, id: &str, name: &str, path: &str) {
            self.database.0.lock().unwrap().execute(
                "INSERT INTO files(id,original_name,storage_path,created_at) VALUES(?1,?2,?3,0)",
                params![id,name,path],
            ).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn filename_lookup_is_independent_of_body_indexes_and_preserves_ambiguity() {
        let f = Fixture::new();
        f.add("a", "test-version.md", "folder-a/test-version.md");
        f.add("b", "test-version.md", "folder-b/test-version.md");
        assert_eq!(
            find_files(&f.database, "test-version.md", false)
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            find_files(&f.database, "TEST-VERSION", false)
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            find_files(&f.database, "folder-b/test-version.md", true).unwrap()[0].file_id,
            "b"
        );
        assert!(find_files(&f.database, "does-not-exist.md", false)
            .unwrap()
            .is_empty());
        assert!(find_files(&f.database, "' OR 1=1 --", false)
            .unwrap()
            .is_empty());
        assert!(find_files(&f.database, "%", false).unwrap().is_empty());
        let connection = f.database.0.lock().unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM file_space_search_documents",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        let plan: String=connection.query_row(
            "EXPLAIN QUERY PLAN SELECT file_id FROM file_space_file_lookup WHERE file_name = ?1 ORDER BY file_id LIMIT 9",
            ["test-version.md"],|r|r.get(3),
        ).unwrap();
        assert!(
            plan.contains("SEARCH") && plan.contains("idx_file_lookup_name"),
            "{plan}"
        );
        let mut statement=connection.prepare(
            "EXPLAIN QUERY PLAN SELECT file_id FROM file_space_file_lookup WHERE file_name LIKE ?1 ESCAPE '\\' ORDER BY file_name,file_id LIMIT 9",
        ).unwrap();
        let plan: Vec<String> = statement
            .query_map(["test-version.%"], |r| r.get(3))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(
            plan.iter()
                .any(|p| p.contains("SEARCH") && p.contains("idx_file_lookup_name")),
            "{plan:?}"
        );
    }

    #[test]
    fn metadata_triggers_track_rename_trash_restore_and_delete() {
        let f = Fixture::new();
        f.add("a", "old.md", "old.md");
        f.database
            .0
            .lock()
            .unwrap()
            .execute(
                "UPDATE files SET original_name='new.md',storage_path='folder/new.md' WHERE id='a'",
                [],
            )
            .unwrap();
        assert!(find_files(&f.database, "old.md", false).unwrap().is_empty());
        assert_eq!(
            find_files(&f.database, "folder/new.md", true)
                .unwrap()
                .len(),
            1
        );
        f.database
            .0
            .lock()
            .unwrap()
            .execute("UPDATE files SET trashed_at=1 WHERE id='a'", [])
            .unwrap();
        assert!(find_files(&f.database, "new.md", false).unwrap().is_empty());
        f.database
            .0
            .lock()
            .unwrap()
            .execute("UPDATE files SET trashed_at=NULL WHERE id='a'", [])
            .unwrap();
        assert_eq!(find_files(&f.database, "new.md", false).unwrap().len(), 1);
        f.database
            .0
            .lock()
            .unwrap()
            .execute("DELETE FROM files WHERE id='a'", [])
            .unwrap();
        assert!(find_files(&f.database, "new.md", false).unwrap().is_empty());
    }

    #[test]
    fn catalog_backfill_is_bounded_resumable_and_workspace_local() {
        let f = Fixture::new();
        let other = Fixture::new();
        for i in 0..140 {
            f.add(&format!("f{i:04}"), "same.md", &format!("{i}/same.md"));
        }
        f.database
            .0
            .lock()
            .unwrap()
            .execute("DELETE FROM file_space_file_lookup", [])
            .unwrap();
        assert!(!lookup_ready(&f.database).unwrap());
        assert!(backfill_file_lookup_batch(&f.database).unwrap());
        assert_eq!(
            f.database
                .0
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM file_space_file_lookup", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            64
        );
        let reopened = crate::database::open_for_test(&f.root.join("test.sqlite3")).unwrap();
        assert!(backfill_file_lookup_batch(&reopened).unwrap());
        assert!(backfill_file_lookup_batch(&reopened).unwrap());
        assert!(lookup_ready(&reopened).unwrap());
        assert!(!backfill_file_lookup_batch(&reopened).unwrap());
        assert_eq!(
            find_files(&reopened, "same.md", false).unwrap().len(),
            CANDIDATE_LIMIT + 1
        );
        assert!(find_files(&other.database, "same.md", false)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn version_count_uses_records_not_content_or_highest_version_and_requeries_changes() {
        let f = Fixture::new();
        f.add("file", "test-version.md", "test-version.md");
        {
            let c = f.database.0.lock().unwrap();
            c.execute("INSERT INTO file_space_artifacts(file_id,task_id,logical_key,created_at,updated_at) VALUES('file','fixture','file',0,0)",[]).unwrap();
            for n in [1, 2, 4] {
                c.execute("INSERT INTO file_space_artifact_versions(id,file_id,version_number,snapshot_path,sha256,size_bytes,produced_name,origin,produced_at,created_at)
                    VALUES(?1,'file',?2,'never-read','synthetic',0,'test-version.md','user_edit',0,0)",params![format!("v{n}"),n]).unwrap();
            }
            c.execute(
                "UPDATE file_space_artifacts SET current_version_id='v2' WHERE file_id='file'",
                [],
            )
            .unwrap();
        }
        let meta = version_metadata(&f.database, "file", true).unwrap();
        assert_eq!(meta.count, 3);
        assert_eq!(meta.current_version_number, Some(2));
        assert_eq!(meta.recent_versions, vec![4, 2, 1]);
        f.database
            .0
            .lock()
            .unwrap()
            .execute("DELETE FROM file_space_artifact_versions WHERE id='v1'", [])
            .unwrap();
        assert_eq!(
            version_metadata(&f.database, "file", false).unwrap().count,
            2
        );
        assert!(version_metadata(&f.database, "missing", false).is_err());
        f.database
            .0
            .lock()
            .unwrap()
            .execute("UPDATE files SET trashed_at=1 WHERE id='file'", [])
            .unwrap();
        assert!(version_metadata(&f.database, "file", false).is_err());
    }
}
