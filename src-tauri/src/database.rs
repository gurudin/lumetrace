use rusqlite::Connection;
use std::{fs, path::PathBuf, sync::Mutex};

const SCHEMA: &str = r#"
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS app_settings (
  key        TEXT PRIMARY KEY,
  value      TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS file_space_folders (
  id            TEXT PRIMARY KEY,
  parent_id     TEXT REFERENCES file_space_folders(id) ON DELETE RESTRICT,
  name          TEXT NOT NULL,
  relative_path TEXT NOT NULL UNIQUE,
  manual_order  INTEGER NOT NULL DEFAULT 0,
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL,
  trashed_at    INTEGER
);

CREATE TABLE IF NOT EXISTS files (
  id            TEXT PRIMARY KEY,
  original_name TEXT NOT NULL,
  storage_path  TEXT,
  mime_type     TEXT,
  size_bytes    INTEGER,
  folder_id     TEXT REFERENCES file_space_folders(id) ON DELETE RESTRICT,
  source_kind   TEXT NOT NULL DEFAULT 'user_import',
  manual_order  INTEGER NOT NULL DEFAULT 0,
  updated_at    INTEGER NOT NULL DEFAULT 0,
  trashed_at    INTEGER,
  created_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS file_space_artifacts (
  file_id              TEXT PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
  task_id              TEXT NOT NULL,
  logical_key          TEXT NOT NULL,
  current_version_id   TEXT,
  observed_size_bytes  INTEGER,
  observed_mtime_ms    INTEGER,
  observed_sha256      TEXT,
  created_at           INTEGER NOT NULL,
  updated_at           INTEGER NOT NULL,
  UNIQUE(task_id, logical_key)
);

CREATE TABLE IF NOT EXISTS file_space_artifact_versions (
  id                 TEXT PRIMARY KEY,
  file_id            TEXT NOT NULL REFERENCES file_space_artifacts(file_id) ON DELETE CASCADE,
  version_number     INTEGER NOT NULL,
  snapshot_path      TEXT NOT NULL,
  sha256             TEXT NOT NULL,
  size_bytes         INTEGER NOT NULL,
  produced_name      TEXT NOT NULL,
  mime_type          TEXT,
  origin             TEXT NOT NULL CHECK (origin IN ('task', 'user_edit')),
  task_id            TEXT,
  task_title         TEXT,
  round_number       INTEGER,
  cell_id            TEXT,
  cell_name          TEXT,
  produced_at        INTEGER NOT NULL,
  created_at         INTEGER NOT NULL,
  source_turn_token  TEXT,
  source_artifact_id TEXT,
  UNIQUE(file_id, version_number),
  UNIQUE(source_artifact_id)
);

CREATE TABLE IF NOT EXISTS file_space_artifact_events (
  id           TEXT PRIMARY KEY,
  file_id      TEXT NOT NULL REFERENCES file_space_artifacts(file_id) ON DELETE CASCADE,
  version_id   TEXT REFERENCES file_space_artifact_versions(id) ON DELETE SET NULL,
  event_type   TEXT NOT NULL CHECK (event_type IN (
    'generated', 'modified', 'renamed', 'deleted', 'restored', 'current_version_changed'
  )),
  actor        TEXT NOT NULL CHECK (actor IN ('task', 'user')),
  details_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(details_json)),
  created_at   INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS file_space_file_tags (
  file_id        TEXT NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  tag            TEXT NOT NULL,
  normalized_tag TEXT NOT NULL,
  created_at     INTEGER NOT NULL,
  PRIMARY KEY(file_id, normalized_tag)
);

CREATE TABLE IF NOT EXISTS file_space_trash_entries (
  id                     TEXT PRIMARY KEY,
  item_type              TEXT NOT NULL CHECK (item_type IN ('file', 'folder')),
  root_file_id           TEXT UNIQUE REFERENCES files(id) ON DELETE CASCADE,
  root_folder_id         TEXT UNIQUE REFERENCES file_space_folders(id) ON DELETE CASCADE,
  original_parent_id     TEXT,
  original_name          TEXT NOT NULL,
  original_relative_path TEXT NOT NULL,
  payload_relative_path  TEXT NOT NULL UNIQUE,
  trashed_at             INTEGER NOT NULL,
  CHECK (
    (root_file_id IS NOT NULL AND root_folder_id IS NULL)
    OR (root_file_id IS NULL AND root_folder_id IS NOT NULL)
  )
);

CREATE TABLE IF NOT EXISTS file_space_trash_entry_files (
  entry_id TEXT NOT NULL REFERENCES file_space_trash_entries(id) ON DELETE CASCADE,
  file_id  TEXT NOT NULL UNIQUE REFERENCES files(id) ON DELETE CASCADE,
  PRIMARY KEY(entry_id, file_id)
);

CREATE TABLE IF NOT EXISTS file_space_trash_entry_folders (
  entry_id  TEXT NOT NULL REFERENCES file_space_trash_entries(id) ON DELETE CASCADE,
  folder_id TEXT NOT NULL UNIQUE REFERENCES file_space_folders(id) ON DELETE CASCADE,
  PRIMARY KEY(entry_id, folder_id)
);

-- Purge journals intentionally do not reference live File Space rows. A purge
-- must remain recoverable after its database transaction deletes those rows.
CREATE TABLE IF NOT EXISTS file_space_purge_operations (
  id         TEXT PRIMARY KEY,
  entry_id   TEXT NOT NULL UNIQUE,
  item_type  TEXT NOT NULL CHECK (item_type IN ('file', 'folder')),
  phase      TEXT NOT NULL CHECK (phase IN ('staging', 'database_committed')),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS file_space_purge_members (
  operation_id TEXT NOT NULL,
  member_type  TEXT NOT NULL CHECK (member_type IN ('file', 'folder')),
  member_id    TEXT NOT NULL,
  depth        INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY(operation_id, member_type, member_id)
);

CREATE TABLE IF NOT EXISTS file_space_purge_paths (
  operation_id          TEXT NOT NULL,
  path_kind             TEXT NOT NULL CHECK (path_kind IN ('trash_entry', 'artifact_directory')),
  base_kind             TEXT NOT NULL CHECK (base_kind IN ('storage', 'artifact_store')),
  original_relative_path TEXT NOT NULL,
  staged_relative_path   TEXT NOT NULL,
  expected_device        TEXT NOT NULL,
  expected_inode         TEXT NOT NULL,
  expected_is_directory  INTEGER NOT NULL CHECK (expected_is_directory IN (0, 1)),
  PRIMARY KEY(operation_id, base_kind, original_relative_path),
  UNIQUE(operation_id, base_kind, staged_relative_path)
);

CREATE TABLE IF NOT EXISTS file_space_search_documents (
  file_id         TEXT PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
  file_name       TEXT NOT NULL,
  body_text       TEXT NOT NULL DEFAULT '',
  extraction_status TEXT NOT NULL DEFAULT 'pending' CHECK (
    extraction_status IN ('pending', 'extracted', 'empty', 'unsupported', 'failed')
  ),
  extraction_error TEXT,
  extraction_version INTEGER NOT NULL DEFAULT 0,
  tag_text        TEXT NOT NULL DEFAULT '',
  task_text       TEXT NOT NULL DEFAULT '',
  cell_text       TEXT NOT NULL DEFAULT '',
  file_updated_at INTEGER NOT NULL,
  size_bytes      INTEGER NOT NULL,
  indexed_at      INTEGER NOT NULL
);

CREATE VIRTUAL TABLE IF NOT EXISTS file_space_search_fts USING fts5(
  file_name,
  body_text,
  tag_text,
  task_text,
  cell_text,
  content='file_space_search_documents',
  content_rowid='rowid',
  tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER IF NOT EXISTS file_space_search_documents_ai
AFTER INSERT ON file_space_search_documents BEGIN
  INSERT INTO file_space_search_fts(rowid, file_name, body_text, tag_text, task_text, cell_text)
  VALUES (new.rowid, new.file_name, new.body_text, new.tag_text, new.task_text, new.cell_text);
END;

CREATE TRIGGER IF NOT EXISTS file_space_search_documents_ad
AFTER DELETE ON file_space_search_documents BEGIN
  INSERT INTO file_space_search_fts(file_space_search_fts, rowid, file_name, body_text, tag_text, task_text, cell_text)
  VALUES ('delete', old.rowid, old.file_name, old.body_text, old.tag_text, old.task_text, old.cell_text);
END;

CREATE TRIGGER IF NOT EXISTS file_space_search_documents_au
AFTER UPDATE ON file_space_search_documents BEGIN
  INSERT INTO file_space_search_fts(file_space_search_fts, rowid, file_name, body_text, tag_text, task_text, cell_text)
  VALUES ('delete', old.rowid, old.file_name, old.body_text, old.tag_text, old.task_text, old.cell_text);
  INSERT INTO file_space_search_fts(rowid, file_name, body_text, tag_text, task_text, cell_text)
  VALUES (new.rowid, new.file_name, new.body_text, new.tag_text, new.task_text, new.cell_text);
END;

CREATE TABLE IF NOT EXISTS file_space_search_chunks (
  id                  TEXT PRIMARY KEY,
  file_id             TEXT NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  version_id          TEXT REFERENCES file_space_artifact_versions(id) ON DELETE SET NULL,
  document_indexed_at INTEGER NOT NULL,
  content_hash        TEXT NOT NULL,
  ordinal             INTEGER NOT NULL,
  start_character     INTEGER NOT NULL,
  end_character       INTEGER NOT NULL,
  body_text           TEXT NOT NULL,
  character_count     INTEGER NOT NULL,
  created_at          INTEGER NOT NULL,
  UNIQUE(file_id, ordinal)
);

CREATE TABLE IF NOT EXISTS file_space_semantic_embeddings (
  chunk_id    TEXT PRIMARY KEY REFERENCES file_space_search_chunks(id) ON DELETE CASCADE,
  model_id    TEXT NOT NULL,
  dimensions  INTEGER NOT NULL,
  vector      BLOB NOT NULL,
  created_at  INTEGER NOT NULL
);

-- The numeric key is the stable bridge between SQLite's source-of-truth
-- chunks and the derived on-disk ANN graph. It is intentionally stored in the
-- workspace database so ANN search results can always be validated against
-- current, non-trashed files before they become AI context.
CREATE TABLE IF NOT EXISTS file_space_semantic_ann_keys (
  ann_key  INTEGER PRIMARY KEY AUTOINCREMENT,
  chunk_id TEXT NOT NULL UNIQUE REFERENCES file_space_search_chunks(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS file_space_semantic_ann_state (
  model_id      TEXT PRIMARY KEY,
  data_revision INTEGER NOT NULL DEFAULT 0,
  file_revision INTEGER NOT NULL DEFAULT -1
);

INSERT OR IGNORE INTO file_space_semantic_ann_state
  (model_id, data_revision, file_revision)
VALUES ('multilingual-e5-small-int8-v1', 0, -1);

CREATE TRIGGER IF NOT EXISTS file_space_semantic_ann_keys_ai
AFTER INSERT ON file_space_semantic_ann_keys BEGIN
  UPDATE file_space_semantic_ann_state
  SET data_revision = data_revision + 1
  WHERE model_id = 'multilingual-e5-small-int8-v1';
END;

CREATE TRIGGER IF NOT EXISTS file_space_semantic_ann_keys_ad
AFTER DELETE ON file_space_semantic_ann_keys BEGIN
  UPDATE file_space_semantic_ann_state
  SET data_revision = data_revision + 1
  WHERE model_id = 'multilingual-e5-small-int8-v1';
END;

CREATE TABLE IF NOT EXISTS file_space_index_jobs (
  file_id                       TEXT PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
  requested_document_indexed_at INTEGER NOT NULL,
  status                        TEXT NOT NULL CHECK (status IN (
    'pending', 'extracting', 'embedding', 'ready', 'failed'
  )),
  retry_count                   INTEGER NOT NULL DEFAULT 0,
  error                         TEXT,
  requested_at                  INTEGER NOT NULL,
  started_at                    INTEGER,
  completed_at                  INTEGER
);

CREATE TABLE IF NOT EXISTS file_space_ai_turns (
  sequence     INTEGER PRIMARY KEY AUTOINCREMENT,
  id           TEXT NOT NULL UNIQUE,
  question     TEXT NOT NULL,
  answer       TEXT NOT NULL DEFAULT '',
  sources_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(sources_json)),
  status       TEXT NOT NULL DEFAULT 'completed' CHECK (status IN ('pending', 'completed', 'failed')),
  error_code   TEXT,
  duration_ms  INTEGER,
  created_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_file_space_folders_parent
  ON file_space_folders(parent_id, trashed_at, manual_order, id);
CREATE INDEX IF NOT EXISTS idx_files_folder
  ON files(folder_id, trashed_at, manual_order, id);
CREATE INDEX IF NOT EXISTS idx_file_space_artifacts_task
  ON file_space_artifacts(task_id, logical_key);
CREATE INDEX IF NOT EXISTS idx_file_space_versions_file
  ON file_space_artifact_versions(file_id, version_number DESC);
CREATE INDEX IF NOT EXISTS idx_file_space_events_file
  ON file_space_artifact_events(file_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_file_space_file_tags_tag
  ON file_space_file_tags(normalized_tag, file_id);
CREATE INDEX IF NOT EXISTS idx_file_space_trash_entries_deleted
  ON file_space_trash_entries(trashed_at DESC, id);
CREATE INDEX IF NOT EXISTS idx_file_space_trash_files_entry
  ON file_space_trash_entry_files(entry_id, file_id);
CREATE INDEX IF NOT EXISTS idx_file_space_trash_folders_entry
  ON file_space_trash_entry_folders(entry_id, folder_id);
CREATE INDEX IF NOT EXISTS idx_file_space_purge_operations_phase
  ON file_space_purge_operations(phase, created_at, id);
CREATE INDEX IF NOT EXISTS idx_file_space_purge_members_operation
  ON file_space_purge_members(operation_id, member_type, depth DESC, member_id);
CREATE INDEX IF NOT EXISTS idx_file_space_purge_paths_operation
  ON file_space_purge_paths(operation_id, base_kind, path_kind);
CREATE INDEX IF NOT EXISTS idx_file_space_search_chunks_file
  ON file_space_search_chunks(file_id, ordinal);
CREATE INDEX IF NOT EXISTS idx_file_space_semantic_embeddings_model
  ON file_space_semantic_embeddings(model_id, chunk_id);
CREATE INDEX IF NOT EXISTS idx_file_space_semantic_ann_keys_chunk
  ON file_space_semantic_ann_keys(chunk_id);
CREATE INDEX IF NOT EXISTS idx_file_space_index_jobs_status
  ON file_space_index_jobs(status, requested_at, file_id);
CREATE INDEX IF NOT EXISTS idx_file_space_ai_turns_created
  ON file_space_ai_turns(created_at, sequence);
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseLocation {
    pub workspace_id: String,
    pub database_path: PathBuf,
    pub artifact_store_path: PathBuf,
}

pub struct Database(pub Mutex<Connection>, Mutex<DatabaseLocation>);

fn open_connection(path: &PathBuf) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Unable to create Lume Trace database directory: {error}"))?;
    }
    let connection = Connection::open(path)
        .map_err(|error| format!("Unable to open Lume Trace database: {error}"))?;
    connection
        .execute_batch(SCHEMA)
        .map_err(|error| format!("Unable to initialize Lume Trace database: {error}"))?;
    let ai_turn_columns = {
        let mut statement = connection
            .prepare("PRAGMA table_info(file_space_ai_turns)")
            .map_err(|error| format!("Unable to inspect AI conversation storage: {error}"))?;
        statement
            .query_map([], |row| row.get::<_, String>(1))
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|error| format!("Unable to inspect AI conversation storage: {error}"))?
    };
    if !ai_turn_columns.iter().any(|column| column == "status") {
        connection
            .execute(
                "ALTER TABLE file_space_ai_turns
                 ADD COLUMN status TEXT NOT NULL DEFAULT 'completed'",
                [],
            )
            .map_err(|error| format!("Unable to migrate AI conversation status: {error}"))?;
    }
    if !ai_turn_columns.iter().any(|column| column == "error_code") {
        connection
            .execute(
                "ALTER TABLE file_space_ai_turns ADD COLUMN error_code TEXT",
                [],
            )
            .map_err(|error| format!("Unable to migrate AI conversation errors: {error}"))?;
    }
    if !ai_turn_columns.iter().any(|column| column == "updated_at") {
        connection
            .execute(
                "ALTER TABLE file_space_ai_turns
                 ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0",
                [],
            )
            .map_err(|error| format!("Unable to migrate AI conversation timestamps: {error}"))?;
    }
    if !ai_turn_columns.iter().any(|column| column == "duration_ms") {
        connection
            .execute(
                "ALTER TABLE file_space_ai_turns ADD COLUMN duration_ms INTEGER",
                [],
            )
            .map_err(|error| format!("Unable to migrate AI answer durations: {error}"))?;
    }
    let search_document_columns = {
        let mut statement = connection
            .prepare("PRAGMA table_info(file_space_search_documents)")
            .map_err(|error| {
                format!("Unable to inspect file content extraction storage: {error}")
            })?;
        statement
            .query_map([], |row| row.get::<_, String>(1))
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|error| {
                format!("Unable to inspect file content extraction storage: {error}")
            })?
    };
    if !search_document_columns
        .iter()
        .any(|column| column == "extraction_status")
    {
        connection
            .execute(
                "ALTER TABLE file_space_search_documents
                 ADD COLUMN extraction_status TEXT NOT NULL DEFAULT 'pending'",
                [],
            )
            .map_err(|error| format!("Unable to migrate file extraction status: {error}"))?;
    }
    if !search_document_columns
        .iter()
        .any(|column| column == "extraction_error")
    {
        connection
            .execute(
                "ALTER TABLE file_space_search_documents ADD COLUMN extraction_error TEXT",
                [],
            )
            .map_err(|error| format!("Unable to migrate file extraction errors: {error}"))?;
    }
    if !search_document_columns
        .iter()
        .any(|column| column == "extraction_version")
    {
        connection
            .execute(
                "ALTER TABLE file_space_search_documents
                 ADD COLUMN extraction_version INTEGER NOT NULL DEFAULT 0",
                [],
            )
            .map_err(|error| format!("Unable to migrate file extraction version: {error}"))?;
    }
    connection
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_file_space_search_documents_extraction
             ON file_space_search_documents(extraction_status, indexed_at, file_id)",
            [],
        )
        .map_err(|error| format!("Unable to index file extraction status: {error}"))?;
    Ok(connection)
}

pub(crate) fn recover_interrupted_ai_turns(database: &Database) -> Result<(), String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    connection
        .execute(
            "UPDATE file_space_ai_turns
             SET status = 'failed', error_code = 'ai_interrupted',
                 updated_at = MAX(updated_at, created_at)
             WHERE status = 'pending'",
            [],
        )
        .map_err(|error| format!("Unable to recover interrupted AI conversation turns: {error}"))?;
    Ok(())
}

pub(crate) fn open_workspace_database(
    database_path: PathBuf,
    artifact_store_path: PathBuf,
    workspace_id: String,
) -> Result<Database, String> {
    let connection = open_connection(&database_path)?;
    Ok(Database(
        Mutex::new(connection),
        Mutex::new(DatabaseLocation {
            workspace_id,
            database_path,
            artifact_store_path,
        }),
    ))
}

#[cfg(test)]
pub(crate) fn open_database(path: PathBuf) -> Result<Database, String> {
    let artifact_store_path = path
        .parent()
        .map(|parent| parent.join("file-space-versions"))
        .unwrap_or_else(|| PathBuf::from("file-space-versions"));
    let database = open_workspace_database(path, artifact_store_path, "test-workspace".to_owned())?;
    recover_interrupted_ai_turns(&database)?;
    Ok(database)
}

impl Database {
    pub(crate) fn location(&self) -> Result<DatabaseLocation, String> {
        self.1
            .lock()
            .map(|location| location.clone())
            .map_err(|_| "Unable to access the current Lume Trace workspace".to_owned())
    }

    pub(crate) fn artifact_store_path(&self) -> Result<PathBuf, String> {
        self.location().map(|location| location.artifact_store_path)
    }

    pub(crate) fn switch_workspace(&self, location: DatabaseLocation) -> Result<(), String> {
        let connection = open_connection(&location.database_path)?;
        let mut current_connection = self
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
        let mut current_location = self
            .1
            .lock()
            .map_err(|_| "Unable to access the current Lume Trace workspace".to_owned())?;
        *current_connection = connection;
        *current_location = location;
        Ok(())
    }
}

#[cfg(test)]
pub fn open_for_test(database_path: &std::path::Path) -> Result<Database, String> {
    if let Some(parent) = database_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Unable to create test database directory: {error}"))?;
    }
    open_database(database_path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn existing_ai_turns_receive_answer_duration_column() {
        let root = std::env::temp_dir().join(format!(
            "lumetrace-ai-duration-migration-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("lumetrace.sqlite3");
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE file_space_ai_turns (
                       sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                       id TEXT NOT NULL UNIQUE,
                       question TEXT NOT NULL,
                       answer TEXT NOT NULL DEFAULT '',
                       sources_json TEXT NOT NULL DEFAULT '[]',
                       status TEXT NOT NULL DEFAULT 'completed',
                       error_code TEXT,
                       created_at INTEGER NOT NULL,
                       updated_at INTEGER NOT NULL DEFAULT 0
                     );
                     INSERT INTO file_space_ai_turns
                       (id, question, answer, created_at, updated_at)
                     VALUES ('legacy-turn', 'question', 'answer', 1000, 2500);",
                )
                .unwrap();
        }
        let database = open_database(path).unwrap();
        let connection = database.0.lock().unwrap();
        let duration = connection
            .query_row(
                "SELECT duration_ms FROM file_space_ai_turns WHERE id = 'legacy-turn'",
                [],
                |row| row.get::<_, Option<i64>>(0),
            )
            .unwrap();
        assert_eq!(duration, None);
        drop(connection);
        drop(database);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn existing_search_documents_receive_content_extraction_columns() {
        let root = std::env::temp_dir().join(format!(
            "lumetrace-content-extraction-migration-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("lumetrace.sqlite3");
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE file_space_search_documents (
                       file_id TEXT PRIMARY KEY,
                       file_name TEXT NOT NULL,
                       body_text TEXT NOT NULL DEFAULT '',
                       tag_text TEXT NOT NULL DEFAULT '',
                       task_text TEXT NOT NULL DEFAULT '',
                       cell_text TEXT NOT NULL DEFAULT '',
                       file_updated_at INTEGER NOT NULL,
                       size_bytes INTEGER NOT NULL,
                       indexed_at INTEGER NOT NULL
                     );",
                )
                .unwrap();
        }
        let database = open_database(path).unwrap();
        let connection = database.0.lock().unwrap();
        let columns = connection
            .prepare("PRAGMA table_info(file_space_search_documents)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(columns.iter().any(|column| column == "extraction_status"));
        assert!(columns.iter().any(|column| column == "extraction_error"));
        assert!(columns.iter().any(|column| column == "extraction_version"));
        drop(connection);
        drop(database);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reopening_a_live_workspace_does_not_fail_pending_ai_turns() {
        let root = std::env::temp_dir().join(format!("lumetrace-live-ai-turn-{}", Uuid::new_v4()));
        let path = root.join("lumetrace.sqlite3");
        let versions = root.join("file-space-versions");
        let database =
            open_workspace_database(path.clone(), versions.clone(), "workspace".to_owned())
                .unwrap();
        database
            .0
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO file_space_ai_turns
                   (id, question, status, created_at, updated_at)
                 VALUES ('pending-turn', 'question', 'pending', 1, 1)",
                [],
            )
            .unwrap();
        drop(database);

        let reopened = open_workspace_database(path, versions, "workspace".to_owned()).unwrap();
        let status: String = reopened
            .0
            .lock()
            .unwrap()
            .query_row(
                "SELECT status FROM file_space_ai_turns WHERE id = 'pending-turn'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "pending");

        recover_interrupted_ai_turns(&reopened).unwrap();
        let recovered_status: String = reopened
            .0
            .lock()
            .unwrap()
            .query_row(
                "SELECT status FROM file_space_ai_turns WHERE id = 'pending-turn'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(recovered_status, "failed");

        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }
}
