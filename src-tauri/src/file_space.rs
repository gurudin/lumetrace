use crate::database::Database;
use rusqlite::{params, params_from_iter, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashSet},
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, Stdio},
    sync::{mpsc::channel, Mutex, MutexGuard, OnceLock},
};
use tauri::{Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

const STORAGE_ROOT_SETTING: &str = "file_space.storage_root";
const PENDING_FILE_MOVE_SETTING: &str = "file_space.pending_file_move";
const TASK_VERSION_PREVIEW_MAX_BYTES: i64 = 100 * 1024 * 1024;
const MARKDOWN_FILE_MAX_BYTES: u64 = 5 * 1024 * 1024;
const FILE_SEARCH_TEXT_MAX_BYTES: u64 = 5 * 1024 * 1024;
const MATERIALIZED_FILE_NAME_MAX_BYTES: usize = 180;
static FILE_SPACE_OPERATION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static FILE_SPACE_IMPORT_CANCELLATIONS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

pub(crate) fn lock_file_space_operations() -> Result<MutexGuard<'static, ()>, String> {
    FILE_SPACE_OPERATION_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "Unable to lock File Space operations".to_owned())
}

fn sync_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        fs::File::open(path)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("Unable to sync {}: {error}", path.display()))?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskFileVersion {
    pub id: String,
    pub version_number: i64,
    pub name: String,
    pub mime_type: Option<String>,
    pub size_bytes: i64,
    pub origin: String,
    pub task_id: Option<String>,
    pub task_title: Option<String>,
    pub round_number: Option<i64>,
    pub cell_id: Option<String>,
    pub cell_name: Option<String>,
    pub produced_at: i64,
    pub is_current: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskFileEvent {
    pub id: String,
    pub version_id: Option<String>,
    pub event_type: String,
    pub actor: String,
    pub details: serde_json::Value,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskFileTimeline {
    pub file_id: String,
    pub logical_key: String,
    pub current_version_id: String,
    pub versions: Vec<TaskFileVersion>,
    pub events: Vec<TaskFileEvent>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileSpaceFolder {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub relative_path: String,
    pub manual_order: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileSpaceFile {
    pub id: String,
    pub folder_id: Option<String>,
    pub name: String,
    pub relative_path: String,
    pub mime_type: Option<String>,
    pub size_bytes: i64,
    pub source_kind: String,
    pub manual_order: i64,
    pub current_version: Option<i64>,
    pub version_count: i64,
    pub tags: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSpaceSearchRequest {
    pub query: String,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSpaceImportProgress {
    pub request_id: String,
    pub phase: String,
    pub processed: usize,
    pub total: usize,
    pub current_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSpaceDroppedFile {
    pub relative_path: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct PendingFileMove {
    version: u32,
    operation_id: String,
    file_id: String,
    source_relative_path: String,
    destination_relative_path: String,
    source_device: u64,
    source_inode: u64,
    created_at: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileSpaceSnapshot {
    pub root_path: Option<String>,
    pub root_name: Option<String>,
    pub root_status: String,
    pub folders: Vec<FileSpaceFolder>,
    pub files: Vec<FileSpaceFile>,
    pub trashed_files: Vec<FileSpaceFile>,
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn sha256_file(path: &Path) -> Result<(String, i64), String> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("Unable to read {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut size = 0_i64;
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("Unable to read {}: {error}", path.display()))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
        size = size
            .checked_add(count as i64)
            .ok_or_else(|| "The produced file is too large".to_owned())?;
    }
    Ok((format!("{:x}", hasher.finalize()), size))
}

fn snapshot_file(source: &Path, destination: &Path) -> Result<(), String> {
    let parent = destination
        .parent()
        .ok_or_else(|| "The artifact snapshot has no parent folder".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Unable to create artifact storage: {error}"))?;
    let temporary = parent.join(format!(".lumetrace-snapshot-{}", Uuid::new_v4()));
    copy_file_exclusive(source, &temporary)?;
    if let Err(error) = fs::rename(&temporary, destination) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("Unable to store the artifact snapshot: {error}"));
    }
    if let Err(error) = sync_directory(parent) {
        let _ = fs::remove_file(destination);
        return Err(error);
    }
    Ok(())
}

fn verify_file_integrity(
    path: &Path,
    expected_sha256: &str,
    expected_size: i64,
) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Unable to verify Task file version integrity: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(
            "Task file version integrity check failed: snapshot is not a regular file".to_owned(),
        );
    }
    let (actual_sha256, actual_size) = sha256_file(path)
        .map_err(|error| format!("Unable to verify Task file version integrity: {error}"))?;
    if actual_sha256 != expected_sha256 || actual_size != expected_size {
        return Err(
            "Task file version integrity check failed: snapshot content changed".to_owned(),
        );
    }
    Ok(())
}

fn read_verified_file(
    path: &Path,
    expected_sha256: &str,
    expected_size: i64,
    max_size: i64,
) -> Result<Vec<u8>, String> {
    if expected_size > max_size {
        return Err("Task file version preview is limited to 100 MB".to_owned());
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Unable to verify Task file version integrity: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(
            "Task file version integrity check failed: snapshot is not a regular file".to_owned(),
        );
    }
    let mut file = fs::File::open(path)
        .map_err(|error| format!("Unable to read Task file version: {error}"))?;
    let capacity = usize::try_from(expected_size)
        .map_err(|_| "Task file version preview is too large".to_owned())?;
    let mut bytes = Vec::with_capacity(capacity);
    file.read_to_end(&mut bytes)
        .map_err(|error| format!("Unable to read Task file version: {error}"))?;
    let actual_size = i64::try_from(bytes.len())
        .map_err(|_| "Task file version preview is too large".to_owned())?;
    let actual_sha256 = format!("{:x}", Sha256::digest(&bytes));
    if actual_sha256 != expected_sha256 || actual_size != expected_size {
        return Err(
            "Task file version integrity check failed: snapshot content changed".to_owned(),
        );
    }
    Ok(bytes)
}

fn copy_verified_snapshot(
    snapshot: &Path,
    destination: &Path,
    expected_sha256: &str,
    expected_size: i64,
) -> Result<(), String> {
    let parent = destination
        .parent()
        .ok_or_else(|| "The working file has no parent folder".to_owned())?;
    let temporary = parent.join(format!(".lumetrace-install-{}", Uuid::new_v4()));
    copy_file_exclusive(snapshot, &temporary)?;
    if let Err(error) = verify_file_integrity(&temporary, expected_sha256, expected_size) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    if let Err(error) = fs::hard_link(&temporary, destination) {
        let _ = fs::remove_file(&temporary);
        return Err(format!(
            "Unable to install the Task file working copy: {error}"
        ));
    }
    let _ = fs::remove_file(&temporary);
    Ok(())
}

fn observation_mtime(modified: std::time::SystemTime) -> Result<i64, String> {
    let nanos = modified
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    i64::try_from(nanos).map_err(|_| "The file modification time is out of range".to_owned())
}

fn file_mtime_marker(path: &Path) -> Result<i64, String> {
    let modified = fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map_err(|error| format!("Unable to inspect {}: {error}", path.display()))?;
    observation_mtime(modified)
}

fn materialize_pending_task_files_unlocked(database: &Database) -> Result<(), String> {
    let Ok(root) = require_ready_root(database) else {
        return Ok(());
    };
    let pending = {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
        let mut statement = connection
            .prepare(
                "SELECT f.id, f.original_name, f.storage_path, v.id, v.snapshot_path,
                        v.sha256, v.size_bytes, v.mime_type, a.observed_sha256
                 FROM files f
                 JOIN file_space_artifacts a ON a.file_id = f.id
                 JOIN file_space_artifact_versions v ON v.id = a.current_version_id
                 WHERE f.trashed_at IS NULL
                   AND (f.storage_path IS NULL
                        OR a.observed_sha256 IS NULL
                        OR a.observed_sha256 <> v.sha256)
                 ORDER BY f.created_at, f.id",
            )
            .map_err(|error| format!("Unable to prepare pending Task files: {error}"))?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            })
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|error| format!("Unable to load pending Task files: {error}"))?
    };

    for (
        file_id,
        requested_name,
        stored_path,
        version_id,
        snapshot_path,
        sha256,
        size_bytes,
        mime_type,
        observed_sha256,
    ) in pending
    {
        let (name, relative_path, destination, pending_update) = match stored_path {
            Some(relative_path) => {
                let destination = physical_path(&root, &relative_path)?;
                let pending_update = if destination.exists() {
                    let (working_sha256, _) = sha256_file(&destination)?;
                    if observed_sha256.as_deref() != Some(working_sha256.as_str()) {
                        let artifact_store = Path::new(&snapshot_path)
                            .parent()
                            .and_then(Path::parent)
                            .ok_or_else(|| "The Task file version path is invalid".to_owned())?;
                        reconcile_task_file(database, artifact_store, &file_id)?;
                        continue;
                    }
                    PendingWorkingCopyUpdate::Replacement(prepare_working_copy_replacement(
                        Path::new(&snapshot_path),
                        &destination,
                        &sha256,
                        size_bytes,
                        observed_sha256
                            .as_deref()
                            .ok_or_else(|| "The Task file observation is missing".to_owned())?,
                    )?)
                } else {
                    copy_verified_snapshot(
                        Path::new(&snapshot_path),
                        &destination,
                        &sha256,
                        size_bytes,
                    )?;
                    PendingWorkingCopyUpdate::Creation(PendingWorkingCopyCreation {
                        destination: destination.clone(),
                        installed_sha256: sha256.clone(),
                    })
                };
                (requested_name, relative_path, destination, pending_update)
            }
            None => {
                let safe_name = materialized_file_name(&requested_name);
                let name = unique_file_name(&root, &safe_name);
                let destination = root.join(&name);
                copy_verified_snapshot(
                    Path::new(&snapshot_path),
                    &destination,
                    &sha256,
                    size_bytes,
                )?;
                let pending_update =
                    PendingWorkingCopyUpdate::Creation(PendingWorkingCopyCreation {
                        destination: destination.clone(),
                        installed_sha256: sha256.clone(),
                    });
                (name.clone(), name, destination, pending_update)
            }
        };
        let materialization_result = (|| -> Result<bool, String> {
            pending_update.verify_installed()?;
            let mtime_ms = file_mtime_marker(&destination)?;
            let now = now_millis();
            let connection = database
                .0
                .lock()
                .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
            let transaction = connection
                .unchecked_transaction()
                .map_err(|error| format!("Unable to begin Task file materialization: {error}"))?;
            let updated = transaction
                .execute(
                    "UPDATE files
                     SET original_name = ?1, storage_path = ?2, mime_type = ?3,
                         size_bytes = ?4, updated_at = ?5
                     WHERE id = ?6 AND trashed_at IS NULL
                       AND EXISTS (
                         SELECT 1 FROM file_space_artifacts
                         WHERE file_id = ?6 AND current_version_id = ?7
                       )",
                    params![
                        name,
                        relative_path,
                        mime_type,
                        size_bytes,
                        now,
                        file_id,
                        version_id
                    ],
                )
                .map_err(|error| format!("Unable to save materialized Task file: {error}"))?;
            if updated != 1 {
                return Ok(false);
            }
            let observed = transaction
                .execute(
                    "UPDATE file_space_artifacts
                     SET observed_size_bytes = ?1, observed_mtime_ms = ?2,
                         observed_sha256 = ?3, updated_at = ?4
                     WHERE file_id = ?5 AND current_version_id = ?6",
                    params![size_bytes, mtime_ms, sha256, now, file_id, version_id],
                )
                .map_err(|error| format!("Unable to save Task file observation: {error}"))?;
            if observed != 1 {
                return Ok(false);
            }
            transaction.commit().map_err(|error| {
                format!("Unable to complete Task file materialization: {error}")
            })?;
            Ok(true)
        })();
        match materialization_result {
            Ok(true) => pending_update.commit(),
            Ok(false) => pending_update.rollback()?,
            Err(error) => {
                return match pending_update.rollback() {
                    Ok(()) => Err(error),
                    Err(cleanup_error) => Err(format!("{error}. {cleanup_error}")),
                };
            }
        }
    }
    Ok(())
}

fn load_task_file_timeline_record(
    database: &Database,
    file_id: &str,
) -> Result<TaskFileTimeline, String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
    let (logical_key, current_version_id) = connection
        .query_row(
            "SELECT logical_key, current_version_id FROM file_space_artifacts WHERE file_id = ?1",
            [file_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()
        .map_err(|error| format!("Unable to load Task file: {error}"))?
        .ok_or_else(|| "The Task file no longer exists".to_owned())?;
    let current_version_id =
        current_version_id.ok_or_else(|| "The Task file has no current version".to_owned())?;
    let mut version_statement = connection
        .prepare(
            "SELECT id, version_number, produced_name, mime_type, size_bytes, origin,
                    task_id, task_title, round_number, cell_id, cell_name, produced_at
             FROM file_space_artifact_versions WHERE file_id = ?1
             ORDER BY version_number DESC",
        )
        .map_err(|error| format!("Unable to prepare Task file versions: {error}"))?;
    let versions = version_statement
        .query_map([file_id], |row| {
            let id = row.get::<_, String>(0)?;
            Ok(TaskFileVersion {
                is_current: id == current_version_id,
                id,
                version_number: row.get(1)?,
                name: row.get(2)?,
                mime_type: row.get(3)?,
                size_bytes: row.get(4)?,
                origin: row.get(5)?,
                task_id: row.get(6)?,
                task_title: row.get(7)?,
                round_number: row.get(8)?,
                cell_id: row.get(9)?,
                cell_name: row.get(10)?,
                produced_at: row.get(11)?,
            })
        })
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .map_err(|error| format!("Unable to load Task file versions: {error}"))?;
    let mut event_statement = connection
        .prepare(
            "SELECT id, version_id, event_type, actor, details_json, created_at
             FROM file_space_artifact_events WHERE file_id = ?1
             ORDER BY created_at DESC, id DESC",
        )
        .map_err(|error| format!("Unable to prepare Task file timeline: {error}"))?;
    let events = event_statement
        .query_map([file_id], |row| {
            let details_json = row.get::<_, String>(4)?;
            Ok(TaskFileEvent {
                id: row.get(0)?,
                version_id: row.get(1)?,
                event_type: row.get(2)?,
                actor: row.get(3)?,
                details: serde_json::from_str(&details_json).unwrap_or_else(|_| json!({})),
                created_at: row.get(5)?,
            })
        })
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .map_err(|error| format!("Unable to load Task file timeline: {error}"))?;
    Ok(TaskFileTimeline {
        file_id: file_id.to_owned(),
        logical_key,
        current_version_id,
        versions,
        events,
    })
}

#[derive(Debug)]
struct PendingWorkingCopyCreation {
    destination: PathBuf,
    installed_sha256: String,
}

impl PendingWorkingCopyCreation {
    fn verify_installed(&self) -> Result<(), String> {
        verify_installed_working_copy(&self.destination, &self.installed_sha256)
    }

    fn rollback(self) -> Result<(), String> {
        let parent = self
            .destination
            .parent()
            .ok_or_else(|| "The working file has no parent folder".to_owned())?;
        let displaced = parent.join(format!(".lumetrace-rollback-{}", Uuid::new_v4()));
        match fs::rename(&self.destination, &displaced) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(format!(
                    "Unable to inspect the uncommitted working file: {error}"
                ))
            }
        }
        let displaced_sha256 = sha256_file(&displaced)
            .map_err(|error| format!("{error}. Working file preserved at {}", displaced.display()))?
            .0;
        if displaced_sha256 == self.installed_sha256 {
            fs::remove_file(&displaced).map_err(|error| {
                format!(
                    "Unable to remove the uncommitted working file preserved at {}: {error}",
                    displaced.display()
                )
            })?;
            return Ok(());
        }
        match fs::hard_link(&displaced, &self.destination) {
            Ok(()) => {
                let _ = fs::remove_file(&displaced);
                Ok(())
            }
            Err(error) if self.destination.exists() => Err(format!(
                "The working file changed during rollback; concurrent content was preserved at {}: {error}",
                displaced.display()
            )),
            Err(error) => Err(format!(
                "Unable to restore the concurrent working file preserved at {}: {error}",
                displaced.display()
            )),
        }
    }
}

#[derive(Debug)]
struct PendingWorkingCopyReplacement {
    destination: PathBuf,
    backup: PathBuf,
    expected_backup_sha256: String,
    installed_sha256: String,
}

impl PendingWorkingCopyReplacement {
    fn verify_installed(&self) -> Result<(), String> {
        verify_installed_working_copy(&self.destination, &self.installed_sha256)
    }

    fn commit(self) {
        if sha256_file(&self.backup).is_ok_and(|(sha256, _)| sha256 == self.expected_backup_sha256)
        {
            let _ = fs::remove_file(&self.backup);
        }
    }

    fn rollback(self) -> Result<(), String> {
        let parent = self
            .destination
            .parent()
            .ok_or_else(|| "The working file has no parent folder".to_owned())?;
        let displaced = parent.join(format!(".lumetrace-rollback-{}", Uuid::new_v4()));
        match fs::rename(&self.destination, &displaced) {
            Ok(()) => {
                let displaced_sha256 = sha256_file(&displaced)
                    .map_err(|error| {
                        format!("{error}. Working file preserved at {}", displaced.display())
                    })?
                    .0;
                if displaced_sha256 != self.installed_sha256 {
                    return match fs::hard_link(&displaced, &self.destination) {
                        Ok(()) => {
                            let _ = fs::remove_file(&displaced);
                            Ok(())
                        }
                        Err(error) => Err(format!(
                            "The working file changed during rollback; concurrent content was preserved at {}: {error}",
                            displaced.display()
                        )),
                    };
                }
                fs::remove_file(&displaced).map_err(|error| {
                    format!(
                        "Unable to remove the uncommitted working file preserved at {}: {error}",
                        displaced.display()
                    )
                })?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "Unable to inspect the uncommitted working file: {error}"
                ))
            }
        }
        match fs::hard_link(&self.backup, &self.destination) {
            Ok(()) => {
                let _ = fs::remove_file(&self.backup);
                Ok(())
            }
            Err(_) if self.destination.exists() => Ok(()),
            Err(error) => Err(format!(
                "Unable to restore the previous working file; backup preserved at {}: {error}",
                self.backup.display()
            )),
        }
    }
}

#[derive(Debug)]
enum PendingWorkingCopyUpdate {
    Creation(PendingWorkingCopyCreation),
    Replacement(PendingWorkingCopyReplacement),
}

impl PendingWorkingCopyUpdate {
    fn verify_installed(&self) -> Result<(), String> {
        match self {
            Self::Creation(creation) => creation.verify_installed(),
            Self::Replacement(replacement) => replacement.verify_installed(),
        }
    }

    fn commit(self) {
        if let Self::Replacement(replacement) = self {
            replacement.commit();
        }
    }

    fn rollback(self) -> Result<(), String> {
        match self {
            Self::Creation(creation) => creation.rollback(),
            Self::Replacement(replacement) => replacement.rollback(),
        }
    }
}

fn verify_installed_working_copy(path: &Path, expected_sha256: &str) -> Result<(), String> {
    match sha256_file(path) {
        Ok((sha256, _)) if sha256 == expected_sha256 => Ok(()),
        Ok(_) => Err(
            "The working file changed while it was being updated; retry the operation".to_owned(),
        ),
        Err(error) => Err(error),
    }
}

fn restore_backup_without_overwrite(backup: &Path, destination: &Path) {
    if !destination.exists() && fs::hard_link(backup, destination).is_ok() {
        let _ = fs::remove_file(backup);
    }
}

fn prepare_working_copy_replacement(
    snapshot: &Path,
    destination: &Path,
    expected_sha256: &str,
    expected_size: i64,
    expected_current_sha256: &str,
) -> Result<PendingWorkingCopyReplacement, String> {
    let parent = destination
        .parent()
        .ok_or_else(|| "The working file has no parent folder".to_owned())?;
    let temporary = parent.join(format!(".lumetrace-current-{}", Uuid::new_v4()));
    let backup = parent.join(format!(".lumetrace-backup-{}", Uuid::new_v4()));
    copy_verified_snapshot(snapshot, &temporary, expected_sha256, expected_size)?;
    if let Err(error) = fs::rename(destination, &backup) {
        let _ = fs::remove_file(&temporary);
        return Err(format!(
            "Unable to prepare the working file update: {error}"
        ));
    }
    let backup_sha256 = match sha256_file(&backup) {
        Ok((sha256, _)) => sha256,
        Err(error) => {
            restore_backup_without_overwrite(&backup, destination);
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
    };
    if backup_sha256 != expected_current_sha256 {
        restore_backup_without_overwrite(&backup, destination);
        let _ = fs::remove_file(&temporary);
        return Err(
            "The working file changed while it was being updated; retry the operation".to_owned(),
        );
    }
    if let Err(error) = fs::hard_link(&temporary, destination) {
        restore_backup_without_overwrite(&backup, destination);
        let _ = fs::remove_file(&temporary);
        return Err(format!("Unable to update the working file: {error}"));
    }
    let _ = fs::remove_file(&temporary);
    Ok(PendingWorkingCopyReplacement {
        destination: destination.to_path_buf(),
        backup,
        expected_backup_sha256: expected_current_sha256.to_owned(),
        installed_sha256: expected_sha256.to_owned(),
    })
}

fn task_artifact_exists(database: &Database, file_id: &str) -> Result<bool, String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM file_space_artifacts WHERE file_id = ?1)",
            [file_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("Unable to inspect File Space file: {error}"))
}

fn task_file_observed_sha256(database: &Database, file_id: &str) -> Result<String, String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
    connection
        .query_row(
            "SELECT observed_sha256 FROM file_space_artifacts WHERE file_id = ?1",
            [file_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(|error| format!("Unable to load Task file observation: {error}"))?
        .flatten()
        .ok_or_else(|| "The Task file observation is missing".to_owned())
}

fn reconcile_task_file(
    database: &Database,
    artifact_store: &Path,
    file_id: &str,
) -> Result<(), String> {
    let root = match require_ready_root(database) {
        Ok(root) => root,
        Err(_) => return Ok(()),
    };
    let record = {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
        connection
            .query_row(
                "SELECT f.storage_path, f.original_name, f.mime_type,
                        a.observed_size_bytes, a.observed_mtime_ms, a.observed_sha256,
                        v.snapshot_path, v.sha256, v.size_bytes
                 FROM files f
                 JOIN file_space_artifacts a ON a.file_id = f.id
                 JOIN file_space_artifact_versions v ON v.id = a.current_version_id
                 WHERE f.id = ?1 AND f.trashed_at IS NULL AND f.storage_path IS NOT NULL",
                [file_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, i64>(8)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| format!("Unable to load Task file observation: {error}"))?
    };
    let Some((
        relative_path,
        name,
        mime_type,
        observed_size,
        observed_mtime,
        observed_sha,
        current_snapshot_path,
        current_sha256,
        current_size,
    )) = record
    else {
        return Ok(());
    };
    let path = physical_path(&root, &relative_path)?;
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .ok_or_else(|| "The Task file working copy has no parent folder".to_owned())?;
            fs::create_dir_all(parent)
                .map_err(|error| format!("Unable to restore Task file folder: {error}"))?;
            copy_verified_snapshot(
                Path::new(&current_snapshot_path),
                &path,
                &current_sha256,
                current_size,
            )?;
            let mtime_ms = file_mtime_marker(&path)?;
            let now = now_millis();
            let connection = database
                .0
                .lock()
                .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
            connection
                .execute(
                    "UPDATE file_space_artifacts
                     SET observed_size_bytes = ?1, observed_mtime_ms = ?2,
                         observed_sha256 = ?3, updated_at = ?4
                     WHERE file_id = ?5",
                    params![current_size, mtime_ms, current_sha256, now, file_id],
                )
                .map_err(|error| format!("Unable to update Task file observation: {error}"))?;
            connection
                .execute(
                    "UPDATE files SET size_bytes = ?1, updated_at = ?2 WHERE id = ?3",
                    params![current_size, now, file_id],
                )
                .map_err(|error| format!("Unable to update restored Task file: {error}"))?;
            return Ok(());
        }
        Err(error) => {
            return Err(format!("Unable to inspect Task file working copy: {error}"));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("The Task file working copy must be a regular file".to_owned());
    }
    let size_bytes =
        i64::try_from(metadata.len()).map_err(|_| "The Task file is too large".to_owned())?;
    let mtime_ms = file_mtime_marker(&path)?;
    if observed_size == Some(size_bytes) && observed_mtime == Some(mtime_ms) {
        return Ok(());
    }
    let (working_sha256, working_size) = sha256_file(&path)?;
    if observed_sha.as_deref() == Some(&working_sha256) {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
        connection
            .execute(
                "UPDATE file_space_artifacts
                 SET observed_size_bytes = ?1, observed_mtime_ms = ?2, updated_at = ?3
                 WHERE file_id = ?4",
                params![working_size, mtime_ms, now_millis(), file_id],
            )
            .map_err(|error| format!("Unable to update Task file observation: {error}"))?;
        return Ok(());
    }

    let (version_number, now) = {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
        let version_number = connection
            .query_row(
                "SELECT COALESCE(MAX(version_number), 0) + 1
                 FROM file_space_artifact_versions WHERE file_id = ?1",
                [file_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| format!("Unable to choose edited Task file version: {error}"))?;
        (version_number, now_millis())
    };
    let version_id = Uuid::new_v4().to_string();
    let snapshot_path = artifact_store
        .join(file_id)
        .join(format!("{version_id}.blob"));
    snapshot_file(&path, &snapshot_path)?;
    let (sha256, actual_size) = match sha256_file(&snapshot_path) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let _ = fs::remove_file(&snapshot_path);
            return Err(error);
        }
    };
    let result = (|| -> Result<(), String> {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
        let transaction = connection
            .unchecked_transaction()
            .map_err(|error| format!("Unable to begin Task file edit capture: {error}"))?;
        transaction
            .execute(
                "INSERT INTO file_space_artifact_versions
                 (id, file_id, version_number, snapshot_path, sha256, size_bytes,
                  produced_name, mime_type, origin, produced_at, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'user_edit', ?9, ?10)",
                params![
                    version_id,
                    file_id,
                    version_number,
                    snapshot_path.to_string_lossy(),
                    sha256,
                    actual_size,
                    name,
                    mime_type,
                    now,
                    now
                ],
            )
            .map_err(|error| format!("Unable to save edited Task file version: {error}"))?;
        transaction
            .execute(
                "UPDATE file_space_artifacts
                 SET current_version_id = ?1, observed_size_bytes = ?2,
                     observed_mtime_ms = ?3, observed_sha256 = ?4, updated_at = ?5
                 WHERE file_id = ?6",
                params![version_id, actual_size, mtime_ms, sha256, now, file_id],
            )
            .map_err(|error| format!("Unable to select edited Task file version: {error}"))?;
        transaction
            .execute(
                "UPDATE files SET size_bytes = ?1, updated_at = ?2 WHERE id = ?3",
                params![actual_size, now, file_id],
            )
            .map_err(|error| format!("Unable to update edited Task file: {error}"))?;
        transaction
            .execute(
                "INSERT INTO file_space_artifact_events
                 (id, file_id, version_id, event_type, actor, details_json, created_at)
                 VALUES (?1, ?2, ?3, 'modified', 'user', ?4, ?5)",
                params![
                    Uuid::new_v4().to_string(),
                    file_id,
                    version_id,
                    json!({ "version": version_number }).to_string(),
                    now
                ],
            )
            .map_err(|error| format!("Unable to save Task file modification event: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("Unable to complete Task file edit capture: {error}"))?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(snapshot_path);
        return Err(error);
    }
    Ok(())
}

fn set_current_task_file_version_record(
    database: &Database,
    artifact_store: &Path,
    file_id: &str,
    version_id: &str,
) -> Result<FileSpaceSnapshot, String> {
    reconcile_task_file(database, artifact_store, file_id)?;
    let root = require_ready_root(database)?;
    let (relative_path, snapshot_path, sha256, size_bytes, observed_sha256) = {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
        connection
            .query_row(
                "SELECT f.storage_path, v.snapshot_path, v.sha256, v.size_bytes,
                        a.observed_sha256
                 FROM files f
                 JOIN file_space_artifacts a ON a.file_id = f.id
                 JOIN file_space_artifact_versions v ON v.file_id = f.id
                 WHERE f.id = ?1 AND v.id = ?2 AND f.trashed_at IS NULL
                   AND f.storage_path IS NOT NULL",
                params![file_id, version_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| format!("Unable to load selected Task file version: {error}"))?
            .ok_or_else(|| "The selected Task file version no longer exists".to_owned())?
    };
    let destination = physical_path(&root, &relative_path)?;
    let replacement = prepare_working_copy_replacement(
        Path::new(&snapshot_path),
        &destination,
        &sha256,
        size_bytes,
        observed_sha256
            .as_deref()
            .ok_or_else(|| "The Task file observation is missing".to_owned())?,
    )?;
    let database_result = (|| -> Result<(), String> {
        replacement.verify_installed()?;
        let mtime_ms = file_mtime_marker(&destination)?;
        let now = now_millis();
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
        let transaction = connection
            .unchecked_transaction()
            .map_err(|error| format!("Unable to begin current version change: {error}"))?;
        transaction
            .execute(
                "UPDATE file_space_artifacts
                 SET current_version_id = ?1, observed_size_bytes = ?2,
                     observed_mtime_ms = ?3, observed_sha256 = ?4, updated_at = ?5
                 WHERE file_id = ?6",
                params![version_id, size_bytes, mtime_ms, sha256, now, file_id],
            )
            .map_err(|error| format!("Unable to select Task file version: {error}"))?;
        transaction
            .execute(
                "UPDATE files SET size_bytes = ?1, updated_at = ?2 WHERE id = ?3",
                params![size_bytes, now, file_id],
            )
            .map_err(|error| format!("Unable to update current Task file: {error}"))?;
        transaction
            .execute(
                "INSERT INTO file_space_artifact_events
                 (id, file_id, version_id, event_type, actor, details_json, created_at)
                 VALUES (?1, ?2, ?3, 'current_version_changed', 'user', '{}', ?4)",
                params![Uuid::new_v4().to_string(), file_id, version_id, now],
            )
            .map_err(|error| format!("Unable to save current version event: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("Unable to complete current version change: {error}"))?;
        Ok(())
    })();
    match database_result {
        Ok(()) => replacement.commit(),
        Err(error) => {
            return match replacement.rollback() {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(format!("{error}. {rollback_error}")),
            };
        }
    }
    load_snapshot_record(database)
}

fn read_task_file_version_record(
    database: &Database,
    file_id: &str,
    version_id: &str,
) -> Result<Vec<u8>, String> {
    let (snapshot_path, expected_sha256, expected_size) = {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
        connection
            .query_row(
                "SELECT snapshot_path, sha256, size_bytes FROM file_space_artifact_versions
                 WHERE file_id = ?1 AND id = ?2",
                params![file_id, version_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| format!("Unable to load Task file version: {error}"))?
            .ok_or_else(|| "The Task file version no longer exists".to_owned())?
    };
    read_verified_file(
        Path::new(&snapshot_path),
        &expected_sha256,
        expected_size,
        TASK_VERSION_PREVIEW_MAX_BYTES,
    )
}

fn artifact_store_path<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("file-space-versions"))
        .map_err(|error| format!("Unable to resolve File Space version storage: {error}"))
}

fn capture_initial_user_versions(database: &Database, artifact_store: &Path) -> Result<(), String> {
    let root = match require_ready_root(database) {
        Ok(root) => root,
        Err(_) => return Ok(()),
    };
    let candidates = {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
        let mut statement = connection
            .prepare(
                "SELECT f.id, f.original_name, f.storage_path, f.mime_type, f.created_at
                 FROM files f
                 LEFT JOIN file_space_artifacts artifact ON artifact.file_id = f.id
                 WHERE f.trashed_at IS NULL
                   AND f.storage_path IS NOT NULL
                   AND artifact.file_id IS NULL
                 ORDER BY f.created_at, f.id",
            )
            .map_err(|error| format!("Unable to prepare unversioned files: {error}"))?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|error| format!("Unable to load unversioned files: {error}"))?
    };

    for (file_id, name, relative_path, mime_type, created_at) in candidates {
        let source = physical_path(&root, &relative_path)?;
        let metadata = fs::symlink_metadata(&source)
            .map_err(|error| format!("Unable to inspect imported file {name}: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(format!("The imported file is not a regular file: {name}"));
        }
        let version_id = Uuid::new_v4().to_string();
        let snapshot_path = artifact_store
            .join(&file_id)
            .join(format!("{version_id}.blob"));
        snapshot_file(&source, &snapshot_path)?;
        let (sha256, size_bytes) = match sha256_file(&snapshot_path) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let _ = fs::remove_file(&snapshot_path);
                return Err(error);
            }
        };
        let mtime_ms = match file_mtime_marker(&source) {
            Ok(mtime) => mtime,
            Err(error) => {
                let _ = fs::remove_file(&snapshot_path);
                return Err(error);
            }
        };
        let now = now_millis();
        let save_result = (|| -> Result<(), String> {
            let connection = database
                .0
                .lock()
                .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
            let transaction = connection
                .unchecked_transaction()
                .map_err(|error| format!("Unable to begin initial version capture: {error}"))?;
            transaction
                .execute(
                    "INSERT OR IGNORE INTO file_space_artifacts
                     (file_id, task_id, logical_key, current_version_id,
                      observed_size_bytes, observed_mtime_ms, observed_sha256,
                      created_at, updated_at)
                     VALUES (?1, 'local', ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![file_id, version_id, size_bytes, mtime_ms, sha256, created_at, now],
                )
                .map_err(|error| format!("Unable to create the logical file history: {error}"))?;
            if transaction.changes() == 0 {
                return Ok(());
            }
            transaction
                .execute(
                    "INSERT INTO file_space_artifact_versions
                     (id, file_id, version_number, snapshot_path, sha256, size_bytes,
                      produced_name, mime_type, origin, produced_at, created_at)
                     VALUES (?1, ?2, 1, ?3, ?4, ?5, ?6, ?7, 'user_edit', ?8, ?9)",
                    params![
                        version_id,
                        file_id,
                        snapshot_path.to_string_lossy(),
                        sha256,
                        size_bytes,
                        name,
                        mime_type,
                        created_at,
                        now
                    ],
                )
                .map_err(|error| format!("Unable to save the initial file version: {error}"))?;
            transaction
                .execute(
                    "INSERT INTO file_space_artifact_events
                     (id, file_id, version_id, event_type, actor, details_json, created_at)
                     VALUES (?1, ?2, ?3, 'generated', 'user', ?4, ?5)",
                    params![
                        Uuid::new_v4().to_string(),
                        file_id,
                        version_id,
                        json!({ "version": 1, "source": "import" }).to_string(),
                        created_at
                    ],
                )
                .map_err(|error| format!("Unable to save the initial file event: {error}"))?;
            transaction
                .commit()
                .map_err(|error| format!("Unable to complete initial version capture: {error}"))?;
            Ok(())
        })();
        if let Err(error) = save_result {
            let _ = fs::remove_file(&snapshot_path);
            return Err(error);
        }
    }
    Ok(())
}

fn read_storage_root(database: &Database) -> Result<Option<PathBuf>, String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
    connection
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [STORAGE_ROOT_SETTING],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map(|path| path.map(PathBuf::from))
        .map_err(|error| format!("Unable to load the File Space storage path: {error}"))
}

fn write_probe(root: &Path) -> Result<(), String> {
    let probe = root.join(format!(".lumetrace-write-probe-{}", Uuid::new_v4()));
    let result = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .and_then(|mut file| file.write_all(b"lumetrace"));
    let _ = fs::remove_file(&probe);
    result.map_err(|error| format!("The selected folder is not writable: {error}"))
}

fn inspect_root(path: Option<&Path>) -> String {
    let Some(path) = path else {
        return "unconfigured".to_owned();
    };
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return "missing".to_owned();
        }
        Err(_) => return "notWritable".to_owned(),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return "notDirectory".to_owned();
    }
    if write_probe(path).is_err() {
        return "notWritable".to_owned();
    }
    "ready".to_owned()
}

fn root_name(root: &Path) -> String {
    root.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| root.to_string_lossy().into_owned())
}

fn relative_child(parent: Option<&str>, name: &str) -> String {
    match parent.filter(|value| !value.is_empty()) {
        Some(parent) => format!("{parent}/{name}"),
        None => name.to_owned(),
    }
}

fn physical_path(root: &Path, relative_path: &str) -> Result<PathBuf, String> {
    let components = relative_path.split('/').collect::<Vec<_>>();
    if components.is_empty()
        || components
            .iter()
            .any(|component| component.is_empty() || *component == "." || *component == "..")
    {
        return Err("The managed path is invalid".to_owned());
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("Unable to resolve the File Space storage path: {error}"))?;
    let mut candidate = canonical_root.clone();
    for (index, component) in components.iter().enumerate() {
        candidate.push(component);
        match fs::symlink_metadata(&candidate) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err("The managed path contains a symbolic link".to_owned());
                }
                if index + 1 < components.len() && !metadata.is_dir() {
                    return Err("The managed path contains a non-folder component".to_owned());
                }
                let canonical = candidate
                    .canonicalize()
                    .map_err(|error| format!("Unable to resolve the managed path: {error}"))?;
                if !canonical.starts_with(&canonical_root) {
                    return Err(
                        "The managed path is outside the File Space storage path".to_owned()
                    );
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                for remaining in &components[index + 1..] {
                    candidate.push(remaining);
                }
                return Ok(candidate);
            }
            Err(error) => {
                return Err(format!("Unable to inspect the managed path: {error}"));
            }
        }
    }
    Ok(candidate)
}

fn validate_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("A name is required".to_owned());
    }
    if name == "." || name == ".." || name.contains(['/', '\\']) {
        return Err("The name contains unsupported path characters".to_owned());
    }
    if name.chars().any(char::is_control) {
        return Err("The name contains unsupported control characters".to_owned());
    }
    Ok(name.to_owned())
}

fn validate_import_name(name: &str) -> Result<String, String> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        return Err("The selected item has an unsupported name".to_owned());
    }
    if name.chars().any(char::is_control) {
        return Err("The selected item name contains unsupported control characters".to_owned());
    }
    Ok(name.to_owned())
}

fn truncate_utf8_bytes(value: &str, max_bytes: usize) -> String {
    let mut bytes = 0;
    value
        .chars()
        .take_while(|character| {
            let next = bytes + character.len_utf8();
            if next > max_bytes {
                false
            } else {
                bytes = next;
                true
            }
        })
        .collect()
}

fn materialized_file_name(name: &str) -> String {
    let sanitized = if validate_import_name(name).is_ok() {
        name.to_owned()
    } else {
        let sanitized = name
            .chars()
            .map(|character| {
                if character == '/' || character == '\\' || character.is_control() {
                    '_'
                } else {
                    character
                }
            })
            .collect::<String>();
        if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
            "artifact".to_owned()
        } else {
            sanitized
        }
    };
    if sanitized.len() <= MATERIALIZED_FILE_NAME_MAX_BYTES {
        return sanitized;
    }
    let path = Path::new(&sanitized);
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| truncate_utf8_bytes(value, 32));
    let suffix = extension
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(|value| format!(".{value}"))
        .unwrap_or_default();
    let stem_budget = MATERIALIZED_FILE_NAME_MAX_BYTES.saturating_sub(suffix.len());
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .map(|value| truncate_utf8_bytes(value, stem_budget))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "artifact".to_owned());
    format!("{stem}{suffix}")
}

fn canonical_import_roots(paths: &[String]) -> Result<Vec<PathBuf>, String> {
    let mut canonical_paths = Vec::with_capacity(paths.len());
    for path in paths {
        let source = PathBuf::from(path);
        let metadata = fs::symlink_metadata(&source)
            .map_err(|error| format!("Unable to inspect {}: {error}", source.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "Symbolic links cannot be imported: {}",
                source.display()
            ));
        }
        if !metadata.is_file() && !metadata.is_dir() {
            return Err(format!(
                "The selected item is not a regular file or folder: {}",
                source.display()
            ));
        }
        let canonical = source
            .canonicalize()
            .map_err(|error| format!("Unable to resolve {}: {error}", source.display()))?;
        canonical_paths.push(canonical);
    }

    canonical_paths.sort_by(|left, right| {
        left.components()
            .count()
            .cmp(&right.components().count())
            .then_with(|| left.cmp(right))
    });
    let mut selected = Vec::<PathBuf>::new();
    for candidate in canonical_paths {
        if selected
            .iter()
            .any(|ancestor| candidate == *ancestor || candidate.starts_with(ancestor))
        {
            continue;
        }
        selected.push(candidate);
    }
    Ok(selected)
}

fn require_ready_root(database: &Database) -> Result<PathBuf, String> {
    let root = read_storage_root(database)?
        .ok_or_else(|| "Set a local File Space storage path first".to_owned())?;
    match inspect_root(Some(&root)).as_str() {
        "ready" => Ok(root),
        "missing" => Err("The configured File Space storage path no longer exists".to_owned()),
        "notDirectory" => Err("The configured File Space storage path is not a folder".to_owned()),
        _ => Err("The configured File Space storage path is not writable".to_owned()),
    }
}

#[derive(Debug)]
struct SearchDocumentCandidate {
    file_id: String,
    file_name: String,
    relative_path: String,
    file_updated_at: i64,
    size_bytes: i64,
    tag_text: String,
    task_text: String,
    cell_text: String,
    indexed_file_name: Option<String>,
    indexed_body_text: Option<String>,
    indexed_tag_text: Option<String>,
    indexed_task_text: Option<String>,
    indexed_cell_text: Option<String>,
    indexed_file_updated_at: Option<i64>,
    indexed_size_bytes: Option<i64>,
}

fn supports_searchable_text(name: &str) -> bool {
    matches!(
        Path::new(name)
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some(
            "md" | "markdown"
                | "txt"
                | "csv"
                | "html"
                | "htm"
                | "json"
                | "xml"
                | "yaml"
                | "yml"
                | "css"
                | "js"
                | "jsx"
                | "ts"
                | "tsx"
                | "svg"
        )
    )
}

fn read_searchable_text(root: &Path, relative_path: &str, name: &str) -> String {
    if !supports_searchable_text(name) {
        return String::new();
    }
    let Ok(canonical_root) = root.canonicalize() else {
        return String::new();
    };
    let Ok(candidate) = physical_path(root, relative_path) else {
        return String::new();
    };
    let Ok(metadata) = fs::symlink_metadata(&candidate) else {
        return String::new();
    };
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > FILE_SEARCH_TEXT_MAX_BYTES
    {
        return String::new();
    }
    let Ok(canonical_file) = candidate.canonicalize() else {
        return String::new();
    };
    if canonical_file == canonical_root || !canonical_file.starts_with(&canonical_root) {
        return String::new();
    }
    fs::read(canonical_file)
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default()
}

fn synchronize_file_search_index(database: &Database, root: Option<&Path>) -> Result<(), String> {
    let Some(root) = root else {
        return Ok(());
    };
    if inspect_root(Some(root)) != "ready" {
        return Ok(());
    }
    let candidates = {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
        let mut statement = connection
            .prepare(
                "SELECT f.id, f.original_name, f.storage_path, f.updated_at,
                        COALESCE(f.size_bytes, 0),
                        COALESCE((
                          SELECT group_concat(tag, char(31))
                          FROM file_space_file_tags tags WHERE tags.file_id = f.id
                        ), ''),
                        COALESCE((
                          SELECT group_concat(DISTINCT task_title)
                          FROM file_space_artifact_versions versions
                          WHERE versions.file_id = f.id AND task_title IS NOT NULL
                        ), ''),
                        COALESCE((
                          SELECT group_concat(DISTINCT cell_name)
                          FROM file_space_artifact_versions versions
                          WHERE versions.file_id = f.id AND cell_name IS NOT NULL
                        ), ''),
                        search.file_name, search.body_text, search.tag_text,
                        search.task_text, search.cell_text, search.file_updated_at,
                        search.size_bytes
                 FROM files f
                 LEFT JOIN file_space_search_documents search ON search.file_id = f.id
                 WHERE f.trashed_at IS NULL AND f.storage_path IS NOT NULL",
            )
            .map_err(|error| format!("Unable to prepare File Space search index: {error}"))?;
        statement
            .query_map([], |row| {
                Ok(SearchDocumentCandidate {
                    file_id: row.get(0)?,
                    file_name: row.get(1)?,
                    relative_path: row.get(2)?,
                    file_updated_at: row.get(3)?,
                    size_bytes: row.get(4)?,
                    tag_text: row.get(5)?,
                    task_text: row.get(6)?,
                    cell_text: row.get(7)?,
                    indexed_file_name: row.get(8)?,
                    indexed_body_text: row.get(9)?,
                    indexed_tag_text: row.get(10)?,
                    indexed_task_text: row.get(11)?,
                    indexed_cell_text: row.get(12)?,
                    indexed_file_updated_at: row.get(13)?,
                    indexed_size_bytes: row.get(14)?,
                })
            })
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|error| format!("Unable to load File Space search index state: {error}"))?
    };

    let mut updates = Vec::new();
    for candidate in candidates {
        let content_changed = candidate.indexed_file_name.as_deref()
            != Some(candidate.file_name.as_str())
            || candidate.indexed_file_updated_at != Some(candidate.file_updated_at)
            || candidate.indexed_size_bytes != Some(candidate.size_bytes);
        let metadata_changed = candidate.indexed_tag_text.as_deref()
            != Some(candidate.tag_text.as_str())
            || candidate.indexed_task_text.as_deref() != Some(candidate.task_text.as_str())
            || candidate.indexed_cell_text.as_deref() != Some(candidate.cell_text.as_str());
        if !content_changed && !metadata_changed {
            continue;
        }
        let body_text = if content_changed {
            read_searchable_text(root, &candidate.relative_path, &candidate.file_name)
        } else {
            candidate.indexed_body_text.clone().unwrap_or_default()
        };
        updates.push((candidate, body_text));
    }

    let mut connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Unable to begin File Space search indexing: {error}"))?;
    transaction
        .execute(
            "DELETE FROM file_space_search_documents
             WHERE file_id NOT IN (
               SELECT id FROM files WHERE trashed_at IS NULL AND storage_path IS NOT NULL
             )",
            [],
        )
        .map_err(|error| format!("Unable to remove stale File Space search records: {error}"))?;
    for (candidate, body_text) in updates {
        transaction
            .execute(
                "INSERT INTO file_space_search_documents
                 (file_id, file_name, body_text, tag_text, task_text, cell_text,
                  file_updated_at, size_bytes, indexed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(file_id) DO UPDATE SET
                   file_name = excluded.file_name,
                   body_text = excluded.body_text,
                   tag_text = excluded.tag_text,
                   task_text = excluded.task_text,
                   cell_text = excluded.cell_text,
                   file_updated_at = excluded.file_updated_at,
                   size_bytes = excluded.size_bytes,
                   indexed_at = excluded.indexed_at",
                params![
                    candidate.file_id,
                    candidate.file_name,
                    body_text,
                    candidate.tag_text,
                    candidate.task_text,
                    candidate.cell_text,
                    candidate.file_updated_at,
                    candidate.size_bytes,
                    now_millis(),
                ],
            )
            .map_err(|error| format!("Unable to update File Space search index: {error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("Unable to complete File Space search indexing: {error}"))
}

fn parse_file_tags(value: String) -> Vec<String> {
    if value.is_empty() {
        Vec::new()
    } else {
        value.split('\u{1f}').map(str::to_owned).collect()
    }
}

fn load_snapshot_record(database: &Database) -> Result<FileSpaceSnapshot, String> {
    recover_pending_file_move(database)?;
    let root = read_storage_root(database)?;
    let status = inspect_root(root.as_deref());
    synchronize_file_search_index(database, root.as_deref())?;
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
    let mut folder_statement = connection
        .prepare(
            "SELECT id, parent_id, name, relative_path, manual_order, created_at, updated_at
             FROM file_space_folders
             WHERE trashed_at IS NULL
             ORDER BY parent_id, manual_order, name COLLATE NOCASE, id",
        )
        .map_err(|error| format!("Unable to prepare File Space folders: {error}"))?;
    let folders = folder_statement
        .query_map([], |row| {
            Ok(FileSpaceFolder {
                id: row.get(0)?,
                parent_id: row.get(1)?,
                name: row.get(2)?,
                relative_path: row.get(3)?,
                manual_order: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .map_err(|error| format!("Unable to load File Space folders: {error}"))?;
    let mut file_statement = connection
        .prepare(
            "SELECT f.id, f.folder_id, f.original_name, f.storage_path, f.mime_type,
                    COALESCE(f.size_bytes, 0), f.source_kind, f.manual_order,
                    current_version.version_number,
                    COALESCE(version_counts.version_count, 0),
                    COALESCE((
                      SELECT group_concat(tag, char(31))
                      FROM file_space_file_tags tags WHERE tags.file_id = f.id
                    ), ''),
                    f.created_at, f.updated_at
             FROM files f
             LEFT JOIN file_space_artifacts artifact ON artifact.file_id = f.id
             LEFT JOIN file_space_artifact_versions current_version
               ON current_version.id = artifact.current_version_id
             LEFT JOIN (
               SELECT file_id, COUNT(*) AS version_count
               FROM file_space_artifact_versions GROUP BY file_id
             ) version_counts ON version_counts.file_id = f.id
             WHERE f.trashed_at IS NULL AND f.storage_path IS NOT NULL
             ORDER BY f.original_name COLLATE NOCASE, f.id",
        )
        .map_err(|error| format!("Unable to prepare File Space files: {error}"))?;
    let files = file_statement
        .query_map([], |row| {
            Ok(FileSpaceFile {
                id: row.get(0)?,
                folder_id: row.get(1)?,
                name: row.get(2)?,
                relative_path: row.get(3)?,
                mime_type: row.get(4)?,
                size_bytes: row.get(5)?,
                source_kind: row.get(6)?,
                manual_order: row.get(7)?,
                current_version: row.get(8)?,
                version_count: row.get(9)?,
                tags: parse_file_tags(row.get(10)?),
                created_at: row.get(11)?,
                updated_at: row.get(12)?,
            })
        })
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .map_err(|error| format!("Unable to load File Space files: {error}"))?;
    let mut trashed_statement = connection
        .prepare(
            "SELECT f.id, f.folder_id, f.original_name, COALESCE(f.storage_path, ''), f.mime_type,
                    COALESCE(f.size_bytes, 0), f.source_kind, f.manual_order,
                    current_version.version_number,
                    COALESCE(version_counts.version_count, 0),
                    COALESCE((
                      SELECT group_concat(tag, char(31))
                      FROM file_space_file_tags tags WHERE tags.file_id = f.id
                    ), ''),
                    f.created_at, f.updated_at
             FROM files f
             JOIN file_space_artifacts artifact ON artifact.file_id = f.id
             LEFT JOIN file_space_artifact_versions current_version
               ON current_version.id = artifact.current_version_id
             LEFT JOIN (
               SELECT file_id, COUNT(*) AS version_count
               FROM file_space_artifact_versions GROUP BY file_id
             ) version_counts ON version_counts.file_id = f.id
             WHERE f.trashed_at IS NOT NULL
             ORDER BY f.trashed_at DESC, f.id",
        )
        .map_err(|error| format!("Unable to prepare deleted Task files: {error}"))?;
    let trashed_files = trashed_statement
        .query_map([], |row| {
            Ok(FileSpaceFile {
                id: row.get(0)?,
                folder_id: row.get(1)?,
                name: row.get(2)?,
                relative_path: row.get(3)?,
                mime_type: row.get(4)?,
                size_bytes: row.get(5)?,
                source_kind: row.get(6)?,
                manual_order: row.get(7)?,
                current_version: row.get(8)?,
                version_count: row.get(9)?,
                tags: parse_file_tags(row.get(10)?),
                created_at: row.get(11)?,
                updated_at: row.get(12)?,
            })
        })
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .map_err(|error| format!("Unable to load deleted Task files: {error}"))?;
    Ok(FileSpaceSnapshot {
        root_path: root
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        root_name: root.as_deref().map(root_name),
        root_status: status,
        folders,
        files,
        trashed_files,
    })
}

fn configure_storage_root_record(
    database: &Database,
    path: &str,
) -> Result<FileSpaceSnapshot, String> {
    let requested = PathBuf::from(path);
    if !requested.exists() {
        return Err("The selected folder does not exist".to_owned());
    }
    if !requested.is_dir() {
        return Err("The selected path is not a folder".to_owned());
    }
    let root = requested
        .canonicalize()
        .map_err(|error| format!("Unable to resolve the selected folder: {error}"))?;
    write_probe(&root)?;
    let now = now_millis();
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
    let managed_paths = {
        let mut statement = connection
            .prepare(
                "SELECT relative_path, 1 AS is_folder
                 FROM file_space_folders WHERE trashed_at IS NULL
                 UNION ALL
                 SELECT storage_path, 0 AS is_folder
                 FROM files WHERE trashed_at IS NULL AND storage_path IS NOT NULL",
            )
            .map_err(|error| format!("Unable to inspect existing File Space records: {error}"))?;
        statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
            })
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|error| format!("Unable to inspect existing File Space paths: {error}"))?
    };
    for (relative_path, is_folder) in managed_paths {
        let candidate = physical_path(&root, &relative_path)?;
        let matches = if is_folder {
            candidate.is_dir()
        } else {
            candidate.is_file()
        };
        if !matches {
            return Err(format!(
                "The selected folder does not contain the existing managed item: {relative_path}"
            ));
        }
    }
    connection
        .execute(
            "INSERT INTO app_settings (key, value, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![STORAGE_ROOT_SETTING, root.to_string_lossy(), now],
        )
        .map_err(|error| format!("Unable to save the File Space storage path: {error}"))?;
    drop(connection);
    load_snapshot_record(database)
}

fn create_folder_record(
    database: &Database,
    parent_id: Option<&str>,
    name: &str,
) -> Result<FileSpaceFolder, String> {
    let root = require_ready_root(database)?;
    let name = validate_name(name)?;
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
    let parent_path = match parent_id {
        Some(parent_id) => Some(
            connection
                .query_row(
                    "SELECT relative_path FROM file_space_folders
                     WHERE id = ?1 AND trashed_at IS NULL",
                    [parent_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| format!("Unable to load the parent folder: {error}"))?
                .ok_or_else(|| "The parent folder no longer exists".to_owned())?,
        ),
        None => None,
    };
    let relative_path = relative_child(parent_path.as_deref(), &name);
    let manual_order = connection
        .query_row(
            "SELECT COALESCE(MAX(manual_order), -1) + 1
             FROM file_space_folders
             WHERE trashed_at IS NULL AND parent_id IS ?1",
            params![parent_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("Unable to determine the new folder order: {error}"))?;
    let destination = physical_path(&root, &relative_path)?;
    if destination.exists() {
        return Err("A file or folder with this name already exists".to_owned());
    }
    fs::create_dir(&destination)
        .map_err(|error| format!("Unable to create the folder on disk: {error}"))?;
    let folder = FileSpaceFolder {
        id: Uuid::new_v4().to_string(),
        parent_id: parent_id.map(str::to_owned),
        name,
        relative_path,
        manual_order,
        created_at: now_millis(),
        updated_at: now_millis(),
    };
    if let Err(error) = connection.execute(
        "INSERT INTO file_space_folders
         (id, parent_id, name, relative_path, manual_order, created_at, updated_at, trashed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
        params![
            folder.id,
            folder.parent_id,
            folder.name,
            folder.relative_path,
            folder.manual_order,
            folder.created_at,
            folder.updated_at
        ],
    ) {
        let _ = fs::remove_dir(&destination);
        return Err(format!("Unable to save the new folder: {error}"));
    }
    Ok(folder)
}

fn rename_folder_record(
    database: &Database,
    artifact_store: &Path,
    folder_id: &str,
    name: &str,
) -> Result<FileSpaceSnapshot, String> {
    let root = require_ready_root(database)?;
    let name = validate_name(name)?;
    let (current_name, current_path, parent_id, task_file_ids) = {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
        let (current_name, current_path, parent_id) = connection
            .query_row(
                "SELECT name, relative_path, parent_id FROM file_space_folders
                 WHERE id = ?1 AND trashed_at IS NULL",
                [folder_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| format!("Unable to load the folder: {error}"))?
            .ok_or_else(|| "The folder no longer exists".to_owned())?;
        let current_prefix = format!("{current_path}/");
        let mut statement = connection
            .prepare(
                "SELECT f.id FROM files f
                 JOIN file_space_artifacts a ON a.file_id = f.id
                 WHERE f.trashed_at IS NULL
                   AND substr(f.storage_path, 1, length(?1)) = ?1",
            )
            .map_err(|error| format!("Unable to inspect Task files in folder: {error}"))?;
        let task_file_ids = statement
            .query_map([&current_prefix], |row| row.get::<_, String>(0))
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|error| format!("Unable to inspect Task files in folder: {error}"))?;
        (current_name, current_path, parent_id, task_file_ids)
    };
    if current_name == name {
        return load_snapshot_record(database);
    }
    for file_id in &task_file_ids {
        reconcile_task_file(database, artifact_store, file_id)?;
    }
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
    let parent_path = match parent_id.as_deref() {
        Some(parent_id) => Some(
            connection
                .query_row(
                    "SELECT relative_path FROM file_space_folders
                     WHERE id = ?1 AND trashed_at IS NULL",
                    [parent_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| format!("Unable to load the parent folder: {error}"))?
                .ok_or_else(|| "The parent folder no longer exists".to_owned())?,
        ),
        None => None,
    };
    let renamed_path = relative_child(parent_path.as_deref(), &name);
    let source = physical_path(&root, &current_path)?;
    let destination = physical_path(&root, &renamed_path)?;
    if destination.exists() {
        return Err("A file or folder with this name already exists".to_owned());
    }
    fs::rename(&source, &destination)
        .map_err(|error| format!("Unable to rename the folder on disk: {error}"))?;

    let transaction = match connection.unchecked_transaction() {
        Ok(transaction) => transaction,
        Err(error) => {
            let _ = fs::rename(&destination, &source);
            return Err(format!("Unable to begin the folder rename: {error}"));
        }
    };
    let current_prefix = format!("{current_path}/");
    let renamed_prefix = format!("{renamed_path}/");
    let now = now_millis();
    let update_result = (|| -> rusqlite::Result<()> {
        transaction.execute(
            "UPDATE file_space_folders
             SET name = ?1, updated_at = ?2
             WHERE id = ?3",
            params![name, now, folder_id],
        )?;
        transaction.execute(
            "UPDATE file_space_folders
             SET relative_path = CASE
                   WHEN relative_path = ?1 THEN ?2
                   ELSE ?3 || substr(relative_path, length(?4) + 1)
                 END,
                 updated_at = ?5
             WHERE relative_path = ?1
                OR substr(relative_path, 1, length(?4)) = ?4",
            params![
                current_path,
                renamed_path,
                renamed_prefix,
                current_prefix,
                now
            ],
        )?;
        transaction.execute(
            "UPDATE files
             SET storage_path = ?1 || substr(storage_path, length(?2) + 1),
                 updated_at = ?3
             WHERE trashed_at IS NULL
               AND substr(storage_path, 1, length(?2)) = ?2",
            params![renamed_prefix, current_prefix, now],
        )?;
        for file_id in &task_file_ids {
            transaction.execute(
                "INSERT INTO file_space_artifact_events
                 (id, file_id, event_type, actor, details_json, created_at)
                 VALUES (?1, ?2, 'renamed', 'user', ?3, ?4)",
                params![
                    Uuid::new_v4().to_string(),
                    file_id,
                    json!({ "folderFrom": current_path, "folderTo": renamed_path }).to_string(),
                    now
                ],
            )?;
        }
        Ok(())
    })();
    if let Err(error) = update_result {
        drop(transaction);
        let _ = fs::rename(&destination, &source);
        return Err(format!("Unable to save the folder rename: {error}"));
    }
    if let Err(error) = transaction.commit() {
        let _ = fs::rename(&destination, &source);
        return Err(format!("Unable to complete the folder rename: {error}"));
    }
    drop(connection);
    load_snapshot_record(database)
}

fn move_folder_record(
    database: &Database,
    artifact_store: &Path,
    folder_id: &str,
    parent_id: Option<&str>,
    ordered_sibling_ids: &[String],
) -> Result<FileSpaceSnapshot, String> {
    if ordered_sibling_ids.is_empty() {
        return Err("No folder order was supplied".to_owned());
    }
    let requested_ids = ordered_sibling_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    if requested_ids.len() != ordered_sibling_ids.len() {
        return Err("The folder order contains duplicates".to_owned());
    }

    let root = require_ready_root(database)?;
    let (name, current_path, current_parent_id, parent_path, task_file_ids) = {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
        let (name, current_path, current_parent_id) = connection
            .query_row(
                "SELECT name, relative_path, parent_id
                 FROM file_space_folders
                 WHERE id = ?1 AND trashed_at IS NULL",
                [folder_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| format!("Unable to load the folder: {error}"))?
            .ok_or_else(|| "The folder no longer exists".to_owned())?;
        let parent_path = match parent_id {
            Some(parent_id) => Some(
                connection
                    .query_row(
                        "SELECT relative_path FROM file_space_folders
                         WHERE id = ?1 AND trashed_at IS NULL",
                        [parent_id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(|error| format!("Unable to load the destination folder: {error}"))?
                    .ok_or_else(|| "The destination folder no longer exists".to_owned())?,
            ),
            None => None,
        };
        if parent_id == Some(folder_id)
            || parent_path.as_ref().is_some_and(|path| {
                path == &current_path || path.starts_with(&format!("{current_path}/"))
            })
        {
            return Err(
                "A folder cannot be moved into itself or one of its descendants".to_owned(),
            );
        }

        let target_sibling_ids = connection
            .prepare(
                "SELECT id FROM file_space_folders
                 WHERE trashed_at IS NULL AND parent_id IS ?1 AND id <> ?2
                 ORDER BY manual_order, name COLLATE NOCASE, id",
            )
            .map_err(|error| format!("Unable to inspect destination folders: {error}"))?
            .query_map(params![parent_id, folder_id], |row| row.get::<_, String>(0))
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|error| format!("Unable to inspect destination folders: {error}"))?;
        let mut expected_ids = target_sibling_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        expected_ids.insert(folder_id);
        if expected_ids != requested_ids {
            return Err("The destination folder order is incomplete or stale".to_owned());
        }

        let task_file_ids = if current_parent_id.as_deref() == parent_id {
            Vec::new()
        } else {
            let current_prefix = format!("{current_path}/");
            connection
                .prepare(
                    "SELECT f.id FROM files f
                     JOIN file_space_artifacts a ON a.file_id = f.id
                     WHERE f.trashed_at IS NULL
                       AND substr(f.storage_path, 1, length(?1)) = ?1",
                )
                .map_err(|error| format!("Unable to inspect Task files in folder: {error}"))?
                .query_map([&current_prefix], |row| row.get::<_, String>(0))
                .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
                .map_err(|error| format!("Unable to inspect Task files in folder: {error}"))?
        };
        (
            name,
            current_path,
            current_parent_id,
            parent_path,
            task_file_ids,
        )
    };

    if current_parent_id.as_deref() == parent_id {
        let mut connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
        let transaction = connection
            .transaction()
            .map_err(|error| format!("Unable to begin folder reordering: {error}"))?;
        for (position, sibling_id) in ordered_sibling_ids.iter().enumerate() {
            transaction
                .execute(
                    "UPDATE file_space_folders SET manual_order = ?1 WHERE id = ?2",
                    params![position as i64, sibling_id],
                )
                .map_err(|error| format!("Unable to save the folder order: {error}"))?;
        }
        transaction
            .commit()
            .map_err(|error| format!("Unable to complete folder reordering: {error}"))?;
        drop(connection);
        return load_snapshot_record(database);
    }

    let moved_path = relative_child(parent_path.as_deref(), &name);
    let source = physical_path(&root, &current_path)?;
    let destination = physical_path(&root, &moved_path)?;
    if destination.exists() {
        return Err("A file or folder with this name already exists in the destination".to_owned());
    }
    for file_id in &task_file_ids {
        reconcile_task_file(database, artifact_store, file_id)?;
    }
    if destination.exists() {
        return Err("A file or folder with this name already exists in the destination".to_owned());
    }
    fs::rename(&source, &destination)
        .map_err(|error| format!("Unable to move the folder on disk: {error}"))?;

    let mut connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
    let transaction = match connection.transaction() {
        Ok(transaction) => transaction,
        Err(error) => {
            let _ = fs::rename(&destination, &source);
            return Err(format!("Unable to begin the folder move: {error}"));
        }
    };
    let current_prefix = format!("{current_path}/");
    let moved_prefix = format!("{moved_path}/");
    let now = now_millis();
    let update_result = (|| -> rusqlite::Result<()> {
        transaction.execute(
            "UPDATE file_space_folders
             SET parent_id = ?1, updated_at = ?2
             WHERE id = ?3",
            params![parent_id, now, folder_id],
        )?;
        transaction.execute(
            "UPDATE file_space_folders
             SET relative_path = CASE
                   WHEN relative_path = ?1 THEN ?2
                   ELSE ?3 || substr(relative_path, length(?4) + 1)
                 END,
                 updated_at = ?5
             WHERE relative_path = ?1
                OR substr(relative_path, 1, length(?4)) = ?4",
            params![current_path, moved_path, moved_prefix, current_prefix, now],
        )?;
        transaction.execute(
            "UPDATE files
             SET storage_path = ?1 || substr(storage_path, length(?2) + 1),
                 updated_at = ?3
             WHERE trashed_at IS NULL
               AND substr(storage_path, 1, length(?2)) = ?2",
            params![moved_prefix, current_prefix, now],
        )?;
        for (position, sibling_id) in ordered_sibling_ids.iter().enumerate() {
            transaction.execute(
                "UPDATE file_space_folders SET manual_order = ?1 WHERE id = ?2",
                params![position as i64, sibling_id],
            )?;
        }
        for file_id in &task_file_ids {
            transaction.execute(
                "INSERT INTO file_space_artifact_events
                 (id, file_id, event_type, actor, details_json, created_at)
                 VALUES (?1, ?2, 'renamed', 'user', ?3, ?4)",
                params![
                    Uuid::new_v4().to_string(),
                    file_id,
                    json!({ "folderFrom": current_path, "folderTo": moved_path }).to_string(),
                    now
                ],
            )?;
        }
        Ok(())
    })();
    if let Err(error) = update_result {
        drop(transaction);
        let _ = fs::rename(&destination, &source);
        return Err(format!("Unable to save the folder move: {error}"));
    }
    if let Err(error) = transaction.commit() {
        let _ = fs::rename(&destination, &source);
        return Err(format!("Unable to complete the folder move: {error}"));
    }
    drop(connection);
    load_snapshot_record(database)
}

#[derive(Debug)]
struct FolderTaskFileObservation {
    file_id: String,
    relative_path: String,
    expected_sha256: String,
}

fn verify_pending_folder_task_files(
    pending_folder: &Path,
    folder_relative_path: &str,
    observations: &[FolderTaskFileObservation],
) -> Result<(), String> {
    let folder_path = Path::new(folder_relative_path);
    for observation in observations {
        let suffix = Path::new(&observation.relative_path)
            .strip_prefix(folder_path)
            .map_err(|_| "A Task file is outside the folder being deleted".to_owned())?;
        let suffix = suffix
            .to_str()
            .ok_or_else(|| "A Task file path is not valid UTF-8".to_owned())?;
        let moved_path = physical_path(pending_folder, suffix)?;
        let metadata = fs::symlink_metadata(&moved_path).map_err(|error| {
            format!("Unable to inspect Task file during folder deletion: {error}")
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("A Task file changed while its folder was being deleted".to_owned());
        }
        let (sha256, _) = sha256_file(&moved_path)?;
        if sha256 != observation.expected_sha256 {
            return Err("A Task file changed while its folder was being deleted".to_owned());
        }
    }
    Ok(())
}

fn delete_folder_record(
    database: &Database,
    artifact_store: &Path,
    folder_id: &str,
) -> Result<FileSpaceSnapshot, String> {
    let root = require_ready_root(database)?;
    let (relative_path, task_file_ids) = {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
        let relative_path = connection
            .query_row(
                "SELECT relative_path FROM file_space_folders
                 WHERE id = ?1 AND trashed_at IS NULL",
                [folder_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("Unable to load the folder: {error}"))?
            .ok_or_else(|| "The folder no longer exists".to_owned())?;
        let source_prefix = format!("{relative_path}/");
        let mut statement = connection
            .prepare(
                "SELECT f.id FROM files f
                 JOIN file_space_artifacts a ON a.file_id = f.id
                 WHERE f.trashed_at IS NULL
                   AND substr(f.storage_path, 1, length(?1)) = ?1",
            )
            .map_err(|error| format!("Unable to inspect Task files in folder: {error}"))?;
        let task_file_ids = statement
            .query_map([&source_prefix], |row| row.get::<_, String>(0))
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|error| format!("Unable to inspect Task files in folder: {error}"))?;
        (relative_path, task_file_ids)
    };
    for file_id in &task_file_ids {
        reconcile_task_file(database, artifact_store, file_id)?;
    }
    let task_file_observations = {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
        let source_prefix = format!("{relative_path}/");
        let mut statement = connection
            .prepare(
                "SELECT f.id, f.storage_path, a.observed_sha256
                 FROM files f
                 JOIN file_space_artifacts a ON a.file_id = f.id
                 WHERE f.trashed_at IS NULL
                   AND substr(f.storage_path, 1, length(?1)) = ?1
                 ORDER BY f.storage_path, f.id",
            )
            .map_err(|error| format!("Unable to inspect Task files in folder: {error}"))?;
        statement
            .query_map([&source_prefix], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|error| format!("Unable to inspect Task files in folder: {error}"))?
            .into_iter()
            .map(|(file_id, relative_path, expected_sha256)| {
                Ok(FolderTaskFileObservation {
                    file_id,
                    relative_path,
                    expected_sha256: expected_sha256
                        .ok_or_else(|| "A Task file observation is missing".to_owned())?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?
    };
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
    let source = physical_path(&root, &relative_path)?;
    let metadata = fs::symlink_metadata(&source)
        .map_err(|error| format!("Unable to inspect the folder on disk: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("The managed folder must be a regular folder".to_owned());
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("Unable to resolve the File Space storage path: {error}"))?;
    let canonical_source = source
        .canonicalize()
        .map_err(|error| format!("Unable to resolve the folder on disk: {error}"))?;
    if !canonical_source.starts_with(&canonical_root) || canonical_source == canonical_root {
        return Err("The managed folder is outside the File Space storage path".to_owned());
    }

    let parent = source
        .parent()
        .ok_or_else(|| "The folder has no parent directory".to_owned())?;
    let pending_delete = parent.join(format!(".lumetrace-delete-{}", Uuid::new_v4()));
    fs::rename(&source, &pending_delete)
        .map_err(|error| format!("Unable to prepare the folder deletion: {error}"))?;
    if let Err(error) =
        verify_pending_folder_task_files(&pending_delete, &relative_path, &task_file_observations)
    {
        let restore_result = fs::rename(&pending_delete, &source);
        return match restore_result {
            Ok(()) => Err(format!("{error}; retry the operation")),
            Err(restore_error) => Err(format!(
                "{error}. Unable to restore the folder: {restore_error}"
            )),
        };
    }

    let transaction = match connection.unchecked_transaction() {
        Ok(transaction) => transaction,
        Err(error) => {
            let _ = fs::rename(&pending_delete, &source);
            return Err(format!("Unable to begin the folder deletion: {error}"));
        }
    };
    let source_prefix = format!("{relative_path}/");
    let delete_result = (|| -> rusqlite::Result<()> {
        transaction.execute(
            "DELETE FROM files
             WHERE trashed_at IS NULL
               AND substr(storage_path, 1, length(?1)) = ?1",
            [&source_prefix],
        )?;
        let mut descendant_statement = transaction.prepare(
            "SELECT id FROM file_space_folders
             WHERE relative_path = ?1
                OR substr(relative_path, 1, length(?2)) = ?2
             ORDER BY length(relative_path) DESC, id",
        )?;
        let descendant_ids = descendant_statement
            .query_map(params![relative_path, source_prefix], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(descendant_statement);
        for descendant_id in descendant_ids {
            transaction.execute(
                "DELETE FROM file_space_folders WHERE id = ?1",
                [descendant_id],
            )?;
        }
        Ok(())
    })();
    if let Err(error) = delete_result {
        drop(transaction);
        let _ = fs::rename(&pending_delete, &source);
        return Err(format!("Unable to save the folder deletion: {error}"));
    }
    if let Err(error) = transaction.commit() {
        let restore_result = fs::rename(&pending_delete, &source);
        return match restore_result {
            Ok(()) => Err(format!("Unable to complete the folder deletion: {error}")),
            Err(restore_error) => Err(format!(
                "Unable to complete the folder deletion: {error}. Unable to restore the folder: {restore_error}"
            )),
        };
    }
    drop(connection);
    let _ = fs::remove_dir_all(&pending_delete);
    for observation in task_file_observations {
        if let Ok(directory) = task_artifact_directory(artifact_store, &observation.file_id) {
            let _ = fs::remove_dir_all(directory);
        }
    }
    load_snapshot_record(database)
}

fn active_file_record(
    database: &Database,
    file_id: &str,
) -> Result<(String, String, Option<String>), String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
    connection
        .query_row(
            "SELECT original_name, storage_path, folder_id
             FROM files
             WHERE id = ?1 AND trashed_at IS NULL AND storage_path IS NOT NULL",
            [file_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("Unable to load the file: {error}"))?
        .ok_or_else(|| "The file no longer exists".to_owned())
}

fn active_file_path(database: &Database, file_id: &str) -> Result<PathBuf, String> {
    let root = require_ready_root(database)?;
    let (_, relative_path, _) = active_file_record(database, file_id)?;
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("Unable to resolve the File Space storage path: {error}"))?;
    let candidate = physical_path(&root, &relative_path)?;
    let metadata = fs::symlink_metadata(&candidate)
        .map_err(|error| format!("Unable to inspect the file on disk: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("The managed file must be a regular file".to_owned());
    }
    let canonical_file = candidate
        .canonicalize()
        .map_err(|error| format!("Unable to resolve the file on disk: {error}"))?;
    if !canonical_file.starts_with(&canonical_root) || canonical_file == canonical_root {
        return Err("The managed file is outside the File Space storage path".to_owned());
    }
    Ok(canonical_file)
}

const FILE_DRAG_PREVIEW_MAX_BYTES: usize = 512 * 1024;

fn valid_file_drag_preview(bytes: &[u8]) -> bool {
    bytes.len() >= 8
        && bytes.len() <= FILE_DRAG_PREVIEW_MAX_BYTES
        && bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a])
}

#[cfg(target_os = "macos")]
fn create_macos_file_drag_preview(path: &Path) -> Option<PathBuf> {
    let preview_path = std::env::temp_dir().join(format!(
        "lumetrace-file-drag-preview-{}.png",
        Uuid::new_v4()
    ));
    let output = ProcessCommand::new("/usr/bin/sips")
        .args(["-s", "format", "png", "-Z", "160"])
        .arg(path)
        .arg("--out")
        .arg(&preview_path)
        .output()
        .ok()?;
    let metadata = fs::metadata(&preview_path).ok();
    if !output.status.success()
        || metadata.as_ref().is_none_or(|value| {
            value.len() == 0 || value.len() > FILE_DRAG_PREVIEW_MAX_BYTES as u64
        })
    {
        let _ = fs::remove_file(&preview_path);
        return None;
    }
    Some(preview_path)
}

fn file_drag_image(path: &Path, preview_bytes: Option<Vec<u8>>) -> (drag::Image, Option<PathBuf>) {
    if let Some(bytes) = preview_bytes.filter(|bytes| valid_file_drag_preview(bytes)) {
        return (drag::Image::Raw(bytes), None);
    }
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if preview_mime_type(file_name, mime_type_for(path).as_deref()).is_some() {
        #[cfg(target_os = "macos")]
        if let Some(preview_path) = create_macos_file_drag_preview(path) {
            return (drag::Image::File(preview_path.clone()), Some(preview_path));
        }
    }
    (
        drag::Image::Raw(include_bytes!("../icons/64x64.png").to_vec()),
        None,
    )
}

fn cleanup_file_drag_preview(path: Option<&PathBuf>) {
    if let Some(path) = path {
        let _ = fs::remove_file(path);
    }
}

fn markdown_file_path(database: &Database, file_id: &str) -> Result<PathBuf, String> {
    let (file_name, _, _) = active_file_record(database, file_id)?;
    let extension = Path::new(&file_name)
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);
    if !matches!(extension.as_deref(), Some("md" | "markdown")) {
        return Err("Only Markdown files can be opened in the Markdown editor".to_owned());
    }
    active_file_path(database, file_id)
}

fn text_file_path(database: &Database, file_id: &str) -> Result<PathBuf, String> {
    let (file_name, _, _) = active_file_record(database, file_id)?;
    let extension = Path::new(&file_name)
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);
    if extension.as_deref() != Some("txt") {
        return Err("Only TXT files can be opened in the text viewer".to_owned());
    }
    active_file_path(database, file_id)
}

fn read_text_file_record(database: &Database, file_id: &str) -> Result<String, String> {
    let path = text_file_path(database, file_id)?;
    let metadata =
        fs::metadata(&path).map_err(|error| format!("Unable to inspect the TXT file: {error}"))?;
    if metadata.len() > MARKDOWN_FILE_MAX_BYTES {
        return Err("The TXT file is larger than the 5 MB preview limit".to_owned());
    }
    let bytes = fs::read(&path).map_err(|error| format!("Unable to read the TXT file: {error}"))?;
    String::from_utf8(bytes).map_err(|_| "The TXT file is not valid UTF-8 text".to_owned())
}

fn read_markdown_file_record(database: &Database, file_id: &str) -> Result<String, String> {
    let path = markdown_file_path(database, file_id)?;
    let metadata = fs::metadata(&path)
        .map_err(|error| format!("Unable to inspect the Markdown file: {error}"))?;
    if metadata.len() > MARKDOWN_FILE_MAX_BYTES {
        return Err("The Markdown file is larger than the 5 MB editor limit".to_owned());
    }
    let bytes =
        fs::read(&path).map_err(|error| format!("Unable to read the Markdown file: {error}"))?;
    String::from_utf8(bytes).map_err(|_| "The Markdown file is not valid UTF-8 text".to_owned())
}

fn replace_markdown_contents(
    path: &Path,
    content: &[u8],
    expected_current_sha256: &str,
) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "The Markdown file has no parent folder".to_owned())?;
    let temporary = parent.join(format!(".lumetrace-markdown-{}", Uuid::new_v4()));
    let backup = parent.join(format!(".lumetrace-markdown-backup-{}", Uuid::new_v4()));
    let permissions = fs::metadata(path)
        .map_err(|error| format!("Unable to inspect the Markdown file: {error}"))?
        .permissions();
    let write_result = (|| -> std::io::Result<()> {
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        output.write_all(content)?;
        output.sync_all()?;
        fs::set_permissions(&temporary, permissions)?;
        Ok(())
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary);
        return Err(format!(
            "Unable to prepare the Markdown file update: {error}"
        ));
    }
    if let Err(error) = fs::rename(path, &backup) {
        let _ = fs::remove_file(&temporary);
        return Err(format!(
            "Unable to prepare the Markdown file replacement: {error}"
        ));
    }
    let backup_sha256 = match sha256_file(&backup) {
        Ok((sha256, _)) => sha256,
        Err(error) => {
            restore_backup_without_overwrite(&backup, path);
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
    };
    if backup_sha256 != expected_current_sha256 {
        restore_backup_without_overwrite(&backup, path);
        let _ = fs::remove_file(&temporary);
        return Err(
            "The Markdown file changed while it was being saved; retry the operation".to_owned(),
        );
    }
    if let Err(error) = fs::hard_link(&temporary, path) {
        restore_backup_without_overwrite(&backup, path);
        let _ = fs::remove_file(&temporary);
        return Err(format!("Unable to replace the Markdown file: {error}"));
    }
    let installed_sha256 = format!("{:x}", Sha256::digest(content));
    let _ = fs::remove_file(&temporary);
    if !sha256_file(path).is_ok_and(|(sha256, _)| sha256 == installed_sha256) {
        return Err(
            "The Markdown file changed while it was being saved; retry the operation".to_owned(),
        );
    }
    if sha256_file(&backup).is_ok_and(|(sha256, _)| sha256 == expected_current_sha256) {
        let _ = fs::remove_file(&backup);
    }
    Ok(())
}

fn invalidate_task_file_observation(database: &Database, file_id: &str) -> Result<(), String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
    connection
        .execute(
            "UPDATE file_space_artifacts SET observed_mtime_ms = NULL WHERE file_id = ?1",
            [file_id],
        )
        .map_err(|error| format!("Unable to prepare Task file version capture: {error}"))?;
    Ok(())
}

fn save_markdown_file_record(
    database: &Database,
    artifact_store: &Path,
    file_id: &str,
    content: &str,
) -> Result<FileSpaceSnapshot, String> {
    if content.len() as u64 > MARKDOWN_FILE_MAX_BYTES {
        return Err("The Markdown content is larger than the 5 MB editor limit".to_owned());
    }
    let is_task_artifact = task_artifact_exists(database, file_id)?;
    if is_task_artifact {
        reconcile_task_file(database, artifact_store, file_id)?;
    }
    let path = markdown_file_path(database, file_id)?;
    let original_content = read_markdown_file_record(database, file_id)?;
    let original_sha256 = format!("{:x}", Sha256::digest(original_content.as_bytes()));
    let updated_sha256 = format!("{:x}", Sha256::digest(content.as_bytes()));
    if is_task_artifact && content != original_content {
        invalidate_task_file_observation(database, file_id)?;
    }
    if let Err(error) = replace_markdown_contents(&path, content.as_bytes(), &original_sha256) {
        if is_task_artifact {
            let _ = reconcile_task_file(database, artifact_store, file_id);
        }
        return Err(error);
    }
    if is_task_artifact && content != original_content {
        if let Err(error) = reconcile_task_file(database, artifact_store, file_id) {
            let restore_result =
                replace_markdown_contents(&path, original_content.as_bytes(), &updated_sha256);
            let _ = reconcile_task_file(database, artifact_store, file_id);
            return match restore_result {
                Ok(()) => Err(error),
                Err(restore_error) => Err(format!(
                    "{error}. Unable to restore the original Markdown file: {restore_error}"
                )),
            };
        }
    } else if !is_task_artifact {
        let size_bytes = i64::try_from(content.len())
            .map_err(|_| "The Markdown content is too large".to_owned())?;
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
        let update_result = connection.execute(
            "UPDATE files SET size_bytes = ?1, updated_at = ?2
                 WHERE id = ?3 AND trashed_at IS NULL",
            params![size_bytes, now_millis(), file_id],
        );
        match update_result {
            Ok(0) => {
                drop(connection);
                let restore_result =
                    replace_markdown_contents(&path, original_content.as_bytes(), &updated_sha256);
                return match restore_result {
                    Ok(()) => Err("The Markdown file record no longer exists".to_owned()),
                    Err(restore_error) => Err(format!(
                        "The Markdown file record no longer exists. Unable to restore the original Markdown file: {restore_error}"
                    )),
                };
            }
            Ok(_) => {}
            Err(error) => {
                drop(connection);
                let restore_result =
                    replace_markdown_contents(&path, original_content.as_bytes(), &updated_sha256);
                return match restore_result {
                    Ok(()) => Err(format!("Unable to update the Markdown file record: {error}")),
                    Err(restore_error) => Err(format!(
                        "Unable to update the Markdown file record: {error}. Unable to restore the original Markdown file: {restore_error}"
                    )),
                };
            }
        }
    }
    load_snapshot_record(database)
}

fn preview_mime_type(file_name: &str, mime_type: Option<&str>) -> Option<&'static str> {
    let extension = Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);
    match (mime_type, extension.as_deref()) {
        (Some("image/png"), Some("png")) => Some("image/png"),
        (Some("image/jpeg"), Some("jpg" | "jpeg")) => Some("image/jpeg"),
        (Some("image/gif"), Some("gif")) => Some("image/gif"),
        (Some("image/webp"), Some("webp")) => Some("image/webp"),
        _ => None,
    }
}

fn load_file_preview_record(
    database: &Database,
    file_id: &str,
) -> Result<(Vec<u8>, &'static str), String> {
    if file_id.is_empty()
        || file_id.len() > 128
        || !file_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("The file preview identifier is invalid".to_owned());
    }
    let (file_name, _, _) = active_file_record(database, file_id)?;
    let path = active_file_path(database, file_id)?;
    let mime_type = preview_mime_type(&file_name, mime_type_for(&path).as_deref())
        .ok_or_else(|| "This file format does not support image preview".to_owned())?;
    let bytes =
        fs::read(&path).map_err(|error| format!("Unable to read the image preview: {error}"))?;
    Ok((bytes, mime_type))
}

pub fn file_preview_response(
    database: &Database,
    request_path: &str,
) -> tauri::http::Response<Vec<u8>> {
    let file_id = request_path.trim_start_matches('/');
    match load_file_preview_record(database, file_id) {
        Ok((bytes, mime_type)) => tauri::http::Response::builder()
            .status(tauri::http::StatusCode::OK)
            .header(tauri::http::header::CONTENT_TYPE, mime_type)
            .header(tauri::http::header::CACHE_CONTROL, "no-store")
            .header(tauri::http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header("X-Content-Type-Options", "nosniff")
            .body(bytes)
            .expect("valid file preview response"),
        Err(error) => tauri::http::Response::builder()
            .status(tauri::http::StatusCode::NOT_FOUND)
            .header(
                tauri::http::header::CONTENT_TYPE,
                "text/plain; charset=utf-8",
            )
            .header(tauri::http::header::CACHE_CONTROL, "no-store")
            .header("X-Content-Type-Options", "nosniff")
            .body(error.into_bytes())
            .expect("valid file preview error response"),
    }
}

fn rename_file_record(
    database: &Database,
    file_id: &str,
    name: &str,
) -> Result<FileSpaceSnapshot, String> {
    let root = require_ready_root(database)?;
    let name = validate_name(name)?;
    let (current_name, _, folder_id) = active_file_record(database, file_id)?;
    if current_name == name {
        return load_snapshot_record(database);
    }
    let folder_path = match folder_id.as_deref() {
        Some(folder_id) => {
            let connection = database
                .0
                .lock()
                .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
            Some(
                connection
                    .query_row(
                        "SELECT relative_path FROM file_space_folders
                         WHERE id = ?1 AND trashed_at IS NULL",
                        [folder_id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(|error| format!("Unable to load the file folder: {error}"))?
                    .ok_or_else(|| "The file folder no longer exists".to_owned())?,
            )
        }
        None => None,
    };
    let source = active_file_path(database, file_id)?;
    let renamed_path = relative_child(folder_path.as_deref(), &name);
    let destination = physical_path(&root, &renamed_path)?;
    if destination.exists() {
        return Err("A file or folder with this name already exists".to_owned());
    }
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
    fs::rename(&source, &destination)
        .map_err(|error| format!("Unable to rename the file on disk: {error}"))?;

    if let Err(error) = connection.execute(
        "UPDATE files
         SET original_name = ?1, storage_path = ?2, mime_type = ?3, updated_at = ?4
         WHERE id = ?5 AND trashed_at IS NULL",
        params![
            name,
            renamed_path,
            mime_type_for(&destination),
            now_millis(),
            file_id
        ],
    ) {
        let _ = fs::rename(&destination, &source);
        return Err(format!("Unable to save the file rename: {error}"));
    }
    drop(connection);
    load_snapshot_record(database)
}

fn rename_task_file_record(
    database: &Database,
    file_id: &str,
    name: &str,
) -> Result<FileSpaceSnapshot, String> {
    let root = require_ready_root(database)?;
    let name = validate_name(name)?;
    let (current_name, _, folder_id) = active_file_record(database, file_id)?;
    if current_name == name {
        return load_snapshot_record(database);
    }
    let folder_path = match folder_id.as_deref() {
        Some(folder_id) => {
            let connection = database
                .0
                .lock()
                .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
            Some(
                connection
                    .query_row(
                        "SELECT relative_path FROM file_space_folders
                         WHERE id = ?1 AND trashed_at IS NULL",
                        [folder_id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(|error| format!("Unable to load the file folder: {error}"))?
                    .ok_or_else(|| "The file folder no longer exists".to_owned())?,
            )
        }
        None => None,
    };
    let source = active_file_path(database, file_id)?;
    let renamed_path = relative_child(folder_path.as_deref(), &name);
    let destination = physical_path(&root, &renamed_path)?;
    if destination.exists() {
        return Err("A file or folder with this name already exists".to_owned());
    }
    fs::rename(&source, &destination)
        .map_err(|error| format!("Unable to rename the file on disk: {error}"))?;

    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
    let transaction = match connection.unchecked_transaction() {
        Ok(transaction) => transaction,
        Err(error) => {
            let _ = fs::rename(&destination, &source);
            return Err(format!("Unable to begin Task file rename: {error}"));
        }
    };
    let now = now_millis();
    let result = (|| -> rusqlite::Result<()> {
        transaction.execute(
            "UPDATE files
             SET original_name = ?1, storage_path = ?2, mime_type = ?3, updated_at = ?4
             WHERE id = ?5 AND trashed_at IS NULL",
            params![
                name,
                renamed_path,
                mime_type_for(&destination),
                now,
                file_id
            ],
        )?;
        transaction.execute(
            "INSERT INTO file_space_artifact_events
             (id, file_id, event_type, actor, details_json, created_at)
             VALUES (?1, ?2, 'renamed', 'user', ?3, ?4)",
            params![
                Uuid::new_v4().to_string(),
                file_id,
                json!({ "from": current_name, "to": name }).to_string(),
                now
            ],
        )?;
        Ok(())
    })();
    if let Err(error) = result {
        drop(transaction);
        let _ = fs::rename(&destination, &source);
        return Err(format!("Unable to save Task file rename: {error}"));
    }
    if let Err(error) = transaction.commit() {
        let restore_result = fs::rename(&destination, &source);
        return match restore_result {
            Ok(()) => Err(format!("Unable to complete Task file rename: {error}")),
            Err(restore_error) => Err(format!(
                "Unable to complete Task file rename: {error}. Unable to restore the file name: {restore_error}"
            )),
        };
    }
    drop(connection);
    load_snapshot_record(database)
}

fn no_replace_move_error(error: std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        "A file or folder with this name already exists in the destination".to_owned()
    } else {
        format!("Unable to move the file without overwriting content: {error}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ManagedFileIdentity {
    device: u64,
    inode: u64,
}

#[derive(Debug)]
enum AtomicFileMoveError {
    NotMoved(String),
    StateUnknown(String),
}

impl AtomicFileMoveError {
    fn into_message(self) -> String {
        match self {
            Self::NotMoved(message) | Self::StateUnknown(message) => message,
        }
    }
}

fn pending_file_move_json(pending: &PendingFileMove) -> Result<String, String> {
    serde_json::to_string(pending)
        .map_err(|error| format!("Unable to encode the pending file move: {error}"))
}

fn persist_pending_file_move(database: &Database, pending: &PendingFileMove) -> Result<(), String> {
    let value = pending_file_move_json(pending)?;
    let mut connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Unable to begin the pending file move journal: {error}"))?;
    transaction
        .execute(
            "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3)",
            params![PENDING_FILE_MOVE_SETTING, value, pending.created_at],
        )
        .map_err(|error| format!("Unable to persist the pending file move: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("Unable to commit the pending file move journal: {error}"))
}

fn clear_pending_file_move(database: &Database, pending: &PendingFileMove) -> Result<(), String> {
    let value = pending_file_move_json(pending)?;
    let mut connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Unable to begin pending file move cleanup: {error}"))?;
    let deleted = transaction
        .execute(
            "DELETE FROM app_settings WHERE key = ?1 AND value = ?2",
            params![PENDING_FILE_MOVE_SETTING, value],
        )
        .map_err(|error| format!("Unable to clear the pending file move: {error}"))?;
    if deleted != 1 {
        return Err("The pending file move journal changed before it could be cleared".to_owned());
    }
    transaction
        .commit()
        .map_err(|error| format!("Unable to complete pending file move cleanup: {error}"))
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "android"))]
fn open_file_space_root_directory(root: &Path) -> Result<fs::File, String> {
    use std::os::unix::fs::OpenOptionsExt;

    let mut root_options = OpenOptions::new();
    root_options
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
    root_options
        .open(root)
        .map_err(|error| format!("Unable to open the File Space root safely: {error}"))
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "android"))]
fn open_managed_parent_directory(
    root_directory: &fs::File,
    relative_path: &str,
) -> Result<(fs::File, std::ffi::CString), String> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};

    let components = relative_path.split('/').collect::<Vec<_>>();
    if components.is_empty()
        || components
            .iter()
            .any(|component| component.is_empty() || *component == "." || *component == "..")
    {
        return Err("The managed path is invalid".to_owned());
    }
    let mut directory = root_directory
        .try_clone()
        .map_err(|error| format!("Unable to access the File Space root safely: {error}"))?;
    for component in &components[..components.len() - 1] {
        let component = CString::new(*component)
            .map_err(|_| "The managed path contains an unsupported null byte".to_owned())?;
        let descriptor = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                component.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if descriptor < 0 {
            return Err(format!(
                "Unable to open a managed file folder safely: {}",
                std::io::Error::last_os_error()
            ));
        }
        directory = unsafe { fs::File::from_raw_fd(descriptor) };
    }
    let name = CString::new(*components.last().expect("validated path component"))
        .map_err(|_| "The managed file name contains an unsupported null byte".to_owned())?;
    Ok((directory, name))
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "android"))]
fn regular_file_identity_at(
    directory: &fs::File,
    name: &std::ffi::CStr,
) -> Result<Option<ManagedFileIdentity>, String> {
    use std::os::fd::AsRawFd;

    let mut metadata = std::mem::MaybeUninit::<libc::stat>::zeroed();
    let result = unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            name.as_ptr(),
            metadata.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result != 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::NotFound {
            return Ok(None);
        }
        return Err(format!("Unable to inspect a managed file safely: {error}"));
    }
    let metadata = unsafe { metadata.assume_init() };
    if metadata.st_mode & libc::S_IFMT != libc::S_IFREG {
        return Err("The managed file move path is not a regular file".to_owned());
    }
    Ok(Some(ManagedFileIdentity {
        device: metadata.st_dev as u64,
        inode: metadata.st_ino as u64,
    }))
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "android"))]
fn managed_regular_file_identity(
    root: &Path,
    relative_path: &str,
) -> Result<Option<ManagedFileIdentity>, String> {
    let root_directory = open_file_space_root_directory(root)?;
    let (directory, name) = open_managed_parent_directory(&root_directory, relative_path)?;
    regular_file_identity_at(&directory, &name)
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "android")))]
fn managed_regular_file_identity(
    _root: &Path,
    _relative_path: &str,
) -> Result<Option<ManagedFileIdentity>, String> {
    Err("Safe managed-file identity checks are not supported on this platform".to_owned())
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "android"))]
fn atomic_move_regular_file_without_overwrite(
    root: &Path,
    source_relative_path: &str,
    destination_relative_path: &str,
    expected_identity: Option<ManagedFileIdentity>,
) -> Result<(), AtomicFileMoveError> {
    use std::os::fd::AsRawFd;

    let root_directory =
        open_file_space_root_directory(root).map_err(AtomicFileMoveError::NotMoved)?;
    let (source_directory, source_name) =
        open_managed_parent_directory(&root_directory, source_relative_path)
            .map_err(AtomicFileMoveError::NotMoved)?;
    let (destination_directory, destination_name) =
        open_managed_parent_directory(&root_directory, destination_relative_path)
            .map_err(AtomicFileMoveError::NotMoved)?;
    let source_identity = regular_file_identity_at(&source_directory, &source_name)
        .map_err(AtomicFileMoveError::NotMoved)?
        .ok_or_else(|| {
            AtomicFileMoveError::NotMoved("The file to move no longer exists".to_owned())
        })?;
    if expected_identity.is_some_and(|expected| expected != source_identity) {
        return Err(AtomicFileMoveError::NotMoved(
            "The file changed before it could be moved safely".to_owned(),
        ));
    }

    #[cfg(target_os = "macos")]
    let result = unsafe {
        libc::renameatx_np(
            source_directory.as_raw_fd(),
            source_name.as_ptr(),
            destination_directory.as_raw_fd(),
            destination_name.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let result = unsafe {
        libc::renameat2(
            source_directory.as_raw_fd(),
            source_name.as_ptr(),
            destination_directory.as_raw_fd(),
            destination_name.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        let destination_identity =
            regular_file_identity_at(&destination_directory, &destination_name)
                .map_err(AtomicFileMoveError::StateUnknown)?
                .ok_or_else(|| {
                    AtomicFileMoveError::StateUnknown(
                        "The moved file could not be found at its destination".to_owned(),
                    )
                })?;
        if destination_identity != source_identity {
            return Err(AtomicFileMoveError::StateUnknown(
                "The moved file identity changed at its destination".to_owned(),
            ));
        }
        source_directory.sync_all().map_err(|error| {
            AtomicFileMoveError::StateUnknown(format!(
                "Unable to make the file move durable at its source: {error}"
            ))
        })?;
        if source_directory.as_raw_fd() != destination_directory.as_raw_fd() {
            destination_directory.sync_all().map_err(|error| {
                AtomicFileMoveError::StateUnknown(format!(
                    "Unable to make the file move durable at its destination: {error}"
                ))
            })?;
        }
        Ok(())
    } else {
        Err(AtomicFileMoveError::NotMoved(no_replace_move_error(
            std::io::Error::last_os_error(),
        )))
    }
}

#[cfg(windows)]
fn atomic_move_regular_file_without_overwrite(
    _root: &Path,
    _source_relative_path: &str,
    _destination_relative_path: &str,
    _expected_identity: Option<ManagedFileIdentity>,
) -> Result<(), AtomicFileMoveError> {
    Err(AtomicFileMoveError::NotMoved(
        "Safe no-overwrite file movement is not supported on Windows".to_owned(),
    ))
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "linux",
    target_os = "android",
    windows
)))]
fn atomic_move_regular_file_without_overwrite(
    _root: &Path,
    _source_relative_path: &str,
    _destination_relative_path: &str,
    _expected_identity: Option<ManagedFileIdentity>,
) -> Result<(), AtomicFileMoveError> {
    Err(AtomicFileMoveError::NotMoved(
        "Safe no-overwrite file movement is not supported on this platform".to_owned(),
    ))
}

#[cfg(test)]
fn move_regular_file_without_overwrite(
    root: &Path,
    source_relative_path: &str,
    destination_relative_path: &str,
) -> Result<(), String> {
    atomic_move_regular_file_without_overwrite(
        root,
        source_relative_path,
        destination_relative_path,
        None,
    )
    .map_err(AtomicFileMoveError::into_message)
}

#[cfg(test)]
fn restore_moved_file_without_overwrite(
    root: &Path,
    destination_relative_path: &str,
    source_relative_path: &str,
) -> Result<(), String> {
    move_regular_file_without_overwrite(root, destination_relative_path, source_relative_path)
        .map_err(|error| {
            if error.contains("already exists") {
                "The original path is now occupied; the moved file was preserved at its destination"
                    .to_owned()
            } else {
                error
            }
        })
}

fn file_move_failure_with_restore(
    message: String,
    database: &Database,
    root: &Path,
    pending: &PendingFileMove,
) -> String {
    let expected_identity = ManagedFileIdentity {
        device: pending.source_device,
        inode: pending.source_inode,
    };
    match atomic_move_regular_file_without_overwrite(
        root,
        &pending.destination_relative_path,
        &pending.source_relative_path,
        Some(expected_identity),
    ) {
        Ok(()) => match clear_pending_file_move(database, pending) {
            Ok(()) => message,
            Err(cleanup_error) => format!(
                "{message}. The file was restored, but its pending move journal could not be cleared: {cleanup_error}"
            ),
        },
        Err(restore_error) => format!(
            "{message}. Unable to restore the file: {}",
            restore_error.into_message()
        ),
    }
}

fn recover_pending_file_move(database: &Database) -> Result<(), String> {
    let mut connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Unable to begin pending file move recovery: {error}"))?;
    let journal_value = transaction
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [PENDING_FILE_MOVE_SETTING],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("Unable to load the pending file move: {error}"))?;
    let Some(journal_value) = journal_value else {
        return Ok(());
    };
    let pending = serde_json::from_str::<PendingFileMove>(&journal_value)
        .map_err(|error| format!("The pending file move journal is invalid: {error}"))?;
    if pending.version != 1
        || pending.operation_id.trim().is_empty()
        || pending.file_id.trim().is_empty()
        || pending.source_relative_path == pending.destination_relative_path
    {
        return Err("The pending file move journal has unsupported contents".to_owned());
    }
    let root = transaction
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [STORAGE_ROOT_SETTING],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("Unable to load the File Space root for recovery: {error}"))?
        .ok_or_else(|| {
            "The File Space root is unavailable while a file move requires recovery".to_owned()
        })?;
    let database_path = transaction
        .query_row(
            "SELECT storage_path FROM files
             WHERE id = ?1 AND trashed_at IS NULL AND storage_path IS NOT NULL",
            [&pending.file_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("Unable to load the file pending move recovery: {error}"))?
        .ok_or_else(|| {
            "The file record is missing while its pending move journal is retained".to_owned()
        })?;
    if database_path != pending.source_relative_path {
        return Err(format!(
            "The file path changed unexpectedly while recovering its pending move; database has {database_path}"
        ));
    }

    let root = PathBuf::from(root);
    let expected_identity = ManagedFileIdentity {
        device: pending.source_device,
        inode: pending.source_inode,
    };
    let source_identity = managed_regular_file_identity(&root, &pending.source_relative_path)?;
    let destination_identity =
        managed_regular_file_identity(&root, &pending.destination_relative_path)?;
    match (source_identity, destination_identity) {
        (Some(source), None) if source == expected_identity => {}
        (None, Some(destination)) if destination == expected_identity => {
            atomic_move_regular_file_without_overwrite(
                &root,
                &pending.destination_relative_path,
                &pending.source_relative_path,
                Some(expected_identity),
            )
            .map_err(|error| {
                format!(
                    "Unable to restore the file during pending move recovery: {}",
                    error.into_message()
                )
            })?;
        }
        (Some(_), Some(_)) => {
            return Err(
                "Pending file move recovery is ambiguous because both paths are occupied; both files and the journal were preserved"
                    .to_owned(),
            );
        }
        (None, None) => {
            return Err(
                "Pending file move recovery is ambiguous because both paths are missing; the journal was preserved"
                    .to_owned(),
            );
        }
        _ => {
            return Err(
                "Pending file move recovery found a different file identity; all data and the journal were preserved"
                    .to_owned(),
            );
        }
    }

    let recovered_source = managed_regular_file_identity(&root, &pending.source_relative_path)?;
    let recovered_destination =
        managed_regular_file_identity(&root, &pending.destination_relative_path)?;
    if recovered_source != Some(expected_identity) || recovered_destination.is_some() {
        return Err(
            "Pending file move recovery changed concurrently; all remaining data and the journal were preserved"
                .to_owned(),
        );
    }
    let deleted = transaction
        .execute(
            "DELETE FROM app_settings WHERE key = ?1 AND value = ?2",
            params![PENDING_FILE_MOVE_SETTING, journal_value],
        )
        .map_err(|error| format!("Unable to clear the recovered file move journal: {error}"))?;
    if deleted != 1 {
        return Err("The pending file move journal changed during recovery".to_owned());
    }
    transaction
        .commit()
        .map_err(|error| format!("Unable to complete pending file move recovery: {error}"))
}

fn move_file_record(
    database: &Database,
    artifact_store: &Path,
    file_id: &str,
    folder_id: Option<&str>,
) -> Result<FileSpaceSnapshot, String> {
    recover_pending_file_move(database)?;
    let is_task_file = task_artifact_exists(database, file_id)?;
    if is_task_file {
        reconcile_task_file(database, artifact_store, file_id)?;
    }
    let root = require_ready_root(database)?;
    let (name, current_path, current_folder_id) = active_file_record(database, file_id)?;
    if current_folder_id.as_deref() == folder_id {
        return load_snapshot_record(database);
    }

    let target_folder_path = match folder_id {
        Some(folder_id) => {
            let connection = database
                .0
                .lock()
                .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
            Some(
                connection
                    .query_row(
                        "SELECT relative_path FROM file_space_folders
                         WHERE id = ?1 AND trashed_at IS NULL",
                        [folder_id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(|error| format!("Unable to load the destination folder: {error}"))?
                    .ok_or_else(|| "The destination folder no longer exists".to_owned())?,
            )
        }
        None => None,
    };
    let moved_path = relative_child(target_folder_path.as_deref(), &name);
    active_file_path(database, file_id)?;
    let destination = physical_path(&root, &moved_path)?;
    let expected_task_sha256 = if is_task_file {
        Some(task_file_observed_sha256(database, file_id)?)
    } else {
        None
    };
    let source_identity = managed_regular_file_identity(&root, &current_path)?
        .ok_or_else(|| "The file to move no longer exists".to_owned())?;
    let pending_move = PendingFileMove {
        version: 1,
        operation_id: Uuid::new_v4().to_string(),
        file_id: file_id.to_owned(),
        source_relative_path: current_path.clone(),
        destination_relative_path: moved_path.clone(),
        source_device: source_identity.device,
        source_inode: source_identity.inode,
        created_at: now_millis(),
    };
    let pending_move_value = pending_file_move_json(&pending_move)?;
    persist_pending_file_move(database, &pending_move)?;
    match atomic_move_regular_file_without_overwrite(
        &root,
        &current_path,
        &moved_path,
        Some(source_identity),
    ) {
        Ok(()) => {}
        Err(AtomicFileMoveError::NotMoved(error)) => {
            return match clear_pending_file_move(database, &pending_move) {
                Ok(()) => Err(error),
                Err(cleanup_error) => Err(format!(
                    "{error}. Unable to clear the unexecuted file move journal: {cleanup_error}"
                )),
            };
        }
        Err(AtomicFileMoveError::StateUnknown(error)) => {
            return Err(format!(
                "{error}. The pending file move journal was preserved for recovery"
            ));
        }
    }
    if let Some(expected_sha256) = expected_task_sha256.as_deref() {
        let moved_sha256 = match sha256_file(&destination) {
            Ok((sha256, _)) => sha256,
            Err(error) => {
                return Err(file_move_failure_with_restore(
                    error,
                    database,
                    &root,
                    &pending_move,
                ));
            }
        };
        if moved_sha256 != expected_sha256 {
            return Err(file_move_failure_with_restore(
                "The Task file changed while it was being moved; retry the operation".to_owned(),
                database,
                &root,
                &pending_move,
            ));
        }
    }

    let database_result = (|| -> Result<(), String> {
        let mut connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
        let transaction = connection
            .transaction()
            .map_err(|error| format!("Unable to begin the file move: {error}"))?;
        let now = now_millis();
        let update_result = (|| -> rusqlite::Result<()> {
            let manual_order = transaction.query_row(
                "SELECT COALESCE(MAX(manual_order), -1) + 1
                 FROM files
                 WHERE trashed_at IS NULL AND storage_path IS NOT NULL",
                [],
                |row| row.get::<_, i64>(0),
            )?;
            let changed = transaction.execute(
                "UPDATE files
                 SET folder_id = ?1, storage_path = ?2, manual_order = ?3, updated_at = ?4
                 WHERE id = ?5 AND trashed_at IS NULL AND storage_path = ?6
                   AND folder_id IS ?7",
                params![
                    folder_id,
                    moved_path,
                    manual_order,
                    now,
                    file_id,
                    current_path,
                    current_folder_id
                ],
            )?;
            if changed != 1 {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            if is_task_file {
                transaction.execute(
                    "INSERT INTO file_space_artifact_events
                     (id, file_id, event_type, actor, details_json, created_at)
                     VALUES (?1, ?2, 'renamed', 'user', ?3, ?4)",
                    params![
                        Uuid::new_v4().to_string(),
                        file_id,
                        json!({
                            "fromPath": current_path,
                            "toPath": moved_path,
                            "fromFolderId": current_folder_id,
                            "toFolderId": folder_id,
                        })
                        .to_string(),
                        now
                    ],
                )?;
            }
            let deleted = transaction.execute(
                "DELETE FROM app_settings WHERE key = ?1 AND value = ?2",
                params![PENDING_FILE_MOVE_SETTING, pending_move_value],
            )?;
            if deleted != 1 {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            Ok(())
        })();
        if let Err(error) = update_result {
            return Err(format!("Unable to save the file move: {error}"));
        }
        transaction
            .commit()
            .map_err(|error| format!("Unable to complete the file move: {error}"))
    })();
    if let Err(error) = database_result {
        return Err(file_move_failure_with_restore(
            error,
            database,
            &root,
            &pending_move,
        ));
    }
    load_snapshot_record(database)
}

#[cfg(test)]
fn soft_delete_task_file_record(
    database: &Database,
    artifact_store: &Path,
    file_id: &str,
) -> Result<FileSpaceSnapshot, String> {
    reconcile_task_file(database, artifact_store, file_id)?;
    let expected_sha256 = task_file_observed_sha256(database, file_id)?;
    let source = active_file_path(database, file_id)?;
    let parent = source
        .parent()
        .ok_or_else(|| "The file has no parent folder".to_owned())?;
    let pending_delete = parent.join(format!(".lumetrace-delete-{}", Uuid::new_v4()));
    fs::rename(&source, &pending_delete)
        .map_err(|error| format!("Unable to prepare Task file deletion: {error}"))?;
    let pending_sha256 = match sha256_file(&pending_delete) {
        Ok((sha256, _)) => sha256,
        Err(error) => {
            let _ = fs::rename(&pending_delete, &source);
            return Err(error);
        }
    };
    if pending_sha256 != expected_sha256 {
        let restore_result = fs::rename(&pending_delete, &source);
        return match restore_result {
            Ok(()) => Err(
                "The working file changed while it was being deleted; retry the operation"
                    .to_owned(),
            ),
            Err(error) => Err(format!(
                "The working file changed while it was being deleted, and could not be restored: {error}"
            )),
        };
    }
    let now = now_millis();
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
    let transaction = match connection.unchecked_transaction() {
        Ok(transaction) => transaction,
        Err(error) => {
            let _ = fs::rename(&pending_delete, &source);
            return Err(format!("Unable to begin Task file deletion: {error}"));
        }
    };
    let result = (|| -> rusqlite::Result<()> {
        transaction.execute(
            "UPDATE files SET trashed_at = ?1, updated_at = ?1
             WHERE id = ?2 AND trashed_at IS NULL",
            params![now, file_id],
        )?;
        transaction.execute(
            "INSERT INTO file_space_artifact_events
             (id, file_id, event_type, actor, details_json, created_at)
             VALUES (?1, ?2, 'deleted', 'user', '{}', ?3)",
            params![Uuid::new_v4().to_string(), file_id, now],
        )?;
        Ok(())
    })();
    if let Err(error) = result {
        drop(transaction);
        let _ = fs::rename(&pending_delete, &source);
        return Err(format!("Unable to save Task file deletion: {error}"));
    }
    if let Err(error) = transaction.commit() {
        let _ = fs::rename(&pending_delete, &source);
        return Err(format!("Unable to complete Task file deletion: {error}"));
    }
    drop(connection);
    if sha256_file(&pending_delete).is_ok_and(|(sha256, _)| sha256 == expected_sha256) {
        let _ = fs::remove_file(&pending_delete);
    }
    load_snapshot_record(database)
}

#[cfg(test)]
fn restore_task_file_record(
    database: &Database,
    file_id: &str,
) -> Result<FileSpaceSnapshot, String> {
    let root = require_ready_root(database)?;
    let (requested_name, snapshot_path, expected_sha256, expected_size, folder_id, folder_path) = {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
        let (requested_name, snapshot_path, expected_sha256, expected_size, requested_folder_id) =
            connection
                .query_row(
                    "SELECT f.original_name, v.snapshot_path, v.sha256, v.size_bytes, f.folder_id
                     FROM files f
                     JOIN file_space_artifacts a ON a.file_id = f.id
                     JOIN file_space_artifact_versions v ON v.id = a.current_version_id
                     WHERE f.id = ?1 AND f.trashed_at IS NOT NULL",
                    [file_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, i64>(3)?,
                            row.get::<_, Option<String>>(4)?,
                        ))
                    },
                )
                .optional()
                .map_err(|error| format!("Unable to load deleted Task file: {error}"))?
                .ok_or_else(|| "The deleted Task file no longer exists".to_owned())?;
        let folder = match requested_folder_id {
            Some(folder_id) => connection
                .query_row(
                    "SELECT relative_path FROM file_space_folders
                     WHERE id = ?1 AND trashed_at IS NULL",
                    [&folder_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| format!("Unable to load Task file restore folder: {error}"))?
                .map(|path| (Some(folder_id), Some(path))),
            None => None,
        }
        .unwrap_or((None, None));
        (
            requested_name,
            snapshot_path,
            expected_sha256,
            expected_size,
            folder.0,
            folder.1,
        )
    };
    let directory = match folder_path.as_deref() {
        Some(path) => physical_path(&root, path)?,
        None => root.clone(),
    };
    let name = unique_file_name(&directory, &requested_name);
    let relative_path = relative_child(folder_path.as_deref(), &name);
    let destination = physical_path(&root, &relative_path)?;
    copy_verified_snapshot(
        Path::new(&snapshot_path),
        &destination,
        &expected_sha256,
        expected_size,
    )?;
    let sha256 = expected_sha256;
    let size_bytes = expected_size;
    let mtime_ms = file_mtime_marker(&destination)?;
    let now = now_millis();
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| format!("Unable to begin Task file restore: {error}"))?;
    let result = (|| -> rusqlite::Result<()> {
        transaction.execute(
            "UPDATE files
             SET original_name = ?1, storage_path = ?2, folder_id = ?3, size_bytes = ?4,
                 mime_type = ?5, trashed_at = NULL, updated_at = ?6
             WHERE id = ?7 AND trashed_at IS NOT NULL",
            params![
                name,
                relative_path,
                folder_id,
                size_bytes,
                mime_type_for(&destination),
                now,
                file_id
            ],
        )?;
        transaction.execute(
            "UPDATE file_space_artifacts
             SET observed_size_bytes = ?1, observed_mtime_ms = ?2,
                 observed_sha256 = ?3, updated_at = ?4 WHERE file_id = ?5",
            params![size_bytes, mtime_ms, sha256, now, file_id],
        )?;
        transaction.execute(
            "INSERT INTO file_space_artifact_events
             (id, file_id, event_type, actor, details_json, created_at)
             VALUES (?1, ?2, 'restored', 'user', '{}', ?3)",
            params![Uuid::new_v4().to_string(), file_id, now],
        )?;
        Ok(())
    })();
    if let Err(error) = result {
        drop(transaction);
        let _ = fs::remove_file(&destination);
        return Err(format!("Unable to save Task file restore: {error}"));
    }
    if let Err(error) = transaction.commit() {
        let _ = fs::remove_file(&destination);
        return Err(format!("Unable to complete Task file restore: {error}"));
    }
    drop(connection);
    load_snapshot_record(database)
}

fn delete_file_record(database: &Database, file_id: &str) -> Result<FileSpaceSnapshot, String> {
    let source = active_file_path(database, file_id)?;
    let parent = source
        .parent()
        .ok_or_else(|| "The file has no parent folder".to_owned())?;
    let pending_delete = parent.join(format!(".lumetrace-delete-{}", Uuid::new_v4()));
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
    fs::rename(&source, &pending_delete)
        .map_err(|error| format!("Unable to prepare the file deletion: {error}"))?;
    let transaction = match connection.unchecked_transaction() {
        Ok(transaction) => transaction,
        Err(error) => {
            let _ = fs::rename(&pending_delete, &source);
            return Err(format!("Unable to begin the file deletion: {error}"));
        }
    };
    if let Err(error) = transaction.execute(
        "DELETE FROM files WHERE id = ?1 AND trashed_at IS NULL",
        [file_id],
    ) {
        drop(transaction);
        let _ = fs::rename(&pending_delete, &source);
        return Err(format!("Unable to save the file deletion: {error}"));
    }
    if let Err(error) = transaction.commit() {
        let restore_result = fs::rename(&pending_delete, &source);
        return match restore_result {
            Ok(()) => Err(format!("Unable to complete the file deletion: {error}")),
            Err(restore_error) => Err(format!(
                "Unable to complete the file deletion: {error}. Unable to restore the file: {restore_error}"
            )),
        };
    }
    drop(connection);
    fs::remove_file(&pending_delete)
        .map_err(|error| format!("Unable to permanently delete the file: {error}"))?;
    load_snapshot_record(database)
}

fn task_artifact_directory(artifact_store: &Path, file_id: &str) -> Result<PathBuf, String> {
    let file_uuid =
        Uuid::parse_str(file_id).map_err(|_| "The Task file identity is invalid".to_owned())?;
    Ok(artifact_store.join(file_uuid.to_string()))
}

fn permanently_delete_task_file_record(
    database: &Database,
    artifact_store: &Path,
    file_id: &str,
) -> Result<FileSpaceSnapshot, String> {
    let artifact_directory = task_artifact_directory(artifact_store, file_id)?;
    let pending_artifact_directory = match fs::symlink_metadata(&artifact_directory) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err("The Task file version storage is invalid".to_owned());
            }
            let pending = artifact_store.join(format!(".lumetrace-delete-{}", Uuid::new_v4()));
            fs::rename(&artifact_directory, &pending).map_err(|error| {
                format!("Unable to prepare Task file history deletion: {error}")
            })?;
            Some(pending)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(format!(
                "Unable to inspect Task file version storage: {error}"
            ));
        }
    };

    match delete_file_record(database, file_id) {
        Ok(snapshot) => {
            if let Some(pending) = pending_artifact_directory {
                let _ = fs::remove_dir_all(pending);
            }
            Ok(snapshot)
        }
        Err(error) => {
            if let Some(pending) = pending_artifact_directory {
                let _ = fs::rename(pending, artifact_directory);
            }
            Err(error)
        }
    }
}

fn process_error(action: &str, output: std::process::Output) -> Result<(), String> {
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if detail.is_empty() {
        Err(format!("Unable to {action}"))
    } else {
        Err(format!("Unable to {action}: {detail}"))
    }
}

fn reveal_file_path(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let output = ProcessCommand::new("open")
        .arg("-R")
        .arg(path)
        .output()
        .map_err(|error| format!("Unable to open Finder: {error}"))?;
    #[cfg(target_os = "windows")]
    let output = ProcessCommand::new("explorer")
        .arg(format!("/select,{}", path.display()))
        .output()
        .map_err(|error| format!("Unable to open File Explorer: {error}"))?;
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let output = ProcessCommand::new("xdg-open")
        .arg(path.parent().unwrap_or(path))
        .output()
        .map_err(|error| format!("Unable to open the file location: {error}"))?;
    process_error("open the file location", output)
}

fn is_external_default_document(name: &str) -> bool {
    matches!(
        Path::new(name)
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "html" | "htm" | "csv")
    )
}

fn external_default_document_path(database: &Database, file_id: &str) -> Result<PathBuf, String> {
    let (file_name, _, _) = active_file_record(database, file_id)?;
    if !is_external_default_document(&file_name) {
        return Err("This file type is not configured to open with a system app".to_owned());
    }
    active_file_path(database, file_id)
}

fn open_file_with_default_application(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let output = ProcessCommand::new("open")
        .arg(path)
        .output()
        .map_err(|error| format!("Unable to ask macOS to open the file: {error}"))?;
    #[cfg(target_os = "windows")]
    let output = ProcessCommand::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Start-Process -FilePath $args[0]",
        ])
        .arg(path)
        .output()
        .map_err(|error| format!("Unable to ask Windows to open the file: {error}"))?;
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let output = ProcessCommand::new("xdg-open")
        .arg(path)
        .output()
        .map_err(|error| format!("Unable to ask the system to open the file: {error}"))?;
    process_error("open the file with its default application", output)
        .map_err(|error| format!("DEFAULT_APPLICATION_UNAVAILABLE:{error}"))
}

fn open_file_with_application(path: &Path, application: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(application)
        .map_err(|error| format!("Unable to inspect the selected application: {error}"))?;
    if metadata.file_type().is_symlink() {
        return Err("The selected application cannot be a symbolic link".to_owned());
    }
    #[cfg(target_os = "macos")]
    {
        if !metadata.is_dir()
            || !application
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
        {
            return Err("Select a macOS application bundle (.app)".to_owned());
        }
        let output = ProcessCommand::new("open")
            .arg("-a")
            .arg(application)
            .arg(path)
            .output()
            .map_err(|error| format!("Unable to open the file with the selected app: {error}"))?;
        return process_error("open the file with the selected application", output);
    }
    #[cfg(target_os = "windows")]
    {
        if !metadata.is_file() {
            return Err("Select a Windows application executable".to_owned());
        }
        let output = ProcessCommand::new(application)
            .arg(path)
            .output()
            .map_err(|error| format!("Unable to open the file with the selected app: {error}"))?;
        return process_error("open the file with the selected application", output);
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    return Err(format!(
        "Choosing another application is not supported on this Linux desktop for {}",
        path.display()
    ));
}

fn copy_file_to_clipboard(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let output = ProcessCommand::new("osascript")
        .args([
            "-e",
            "on run argv",
            "-e",
            "set the clipboard to (POSIX file (item 1 of argv))",
            "-e",
            "end run",
        ])
        .arg(path)
        .output()
        .map_err(|error| format!("Unable to copy the file: {error}"))?;
    #[cfg(target_os = "windows")]
    let output = ProcessCommand::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Add-Type -AssemblyName System.Windows.Forms; $items = New-Object System.Collections.Specialized.StringCollection; [void]$items.Add($args[0]); [System.Windows.Forms.Clipboard]::SetFileDropList($items)",
        ])
        .arg(path)
        .output()
        .map_err(|error| format!("Unable to copy the file: {error}"))?;
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let output = {
        let mut child = ProcessCommand::new("xclip")
            .args(["-selection", "clipboard", "-t", "text/uri-list"])
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|error| format!("Unable to copy the file: {error}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            writeln!(stdin, "file://{}", path.display())
                .map_err(|error| format!("Unable to copy the file: {error}"))?;
        }
        child
            .wait_with_output()
            .map_err(|error| format!("Unable to copy the file: {error}"))?
    };
    process_error("copy the file", output)
}

fn copy_path_to_clipboard(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut child = ProcessCommand::new("pbcopy")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Unable to copy the file path: {error}"))?;
    #[cfg(target_os = "windows")]
    let mut child = ProcessCommand::new("clip")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Unable to copy the file path: {error}"))?;
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let mut child = ProcessCommand::new("xclip")
        .args(["-selection", "clipboard"])
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Unable to copy the file path: {error}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(path.to_string_lossy().as_bytes())
            .map_err(|error| format!("Unable to copy the file path: {error}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("Unable to copy the file path: {error}"))?;
    process_error("copy the file path", output)
}

fn mime_type_for(path: &Path) -> Option<String> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    let mime = match extension.as_str() {
        "txt" | "md" | "markdown" => "text/plain",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "csv" => "text/csv",
        "json" => "application/json",
        "zip" => "application/zip",
        _ => return None,
    };
    Some(mime.to_owned())
}

fn unique_file_name(directory: &Path, original_name: &str) -> String {
    if !directory.join(original_name).exists() {
        return original_name.to_owned();
    }
    let original = Path::new(original_name);
    let stem = original
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("file");
    let extension = original.extension().and_then(|value| value.to_str());
    for index in 2..10_000 {
        let candidate = match extension {
            Some(extension) => format!("{stem} ({index}).{extension}"),
            None => format!("{stem} ({index})"),
        };
        if !directory.join(&candidate).exists() {
            return candidate;
        }
    }
    format!("{}-{original_name}", Uuid::new_v4())
}

fn unique_folder_name(directory: &Path, original_name: &str) -> String {
    if !directory.join(original_name).exists() {
        return original_name.to_owned();
    }
    for index in 2..10_000 {
        let candidate = format!("{original_name} ({index})");
        if !directory.join(&candidate).exists() {
            return candidate;
        }
    }
    format!("{}-{original_name}", Uuid::new_v4())
}

fn copy_file_exclusive(source: &Path, destination: &Path) -> Result<u64, String> {
    let original_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("selected file");
    let mut input = fs::File::open(source)
        .map_err(|error| format!("Unable to read {original_name}: {error}"))?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| format!("Unable to create {original_name}: {error}"))?;
    match std::io::copy(&mut input, &mut output).and_then(|bytes| {
        output.sync_all()?;
        Ok(bytes)
    }) {
        Ok(bytes) => Ok(bytes),
        Err(error) => {
            drop(output);
            let _ = fs::remove_file(destination);
            Err(format!("Unable to copy {original_name}: {error}"))
        }
    }
}

fn insert_imported_file(
    transaction: &rusqlite::Transaction<'_>,
    source: &Path,
    destination: &Path,
    name: String,
    folder_id: Option<&str>,
    folder_path: Option<&str>,
) -> Result<(), String> {
    let copied_bytes = copy_file_exclusive(source, destination)?;
    let size_bytes = i64::try_from(copied_bytes)
        .map_err(|_| format!("The imported file is too large: {}", source.display()))?;
    let now = now_millis();
    let manual_order = transaction
        .query_row(
            "SELECT COALESCE(MAX(manual_order), -1) + 1 FROM files",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("Unable to choose the imported file position: {error}"))?;
    let file = FileSpaceFile {
        id: Uuid::new_v4().to_string(),
        folder_id: folder_id.map(str::to_owned),
        name: name.clone(),
        relative_path: relative_child(folder_path, &name),
        mime_type: mime_type_for(destination),
        size_bytes,
        source_kind: "user_import".to_owned(),
        manual_order,
        current_version: None,
        version_count: 0,
        tags: Vec::new(),
        created_at: now,
        updated_at: now,
    };
    if let Err(error) = transaction.execute(
        "INSERT INTO files
             (id, original_name, storage_path, mime_type, size_bytes, folder_id,
              source_kind, manual_order, updated_at, trashed_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10)",
        params![
            file.id,
            file.name,
            file.relative_path,
            file.mime_type,
            file.size_bytes,
            file.folder_id,
            file.source_kind,
            file.manual_order,
            file.updated_at,
            file.created_at
        ],
    ) {
        let _ = fs::remove_file(destination);
        return Err(format!("Unable to save the imported file: {error}"));
    }
    Ok(())
}

fn import_directory_contents<F>(
    transaction: &rusqlite::Transaction<'_>,
    source_directory: &Path,
    destination_directory: &Path,
    destination_folder_id: &str,
    destination_folder_path: &str,
    progress: &mut F,
) -> Result<(), String>
where
    F: FnMut(&Path) -> Result<(), String>,
{
    let entries = fs::read_dir(source_directory)
        .map_err(|error| {
            format!(
                "Unable to read the selected folder {}: {error}",
                source_directory.display()
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            format!(
                "Unable to read the selected folder {}: {error}",
                source_directory.display()
            )
        })?;
    let mut entries = entries;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let source = entry.path();
        progress(&source)?;
        let metadata = fs::symlink_metadata(&source)
            .map_err(|error| format!("Unable to inspect {}: {error}", source.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "Symbolic links cannot be imported: {}",
                source.display()
            ));
        }
        let name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| {
                format!(
                    "The selected item has an unsupported name: {}",
                    source.display()
                )
            })
            .and_then(validate_import_name)?;
        let destination = destination_directory.join(&name);
        if metadata.is_dir() {
            fs::create_dir(&destination)
                .map_err(|error| format!("Unable to create the imported folder {name}: {error}"))?;
            let relative_path = relative_child(Some(destination_folder_path), &name);
            let folder_id = Uuid::new_v4().to_string();
            let now = now_millis();
            transaction
                .execute(
                    "INSERT INTO file_space_folders
                     (id, parent_id, name, relative_path, manual_order,
                      created_at, updated_at, trashed_at)
                     VALUES (?1, ?2, ?3, ?4,
                             (SELECT COALESCE(MAX(manual_order), -1) + 1
                              FROM file_space_folders WHERE parent_id IS ?2),
                             ?5, ?6, NULL)",
                    params![
                        folder_id,
                        destination_folder_id,
                        name,
                        relative_path,
                        now,
                        now
                    ],
                )
                .map_err(|error| format!("Unable to save the imported folder: {error}"))?;
            import_directory_contents(
                transaction,
                &source,
                &destination,
                &folder_id,
                &relative_path,
                progress,
            )?;
        } else if metadata.is_file() {
            insert_imported_file(
                transaction,
                &source,
                &destination,
                name,
                Some(destination_folder_id),
                Some(destination_folder_path),
            )?;
        } else {
            return Err(format!(
                "The selected item is not a regular file or folder: {}",
                source.display()
            ));
        }
    }
    Ok(())
}

#[derive(Debug)]
enum ImportedDestination {
    File(PathBuf),
    Directory(PathBuf),
}

fn rollback_imported_destinations(destinations: &[ImportedDestination]) -> Result<(), String> {
    let mut errors = Vec::new();
    for destination in destinations.iter().rev() {
        let result = match destination {
            ImportedDestination::File(path) => fs::remove_file(path),
            ImportedDestination::Directory(path) => fs::remove_dir_all(path),
        };
        if let Err(error) = result {
            errors.push(error.to_string());
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn import_files_record_with_progress<F>(
    database: &Database,
    folder_id: Option<&str>,
    paths: &[String],
    mut progress: F,
) -> Result<FileSpaceSnapshot, String>
where
    F: FnMut(&Path) -> Result<(), String>,
{
    let root = require_ready_root(database)?;
    if paths.is_empty() {
        return load_snapshot_record(database);
    }
    let import_roots = canonical_import_roots(paths)?;
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| format!("Unable to begin the file import: {error}"))?;
    let mut imported_destinations = Vec::new();
    let import_result = (|| -> Result<(), String> {
        let folder_path = match folder_id {
            Some(folder_id) => Some(
                transaction
                    .query_row(
                        "SELECT relative_path FROM file_space_folders
                     WHERE id = ?1 AND trashed_at IS NULL",
                        [folder_id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(|error| format!("Unable to load the destination folder: {error}"))?
                    .ok_or_else(|| "The destination folder no longer exists".to_owned())?,
            ),
            None => None,
        };
        let directory = match folder_path.as_deref() {
            Some(path) => physical_path(&root, path)?,
            None => root.clone(),
        };
        let canonical_root = root
            .canonicalize()
            .map_err(|error| format!("Unable to resolve the File Space storage path: {error}"))?;
        if import_roots
            .iter()
            .any(|source| source != &canonical_root && source.starts_with(&canonical_root))
        {
            return Err(
                "A file or folder already inside File Space cannot be imported again".to_owned(),
            );
        }
        let canonical_directory = directory
            .canonicalize()
            .map_err(|error| format!("Unable to resolve the destination folder: {error}"))?;
        if !canonical_directory.starts_with(&canonical_root) {
            return Err("The destination folder is outside the File Space storage path".to_owned());
        }
        for source in &import_roots {
            progress(source)?;
            let metadata = fs::symlink_metadata(&source)
                .map_err(|error| format!("Unable to inspect {}: {error}", source.display()))?;
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "Symbolic links cannot be imported: {}",
                    source.display()
                ));
            }
            let original_name = source
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| "A selected item has an unsupported name".to_owned())?;
            let original_name = validate_import_name(original_name)?;
            if metadata.is_dir() {
                if canonical_directory.starts_with(source) {
                    return Err(format!(
                        "A folder cannot be imported into itself: {}",
                        source.display()
                    ));
                }
                let name = unique_folder_name(&directory, &original_name);
                let destination = directory.join(&name);
                fs::create_dir(&destination).map_err(|error| {
                    format!("Unable to create the imported folder {name}: {error}")
                })?;
                imported_destinations.push(ImportedDestination::Directory(destination.clone()));
                let relative_path = relative_child(folder_path.as_deref(), &name);
                let imported_folder_id = Uuid::new_v4().to_string();
                let now = now_millis();
                transaction
                    .execute(
                        "INSERT INTO file_space_folders
                         (id, parent_id, name, relative_path, manual_order,
                          created_at, updated_at, trashed_at)
                         VALUES (?1, ?2, ?3, ?4,
                                 (SELECT COALESCE(MAX(manual_order), -1) + 1
                                  FROM file_space_folders WHERE parent_id IS ?2),
                                 ?5, ?6, NULL)",
                        params![imported_folder_id, folder_id, name, relative_path, now, now],
                    )
                    .map_err(|error| format!("Unable to save the imported folder: {error}"))?;
                import_directory_contents(
                    &transaction,
                    source,
                    &destination,
                    &imported_folder_id,
                    &relative_path,
                    &mut progress,
                )?;
            } else if metadata.is_file() {
                let name = unique_file_name(&directory, &original_name);
                let destination = directory.join(&name);
                insert_imported_file(
                    &transaction,
                    source,
                    &destination,
                    name,
                    folder_id,
                    folder_path.as_deref(),
                )?;
                imported_destinations.push(ImportedDestination::File(destination));
            } else {
                return Err(format!(
                    "The selected item is not a regular file or folder: {}",
                    source.display()
                ));
            }
        }
        Ok(())
    })();

    match import_result {
        Ok(()) => {}
        Err(error) => {
            drop(transaction);
            if let Err(rollback_error) = rollback_imported_destinations(&imported_destinations) {
                return Err(format!(
                    "{error}. Unable to fully roll back the imported items: {rollback_error}"
                ));
            }
            return Err(error);
        }
    }
    if let Err(error) = transaction.commit() {
        if let Err(rollback_error) = rollback_imported_destinations(&imported_destinations) {
            return Err(format!(
                "Unable to complete the file import: {error}. Unable to fully roll back the imported items: {rollback_error}"
            ));
        }
        return Err(format!("Unable to complete the file import: {error}"));
    }
    drop(connection);
    load_snapshot_record(database)
}

#[cfg(test)]
fn import_files_record(
    database: &Database,
    folder_id: Option<&str>,
    paths: &[String],
) -> Result<FileSpaceSnapshot, String> {
    import_files_record_with_progress(database, folder_id, paths, |_| Ok(()))
}

fn dropped_relative_path(value: &str) -> Result<PathBuf, String> {
    let value = value.trim_start_matches('/');
    if value.is_empty() {
        return Err("A dropped file has no name".to_owned());
    }
    let mut relative_path = PathBuf::new();
    for component in value.split('/') {
        relative_path.push(validate_import_name(component)?);
    }
    Ok(relative_path)
}

fn stage_dropped_files(
    staging_root: &Path,
    files: &[FileSpaceDroppedFile],
) -> Result<Vec<String>, String> {
    if files.is_empty() {
        return Err("The dropped content does not contain any files".to_owned());
    }
    fs::create_dir(staging_root)
        .map_err(|error| format!("Unable to prepare the dropped files: {error}"))?;
    let mut roots = BTreeSet::new();
    for file in files {
        let relative_path = dropped_relative_path(&file.relative_path)?;
        let root_name = relative_path
            .components()
            .next()
            .ok_or_else(|| "A dropped file has no name".to_owned())?
            .as_os_str()
            .to_owned();
        let destination = staging_root.join(&relative_path);
        let parent = destination
            .parent()
            .ok_or_else(|| "A dropped file has no parent folder".to_owned())?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("Unable to prepare the dropped folder: {error}"))?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .map_err(|error| {
                format!(
                    "Unable to prepare the dropped file {}: {error}",
                    file.relative_path
                )
            })?;
        output.write_all(&file.bytes).map_err(|error| {
            format!(
                "Unable to prepare the dropped file {}: {error}",
                file.relative_path
            )
        })?;
        output.sync_all().map_err(|error| {
            format!(
                "Unable to finish preparing the dropped file {}: {error}",
                file.relative_path
            )
        })?;
        roots.insert(staging_root.join(root_name).to_string_lossy().into_owned());
    }
    Ok(roots.into_iter().collect())
}

fn cleanup_staged_drop(
    staging_root: &Path,
    result: Result<FileSpaceSnapshot, String>,
) -> Result<FileSpaceSnapshot, String> {
    let cleanup = fs::remove_dir_all(staging_root);
    match (result, cleanup) {
        (Ok(snapshot), _) => Ok(snapshot),
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(cleanup_error)) => Err(format!(
            "{error}. Temporary drop data could not be removed: {cleanup_error}"
        )),
    }
}

fn count_import_entries(path: &Path, request_id: &str) -> Result<usize, String> {
    if import_cancelled(request_id) {
        return Err("IMPORT_CANCELLED:The file import was cancelled".to_owned());
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Unable to inspect {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "Symbolic links cannot be imported: {}",
            path.display()
        ));
    }
    if metadata.is_file() {
        return Ok(1);
    }
    if !metadata.is_dir() {
        return Err(format!(
            "The selected item is not a regular file or folder: {}",
            path.display()
        ));
    }
    let mut total = 1;
    for entry in fs::read_dir(path).map_err(|error| {
        format!(
            "Unable to read the selected folder {}: {error}",
            path.display()
        )
    })? {
        let entry = entry.map_err(|error| {
            format!(
                "Unable to read the selected folder {}: {error}",
                path.display()
            )
        })?;
        total += count_import_entries(&entry.path(), request_id)?;
    }
    Ok(total)
}

fn import_cancelled(request_id: &str) -> bool {
    FILE_SPACE_IMPORT_CANCELLATIONS
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .map(|requests| requests.contains(request_id))
        .unwrap_or(true)
}

fn clear_import_cancellation(request_id: &str) {
    if let Ok(mut requests) = FILE_SPACE_IMPORT_CANCELLATIONS
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
    {
        requests.remove(request_id);
    }
}

fn emit_import_progress<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    request_id: &str,
    phase: &str,
    processed: usize,
    total: usize,
    current_name: Option<String>,
) {
    let _ = app.emit(
        "file-space-import-progress",
        FileSpaceImportProgress {
            request_id: request_id.to_owned(),
            phase: phase.to_owned(),
            processed,
            total,
            current_name,
        },
    );
}

fn normalized_search_scopes(scopes: &[String]) -> Vec<&'static str> {
    let mut normalized = Vec::new();
    for scope in scopes {
        let column = match scope.as_str() {
            "name" => "file_name",
            "content" => "body_text",
            "tag" => "tag_text",
            "task" => "task_text",
            "cell" => "cell_text",
            _ => continue,
        };
        if !normalized.contains(&column) {
            normalized.push(column);
        }
    }
    if normalized.is_empty() {
        vec![
            "file_name",
            "body_text",
            "tag_text",
            "task_text",
            "cell_text",
        ]
    } else {
        normalized
    }
}

fn fts_term(value: &str) -> String {
    format!("\"{}\"*", value.replace('"', "\"\""))
}

fn search_file_space_records(
    database: &Database,
    request: &FileSpaceSearchRequest,
) -> Result<Vec<String>, String> {
    let query = request.query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let root = read_storage_root(database)?;
    synchronize_file_search_index(database, root.as_deref())?;
    let scopes = normalized_search_scopes(&request.scopes);
    let terms = query
        .split_whitespace()
        .filter(|term| !term.is_empty())
        .map(fts_term)
        .collect::<Vec<_>>();
    let mut matches = HashSet::new();
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access LumeTrace database".to_owned())?;

    if !terms.is_empty() {
        let expression = scopes
            .iter()
            .map(|column| format!("{column} : ({})", terms.join(" AND ")))
            .collect::<Vec<_>>()
            .join(" OR ");
        let mut statement = connection
            .prepare(
                "SELECT documents.file_id
                 FROM file_space_search_fts
                 JOIN file_space_search_documents documents
                   ON documents.rowid = file_space_search_fts.rowid
                 WHERE file_space_search_fts MATCH ?1",
            )
            .map_err(|error| format!("Unable to prepare File Space full-text search: {error}"))?;
        let rows = statement.query_map([expression], |row| row.get::<_, String>(0));
        if let Ok(rows) = rows {
            matches.extend(rows.flatten());
        }
    }

    let clauses = scopes
        .iter()
        .map(|column| format!("instr(lower({column}), lower(?)) > 0"))
        .collect::<Vec<_>>();
    let sql = format!(
        "SELECT file_id FROM file_space_search_documents WHERE {}",
        clauses.join(" OR ")
    );
    let patterns = vec![query.to_owned(); scopes.len()];
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| format!("Unable to prepare File Space substring search: {error}"))?;
    let rows = statement
        .query_map(params_from_iter(patterns), |row| row.get::<_, String>(0))
        .map_err(|error| format!("Unable to search File Space: {error}"))?;
    for row in rows {
        matches.insert(
            row.map_err(|error| format!("Unable to read File Space search result: {error}"))?,
        );
    }
    Ok(matches.into_iter().collect())
}

fn validate_file_tags(tags: Vec<String>) -> Result<Vec<(String, String)>, String> {
    if tags.len() > 12 {
        return Err("A file can have at most 12 Tags".to_owned());
    }
    let mut normalized = HashSet::new();
    let mut result = Vec::new();
    for tag in tags {
        let tag = tag.trim().to_owned();
        if tag.is_empty() {
            continue;
        }
        if tag.chars().count() > 32 || tag.chars().any(char::is_control) {
            return Err("Each Tag must contain 1 to 32 visible characters".to_owned());
        }
        let key = tag.to_lowercase();
        if normalized.insert(key.clone()) {
            result.push((tag, key));
        }
    }
    Ok(result)
}

fn set_file_tags_record(
    database: &Database,
    file_id: &str,
    tags: Vec<String>,
) -> Result<FileSpaceSnapshot, String> {
    let tags = validate_file_tags(tags)?;
    let mut connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
    let exists = connection
        .query_row(
            "SELECT EXISTS(
               SELECT 1 FROM files
               WHERE id = ?1 AND trashed_at IS NULL AND storage_path IS NOT NULL
             )",
            [file_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| format!("Unable to verify the tagged file: {error}"))?;
    if !exists {
        return Err("The file no longer exists".to_owned());
    }
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Unable to begin Tag update: {error}"))?;
    transaction
        .execute(
            "DELETE FROM file_space_file_tags WHERE file_id = ?1",
            [file_id],
        )
        .map_err(|error| format!("Unable to replace file Tags: {error}"))?;
    let now = now_millis();
    for (tag, normalized_tag) in tags {
        transaction
            .execute(
                "INSERT INTO file_space_file_tags
                 (file_id, tag, normalized_tag, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![file_id, tag, normalized_tag, now],
            )
            .map_err(|error| format!("Unable to save file Tag: {error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("Unable to complete Tag update: {error}"))?;
    drop(connection);
    load_snapshot_record(database)
}

fn reorder_file_records(
    database: &Database,
    ordered_file_ids: &[String],
) -> Result<FileSpaceSnapshot, String> {
    if ordered_file_ids.is_empty() {
        return Err("No files were supplied for reordering".to_owned());
    }
    let requested = ordered_file_ids.iter().collect::<HashSet<_>>();
    if requested.len() != ordered_file_ids.len() {
        return Err("The reordered file list contains duplicates".to_owned());
    }

    let mut connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access LumeTrace database".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Unable to begin file reordering: {error}"))?;
    let mut current_order = transaction
        .prepare(
            "SELECT id FROM files
             WHERE trashed_at IS NULL AND storage_path IS NOT NULL
             ORDER BY manual_order, created_at, id",
        )
        .map_err(|error| format!("Unable to prepare the current file order: {error}"))?
        .query_map([], |row| row.get::<_, String>(0))
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .map_err(|error| format!("Unable to load the current file order: {error}"))?;
    let target_slots = current_order
        .iter()
        .enumerate()
        .filter_map(|(index, id)| requested.contains(id).then_some(index))
        .collect::<Vec<_>>();
    if target_slots.len() != ordered_file_ids.len() {
        return Err("One or more reordered files no longer exist".to_owned());
    }
    for (slot, file_id) in target_slots.into_iter().zip(ordered_file_ids) {
        current_order[slot] = file_id.clone();
    }
    for (position, file_id) in current_order.iter().enumerate() {
        transaction
            .execute(
                "UPDATE files SET manual_order = ?1 WHERE id = ?2",
                params![position as i64, file_id],
            )
            .map_err(|error| format!("Unable to save the file order: {error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("Unable to complete file reordering: {error}"))?;
    drop(connection);
    load_snapshot_record(database)
}

#[tauri::command]
pub fn get_file_space_snapshot<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    database: State<'_, Database>,
) -> Result<FileSpaceSnapshot, String> {
    let _operation = lock_file_space_operations()?;
    capture_initial_user_versions(database.inner(), &artifact_store_path(&app)?)?;
    materialize_pending_task_files_unlocked(database.inner())?;
    load_snapshot_record(database.inner())
}

#[tauri::command]
pub fn search_file_space_files(
    request: FileSpaceSearchRequest,
    database: State<'_, Database>,
) -> Result<Vec<String>, String> {
    let _operation = lock_file_space_operations()?;
    search_file_space_records(database.inner(), &request)
}

#[tauri::command]
pub fn set_file_space_file_tags(
    file_id: String,
    tags: Vec<String>,
    database: State<'_, Database>,
) -> Result<FileSpaceSnapshot, String> {
    let _operation = lock_file_space_operations()?;
    set_file_tags_record(database.inner(), &file_id, tags)
}

#[tauri::command]
pub fn reorder_file_space_files(
    ordered_file_ids: Vec<String>,
    database: State<'_, Database>,
) -> Result<FileSpaceSnapshot, String> {
    let _operation = lock_file_space_operations()?;
    reorder_file_records(database.inner(), &ordered_file_ids)
}

#[tauri::command]
pub fn configure_file_space_root(
    path: String,
    database: State<'_, Database>,
) -> Result<FileSpaceSnapshot, String> {
    let _operation = lock_file_space_operations()?;
    configure_storage_root_record(database.inner(), &path)?;
    materialize_pending_task_files_unlocked(database.inner())?;
    load_snapshot_record(database.inner())
}

#[tauri::command]
pub fn create_file_space_folder(
    parent_id: Option<String>,
    name: String,
    database: State<'_, Database>,
) -> Result<FileSpaceFolder, String> {
    let _operation = lock_file_space_operations()?;
    create_folder_record(database.inner(), parent_id.as_deref(), &name)
}

#[tauri::command]
pub fn rename_file_space_folder<R: tauri::Runtime>(
    folder_id: String,
    name: String,
    app: tauri::AppHandle<R>,
    database: State<'_, Database>,
) -> Result<FileSpaceSnapshot, String> {
    let _operation = lock_file_space_operations()?;
    rename_folder_record(
        database.inner(),
        &artifact_store_path(&app)?,
        &folder_id,
        &name,
    )
}

#[tauri::command]
pub fn move_file_space_folder<R: tauri::Runtime>(
    folder_id: String,
    parent_id: Option<String>,
    ordered_sibling_ids: Vec<String>,
    app: tauri::AppHandle<R>,
    database: State<'_, Database>,
) -> Result<FileSpaceSnapshot, String> {
    let _operation = lock_file_space_operations()?;
    move_folder_record(
        database.inner(),
        &artifact_store_path(&app)?,
        &folder_id,
        parent_id.as_deref(),
        &ordered_sibling_ids,
    )
}

#[tauri::command]
pub fn delete_file_space_folder<R: tauri::Runtime>(
    folder_id: String,
    app: tauri::AppHandle<R>,
    database: State<'_, Database>,
) -> Result<FileSpaceSnapshot, String> {
    let _operation = lock_file_space_operations()?;
    delete_folder_record(database.inner(), &artifact_store_path(&app)?, &folder_id)
}

#[tauri::command]
pub fn rename_file_space_file<R: tauri::Runtime>(
    file_id: String,
    name: String,
    app: tauri::AppHandle<R>,
    database: State<'_, Database>,
) -> Result<FileSpaceSnapshot, String> {
    let _operation = lock_file_space_operations()?;
    if task_artifact_exists(database.inner(), &file_id)? {
        reconcile_task_file(database.inner(), &artifact_store_path(&app)?, &file_id)?;
        rename_task_file_record(database.inner(), &file_id, &name)
    } else {
        rename_file_record(database.inner(), &file_id, &name)
    }
}

#[tauri::command]
pub fn move_file_space_file<R: tauri::Runtime>(
    file_id: String,
    folder_id: Option<String>,
    app: tauri::AppHandle<R>,
    database: State<'_, Database>,
) -> Result<FileSpaceSnapshot, String> {
    let _operation = lock_file_space_operations()?;
    move_file_record(
        database.inner(),
        &artifact_store_path(&app)?,
        &file_id,
        folder_id.as_deref(),
    )
}

#[tauri::command]
pub fn delete_file_space_file<R: tauri::Runtime>(
    file_id: String,
    app: tauri::AppHandle<R>,
    database: State<'_, Database>,
) -> Result<FileSpaceSnapshot, String> {
    let _operation = lock_file_space_operations()?;
    if task_artifact_exists(database.inner(), &file_id)? {
        permanently_delete_task_file_record(database.inner(), &artifact_store_path(&app)?, &file_id)
    } else {
        delete_file_record(database.inner(), &file_id)
    }
}

fn reconcile_before_file_action<R: tauri::Runtime>(
    database: &Database,
    app: &tauri::AppHandle<R>,
    file_id: &str,
) -> Result<(), String> {
    if task_artifact_exists(database, file_id)? {
        reconcile_task_file(database, &artifact_store_path(app)?, file_id)?;
    }
    Ok(())
}

#[tauri::command]
pub fn reveal_file_space_file<R: tauri::Runtime>(
    file_id: String,
    app: tauri::AppHandle<R>,
    database: State<'_, Database>,
) -> Result<(), String> {
    let _operation = lock_file_space_operations()?;
    reconcile_before_file_action(database.inner(), &app, &file_id)?;
    reveal_file_path(&active_file_path(database.inner(), &file_id)?)
}

#[tauri::command]
pub fn copy_file_space_file<R: tauri::Runtime>(
    file_id: String,
    app: tauri::AppHandle<R>,
    database: State<'_, Database>,
) -> Result<(), String> {
    let _operation = lock_file_space_operations()?;
    reconcile_before_file_action(database.inner(), &app, &file_id)?;
    copy_file_to_clipboard(&active_file_path(database.inner(), &file_id)?)
}

#[tauri::command]
pub fn copy_file_space_file_path<R: tauri::Runtime>(
    file_id: String,
    app: tauri::AppHandle<R>,
    database: State<'_, Database>,
) -> Result<(), String> {
    let _operation = lock_file_space_operations()?;
    reconcile_before_file_action(database.inner(), &app, &file_id)?;
    copy_path_to_clipboard(&active_file_path(database.inner(), &file_id)?)
}

#[tauri::command]
pub async fn start_file_space_drag_out<R: tauri::Runtime>(
    file_id: String,
    preview_bytes: Option<Vec<u8>>,
    window: tauri::Window<R>,
    app: tauri::AppHandle<R>,
    database: State<'_, Database>,
) -> Result<(), String> {
    let path = {
        let _operation = lock_file_space_operations()?;
        reconcile_before_file_action(database.inner(), &app, &file_id)?;
        active_file_path(database.inner(), &file_id)?
    };
    let (drag_image, preview_cleanup) = file_drag_image(&path, preview_bytes);
    let schedule_cleanup = preview_cleanup.clone();
    let (sender, receiver) = channel::<Result<(), String>>();
    app.run_on_main_thread(move || {
        #[cfg(target_os = "linux")]
        let raw_window = window.gtk_window();
        #[cfg(not(target_os = "linux"))]
        let raw_window = tauri::Result::Ok(window.clone());
        let Ok(raw_window) = raw_window else {
            cleanup_file_drag_preview(preview_cleanup.as_ref());
            let _ = sender.send(Err("Unable to access the application window".to_owned()));
            return;
        };
        let drop_sender = sender.clone();
        let drop_cleanup = preview_cleanup.clone();
        if let Err(error) = drag::start_drag(
            &raw_window,
            drag::DragItem::Files(vec![path]),
            drag_image,
            move |_, _| {
                cleanup_file_drag_preview(drop_cleanup.as_ref());
                let _ = drop_sender.send(Ok(()));
            },
            drag::Options::default(),
        ) {
            cleanup_file_drag_preview(preview_cleanup.as_ref());
            let _ = sender.send(Err(format!(
                "Unable to start the system file drag: {error}"
            )));
        }
    })
    .map_err(|error| {
        cleanup_file_drag_preview(schedule_cleanup.as_ref());
        format!("Unable to start the system file drag: {error}")
    })?;
    receiver
        .recv()
        .map_err(|error| format!("Unable to start the system file drag: {error}"))?
}

#[tauri::command]
pub fn open_file_space_file<R: tauri::Runtime>(
    file_id: String,
    app: tauri::AppHandle<R>,
    database: State<'_, Database>,
) -> Result<(), String> {
    let _operation = lock_file_space_operations()?;
    reconcile_before_file_action(database.inner(), &app, &file_id)?;
    open_file_with_default_application(&external_default_document_path(database.inner(), &file_id)?)
}

#[tauri::command]
pub async fn choose_application_for_file_space_file<R: tauri::Runtime>(
    file_id: String,
    app: tauri::AppHandle<R>,
    database: State<'_, Database>,
) -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    let picker = app
        .dialog()
        .file()
        .set_title("Choose an application")
        .set_directory("/Applications")
        .add_filter("Applications", &["app"]);
    #[cfg(target_os = "windows")]
    let picker = app
        .dialog()
        .file()
        .set_title("Choose an application")
        .add_filter("Applications", &["exe"]);
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    return Err("Choosing another application is not supported on this Linux desktop".to_owned());
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let Some(application) =
        tauri::async_runtime::spawn_blocking(move || picker.blocking_pick_file())
            .await
            .map_err(|error| format!("Unable to open the application chooser: {error}"))?
    else {
        return Ok(false);
    };
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let application = application
        .into_path()
        .map_err(|error| format!("Unable to resolve the selected application: {error}"))?;
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let _operation = lock_file_space_operations()?;
        reconcile_before_file_action(database.inner(), &app, &file_id)?;
        let file_path = external_default_document_path(database.inner(), &file_id)?;
        open_file_with_application(&file_path, &application)?;
        Ok(true)
    }
}

#[tauri::command]
pub fn get_task_file_timeline<R: tauri::Runtime>(
    file_id: String,
    app: tauri::AppHandle<R>,
    database: State<'_, Database>,
) -> Result<TaskFileTimeline, String> {
    let _operation = lock_file_space_operations()?;
    reconcile_task_file(database.inner(), &artifact_store_path(&app)?, &file_id)?;
    load_task_file_timeline_record(database.inner(), &file_id)
}

#[tauri::command]
pub fn set_current_task_file_version<R: tauri::Runtime>(
    file_id: String,
    version_id: String,
    app: tauri::AppHandle<R>,
    database: State<'_, Database>,
) -> Result<FileSpaceSnapshot, String> {
    let _operation = lock_file_space_operations()?;
    set_current_task_file_version_record(
        database.inner(),
        &artifact_store_path(&app)?,
        &file_id,
        &version_id,
    )
}

#[tauri::command]
pub fn read_task_file_version(
    file_id: String,
    version_id: String,
    database: State<'_, Database>,
) -> Result<Vec<u8>, String> {
    read_task_file_version_record(database.inner(), &file_id, &version_id)
}

#[tauri::command]
pub fn read_file_space_markdown(
    file_id: String,
    database: State<'_, Database>,
) -> Result<String, String> {
    let _operation = lock_file_space_operations()?;
    read_markdown_file_record(database.inner(), &file_id)
}

#[tauri::command]
pub fn read_file_space_text(
    file_id: String,
    database: State<'_, Database>,
) -> Result<String, String> {
    let _operation = lock_file_space_operations()?;
    read_text_file_record(database.inner(), &file_id)
}

#[tauri::command]
pub fn save_file_space_markdown<R: tauri::Runtime>(
    file_id: String,
    content: String,
    app: tauri::AppHandle<R>,
    database: State<'_, Database>,
) -> Result<FileSpaceSnapshot, String> {
    let _operation = lock_file_space_operations()?;
    save_markdown_file_record(
        database.inner(),
        &artifact_store_path(&app)?,
        &file_id,
        &content,
    )
}

#[tauri::command]
pub async fn import_file_space_files<R: tauri::Runtime>(
    request_id: String,
    folder_id: Option<String>,
    paths: Vec<String>,
    app: tauri::AppHandle<R>,
) -> Result<FileSpaceSnapshot, String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut completed_total = 0_usize;
        let result = (|| {
            emit_import_progress(&worker_app, &request_id, "scanning", 0, 0, None);
            let roots = canonical_import_roots(&paths)?;
            let total = roots.iter().try_fold(0_usize, |total, path| {
                count_import_entries(path, &request_id).map(|count| total + count)
            })?;
            completed_total = total;
            if import_cancelled(&request_id) {
                return Err("IMPORT_CANCELLED:The file import was cancelled".to_owned());
            }
            emit_import_progress(&worker_app, &request_id, "importing", 0, total, None);
            let database = worker_app.state::<Database>();
            let _operation = lock_file_space_operations()?;
            let mut processed = 0_usize;
            let _snapshot = import_files_record_with_progress(
                database.inner(),
                folder_id.as_deref(),
                &paths,
                |path| {
                    if import_cancelled(&request_id) {
                        return Err("IMPORT_CANCELLED:The file import was cancelled".to_owned());
                    }
                    processed += 1;
                    emit_import_progress(
                        &worker_app,
                        &request_id,
                        "importing",
                        processed,
                        total,
                        path.file_name()
                            .map(|name| name.to_string_lossy().into_owned()),
                    );
                    Ok(())
                },
            )?;
            capture_initial_user_versions(database.inner(), &artifact_store_path(&worker_app)?)?;
            load_snapshot_record(database.inner())
        })();
        if result.is_ok() {
            emit_import_progress(
                &worker_app,
                &request_id,
                "completed",
                completed_total,
                completed_total,
                None,
            );
        }
        clear_import_cancellation(&request_id);
        result
    })
    .await
    .map_err(|error| format!("Unable to run the file import: {error}"))?
}

#[tauri::command]
pub async fn import_file_space_dropped_files<R: tauri::Runtime>(
    request_id: String,
    folder_id: Option<String>,
    files: Vec<FileSpaceDroppedFile>,
    app: tauri::AppHandle<R>,
) -> Result<FileSpaceSnapshot, String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let total_files = files.len();
        let mut completed_total = 0_usize;
        let staging_root =
            std::env::temp_dir().join(format!("lumetrace-file-space-drop-{}", Uuid::new_v4()));
        let result = (|| {
            emit_import_progress(&worker_app, &request_id, "scanning", 0, total_files, None);
            if import_cancelled(&request_id) {
                return Err("IMPORT_CANCELLED:The file import was cancelled".to_owned());
            }
            let roots = stage_dropped_files(&staging_root, &files)?;
            let total = roots.iter().try_fold(0_usize, |total, path| {
                count_import_entries(Path::new(path), &request_id).map(|count| total + count)
            })?;
            completed_total = total;
            emit_import_progress(&worker_app, &request_id, "importing", 0, total, None);
            let database = worker_app.state::<Database>();
            let _operation = lock_file_space_operations()?;
            let mut processed = 0_usize;
            let _snapshot = import_files_record_with_progress(
                database.inner(),
                folder_id.as_deref(),
                &roots,
                |path| {
                    if import_cancelled(&request_id) {
                        return Err("IMPORT_CANCELLED:The file import was cancelled".to_owned());
                    }
                    processed += 1;
                    emit_import_progress(
                        &worker_app,
                        &request_id,
                        "importing",
                        processed,
                        total,
                        path.file_name()
                            .map(|name| name.to_string_lossy().into_owned()),
                    );
                    Ok(())
                },
            )?;
            capture_initial_user_versions(database.inner(), &artifact_store_path(&worker_app)?)?;
            load_snapshot_record(database.inner())
        })();
        let result = if staging_root.exists() {
            cleanup_staged_drop(&staging_root, result)
        } else {
            result
        };
        if result.is_ok() {
            emit_import_progress(
                &worker_app,
                &request_id,
                "completed",
                completed_total,
                completed_total,
                None,
            );
        }
        clear_import_cancellation(&request_id);
        result
    })
    .await
    .map_err(|error| format!("Unable to run the dropped-file import: {error}"))?
}

#[tauri::command]
pub fn cancel_file_space_import(request_id: String) -> Result<(), String> {
    FILE_SPACE_IMPORT_CANCELLATIONS
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .map_err(|_| "Unable to access File Space import cancellation".to_owned())?
        .insert(request_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_workspace(label: &str) -> (PathBuf, PathBuf, PathBuf, Database) {
        let root = std::env::temp_dir().join(format!("lumetrace-{label}-{}", Uuid::new_v4()));
        let storage = root.join("storage");
        let versions = root.join("versions");
        fs::create_dir_all(&storage).unwrap();
        let database = crate::database::open_for_test(&root.join("lumetrace.sqlite3")).unwrap();
        configure_storage_root_record(&database, storage.to_string_lossy().as_ref()).unwrap();
        (root, storage, versions, database)
    }

    #[test]
    fn imported_file_gets_version_history_and_external_edits_are_captured() {
        let (root, storage, versions, database) = test_workspace("version-history");
        let source = root.join("source.md");
        fs::write(&source, b"first version").unwrap();

        import_files_record(&database, None, &[source.to_string_lossy().into_owned()]).unwrap();
        capture_initial_user_versions(&database, &versions).unwrap();

        let first_snapshot = load_snapshot_record(&database).unwrap();
        assert_eq!(first_snapshot.files.len(), 1);
        assert_eq!(first_snapshot.files[0].current_version, Some(1));
        assert_eq!(first_snapshot.files[0].version_count, 1);
        let file_id = first_snapshot.files[0].id.clone();
        let first_timeline = load_task_file_timeline_record(&database, &file_id).unwrap();
        assert_eq!(first_timeline.versions.len(), 1);
        assert_eq!(first_timeline.versions[0].origin, "user_edit");

        fs::write(storage.join("source.md"), b"second version with changes").unwrap();
        reconcile_task_file(&database, &versions, &file_id).unwrap();

        let second_timeline = load_task_file_timeline_record(&database, &file_id).unwrap();
        assert_eq!(second_timeline.versions.len(), 2);
        assert_eq!(second_timeline.versions[0].version_number, 2);
        assert!(second_timeline
            .events
            .iter()
            .any(|event| event.event_type == "modified" && event.actor == "user"));
        assert_eq!(
            read_task_file_version_record(&database, &file_id, &second_timeline.versions[1].id,)
                .unwrap(),
            b"first version"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn full_text_search_finds_versioned_imported_content() {
        let (root, _storage, versions, database) = test_workspace("search");
        let source = root.join("research.txt");
        fs::write(&source, "LumeTrace keeps quoted evidence searchable.").unwrap();

        import_files_record(&database, None, &[source.to_string_lossy().into_owned()]).unwrap();
        capture_initial_user_versions(&database, &versions).unwrap();
        let snapshot = load_snapshot_record(&database).unwrap();
        let matches = search_file_space_records(
            &database,
            &FileSpaceSearchRequest {
                query: "evidence".to_owned(),
                scopes: vec!["content".to_owned()],
            },
        )
        .unwrap();

        assert_eq!(matches, vec![snapshot.files[0].id.clone()]);
        fs::remove_dir_all(root).unwrap();
    }
}
