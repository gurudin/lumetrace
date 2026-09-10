//! Internal integration seam for isolated file-space materializations.
//! The caller owns transport and any independently versioned extension tables.
//! Opening this connection does not register/switch the active workspace, start
//! filesystem watchers or run extraction/AI workers against remote paths.
use std::path::Path;

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
