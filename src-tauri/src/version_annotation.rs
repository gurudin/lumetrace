//! Version labels are local metadata, never edits to a snapshot or working file.
use crate::database::Database;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnnotationPatch {
    workspace_id: String,
    file_id: String,
    version_id: String,
    note: Option<String>,
    is_milestone: Option<bool>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VersionAnnotation {
    workspace_id: String,
    file_id: String,
    version_id: String,
    note: String,
    is_milestone: bool,
}

fn update_record(database: &Database, patch: AnnotationPatch) -> Result<VersionAnnotation, String> {
    if patch.note.is_none() && patch.is_milestone.is_none() {
        return Err("No version annotation changes supplied".to_owned());
    }
    if patch
        .note
        .as_ref()
        .is_some_and(|note| note.chars().count() > 50 || note.contains('\0'))
    {
        return Err(
            "Version notes must be at most 50 characters and contain no null bytes".to_owned(),
        );
    }
    // Match switch_workspace's lock order. A late request from the old space
    // cannot annotate an identically named file/version in the new space.
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access the workspace database")?;
    if database.location()?.workspace_id != patch.workspace_id {
        return Err("The workspace changed. Reopen the file history before editing.".to_owned());
    }
    let note = patch.note.as_deref().map(str::trim);
    let (note, is_milestone) = connection
        .query_row(
            "UPDATE file_space_artifact_versions
         SET note = COALESCE(?1, note), is_milestone = COALESCE(?2, is_milestone)
         WHERE id = ?3 AND file_id = ?4
           AND EXISTS (SELECT 1 FROM files WHERE id = ?4 AND trashed_at IS NULL)
         RETURNING note, is_milestone",
            params![note, patch.is_milestone, patch.version_id, patch.file_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)),
        )
        .optional()
        .map_err(|error| format!("Unable to save the version annotation: {error}"))?
        .ok_or_else(|| "The file version is unavailable or in the trash".to_owned())?;
    Ok(VersionAnnotation {
        workspace_id: patch.workspace_id,
        file_id: patch.file_id,
        version_id: patch.version_id,
        note,
        is_milestone,
    })
}

#[tauri::command]
pub fn update_file_version_annotation(
    request: AnnotationPatch,
    database: State<'_, Database>,
) -> Result<VersionAnnotation, String> {
    update_record(database.inner(), request)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionSummary {
    workspace_id: String,
    file_id: String,
    versions: Vec<VersionSummaryRow>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct VersionSummaryRow {
    id: String,
    version_number: i64,
    produced_at: i64,
    is_current: bool,
    note: String,
    is_milestone: bool,
}

// Hover is metadata-only: no reconciliation, snapshot reads, writes or AI calls.
fn load_summary(
    database: &Database,
    workspace_id: String,
    file_id: String,
) -> Result<VersionSummary, String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access the workspace database")?;
    if database.location()?.workspace_id != workspace_id {
        return Err("The workspace changed. Reopen the version summary.".into());
    }
    let available: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM files WHERE id=?1 AND trashed_at IS NULL)",
            [&file_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if !available {
        return Err("The file is unavailable or in the trash".into());
    }
    let mut statement = connection.prepare(
        "SELECT v.id, v.version_number, v.produced_at, v.id = a.current_version_id, v.note, v.is_milestone
         FROM file_space_artifact_versions v JOIN file_space_artifacts a ON a.file_id=v.file_id
         WHERE v.file_id=?1 ORDER BY v.version_number DESC LIMIT 5"
    ).map_err(|error| error.to_string())?;
    let versions = statement
        .query_map([&file_id], |row| {
            Ok(VersionSummaryRow {
                id: row.get(0)?,
                version_number: row.get(1)?,
                produced_at: row.get(2)?,
                is_current: row.get(3)?,
                note: row.get(4)?,
                is_milestone: row.get(5)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(VersionSummary {
        workspace_id,
        file_id,
        versions,
    })
}

#[tauri::command]
pub fn get_file_version_summary(
    workspace_id: String,
    file_id: String,
    database: State<'_, Database>,
) -> Result<VersionSummary, String> {
    load_summary(database.inner(), workspace_id, file_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{open_for_test, open_workspace_database};
    use std::fs;
    use uuid::Uuid;

    fn seed(database: &Database) {
        database.0.lock().unwrap().execute_batch(
            "INSERT INTO files(id,original_name,storage_path,created_at) VALUES('file','test-version.md','test-version.md',1);
             INSERT INTO file_space_artifacts(file_id,task_id,logical_key,current_version_id,created_at,updated_at)
             VALUES('file','synthetic','test-version.md','v3',1,3);
             INSERT INTO file_space_artifact_versions(id,file_id,version_number,snapshot_path,sha256,size_bytes,produced_name,origin,produced_at,created_at)
             VALUES('v1','file',1,'one','hash-one',11,'test-version.md','user_edit',1,1),
                   ('v2','file',2,'two','hash-two',11,'test-version.md','user_edit',2,2),
                   ('v3','file',3,'three','hash-three',13,'test-version.md','user_edit',3,3);"
        ).unwrap();
    }

    fn patch(note: Option<&str>, star: Option<bool>) -> AnnotationPatch {
        AnnotationPatch {
            workspace_id: "test-workspace".into(),
            file_id: "file".into(),
            version_id: "v2".into(),
            note: note.map(str::to_owned),
            is_milestone: star,
        }
    }

    #[test]
    fn notes_and_milestones_persist_without_changing_versions_or_current_file() {
        let root = std::env::temp_dir().join(format!("lumetrace-annotation-{}", Uuid::new_v4()));
        let path = root.join("test.sqlite3");
        let database = open_for_test(&path).unwrap();
        seed(&database);
        update_record(
            &database,
            patch(Some("  Approved release\nversion two  "), None),
        )
        .unwrap();
        let marked = update_record(&database, patch(None, Some(true))).unwrap();
        assert_eq!(marked.note, "Approved release\nversion two");
        assert!(marked.is_milestone);
        drop(database);
        let reopened = open_for_test(&path).unwrap();
        assert!(reopened
            .0
            .lock()
            .unwrap()
            .query_row(
                "SELECT is_milestone FROM file_space_artifact_versions WHERE id='v2'",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap());
        let unstarred = update_record(&reopened, patch(None, Some(false))).unwrap();
        assert_eq!(unstarred.note, marked.note);
        assert!(!unstarred.is_milestone);
        let cleared = update_record(&reopened, patch(Some(""), None)).unwrap();
        assert_eq!(cleared.note, "");
        let connection = reopened.0.lock().unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM file_space_artifact_versions",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            3
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT current_version_id FROM file_space_artifacts",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "v3"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT sha256 FROM file_space_artifact_versions WHERE id='v2'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "hash-two"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT note FROM file_space_artifact_versions WHERE id='v1'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            ""
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM file_space_artifact_events", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        drop(connection);
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_wrong_space_file_version_trash_and_invalid_notes() {
        let root =
            std::env::temp_dir().join(format!("lumetrace-annotation-isolation-{}", Uuid::new_v4()));
        let database = open_for_test(&root.join("one.sqlite3")).unwrap();
        seed(&database);
        for (field, value) in [
            ("space", "other"),
            ("file", "other"),
            ("version", "missing"),
        ] {
            let mut request = patch(Some("must not save"), Some(true));
            match field {
                "space" => request.workspace_id = value.into(),
                "file" => request.file_id = value.into(),
                _ => request.version_id = value.into(),
            }
            assert!(update_record(&database, request).is_err());
        }
        assert!(update_record(&database, patch(Some(&"a".repeat(51)), None)).is_err());
        assert!(update_record(&database, patch(Some("a\0b"), None)).is_err());
        assert!(update_record(&database, patch(None, None)).is_err());
        update_record(&database, patch(Some("version two"), Some(true))).unwrap();
        database
            .0
            .lock()
            .unwrap()
            .execute("UPDATE files SET trashed_at=4 WHERE id='file'", [])
            .unwrap();
        assert!(update_record(&database, patch(Some("trash write"), None)).is_err());
        database
            .0
            .lock()
            .unwrap()
            .execute("UPDATE files SET trashed_at=NULL WHERE id='file'", [])
            .unwrap();
        assert_eq!(
            update_record(&database, patch(None, Some(false)))
                .unwrap()
                .note,
            "version two"
        );
        let other = open_workspace_database(
            root.join("two.sqlite3"),
            root.join("versions"),
            "other".into(),
        )
        .unwrap();
        seed(&other);
        database
            .switch_workspace(other.location().unwrap())
            .unwrap();
        assert!(update_record(&database, patch(Some("stale space"), Some(true))).is_err());
        let connection = other.0.lock().unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT note FROM file_space_artifact_versions WHERE id='v2'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            ""
        );
        drop(connection);
        drop(database);
        drop(other);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fifty_character_limit_preserves_legacy_notes_and_counts_unicode() {
        let root = std::env::temp_dir().join(format!("lumetrace-note-limit-{}", Uuid::new_v4()));
        let database = open_for_test(&root.join("test.sqlite3")).unwrap();
        seed(&database);
        for character in ["a", "版", "🌟"] {
            assert!(update_record(&database, patch(Some(&character.repeat(50)), None)).is_ok());
            assert!(update_record(&database, patch(Some(&character.repeat(51)), None)).is_err());
        }
        let legacy = "旧".repeat(80);
        database
            .0
            .lock()
            .unwrap()
            .execute(
                "UPDATE file_space_artifact_versions SET note=?1 WHERE id='v2'",
                [&legacy],
            )
            .unwrap();
        assert_eq!(
            update_record(&database, patch(None, Some(true)))
                .unwrap()
                .note,
            legacy
        );
        assert_eq!(
            load_summary(&database, "test-workspace".into(), "file".into())
                .unwrap()
                .versions[1]
                .note,
            legacy
        );
        assert!(update_record(&database, patch(Some("short note"), None)).is_ok());
        drop(database);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn summary_is_bounded_scoped_and_does_not_require_snapshot_files() {
        let root = std::env::temp_dir().join(format!("lumetrace-summary-{}", Uuid::new_v4()));
        let database = open_for_test(&root.join("test.sqlite3")).unwrap();
        seed(&database);
        {
            let connection = database.0.lock().unwrap();
            for n in 4..=40 {
                connection.execute("INSERT INTO file_space_artifact_versions(id,file_id,version_number,snapshot_path,sha256,size_bytes,produced_name,origin,produced_at,created_at) VALUES(?1,'file',?2,'missing','hash',1,'test.md','user_edit',?2,?2)", params![format!("v{n}"), n]).unwrap();
            }
            connection
                .execute(
                    "UPDATE file_space_artifacts SET current_version_id='v38'",
                    [],
                )
                .unwrap();
            connection.execute("UPDATE file_space_artifact_versions SET note='Release', is_milestone=1 WHERE id='v39'", []).unwrap();
        }
        let summary = load_summary(&database, "test-workspace".into(), "file".into()).unwrap();
        assert_eq!(
            summary
                .versions
                .iter()
                .map(|v| v.version_number)
                .collect::<Vec<_>>(),
            vec![40, 39, 38, 37, 36]
        );
        assert_eq!(summary.versions[1].note, "Release");
        assert!(summary.versions[1].is_milestone);
        assert!(summary.versions[2].is_current);
        assert!(load_summary(&database, "other".into(), "file".into()).is_err());
        assert!(load_summary(&database, "test-workspace".into(), "missing".into()).is_err());
        {
            let connection = database.0.lock().unwrap();
            assert_eq!(
                connection
                    .query_row(
                        "SELECT COUNT(*) FROM file_space_artifact_versions",
                        [],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap(),
                40
            );
            assert_eq!(
                connection
                    .query_row("SELECT COUNT(*) FROM file_space_artifact_events", [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                0
            );
            connection
                .execute("UPDATE files SET trashed_at=1 WHERE id='file'", [])
                .unwrap();
        }
        assert!(load_summary(&database, "test-workspace".into(), "file".into()).is_err());
        drop(database);
        fs::remove_dir_all(root).unwrap();
    }
}
