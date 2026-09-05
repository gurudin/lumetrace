use rusqlite::{Connection, Transaction, TransactionBehavior};
use std::{fs, path::PathBuf, sync::Mutex};

// Migration numbers are append-only once released. Never change or reuse an
// existing number; add a new migration and advance CURRENT_SCHEMA_VERSION.
const CURRENT_SCHEMA_VERSION: i64 = 3;

const MIGRATION_1_SCHEMA: &str = r#"

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

-- AI retrieval searches passages, not whole files. Keep this index independent
-- from file_space_search_fts so a natural-language question can retrieve the
-- exact evidence chunks that will be sent to the answer model. The trigram
-- tokenizer supports fast substring recall for CJK text as well as ordinary
-- Latin text without scanning file bodies.
CREATE VIRTUAL TABLE IF NOT EXISTS file_space_search_chunks_fts USING fts5(
  chunk_id UNINDEXED,
  file_id UNINDEXED,
  body_text,
  tokenize='trigram'
);

CREATE TRIGGER IF NOT EXISTS file_space_search_chunks_fts_ai
AFTER INSERT ON file_space_search_chunks BEGIN
  INSERT INTO file_space_search_chunks_fts(rowid, chunk_id, file_id, body_text)
  VALUES (new.rowid, new.id, new.file_id, new.body_text);
END;

CREATE TRIGGER IF NOT EXISTS file_space_search_chunks_fts_ad
AFTER DELETE ON file_space_search_chunks BEGIN
  DELETE FROM file_space_search_chunks_fts WHERE rowid = old.rowid;
END;

CREATE TRIGGER IF NOT EXISTS file_space_search_chunks_fts_au
AFTER UPDATE ON file_space_search_chunks BEGIN
  DELETE FROM file_space_search_chunks_fts WHERE rowid = old.rowid;
  INSERT INTO file_space_search_chunks_fts(rowid, chunk_id, file_id, body_text)
  VALUES (new.rowid, new.id, new.file_id, new.body_text);
END;

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

fn table_columns(connection: &Connection, table_name: &str) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table_name})"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(columns)
}

fn table_exists(connection: &Connection, table_name: &str) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS(
           SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
         )",
        [table_name],
        |row| row.get(0),
    )
}

fn migrate_to_v1(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    // Existing unversioned workspaces may already contain a fully populated
    // index. Record this before CREATE IF NOT EXISTS so adopting the v1 marker
    // never turns application startup into a synchronous full-index rebuild.
    let document_fts_existed = table_exists(transaction, "file_space_search_fts")?;
    let chunk_fts_existed = table_exists(transaction, "file_space_search_chunks_fts")?;

    transaction.execute_batch(MIGRATION_1_SCHEMA)?;

    // Databases created before schema versioning already contain these tables,
    // so CREATE TABLE IF NOT EXISTS cannot add the later columns for us.
    let ai_turn_columns = table_columns(transaction, "file_space_ai_turns")?;
    if !ai_turn_columns.iter().any(|column| column == "status") {
        transaction.execute(
            "ALTER TABLE file_space_ai_turns
             ADD COLUMN status TEXT NOT NULL DEFAULT 'completed'",
            [],
        )?;
    }
    if !ai_turn_columns.iter().any(|column| column == "error_code") {
        transaction.execute(
            "ALTER TABLE file_space_ai_turns ADD COLUMN error_code TEXT",
            [],
        )?;
    }
    if !ai_turn_columns.iter().any(|column| column == "updated_at") {
        transaction.execute(
            "ALTER TABLE file_space_ai_turns
             ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    if !ai_turn_columns.iter().any(|column| column == "duration_ms") {
        transaction.execute(
            "ALTER TABLE file_space_ai_turns ADD COLUMN duration_ms INTEGER",
            [],
        )?;
    }

    let search_document_columns = table_columns(transaction, "file_space_search_documents")?;
    if !search_document_columns
        .iter()
        .any(|column| column == "extraction_status")
    {
        transaction.execute(
            "ALTER TABLE file_space_search_documents
             ADD COLUMN extraction_status TEXT NOT NULL DEFAULT 'pending'",
            [],
        )?;
    }
    if !search_document_columns
        .iter()
        .any(|column| column == "extraction_error")
    {
        transaction.execute(
            "ALTER TABLE file_space_search_documents ADD COLUMN extraction_error TEXT",
            [],
        )?;
    }
    if !search_document_columns
        .iter()
        .any(|column| column == "extraction_version")
    {
        transaction.execute(
            "ALTER TABLE file_space_search_documents
             ADD COLUMN extraction_version INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    transaction.execute(
        "CREATE INDEX IF NOT EXISTS idx_file_space_search_documents_extraction
         ON file_space_search_documents(extraction_status, indexed_at, file_id)",
        [],
    )?;

    // Only a table created by this migration needs historical rows backfilled.
    // Existing FTS tables are left untouched to keep large-workspace startup
    // bounded and to preserve their already-built index state.
    if !document_fts_existed {
        transaction.execute(
            "INSERT INTO file_space_search_fts(file_space_search_fts) VALUES('rebuild')",
            [],
        )?;
    }
    if !chunk_fts_existed {
        transaction.execute(
            "INSERT INTO file_space_search_chunks_fts(rowid, chunk_id, file_id, body_text)
             SELECT rowid, id, file_id, body_text FROM file_space_search_chunks",
            [],
        )?;
    }

    Ok(())
}

fn migrate_connection(connection: &mut Connection) -> Result<(), String> {
    let mut version = connection
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .map_err(|error| format!("Unable to read Lume Trace schema version: {error}"))?;

    if version > CURRENT_SCHEMA_VERSION {
        return Err(format!(
            "This Lume Trace database uses schema version {version}, but this app supports up to version {CURRENT_SCHEMA_VERSION}"
        ));
    }

    while version < CURRENT_SCHEMA_VERSION {
        let next_version = version + 1;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| {
                format!("Unable to start Lume Trace schema migration {next_version}: {error}")
            })?;

        match next_version {
            1 => migrate_to_v1(&transaction),
            2 => transaction.execute_batch(
                "ALTER TABLE file_space_ai_turns ADD COLUMN context_json TEXT NOT NULL DEFAULT '[]';",
            ),
            3 => transaction.execute_batch(
                // Empty at migration time: populate old rows in small background
                // batches, not by scanning a large workspace during startup.
                "CREATE TABLE file_space_file_lookup (
                   file_id TEXT PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
                   file_name TEXT NOT NULL COLLATE NOCASE,
                   relative_path TEXT NOT NULL COLLATE NOCASE
                 );
                 CREATE INDEX idx_file_lookup_name ON file_space_file_lookup(file_name, file_id);
                 CREATE INDEX idx_file_lookup_path ON file_space_file_lookup(relative_path, file_id);
                 CREATE TRIGGER file_lookup_insert AFTER INSERT ON files
                 WHEN new.trashed_at IS NULL AND new.storage_path IS NOT NULL BEGIN
                   INSERT INTO file_space_file_lookup VALUES(new.id, new.original_name, new.storage_path);
                 END;
                 CREATE TRIGGER file_lookup_update AFTER UPDATE OF original_name, storage_path, trashed_at ON files BEGIN
                   DELETE FROM file_space_file_lookup WHERE file_id = old.id;
                   INSERT INTO file_space_file_lookup SELECT new.id, new.original_name, new.storage_path
                   WHERE new.trashed_at IS NULL AND new.storage_path IS NOT NULL;
                 END;",
            ),
            // CURRENT_SCHEMA_VERSION and this match must advance together.
            _ => unreachable!("missing migration for schema version {next_version}"),
        }
        .map_err(|error| {
            format!("Unable to apply Lume Trace schema migration {next_version}: {error}")
        })?;

        transaction
            .pragma_update(None, "user_version", next_version)
            .map_err(|error| {
                format!("Unable to record Lume Trace schema version {next_version}: {error}")
            })?;
        transaction.commit().map_err(|error| {
            format!("Unable to commit Lume Trace schema migration {next_version}: {error}")
        })?;
        version = next_version;
    }

    Ok(())
}

fn open_connection(path: &PathBuf) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Unable to create Lume Trace database directory: {error}"))?;
    }
    let mut connection = Connection::open(path)
        .map_err(|error| format!("Unable to open Lume Trace database: {error}"))?;
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(|error| format!("Unable to enable Lume Trace database integrity: {error}"))?;
    migrate_connection(&mut connection)?;
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

    fn schema_version(connection: &Connection) -> i64 {
        connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap()
    }

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
    fn legacy_unversioned_database_is_upgraded_without_losing_ai_or_search_data() {
        let root = std::env::temp_dir().join(format!(
            "lumetrace-schema-baseline-migration-{}",
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
                       created_at INTEGER NOT NULL
                     );
                     INSERT INTO file_space_ai_turns
                       (id, question, answer, sources_json, created_at)
                     VALUES (
                       'legacy-turn',
                       '断流后如何恢复？',
                       '保留的旧回答',
                       '[{\"fileId\":\"legacy-file\"}]',
                       1234
                     );

                     CREATE TABLE file_space_search_documents (
                       file_id TEXT PRIMARY KEY,
                       file_name TEXT NOT NULL,
                       body_text TEXT NOT NULL DEFAULT '',
                       tag_text TEXT NOT NULL DEFAULT '',
                       task_text TEXT NOT NULL DEFAULT '',
                       cell_text TEXT NOT NULL DEFAULT '',
                       file_updated_at INTEGER NOT NULL,
                       size_bytes INTEGER NOT NULL,
                       indexed_at INTEGER NOT NULL
                     );
                     INSERT INTO file_space_search_documents
                       (file_id, file_name, body_text, tag_text, task_text, cell_text,
                        file_updated_at, size_bytes, indexed_at)
                     VALUES (
                       'legacy-file',
                       '8月第二周-周报.md',
                       '断流恢复策略',
                       '周报',
                       '任务计划',
                       '',
                       100,
                       16,
                       200
                     );",
                )
                .unwrap();
            assert_eq!(schema_version(&connection), 0);
        }

        let database = open_database(path.clone()).unwrap();
        {
            let connection = database.0.lock().unwrap();
            assert_eq!(schema_version(&connection), CURRENT_SCHEMA_VERSION);

            let ai_turn = connection
                .query_row(
                    "SELECT question, answer, sources_json, status, error_code,
                            duration_ms, created_at, updated_at
                     FROM file_space_ai_turns WHERE id = 'legacy-turn'",
                    [],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, Option<String>>(4)?,
                            row.get::<_, Option<i64>>(5)?,
                            row.get::<_, i64>(6)?,
                            row.get::<_, i64>(7)?,
                        ))
                    },
                )
                .unwrap();
            assert_eq!(ai_turn.0, "断流后如何恢复？");
            assert_eq!(ai_turn.1, "保留的旧回答");
            assert_eq!(ai_turn.2, "[{\"fileId\":\"legacy-file\"}]");
            assert_eq!(ai_turn.3, "completed");
            assert_eq!(ai_turn.4, None);
            assert_eq!(ai_turn.5, None);
            assert_eq!(ai_turn.6, 1234);
            assert_eq!(ai_turn.7, 0);

            let search_document = connection
                .query_row(
                    "SELECT file_name, body_text, tag_text, task_text,
                            extraction_status, extraction_error, extraction_version,
                            indexed_at
                     FROM file_space_search_documents WHERE file_id = 'legacy-file'",
                    [],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, Option<String>>(5)?,
                            row.get::<_, i64>(6)?,
                            row.get::<_, i64>(7)?,
                        ))
                    },
                )
                .unwrap();
            assert_eq!(search_document.0, "8月第二周-周报.md");
            assert_eq!(search_document.1, "断流恢复策略");
            assert_eq!(search_document.2, "周报");
            assert_eq!(search_document.3, "任务计划");
            assert_eq!(search_document.4, "pending");
            assert_eq!(search_document.5, None);
            assert_eq!(search_document.6, 0);
            assert_eq!(search_document.7, 200);

            let indexed_count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM file_space_search_fts
                     WHERE file_space_search_fts MATCH '断流恢复策略'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(indexed_count, 1);
        }
        drop(database);

        // A second open must not replay or duplicate an already-applied migration.
        let reopened = open_database(path).unwrap();
        let connection = reopened.0.lock().unwrap();
        assert_eq!(schema_version(&connection), CURRENT_SCHEMA_VERSION);
        let ai_turn_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM file_space_ai_turns WHERE id = 'legacy-turn'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let search_document_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM file_space_search_documents
                 WHERE file_id = 'legacy-file'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(ai_turn_count, 1);
        assert_eq!(search_document_count, 1);
        drop(connection);
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn adopting_an_existing_unversioned_database_does_not_rebuild_fts_tables() {
        let root = std::env::temp_dir().join(format!(
            "lumetrace-existing-fts-migration-{}",
            Uuid::new_v4()
        ));
        let path = root.join("lumetrace.sqlite3");
        fs::create_dir_all(&root).unwrap();
        {
            // Build the historical schema itself, not today's schema with its
            // version flag reset (which would retain columns added by v2+).
            let mut connection = Connection::open(&path).unwrap();
            let transaction = connection.transaction().unwrap();
            migrate_to_v1(&transaction).unwrap();
            transaction.commit().unwrap();
            // Orphan sentinel rows make a rebuild or clear directly observable
            // without allocating a large test database.
            connection
                .execute(
                    "INSERT INTO file_space_search_fts
                       (rowid, file_name, body_text, tag_text, task_text, cell_text)
                     VALUES (900001, 'document_fts_sentinel', '', '', '', '')",
                    [],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO file_space_search_chunks_fts
                       (rowid, chunk_id, file_id, body_text)
                     VALUES (900002, 'chunk-sentinel', 'file-sentinel',
                             'chunk_fts_sentinel')",
                    [],
                )
                .unwrap();
            connection.pragma_update(None, "user_version", 0).unwrap();
        }

        let reopened = open_database(path).unwrap();
        let connection = reopened.0.lock().unwrap();
        assert_eq!(schema_version(&connection), CURRENT_SCHEMA_VERSION);
        let document_sentinel_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM file_space_search_fts
                 WHERE file_space_search_fts MATCH 'document_fts_sentinel'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let chunk_sentinel_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM file_space_search_chunks_fts
                 WHERE file_space_search_chunks_fts MATCH 'chunk_fts_sentinel'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(document_sentinel_count, 1);
        assert_eq!(chunk_sentinel_count, 1);
        drop(connection);
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_schema_migration_rolls_back_ddl_and_version_together() {
        let connection = &mut Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE VIEW file_space_search_documents AS
                 SELECT 'file' AS file_id, 'name' AS file_name;",
            )
            .unwrap();

        let error = migrate_connection(connection).unwrap_err();
        assert!(error.contains("schema migration 1"));
        assert_eq!(schema_version(connection), 0);
        let created_table_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table' AND name = 'app_settings'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(created_table_count, 0);
    }

    #[test]
    fn database_from_a_newer_app_is_not_opened_by_an_older_schema() {
        let connection = &mut Connection::open_in_memory().unwrap();
        connection
            .pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION + 1)
            .unwrap();

        let error = migrate_connection(connection).unwrap_err();
        assert!(error.contains(&format!("supports up to version {CURRENT_SCHEMA_VERSION}")));
        assert_eq!(schema_version(connection), CURRENT_SCHEMA_VERSION + 1);
    }

    #[test]
    fn schema_two_preserves_existing_turns_and_is_not_reapplied() {
        let mut connection = Connection::open_in_memory().unwrap();
        let transaction = connection.transaction().unwrap();
        migrate_to_v1(&transaction).unwrap();
        transaction.pragma_update(None, "user_version", 1).unwrap();
        transaction.commit().unwrap();
        connection.execute(
            "INSERT INTO file_space_ai_turns (id, question, answer, sources_json, status, created_at, updated_at)
             VALUES ('synthetic-turn', 'version one', 'version two', '[]', 'completed', 1, 2)", []
        ).unwrap();
        migrate_connection(&mut connection).unwrap();
        let record: (String, String, String) = connection.query_row(
            "SELECT question, answer, context_json FROM file_space_ai_turns WHERE id = 'synthetic-turn'", [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        ).unwrap();
        assert_eq!(
            record,
            ("version one".into(), "version two".into(), "[]".into())
        );
        connection.execute("UPDATE file_space_ai_turns SET context_json = '[\"synthetic-file\"]' WHERE id = 'synthetic-turn'", []).unwrap();
        migrate_connection(&mut connection).unwrap();
        let context: String = connection
            .query_row(
                "SELECT context_json FROM file_space_ai_turns WHERE id = 'synthetic-turn'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(context, "[\"synthetic-file\"]");
        assert_eq!(schema_version(&connection), CURRENT_SCHEMA_VERSION);
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

    #[test]
    fn schema_three_upgrades_v2_without_scanning_files_or_touching_history() {
        let mut connection = Connection::open_in_memory().unwrap();
        let transaction = connection.transaction().unwrap();
        migrate_to_v1(&transaction).unwrap();
        transaction.execute_batch("ALTER TABLE file_space_ai_turns ADD COLUMN context_json TEXT NOT NULL DEFAULT '[]';
            INSERT INTO files(id,original_name,storage_path,created_at) VALUES('existing','existing.md','existing.md',0);
            INSERT INTO file_space_ai_turns(id,question,status,created_at,updated_at,context_json)
            VALUES('turn','synthetic','completed',0,0,'[\"preserved\"]');").unwrap();
        transaction.pragma_update(None, "user_version", 2).unwrap();
        transaction.commit().unwrap();
        migrate_connection(&mut connection).unwrap();
        assert_eq!(schema_version(&connection), 3);
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM file_space_file_lookup", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT context_json FROM file_space_ai_turns WHERE id='turn'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "[\"preserved\"]"
        );
        connection.execute("INSERT INTO files(id,original_name,storage_path,created_at) VALUES('new','new.md','new.md',0)",[]).unwrap();
        migrate_connection(&mut connection).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM file_space_file_lookup", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}
