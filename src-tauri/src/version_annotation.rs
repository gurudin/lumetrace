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
        .is_some_and(|note| note.chars().count() > 1000 || note.contains('\0'))
    {
        return Err(
            "Version notes must be at most 1000 characters and contain no null bytes".to_owned(),
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
        assert!(update_record(&database, patch(Some(&"a".repeat(1001)), None)).is_err());
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
}
