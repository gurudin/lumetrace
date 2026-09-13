//! Internal integration seam for isolated file-space materializations.
//! The caller owns transport and any independently versioned extension tables.
//! Opening this connection does not register/switch the active workspace, start
//! filesystem watchers or run extraction/AI workers against remote paths.
use std::path::Path;

/// A portable backup restored into an entirely new, isolated database.
/// Registration and selection are separate, so a delayed response cannot
/// replace the database currently being used by another workspace.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoredBackupWorkspace {
    pub workspace_id: String,
    #[serde(skip)]
    pub database_path: std::path::PathBuf,
    #[serde(skip)]
    pub artifact_store_path: std::path::PathBuf,
    #[serde(flatten)]
    pub snapshot: crate::file_space::FileSpaceSnapshot,
}

pub fn restore_backup_to_new_workspace(
    managed_directory: &Path,
    backup_path: &Path,
    destination_directory: &Path,
) -> Result<RestoredBackupWorkspace, String> {
    if !managed_directory.is_absolute()
        || !backup_path.is_absolute()
        || !destination_directory.is_absolute()
    {
        return Err("Backup recovery requires absolute paths".into());
    }
    let workspace_id = uuid::Uuid::new_v4().to_string();
    let parent = managed_directory.join("workspaces");
    std::fs::create_dir_all(&parent)
        .map_err(|e| format!("Unable to prepare workspace recovery: {e}"))?;
    let directory = parent.join(&workspace_id);
    std::fs::create_dir(&directory)
        .map_err(|e| format!("Unable to prepare workspace recovery: {e}"))?;
    let result = (|| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| format!("Unable to protect workspace recovery: {e}"))?;
        }
        let database_path = directory.join("lumetrace.sqlite3");
        let artifact_store_path = directory.join("file-space-versions");
        let database = crate::database::open_workspace_database(
            database_path.clone(),
            artifact_store_path.clone(),
            workspace_id.clone(),
        )?;
        let snapshot = crate::file_space::restore_file_space_backup_record(
            &database,
            &artifact_store_path,
            &backup_path.to_string_lossy(),
            &destination_directory.to_string_lossy(),
        )?;
        Ok(RestoredBackupWorkspace {
            workspace_id,
            database_path,
            artifact_store_path,
            snapshot,
        })
    })();
    if result.is_err() {
        // This is the unique directory created above, never an existing workspace.
        let _ = std::fs::remove_dir_all(&directory);
    }
    result
}

/// Apply the same filename MIME fallback as ordinary local file imports.
pub fn file_mime_type(path: &Path) -> Option<String> {
    crate::file_space::mime_type_for(path)
}

pub fn open_isolated(path: &Path) -> Result<rusqlite::Connection, String> {
    if !path.is_absolute() {
        return Err("An isolated workspace database requires an absolute path".into());
    }
    crate::database::open_connection(&path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_databases_reuse_the_complete_shared_schema_without_registration() {
        let root = std::env::temp_dir().join(format!("lumetrace-schema-{}", uuid::Uuid::new_v4()));
        let a = open_isolated(&root.join("a/lumetrace.sqlite3")).unwrap();
        let b = open_isolated(&root.join("b/lumetrace.sqlite3")).unwrap();
        let schema = |db: &rusqlite::Connection| -> Vec<String> {
            db.prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        };
        assert_eq!(schema(&a), schema(&b));
        for table in [
            "file_space_artifact_versions",
            "file_space_search_fts",
            "file_space_semantic_embeddings",
            "file_space_file_lookup",
        ] {
            assert!(schema(&a).contains(&table.to_string()));
        }
        a.execute("INSERT INTO files(id,original_name,storage_path,created_at) VALUES('one','version one.md','one.md',1)", []).unwrap();
        assert_eq!(
            b.query_row("SELECT count(*) FROM files", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            a.query_row("SELECT file_name FROM file_space_file_lookup", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
            "version one.md"
        );
        assert_eq!(
            a.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(!root.join("workspaces.json").exists());
        drop((a, b));
        std::fs::remove_dir_all(root).unwrap();
        assert!(open_isolated(Path::new("relative.sqlite3")).is_err());
        assert_eq!(
            file_mime_type(Path::new("nested/version.md")).as_deref(),
            Some("text/plain")
        );
        assert_eq!(
            file_mime_type(Path::new("photo.PNG")).as_deref(),
            Some("image/png")
        );
    }
}
