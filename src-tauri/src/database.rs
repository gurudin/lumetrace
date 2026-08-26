use rusqlite::Connection;
use std::{fs, path::PathBuf, sync::Mutex};
use tauri::{AppHandle, Manager};

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

CREATE TABLE IF NOT EXISTS file_space_search_documents (
  file_id         TEXT PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
  file_name       TEXT NOT NULL,
  body_text       TEXT NOT NULL DEFAULT '',
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
"#;

pub struct Database(pub Mutex<Connection>);

fn open_database(path: PathBuf) -> Result<Database, String> {
    let connection = Connection::open(path)
        .map_err(|error| format!("Unable to open LumeTrace database: {error}"))?;
    connection
        .execute_batch(SCHEMA)
        .map_err(|error| format!("Unable to initialize LumeTrace database: {error}"))?;
    Ok(Database(Mutex::new(connection)))
}

pub fn initialize(app: &AppHandle) -> Result<Database, String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Unable to resolve application data directory: {error}"))?;
    fs::create_dir_all(&data_dir)
        .map_err(|error| format!("Unable to create application data directory: {error}"))?;
    open_database(data_dir.join("lumetrace.sqlite3"))
}

#[cfg(test)]
pub fn open_for_test(database_path: &std::path::Path) -> Result<Database, String> {
    if let Some(parent) = database_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Unable to create test database directory: {error}"))?;
    }
    open_database(database_path.to_path_buf())
}
