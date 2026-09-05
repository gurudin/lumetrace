use crate::{
    content_extractor::{
        extract_file_content, ContentExtraction, ExtractionStatus, EXTRACTION_VERSION,
    },
    database::Database,
};
use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use rusqlite::{params, params_from_iter, types::Value, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    fs::{self, OpenOptions},
    io::{Read, Seek, Write},
    panic::{catch_unwind, AssertUnwindSafe},
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering},
        mpsc::channel,
        Mutex, MutexGuard, OnceLock,
    },
    time::{Duration, Instant},
};
use tauri::{Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

const STORAGE_ROOT_SETTING: &str = "file_space.storage_root";
const PENDING_FILE_MOVE_SETTING: &str = "file_space.pending_file_move";
const PENDING_TRASH_OPERATION_SETTING: &str = "file_space.pending_trash_operation";
const TRASH_DIRECTORY_NAME: &str = ".lumetrace-trash";
const TASK_VERSION_PREVIEW_MAX_BYTES: i64 = 100 * 1024 * 1024;
const MARKDOWN_FILE_MAX_BYTES: u64 = 5 * 1024 * 1024;
const MATERIALIZED_FILE_NAME_MAX_BYTES: usize = 180;
const TRASH_RETENTION_MILLIS: i64 = 30 * 24 * 60 * 60 * 1_000;
const EXPIRED_TRASH_PURGE_BATCH_SIZE: usize = 20;
const PURGE_DIRECTORY_NAME: &str = "purging";
const ARTIFACT_PURGE_DIRECTORY_NAME: &str = ".lumetrace-purging";
const BACKUP_FORMAT_VERSION: u32 = 1;
const BACKUP_MANIFEST_MAX_BYTES: u64 = 1024 * 1024;
const FILE_SPACE_WATCH_DEBOUNCE: Duration = Duration::from_millis(1_500);
const FILE_SPACE_WATCH_ROOT_REFRESH: Duration = Duration::from_millis(750);
const FILE_SPACE_VERSION_NOTIFICATION_LIMIT: usize = 100;
const FILE_SPACE_INITIAL_PAGE_LIMIT: usize = 160;
const FILE_SPACE_FILE_PAGE_MAX_LIMIT: usize = 320;
const CONTENT_EXTRACTION_DOCUMENT_PAUSE: Duration = Duration::from_millis(40);
const EDIT_INDEX_REPAIR_BATCH_SIZE: usize = 64;
const EDIT_INDEX_REPAIR_KEY: &str = "file_space.edit_index_repair.v1";
const SEARCH_CONTENT_MIN_WEIGHT: usize = 2;
static FILE_SPACE_OPERATION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static FILE_SPACE_IMPORT_CANCELLATIONS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static FILE_SPACE_SEARCH_GENERATION: AtomicU64 = AtomicU64::new(0);

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
pub struct FileSpaceVersionCreatedNotification {
    pub version_id: String,
    pub file_id: String,
    pub file_name: String,
    pub version_number: i64,
    pub created_at: i64,
}

#[derive(Default)]
pub struct FileSpaceVersionNotificationQueue(Mutex<VecDeque<FileSpaceVersionCreatedNotification>>);

#[derive(Debug, Clone)]
struct FileSpaceWatcherRuntimeState {
    state: String,
    workspace_id: Option<String>,
    root_path: Option<String>,
    last_checked_at: Option<i64>,
    last_event_at: Option<i64>,
    error: Option<String>,
}

impl Default for FileSpaceWatcherRuntimeState {
    fn default() -> Self {
        Self {
            state: "starting".to_owned(),
            workspace_id: None,
            root_path: None,
            last_checked_at: None,
            last_event_at: None,
            error: None,
        }
    }
}

#[derive(Default)]
pub struct FileSpaceBackgroundRuntime {
    paused: AtomicBool,
    watcher: Mutex<FileSpaceWatcherRuntimeState>,
}

impl FileSpaceBackgroundRuntime {
    pub(crate) fn is_paused(&self) -> bool {
        self.paused.load(AtomicOrdering::Relaxed)
    }

    fn set_paused(&self, paused: bool) {
        self.paused.store(paused, AtomicOrdering::SeqCst);
    }

    fn record_watcher_state(
        &self,
        workspace_id: Option<String>,
        root_path: Option<String>,
        state: &str,
        error: Option<String>,
    ) {
        if let Ok(mut watcher) = self.watcher.lock() {
            let workspace_changed = watcher.workspace_id != workspace_id;
            watcher.state = state.to_owned();
            watcher.workspace_id = workspace_id;
            watcher.root_path = root_path;
            watcher.last_checked_at = Some(now_millis());
            watcher.error = error;
            if workspace_changed {
                watcher.last_event_at = None;
            }
        }
    }

    fn record_watcher_event(&self) {
        if let Ok(mut watcher) = self.watcher.lock() {
            watcher.last_event_at = Some(now_millis());
        }
    }

    fn record_watcher_error(&self, error: String) {
        if let Ok(mut watcher) = self.watcher.lock() {
            watcher.state = "failed".to_owned();
            watcher.last_checked_at = Some(now_millis());
            watcher.error = Some(error);
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileSpaceBackgroundPipelineStatus {
    pub state: String,
    pub completed_files: i64,
    pub total_files: i64,
    pub pending_files: i64,
    pub failed_files: i64,
    pub current_file: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileSpaceWatcherStatus {
    pub state: String,
    pub root_path: Option<String>,
    pub last_checked_at: Option<i64>,
    pub last_event_at: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileSpaceBackgroundStatus {
    pub state: String,
    pub paused: bool,
    pub content_index: FileSpaceBackgroundPipelineStatus,
    pub semantic_index: FileSpaceBackgroundPipelineStatus,
    pub semantic_model_installed: bool,
    pub watcher: FileSpaceWatcherStatus,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileSpaceFolder {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub relative_path: String,
    pub manual_order: i64,
    pub direct_file_count: i64,
    pub file_count: i64,
    pub child_folder_count: i64,
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

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileSpaceTrashItem {
    pub id: String,
    pub root_id: String,
    pub item_type: String,
    pub name: String,
    pub original_relative_path: String,
    pub size_bytes: i64,
    pub file_count: i64,
    pub folder_count: i64,
    pub version_count: i64,
    pub trashed_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSpaceSearchRequest {
    pub query: String,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileSpaceSearchMatch {
    pub file_id: String,
    pub lexical_match: bool,
    pub semantic_similarity: Option<f32>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileSpaceFilePageCursor {
    pub sort: String,
    pub number: Option<i64>,
    pub text: Option<String>,
    pub secondary_text: Option<String>,
    pub id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSpaceFilePageRequest {
    pub folder_id: Option<String>,
    pub sort: String,
    pub type_filter: String,
    pub tag_filter: Option<String>,
    pub updated_after: Option<i64>,
    #[serde(default)]
    pub match_ids: Vec<String>,
    pub cursor: Option<FileSpaceFilePageCursor>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileSpaceFilePage {
    pub files: Vec<FileSpaceFile>,
    pub total_count: i64,
    pub next_cursor: Option<FileSpaceFilePageCursor>,
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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileSpaceDroppedFileConflict {
    pub relative_path: String,
    pub file_name: String,
    pub existing_file_id: String,
    pub existing_version: i64,
    pub existing_size_bytes: i64,
    pub incoming_size_bytes: i64,
    pub identical: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileSpaceDroppedFileConflictInspection {
    pub conflicts: Vec<FileSpaceDroppedFileConflict>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DroppedFileConflictAction {
    Rename,
    LatestVersion,
}

impl DroppedFileConflictAction {
    fn parse(value: Option<&str>) -> Result<Option<Self>, String> {
        match value {
            None => Ok(None),
            Some("rename") => Ok(Some(Self::Rename)),
            Some("latestVersion") => Ok(Some(Self::LatestVersion)),
            Some(_) => Err("The selected file conflict action is not supported".to_owned()),
        }
    }
}

#[derive(Debug)]
struct ExistingFolderCandidate {
    id: String,
    parent_id: Option<String>,
    name: String,
    relative_path: String,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug)]
struct ExistingFileCandidate {
    id: String,
    folder_id: Option<String>,
    name: String,
    relative_path: String,
    mime_type: Option<String>,
    size_bytes: i64,
    created_at: i64,
    updated_at: i64,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct PendingTrashOperation {
    version: u32,
    operation_id: String,
    entry_id: String,
    action: String,
    source_relative_path: String,
    destination_relative_path: String,
    #[serde(default)]
    source_device: Option<u64>,
    #[serde(default)]
    source_inode: Option<u64>,
    #[serde(default)]
    source_is_directory: Option<bool>,
    created_at: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileSpaceSnapshot {
    pub root_path: Option<String>,
    pub root_name: Option<String>,
    pub root_status: String,
    pub file_count: i64,
    pub tags: Vec<String>,
    pub folders: Vec<FileSpaceFolder>,
    pub files: Vec<FileSpaceFile>,
    pub trashed_files: Vec<FileSpaceFile>,
    pub trash_items: Vec<FileSpaceTrashItem>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileSpaceBackupResult {
    pub path: String,
    pub workspace_file_count: usize,
    pub archive_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileSpaceCreatedFileResult {
    pub file: FileSpaceFile,
    pub snapshot: FileSpaceSnapshot,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileSpaceBackupInspection {
    pub path: String,
    pub exported_at: i64,
    pub workspace_name: String,
    pub workspace_file_count: usize,
    pub workspace_size_bytes: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileSpaceBackupManifest {
    format_version: u32,
    product: String,
    exported_at: i64,
    workspace: FileSpaceBackupManifestContent,
    versions: FileSpaceBackupManifestContent,
    database: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileSpaceBackupManifestContent {
    name: Option<String>,
    file_count: usize,
    size_bytes: u64,
    path: String,
}

#[derive(Debug, Default)]
struct BackupContentSummary {
    file_count: usize,
    size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileSpaceFileMoveFailure {
    pub file_id: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileSpaceFilesMoveResult {
    pub snapshot: FileSpaceSnapshot,
    pub moved_ids: Vec<String>,
    pub unchanged_ids: Vec<String>,
    pub failed: Vec<FileSpaceFileMoveFailure>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileSpaceTrashPurgeResult {
    pub snapshot: FileSpaceSnapshot,
    pub purged_count: usize,
    pub failed_count: usize,
    pub failure_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PurgeOperationRecord {
    id: String,
    entry_id: String,
    item_type: String,
    phase: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PurgeMemberRecord {
    member_type: String,
    member_id: String,
    depth: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PurgePathRecord {
    path_kind: String,
    base_kind: String,
    original_relative_path: String,
    staged_relative_path: String,
    expected_identity: ManagedFileIdentity,
    expected_is_directory: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct PurgeBatchSummary {
    purged_count: usize,
    failed_count: usize,
    failure_messages: Vec<String>,
    cleanup_warnings: Vec<String>,
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
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
                .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
) -> Result<Option<FileSpaceVersionCreatedNotification>, String> {
    reconcile_task_file_with_options(database, artifact_store, file_id, false)
}

fn reconcile_task_file_with_options(
    database: &Database,
    artifact_store: &Path,
    file_id: &str,
    force_content_check: bool,
) -> Result<Option<FileSpaceVersionCreatedNotification>, String> {
    let root = match require_ready_root(database) {
        Ok(root) => root,
        Err(_) => return Ok(None),
    };
    let record = {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
        return Ok(None);
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
                .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
            return Ok(None);
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
    if !force_content_check && observed_size == Some(size_bytes) && observed_mtime == Some(mtime_ms)
    {
        return Ok(None);
    }
    let (working_sha256, working_size) = sha256_file(&path)?;
    if observed_sha.as_deref() == Some(&working_sha256) {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
        connection
            .execute(
                "UPDATE file_space_artifacts
                 SET observed_size_bytes = ?1, observed_mtime_ms = ?2, updated_at = ?3
                 WHERE file_id = ?4",
                params![working_size, mtime_ms, now_millis(), file_id],
            )
            .map_err(|error| format!("Unable to update Task file observation: {error}"))?;
        return Ok(None);
    }

    let (version_number, now) = {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
        queue_changed_file_index(&transaction, file_id)
            .map_err(|error| format!("Unable to queue edited file indexing: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("Unable to complete Task file edit capture: {error}"))?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(snapshot_path);
        return Err(error);
    }
    Ok(Some(FileSpaceVersionCreatedNotification {
        version_id,
        file_id: file_id.to_owned(),
        file_name: name,
        version_number,
        created_at: now,
    }))
}

fn is_internal_watcher_path(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return true;
    };
    relative.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        name == TRASH_DIRECTORY_NAME
            || name == ARTIFACT_PURGE_DIRECTORY_NAME
            || name.starts_with(".lumetrace-")
    })
}

fn watcher_paths_match_file(
    root: &Path,
    file_path: &Path,
    changed_paths: &HashSet<PathBuf>,
) -> bool {
    let file_parent = file_path.parent();
    changed_paths.iter().any(|changed_path| {
        let requested_path = if changed_path.is_absolute() {
            changed_path.clone()
        } else {
            root.join(changed_path)
        };
        let changed_path = requested_path.canonicalize().unwrap_or_else(|_| {
            requested_path
                .parent()
                .and_then(|parent| parent.canonicalize().ok())
                .and_then(|parent| requested_path.file_name().map(|name| parent.join(name)))
                .unwrap_or(requested_path)
        });
        if is_internal_watcher_path(root, &changed_path) {
            return false;
        }
        changed_path == file_path
            || file_path.starts_with(&changed_path)
            || (changed_path.parent().is_some() && changed_path.parent() == file_parent)
    })
}

fn capture_external_version_changes(
    database: &Database,
    artifact_store: &Path,
    changed_paths: Option<&HashSet<PathBuf>>,
) -> Result<Vec<FileSpaceVersionCreatedNotification>, String> {
    let root = match require_ready_root(database) {
        Ok(root) => root,
        Err(_) => return Ok(Vec::new()),
    };
    let candidates = {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
        let mut statement = connection
            .prepare(
                "SELECT file.id, file.storage_path,
                        artifact.observed_size_bytes, artifact.observed_mtime_ms
                 FROM files file
                 JOIN file_space_artifacts artifact ON artifact.file_id = file.id
                 WHERE file.trashed_at IS NULL AND file.storage_path IS NOT NULL
                 ORDER BY file.updated_at, file.id",
            )
            .map_err(|error| format!("Unable to prepare automatic version tracking: {error}"))?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                ))
            })
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|error| {
                format!("Unable to load files for automatic version tracking: {error}")
            })?
    };

    let mut created = Vec::new();
    for (file_id, relative_path, observed_size, observed_mtime) in candidates {
        let path = physical_path(&root, &relative_path)?;
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) if !metadata.file_type().is_symlink() && metadata.is_file() => metadata,
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "Unable to inspect a file for automatic version tracking: {error}"
                ));
            }
        };
        let event_matched = changed_paths
            .map(|paths| watcher_paths_match_file(&root, &path, paths))
            .unwrap_or(false);
        if changed_paths.is_some() && !event_matched {
            continue;
        }
        if changed_paths.is_none() {
            let size = i64::try_from(metadata.len())
                .map_err(|_| "A tracked file is too large".to_owned())?;
            let mtime = observation_mtime(
                metadata
                    .modified()
                    .map_err(|error| format!("Unable to inspect a tracked file: {error}"))?,
            )?;
            if observed_size == Some(size) && observed_mtime == Some(mtime) {
                continue;
            }
        }
        if let Some(notification) =
            reconcile_task_file_with_options(database, artifact_store, &file_id, event_matched)?
        {
            created.push(notification);
        }
    }
    Ok(created)
}

impl FileSpaceVersionNotificationQueue {
    fn push(&self, notification: FileSpaceVersionCreatedNotification) -> Result<(), String> {
        let mut queue = self
            .0
            .lock()
            .map_err(|_| "Unable to save the automatic version notification".to_owned())?;
        if queue
            .iter()
            .any(|queued| queued.version_id == notification.version_id)
        {
            return Ok(());
        }
        queue.push_back(notification);
        while queue.len() > FILE_SPACE_VERSION_NOTIFICATION_LIMIT {
            queue.pop_front();
        }
        Ok(())
    }

    fn drain(&self) -> Result<Vec<FileSpaceVersionCreatedNotification>, String> {
        let mut queue = self
            .0
            .lock()
            .map_err(|_| "Unable to load automatic version notifications".to_owned())?;
        Ok(queue.drain(..).collect())
    }

    pub(crate) fn clear(&self) -> Result<(), String> {
        let mut queue = self
            .0
            .lock()
            .map_err(|_| "Unable to clear automatic version notifications".to_owned())?;
        queue.clear();
        Ok(())
    }
}

fn available_watcher_root(database: &Database) -> Option<PathBuf> {
    let root = read_storage_root(database).ok().flatten()?;
    let metadata = fs::symlink_metadata(&root).ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return None;
    }
    root.canonicalize().ok()
}

fn watcher_event_affects_content(kind: &EventKind) -> bool {
    !matches!(kind, EventKind::Access(_))
}

fn publish_version_notifications<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    notifications: Vec<FileSpaceVersionCreatedNotification>,
) {
    let queue = app.state::<FileSpaceVersionNotificationQueue>();
    for notification in notifications {
        if queue.push(notification.clone()).is_ok() {
            let _ = app.emit("file-space-version-created", notification);
        }
    }
}

pub(crate) fn start_file_space_watcher<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<(), String> {
    let (sender, receiver) = channel();
    let mut watcher = RecommendedWatcher::new(
        move |event| {
            let _ = sender.send(event);
        },
        Config::default(),
    )
    .map_err(|error| format!("Unable to start automatic version tracking: {error}"))?;

    std::thread::Builder::new()
        .name("lumetrace-file-watcher".to_owned())
        .spawn(move || {
            let mut watched_root: Option<PathBuf> = None;
            let mut pending_paths = HashSet::<PathBuf>::new();
            let mut last_content_event: Option<Instant> = None;
            let mut last_root_refresh = Instant::now()
                .checked_sub(FILE_SPACE_WATCH_ROOT_REFRESH)
                .unwrap_or_else(Instant::now);

            loop {
                if last_root_refresh.elapsed() >= FILE_SPACE_WATCH_ROOT_REFRESH {
                    let (workspace_id, configured_root, next_root) = {
                        let database = app.state::<Database>();
                        (
                            database
                                .location()
                                .ok()
                                .map(|location| location.workspace_id),
                            read_storage_root(database.inner()).ok().flatten(),
                            available_watcher_root(database.inner()),
                        )
                    };
                    if next_root != watched_root {
                        if let Some(root) = watched_root.take() {
                            let _ = watcher.unwatch(&root);
                        }
                        pending_paths.clear();
                        last_content_event = None;
                        if let Some(root) = next_root {
                            match watcher.watch(&root, RecursiveMode::Recursive) {
                                Ok(()) => {
                                    app.state::<FileSpaceBackgroundRuntime>()
                                        .record_watcher_state(
                                            workspace_id,
                                            Some(root.to_string_lossy().to_string()),
                                            "watching",
                                            None,
                                        );
                                    watched_root = Some(root);
                                }
                                Err(error) => {
                                    let error = format!(
                                        "Unable to watch the Lume Trace file workspace: {error}"
                                    );
                                    app.state::<FileSpaceBackgroundRuntime>()
                                        .record_watcher_state(
                                            workspace_id,
                                            configured_root
                                                .map(|root| root.to_string_lossy().to_string()),
                                            "failed",
                                            Some(error.clone()),
                                        );
                                    eprintln!("{error}");
                                }
                            }
                        } else {
                            app.state::<FileSpaceBackgroundRuntime>()
                                .record_watcher_state(
                                    workspace_id,
                                    configured_root.map(|root| root.to_string_lossy().to_string()),
                                    "unavailable",
                                    None,
                                );
                        }
                    } else {
                        app.state::<FileSpaceBackgroundRuntime>()
                            .record_watcher_state(
                                workspace_id,
                                configured_root.map(|root| root.to_string_lossy().to_string()),
                                if watched_root.is_some() {
                                    "watching"
                                } else {
                                    "unavailable"
                                },
                                None,
                            );
                    }
                    last_root_refresh = Instant::now();
                }

                match receiver.recv_timeout(Duration::from_millis(200)) {
                    Ok(Ok(event)) if watcher_event_affects_content(&event.kind) => {
                        if let Some(root) = watched_root.as_deref() {
                            pending_paths.extend(
                                event
                                    .paths
                                    .into_iter()
                                    .filter(|path| !is_internal_watcher_path(root, path)),
                            );
                            if !pending_paths.is_empty() {
                                last_content_event = Some(Instant::now());
                                app.state::<FileSpaceBackgroundRuntime>()
                                    .record_watcher_event();
                            }
                        }
                    }
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => {
                        app.state::<FileSpaceBackgroundRuntime>()
                            .record_watcher_error(error.to_string());
                        eprintln!("Lume Trace file watcher error: {error}");
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        app.state::<FileSpaceBackgroundRuntime>()
                            .record_watcher_error(
                                "The file watcher stopped unexpectedly".to_owned(),
                            );
                        break;
                    }
                }

                if last_content_event
                    .is_some_and(|event_at| event_at.elapsed() >= FILE_SPACE_WATCH_DEBOUNCE)
                {
                    let changed_paths = std::mem::take(&mut pending_paths);
                    last_content_event = None;
                    let notifications = (|| -> Result<Vec<_>, String> {
                        let _operation = lock_file_space_operations()?;
                        let database = app.state::<Database>();
                        let artifact_store = artifact_store_path(&app)?;
                        capture_external_version_changes(
                            database.inner(),
                            &artifact_store,
                            Some(&changed_paths),
                        )
                    })();
                    match notifications {
                        Ok(notifications) => publish_version_notifications(&app, notifications),
                        Err(error) => {
                            eprintln!("Unable to capture an external file version: {error}");
                        }
                    }
                }
            }
        })
        .map_err(|error| format!("Unable to start automatic version tracking: {error}"))?;
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
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
    app.state::<Database>().artifact_store_path()
}

fn backup_entry_name(
    prefix: &str,
    root: &Path,
    path: &Path,
    directory: bool,
) -> Result<String, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| format!("Unable to resolve backup path: {}", path.display()))?;
    let relative = relative.to_string_lossy().replace('\\', "/");
    let mut name = prefix.trim_end_matches('/').to_owned();
    if !relative.is_empty() {
        name.push('/');
        name.push_str(&relative);
    }
    if directory && !name.ends_with('/') {
        name.push('/');
    }
    Ok(name)
}

fn add_backup_tree<W: Write + Seek>(
    archive: &mut ZipWriter<W>,
    source: &Path,
    archive_prefix: &str,
    summary: &mut BackupContentSummary,
) -> Result<(), String> {
    let directory_options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .unix_permissions(0o755);
    let file_options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .large_file(true)
        .unix_permissions(0o644);
    archive
        .add_directory(
            format!("{}/", archive_prefix.trim_end_matches('/')),
            directory_options,
        )
        .map_err(|error| format!("Unable to create the backup archive: {error}"))?;
    if !source.exists() {
        return Ok(());
    }
    if !source.is_dir() {
        return Err(format!(
            "The backup source is not a folder: {}",
            source.display()
        ));
    }

    fn visit<W: Write + Seek>(
        archive: &mut ZipWriter<W>,
        root: &Path,
        current: &Path,
        archive_prefix: &str,
        directory_options: SimpleFileOptions,
        file_options: SimpleFileOptions,
        summary: &mut BackupContentSummary,
    ) -> Result<(), String> {
        let mut entries = fs::read_dir(current)
            .map_err(|error| {
                format!(
                    "Unable to read {} while exporting: {error}",
                    current.display()
                )
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                format!(
                    "Unable to read {} while exporting: {error}",
                    current.display()
                )
            })?;
        entries.sort_by_key(|entry| entry.file_name());

        for entry in entries {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|error| {
                format!(
                    "Unable to inspect {} while exporting: {error}",
                    path.display()
                )
            })?;
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "The backup cannot include symbolic links: {}",
                    path.display()
                ));
            }
            if metadata.is_dir() {
                let name = backup_entry_name(archive_prefix, root, &path, true)?;
                archive
                    .add_directory(name, directory_options)
                    .map_err(|error| format!("Unable to create the backup archive: {error}"))?;
                visit(
                    archive,
                    root,
                    &path,
                    archive_prefix,
                    directory_options,
                    file_options,
                    summary,
                )?;
                continue;
            }
            if !metadata.is_file() {
                return Err(format!(
                    "The backup can only include regular files and folders: {}",
                    path.display()
                ));
            }

            let name = backup_entry_name(archive_prefix, root, &path, false)?;
            archive
                .start_file(name, file_options)
                .map_err(|error| format!("Unable to create the backup archive: {error}"))?;
            let mut file = fs::File::open(&path).map_err(|error| {
                format!("Unable to read {} while exporting: {error}", path.display())
            })?;
            let copied = std::io::copy(&mut file, archive)
                .map_err(|error| format!("Unable to export {}: {error}", path.display()))?;
            if copied != metadata.len() {
                return Err(format!(
                    "The file changed while the backup was being exported: {}",
                    path.display()
                ));
            }
            summary.file_count = summary
                .file_count
                .checked_add(1)
                .ok_or_else(|| "The backup contains too many files".to_owned())?;
            summary.size_bytes = summary
                .size_bytes
                .checked_add(copied)
                .ok_or_else(|| "The backup is too large".to_owned())?;
        }
        Ok(())
    }

    visit(
        archive,
        source,
        source,
        archive_prefix,
        directory_options,
        file_options,
        summary,
    )
}

fn normalize_backup_destination(
    destination_path: &str,
    storage_root: &Path,
) -> Result<PathBuf, String> {
    let destination_path = destination_path.trim();
    if destination_path.is_empty() {
        return Err("Choose a location for the backup first".to_owned());
    }
    let mut requested = PathBuf::from(destination_path);
    if requested
        .extension()
        .and_then(|extension| extension.to_str())
        != Some("lumetrace")
    {
        requested.set_extension("lumetrace");
    }
    let file_name = requested
        .file_name()
        .ok_or_else(|| "The backup name is invalid".to_owned())?;
    let parent = requested
        .parent()
        .ok_or_else(|| "The backup location is invalid".to_owned())?
        .canonicalize()
        .map_err(|error| format!("Unable to access the backup location: {error}"))?;
    if !parent.is_dir() {
        return Err("The backup location is not a folder".to_owned());
    }
    let destination = parent.join(file_name);
    if destination.exists() {
        return Err("A backup with this name already exists".to_owned());
    }
    if destination.starts_with(storage_root) {
        return Err("Save the backup outside the current File Space folder".to_owned());
    }
    Ok(destination)
}

fn reconcile_live_files_for_backup(
    database: &Database,
    artifact_store: &Path,
) -> Result<(), String> {
    let file_ids = {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
        let mut statement = connection
            .prepare(
                "SELECT file_id FROM file_space_artifacts artifact
                 JOIN files file ON file.id = artifact.file_id
                 WHERE file.trashed_at IS NULL AND file.storage_path IS NOT NULL
                 ORDER BY file.created_at, file.id",
            )
            .map_err(|error| format!("Unable to prepare files for backup: {error}"))?;
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|error| format!("Unable to load files for backup: {error}"))?
    };
    for file_id in file_ids {
        reconcile_task_file(database, artifact_store, &file_id)?;
    }
    Ok(())
}

fn export_file_space_backup_record(
    database: &Database,
    artifact_store: &Path,
    destination_path: &str,
) -> Result<FileSpaceBackupResult, String> {
    recover_pending_purge_operations_if_root_ready(database, artifact_store)?;
    materialize_pending_task_files_unlocked(database)?;
    capture_initial_user_versions(database, artifact_store)?;
    reconcile_live_files_for_backup(database, artifact_store)?;

    let storage_root = require_ready_root(database)?;
    let destination = normalize_backup_destination(destination_path, &storage_root)?;
    let parent = destination
        .parent()
        .ok_or_else(|| "The backup location is invalid".to_owned())?;
    let destination_name = destination
        .file_name()
        .ok_or_else(|| "The backup name is invalid".to_owned())?
        .to_string_lossy();
    let temporary_archive = parent.join(format!(".{destination_name}.partial-{}", Uuid::new_v4()));
    let staging = std::env::temp_dir().join(format!("lumetrace-backup-{}", Uuid::new_v4()));
    let staged_database = staging.join("lumetrace.sqlite3");

    let result = (|| {
        fs::create_dir_all(&staging)
            .map_err(|error| format!("Unable to prepare the backup: {error}"))?;
        {
            let connection = database
                .0
                .lock()
                .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
            connection
                .execute("VACUUM INTO ?1", params![staged_database.to_string_lossy()])
                .map_err(|error| format!("Unable to export the local backup index: {error}"))?;
        }
        {
            let staged_connection = Connection::open(&staged_database)
                .map_err(|error| format!("Unable to prepare the backup index: {error}"))?;
            staged_connection
                .execute_batch(
                    "PRAGMA foreign_keys = ON;
                     DELETE FROM app_settings WHERE key = 'ai.cloud_api_key';
                     DELETE FROM file_space_search_chunks;
                     DELETE FROM file_space_index_jobs;
                     VACUUM;",
                )
                .map_err(|error| {
                    format!("Unable to trim derived semantic data from the backup: {error}")
                })?;
        }

        let output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_archive)
            .map_err(|error| format!("Unable to create the backup: {error}"))?;
        let mut archive = ZipWriter::new(output);
        let file_options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .large_file(true)
            .unix_permissions(0o644);
        let mut workspace_summary = BackupContentSummary::default();
        add_backup_tree(
            &mut archive,
            &storage_root,
            "workspace",
            &mut workspace_summary,
        )?;
        let mut version_summary = BackupContentSummary::default();
        add_backup_tree(
            &mut archive,
            artifact_store,
            "versions",
            &mut version_summary,
        )?;

        archive
            .start_file("metadata/lumetrace.sqlite3", file_options)
            .map_err(|error| format!("Unable to create the backup archive: {error}"))?;
        let mut database_file = fs::File::open(&staged_database)
            .map_err(|error| format!("Unable to read the exported local index: {error}"))?;
        std::io::copy(&mut database_file, &mut archive)
            .map_err(|error| format!("Unable to add the local index to the backup: {error}"))?;

        let manifest = serde_json::to_vec_pretty(&json!({
            "formatVersion": 1,
            "product": "Lume Trace",
            "exportedAt": now_millis(),
            "workspace": {
                "name": storage_root.file_name().map(|name| name.to_string_lossy()).unwrap_or_default(),
                "fileCount": workspace_summary.file_count,
                "sizeBytes": workspace_summary.size_bytes,
                "path": "workspace/"
            },
            "versions": {
                "fileCount": version_summary.file_count,
                "sizeBytes": version_summary.size_bytes,
                "path": "versions/"
            },
            "database": "metadata/lumetrace.sqlite3"
        }))
        .map_err(|error| format!("Unable to create the backup manifest: {error}"))?;
        archive
            .start_file("manifest.json", file_options)
            .map_err(|error| format!("Unable to create the backup archive: {error}"))?;
        archive
            .write_all(&manifest)
            .map_err(|error| format!("Unable to write the backup manifest: {error}"))?;
        let output = archive
            .finish()
            .map_err(|error| format!("Unable to finish the backup archive: {error}"))?;
        output
            .sync_all()
            .map_err(|error| format!("Unable to finish writing the backup: {error}"))?;
        fs::rename(&temporary_archive, &destination)
            .map_err(|error| format!("Unable to save the backup: {error}"))?;
        sync_directory(parent)?;
        let archive_size_bytes = fs::metadata(&destination)
            .map_err(|error| format!("Unable to inspect the completed backup: {error}"))?
            .len();
        Ok(FileSpaceBackupResult {
            path: destination.to_string_lossy().into_owned(),
            workspace_file_count: workspace_summary.file_count,
            archive_size_bytes,
        })
    })();

    if temporary_archive.exists() {
        let _ = fs::remove_file(&temporary_archive);
    }
    if staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn canonical_backup_file(path: &str) -> Result<PathBuf, String> {
    let requested = PathBuf::from(path.trim());
    let extension = requested
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    if !matches!(extension.as_deref(), Some("lumetrace" | "lumetrace-backup")) {
        return Err("Choose a Lume Trace backup file".to_owned());
    }
    let canonical = requested
        .canonicalize()
        .map_err(|error| format!("Unable to access the selected backup: {error}"))?;
    let metadata = fs::symlink_metadata(&canonical)
        .map_err(|error| format!("Unable to inspect the selected backup: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("The selected backup is not a regular file".to_owned());
    }
    Ok(canonical)
}

fn read_backup_manifest(
    archive: &mut ZipArchive<fs::File>,
) -> Result<FileSpaceBackupManifest, String> {
    let mut entry = archive
        .by_name("manifest.json")
        .map_err(|_| "The selected file is missing its Lume Trace backup manifest".to_owned())?;
    if entry.size() > BACKUP_MANIFEST_MAX_BYTES {
        return Err("The Lume Trace backup manifest is too large".to_owned());
    }
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Unable to read the Lume Trace backup manifest: {error}"))?;
    let manifest: FileSpaceBackupManifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("The Lume Trace backup manifest is invalid: {error}"))?;
    if manifest.format_version != BACKUP_FORMAT_VERSION {
        return Err(format!(
            "This backup format is not supported (version {})",
            manifest.format_version
        ));
    }
    if manifest.product != "Lume Trace"
        || manifest.workspace.path != "workspace/"
        || manifest.versions.path != "versions/"
        || manifest.database != "metadata/lumetrace.sqlite3"
    {
        return Err("The selected file is not a valid Lume Trace backup".to_owned());
    }
    Ok(manifest)
}

fn inspect_file_space_backup_record(path: &str) -> Result<FileSpaceBackupInspection, String> {
    let canonical = canonical_backup_file(path)?;
    let file = fs::File::open(&canonical)
        .map_err(|error| format!("Unable to open the selected backup: {error}"))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|error| format!("Unable to open the Lume Trace backup: {error}"))?;
    let manifest = read_backup_manifest(&mut archive)?;
    let workspace_name = manifest
        .workspace
        .name
        .as_deref()
        .and_then(|name| validate_name(name).ok())
        .unwrap_or_else(|| "Lume Trace Files".to_owned());
    Ok(FileSpaceBackupInspection {
        path: canonical.to_string_lossy().into_owned(),
        exported_at: manifest.exported_at,
        workspace_name,
        workspace_file_count: manifest.workspace.file_count,
        workspace_size_bytes: manifest.workspace.size_bytes,
    })
}

fn numbered_restored_workspace_path(
    parent: &Path,
    preferred_name: &str,
) -> Result<PathBuf, String> {
    let preferred = Path::new(preferred_name);
    let stem = preferred
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("Lume Trace Files");
    let extension = preferred.extension().and_then(|value| value.to_str());
    for index in 1..=10_000_u32 {
        let name = if index == 1 {
            preferred_name.to_owned()
        } else if let Some(extension) = extension {
            format!("{stem} {index}.{extension}")
        } else {
            format!("{stem} {index}")
        };
        let candidate = parent.join(name);
        match fs::symlink_metadata(&candidate) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(candidate),
            Ok(_) => continue,
            Err(error) => {
                return Err(format!("Unable to inspect the restore location: {error}"));
            }
        }
    }
    Err("Unable to find an available folder name for the restored files".to_owned())
}

fn write_restored_zip_entry<R: Read>(
    entry: &mut R,
    destination: &Path,
    is_directory: bool,
) -> Result<u64, String> {
    if is_directory {
        fs::create_dir_all(destination)
            .map_err(|error| format!("Unable to create restored folder: {error}"))?;
        return Ok(0);
    }
    let parent = destination
        .parent()
        .ok_or_else(|| "The restored file path is invalid".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Unable to create restored folder: {error}"))?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| format!("Unable to create restored file: {error}"))?;
    let size = std::io::copy(entry, &mut output)
        .map_err(|error| format!("Unable to restore file contents: {error}"))?;
    output
        .sync_all()
        .map_err(|error| format!("Unable to finish restoring a file: {error}"))?;
    Ok(size)
}

fn extract_file_space_backup(
    backup_path: &Path,
    workspace_staging: &Path,
    versions_staging: &Path,
    database_staging: &Path,
) -> Result<FileSpaceBackupManifest, String> {
    let file = fs::File::open(backup_path)
        .map_err(|error| format!("Unable to open the selected backup: {error}"))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|error| format!("Unable to open the Lume Trace backup: {error}"))?;
    let manifest = read_backup_manifest(&mut archive)?;
    fs::create_dir_all(workspace_staging)
        .map_err(|error| format!("Unable to prepare the restored workspace: {error}"))?;
    fs::create_dir_all(versions_staging)
        .map_err(|error| format!("Unable to prepare the restored version store: {error}"))?;

    let mut seen_entries = HashSet::<PathBuf>::new();
    let mut database_seen = false;
    let mut workspace_summary = BackupContentSummary::default();
    let mut version_summary = BackupContentSummary::default();
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("Unable to read the backup archive: {error}"))?;
        let path = entry
            .enclosed_name()
            .ok_or_else(|| "The backup contains an unsafe file path".to_owned())?;
        if let Some(mode) = entry.unix_mode() {
            if mode & 0o170000 == 0o120000 {
                return Err("The backup contains an unsupported symbolic link".to_owned());
            }
        }
        if !seen_entries.insert(path.clone()) {
            return Err("The backup contains duplicate file paths".to_owned());
        }
        if path == Path::new("manifest.json") {
            continue;
        }
        if path == Path::new("metadata") && entry.is_dir() {
            continue;
        }
        if path == Path::new("metadata/lumetrace.sqlite3") {
            if entry.is_dir() || database_seen {
                return Err("The backup contains an invalid local index".to_owned());
            }
            write_restored_zip_entry(&mut entry, database_staging, false)?;
            database_seen = true;
            continue;
        }

        let (root, relative, summary) = if let Ok(relative) = path.strip_prefix("workspace") {
            (workspace_staging, relative, &mut workspace_summary)
        } else if let Ok(relative) = path.strip_prefix("versions") {
            (versions_staging, relative, &mut version_summary)
        } else {
            return Err(format!(
                "The backup contains an unsupported entry: {}",
                path.display()
            ));
        };
        if relative.as_os_str().is_empty() {
            if !entry.is_dir() {
                return Err("The backup contains an invalid root entry".to_owned());
            }
            continue;
        }
        let destination = root.join(relative);
        let is_directory = entry.is_dir();
        let size = write_restored_zip_entry(&mut entry, &destination, is_directory)?;
        if !is_directory {
            summary.file_count = summary
                .file_count
                .checked_add(1)
                .ok_or_else(|| "The backup contains too many files".to_owned())?;
            summary.size_bytes = summary
                .size_bytes
                .checked_add(size)
                .ok_or_else(|| "The backup is too large".to_owned())?;
        }
    }
    if !database_seen {
        return Err("The backup is missing its local index".to_owned());
    }
    if workspace_summary.file_count != manifest.workspace.file_count
        || workspace_summary.size_bytes != manifest.workspace.size_bytes
        || version_summary.file_count != manifest.versions.file_count
        || version_summary.size_bytes != manifest.versions.size_bytes
    {
        return Err("The backup contents do not match its manifest".to_owned());
    }
    Ok(manifest)
}

fn validate_restored_database(
    database_path: &Path,
    workspace: &Path,
    versions: &Path,
) -> Result<(), String> {
    let connection =
        Connection::open_with_flags(database_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| format!("Unable to open the restored local index: {error}"))?;
    let integrity = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
        .map_err(|error| format!("Unable to verify the restored local index: {error}"))?;
    if integrity != "ok" {
        return Err("The restored local index failed its integrity check".to_owned());
    }
    {
        let mut statement = connection
            .prepare("PRAGMA foreign_key_check")
            .map_err(|error| format!("Unable to verify backup relationships: {error}"))?;
        if statement
            .query([])
            .and_then(|mut rows| rows.next().map(|row| row.is_some()))
            .map_err(|error| format!("Unable to verify backup relationships: {error}"))?
        {
            return Err("The backup contains inconsistent file relationships".to_owned());
        }
    }

    let versions_to_verify = {
        let mut statement = connection
            .prepare(
                "SELECT id, file_id, sha256, size_bytes
                 FROM file_space_artifact_versions ORDER BY file_id, version_number",
            )
            .map_err(|error| format!("The backup version index is not supported: {error}"))?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|error| format!("Unable to verify backup versions: {error}"))?
    };
    for (version_id, file_id, expected_sha256, expected_size) in versions_to_verify {
        let snapshot = versions.join(&file_id).join(format!("{version_id}.blob"));
        let (sha256, size) = sha256_file(&snapshot)
            .map_err(|error| format!("Unable to verify a restored version: {error}"))?;
        if sha256 != expected_sha256 || size != expected_size {
            return Err(format!(
                "A restored version failed verification: {version_id}"
            ));
        }
    }

    let folders = {
        let mut statement = connection
            .prepare("SELECT relative_path FROM file_space_folders WHERE trashed_at IS NULL")
            .map_err(|error| format!("The backup folder index is not supported: {error}"))?;
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|error| format!("Unable to verify restored folders: {error}"))?
    };
    for relative_path in folders {
        let path = physical_path(workspace, &relative_path)?;
        if !path.is_dir() {
            return Err(format!("A restored folder is missing: {relative_path}"));
        }
    }
    let files = {
        let mut statement = connection
            .prepare(
                "SELECT storage_path FROM files
                 WHERE trashed_at IS NULL AND storage_path IS NOT NULL",
            )
            .map_err(|error| format!("The backup file index is not supported: {error}"))?;
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|error| format!("Unable to verify restored files: {error}"))?
    };
    for relative_path in files {
        let path = physical_path(workspace, &relative_path)?;
        if !path.is_file() {
            return Err(format!("A restored file is missing: {relative_path}"));
        }
    }
    let trash_entries = {
        let mut statement = connection
            .prepare("SELECT payload_relative_path, item_type FROM file_space_trash_entries")
            .map_err(|error| format!("The backup Trash index is not supported: {error}"))?;
        statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|error| format!("Unable to verify restored Trash contents: {error}"))?
    };
    for (relative_path, item_type) in trash_entries {
        let path = physical_path(workspace, &relative_path)?;
        let matches = if item_type == "folder" {
            path.is_dir()
        } else {
            path.is_file()
        };
        if !matches {
            return Err(format!("A restored Trash item is missing: {relative_path}"));
        }
    }
    Ok(())
}

fn install_restored_versions(staged: &Path, active: &Path) -> Result<Option<PathBuf>, String> {
    let parent = active
        .parent()
        .ok_or_else(|| "The local version storage path is invalid".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Unable to prepare local version storage: {error}"))?;
    let rollback = parent.join(format!(".lumetrace-versions-rollback-{}", Uuid::new_v4()));
    let had_active = active.exists();
    if had_active {
        let metadata = fs::symlink_metadata(active)
            .map_err(|error| format!("Unable to inspect local version storage: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("The local version storage is invalid".to_owned());
        }
        fs::rename(active, &rollback)
            .map_err(|error| format!("Unable to preserve current version history: {error}"))?;
    }
    if let Err(error) = fs::rename(staged, active) {
        if had_active {
            let _ = fs::rename(&rollback, active);
        }
        return Err(format!(
            "Unable to install restored version history: {error}"
        ));
    }
    Ok(had_active.then_some(rollback))
}

fn rollback_restored_versions(active: &Path, rollback: Option<&Path>) {
    if active.exists() {
        let _ = fs::remove_dir_all(active);
    }
    if let Some(rollback) = rollback {
        let _ = fs::rename(rollback, active);
    }
}

fn replace_file_space_records_from_backup(
    database: &Database,
    source_database: &Path,
    restored_root: &Path,
    active_versions: &Path,
) -> Result<(), String> {
    let mut connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    connection
        .execute(
            "ATTACH DATABASE ?1 AS restore_source",
            params![source_database.to_string_lossy()],
        )
        .map_err(|error| format!("Unable to attach the restored local index: {error}"))?;
    let restore_result = (|| {
        let transaction = connection
            .transaction()
            .map_err(|error| format!("Unable to begin restoring the local index: {error}"))?;
        transaction
            .execute_batch(
                "PRAGMA defer_foreign_keys = ON;
                 DELETE FROM file_space_purge_paths;
                 DELETE FROM file_space_purge_members;
                 DELETE FROM file_space_purge_operations;
                 DELETE FROM file_space_search_documents;
                 DELETE FROM file_space_file_tags;
                 DELETE FROM file_space_artifact_events;
                 DELETE FROM file_space_artifact_versions;
                 DELETE FROM file_space_artifacts;
                 DELETE FROM file_space_trash_entry_files;
                 DELETE FROM file_space_trash_entry_folders;
                 DELETE FROM file_space_trash_entries;
                 DELETE FROM files;
                 DELETE FROM file_space_folders;

                 INSERT INTO file_space_folders
                   (id, parent_id, name, relative_path, manual_order, created_at, updated_at, trashed_at)
                 SELECT id, parent_id, name, relative_path, manual_order, created_at, updated_at, trashed_at
                 FROM restore_source.file_space_folders;
                 INSERT INTO files
                   (id, original_name, storage_path, mime_type, size_bytes, folder_id,
                    source_kind, manual_order, updated_at, trashed_at, created_at)
                 SELECT id, original_name, storage_path, mime_type, size_bytes, folder_id,
                        source_kind, manual_order, updated_at, trashed_at, created_at
                 FROM restore_source.files;
                 INSERT INTO file_space_artifacts
                   (file_id, task_id, logical_key, current_version_id, observed_size_bytes,
                    observed_mtime_ms, observed_sha256, created_at, updated_at)
                 SELECT file_id, task_id, logical_key, current_version_id, observed_size_bytes,
                        observed_mtime_ms, observed_sha256, created_at, updated_at
                 FROM restore_source.file_space_artifacts;
                 INSERT INTO file_space_artifact_versions
                   (id, file_id, version_number, snapshot_path, sha256, size_bytes, produced_name,
                    mime_type, origin, task_id, task_title, round_number, cell_id, cell_name,
                    produced_at, created_at, source_turn_token, source_artifact_id)
                 SELECT id, file_id, version_number, snapshot_path, sha256, size_bytes, produced_name,
                        mime_type, origin, task_id, task_title, round_number, cell_id, cell_name,
                        produced_at, created_at, source_turn_token, source_artifact_id
                 FROM restore_source.file_space_artifact_versions;
                 INSERT INTO file_space_artifact_events
                   (id, file_id, version_id, event_type, actor, details_json, created_at)
                 SELECT id, file_id, version_id, event_type, actor, details_json, created_at
                 FROM restore_source.file_space_artifact_events;
                 INSERT INTO file_space_file_tags
                   (file_id, tag, normalized_tag, created_at)
                 SELECT file_id, tag, normalized_tag, created_at
                 FROM restore_source.file_space_file_tags;
                 INSERT INTO file_space_trash_entries
                   (id, item_type, root_file_id, root_folder_id, original_parent_id,
                    original_name, original_relative_path, payload_relative_path, trashed_at)
                 SELECT id, item_type, root_file_id, root_folder_id, original_parent_id,
                        original_name, original_relative_path, payload_relative_path, trashed_at
                 FROM restore_source.file_space_trash_entries;
                 INSERT INTO file_space_trash_entry_files (entry_id, file_id)
                 SELECT entry_id, file_id FROM restore_source.file_space_trash_entry_files;
                 INSERT INTO file_space_trash_entry_folders (entry_id, folder_id)
                 SELECT entry_id, folder_id FROM restore_source.file_space_trash_entry_folders;
                 INSERT INTO file_space_search_documents
                   (file_id, file_name, body_text, tag_text, task_text, cell_text,
                    file_updated_at, size_bytes, indexed_at)
                 SELECT file_id, file_name, body_text, tag_text, task_text, cell_text,
                        file_updated_at, size_bytes, indexed_at
                 FROM restore_source.file_space_search_documents;",
            )
            .map_err(|error| format!("Unable to restore the local file index: {error}"))?;

        let restored_versions = {
            let mut statement = transaction
                .prepare("SELECT id, file_id FROM file_space_artifact_versions")
                .map_err(|error| format!("Unable to prepare restored version paths: {error}"))?;
            statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
                .map_err(|error| format!("Unable to load restored version paths: {error}"))?
        };
        for (version_id, file_id) in restored_versions {
            let snapshot_path = active_versions
                .join(file_id)
                .join(format!("{version_id}.blob"));
            transaction
                .execute(
                    "UPDATE file_space_artifact_versions SET snapshot_path = ?1 WHERE id = ?2",
                    params![snapshot_path.to_string_lossy(), version_id],
                )
                .map_err(|error| format!("Unable to update a restored version path: {error}"))?;
        }
        transaction
            .execute("DELETE FROM app_settings WHERE key LIKE 'file_space.%'", [])
            .map_err(|error| format!("Unable to clear previous File Space settings: {error}"))?;
        transaction
            .execute(
                "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3)",
                params![
                    STORAGE_ROOT_SETTING,
                    restored_root.to_string_lossy(),
                    now_millis()
                ],
            )
            .map_err(|error| format!("Unable to save the restored File Space path: {error}"))?;
        let foreign_key_failure = {
            let mut statement = transaction
                .prepare("PRAGMA foreign_key_check")
                .map_err(|error| format!("Unable to verify restored relationships: {error}"))?;
            statement
                .query([])
                .and_then(|mut rows| rows.next().map(|row| row.is_some()))
                .map_err(|error| format!("Unable to verify restored relationships: {error}"))?
        };
        if foreign_key_failure {
            return Err("The restored index contains inconsistent relationships".to_owned());
        }
        transaction
            .commit()
            .map_err(|error| format!("Unable to finish restoring the local index: {error}"))?;
        Ok(())
    })();
    let _ = connection.execute_batch("DETACH DATABASE restore_source");
    restore_result
}

fn restore_file_space_backup_record(
    database: &Database,
    artifact_store: &Path,
    backup_path: &str,
    destination_directory: &str,
) -> Result<FileSpaceSnapshot, String> {
    let backup = canonical_backup_file(backup_path)?;
    let inspection = inspect_file_space_backup_record(backup.to_string_lossy().as_ref())?;
    let destination_parent = PathBuf::from(destination_directory.trim())
        .canonicalize()
        .map_err(|error| format!("Unable to access the restore location: {error}"))?;
    if !destination_parent.is_dir() {
        return Err("The restore location is not a folder".to_owned());
    }
    write_probe(&destination_parent)?;
    if let Ok(current_root) = require_ready_root(database) {
        if destination_parent.starts_with(&current_root) {
            return Err(
                "Choose a restore location outside the current File Space folder".to_owned(),
            );
        }
    }
    let restored_root =
        numbered_restored_workspace_path(&destination_parent, &inspection.workspace_name)?;
    let workspace_staging =
        destination_parent.join(format!(".lumetrace-workspace-restore-{}", Uuid::new_v4()));
    let app_data = artifact_store
        .parent()
        .ok_or_else(|| "Unable to resolve Lume Trace application data".to_owned())?;
    let app_staging = app_data.join(format!(".lumetrace-restore-{}", Uuid::new_v4()));
    let versions_staging = app_staging.join("file-space-versions");
    let database_staging = app_staging.join("lumetrace.sqlite3");

    let result = (|| {
        fs::create_dir_all(&app_staging)
            .map_err(|error| format!("Unable to prepare the restore operation: {error}"))?;
        extract_file_space_backup(
            &backup,
            &workspace_staging,
            &versions_staging,
            &database_staging,
        )?;
        validate_restored_database(&database_staging, &workspace_staging, &versions_staging)?;
        fs::rename(&workspace_staging, &restored_root)
            .map_err(|error| format!("Unable to install the restored files: {error}"))?;
        sync_directory(&destination_parent)?;

        let versions_rollback = match install_restored_versions(&versions_staging, artifact_store) {
            Ok(rollback) => rollback,
            Err(error) => {
                let _ = fs::remove_dir_all(&restored_root);
                return Err(error);
            }
        };
        if let Err(error) = replace_file_space_records_from_backup(
            database,
            &database_staging,
            &restored_root,
            artifact_store,
        ) {
            rollback_restored_versions(artifact_store, versions_rollback.as_deref());
            let _ = fs::remove_dir_all(&restored_root);
            return Err(error);
        }
        if let Some(rollback) = versions_rollback {
            let _ = fs::remove_dir_all(rollback);
        }
        if let Err(error) = crate::semantic_search::schedule_outdated_documents(database) {
            eprintln!("Unable to queue restored files for semantic indexing: {error}");
        }
        load_snapshot_record(database)
    })();

    if workspace_staging.exists() {
        let _ = fs::remove_dir_all(&workspace_staging);
    }
    if app_staging.exists() {
        let _ = fs::remove_dir_all(&app_staging);
    }
    result
}

fn capture_initial_user_versions(database: &Database, artifact_store: &Path) -> Result<(), String> {
    capture_initial_user_versions_with_progress(database, artifact_store, |_, _, _| {})
}

fn capture_initial_user_versions_with_progress<F>(
    database: &Database,
    artifact_store: &Path,
    mut progress: F,
) -> Result<(), String>
where
    F: FnMut(usize, usize, &str),
{
    let root = match require_ready_root(database) {
        Ok(root) => root,
        Err(_) => return Ok(()),
    };
    let candidates = {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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

    let total = candidates.len();
    let mut captured_file_ids = Vec::with_capacity(total);
    for (index, (file_id, name, relative_path, mime_type, created_at)) in
        candidates.into_iter().enumerate()
    {
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
                .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
        progress(index + 1, total, &name);
        captured_file_ids.push(file_id);
    }
    synchronize_file_search_index_for_files(database, Some(&root), &captured_file_ids)
}

fn read_storage_root(database: &Database) -> Result<Option<PathBuf>, String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
    file_updated_at: i64,
    size_bytes: i64,
    tag_text: String,
    task_text: String,
    cell_text: String,
    indexed_file_name: Option<String>,
    indexed_body_text: Option<String>,
    indexed_extraction_status: Option<String>,
    indexed_extraction_error: Option<String>,
    indexed_extraction_version: Option<i64>,
    indexed_tag_text: Option<String>,
    indexed_task_text: Option<String>,
    indexed_cell_text: Option<String>,
    indexed_file_updated_at: Option<i64>,
    indexed_size_bytes: Option<i64>,
}

fn failed_content_extraction(error: impl Into<String>) -> ContentExtraction {
    ContentExtraction {
        text: String::new(),
        status: ExtractionStatus::Failed,
        error: Some(error.into()),
    }
}

fn protected_content_extraction(extract: impl FnOnce() -> ContentExtraction) -> ContentExtraction {
    catch_unwind(AssertUnwindSafe(extract)).unwrap_or_else(|_| {
        failed_content_extraction(
            "Content extraction stopped unexpectedly; other files will continue processing",
        )
    })
}

fn read_searchable_text(root: &Path, relative_path: &str, name: &str) -> ContentExtraction {
    let Ok(canonical_root) = root.canonicalize() else {
        return failed_content_extraction(
            "Unable to resolve the File Space root for content extraction",
        );
    };
    let Ok(candidate) = physical_path(root, relative_path) else {
        return failed_content_extraction("Unable to resolve the file path for content extraction");
    };
    let Ok(metadata) = fs::symlink_metadata(&candidate) else {
        return failed_content_extraction("Unable to inspect the file for content extraction");
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return failed_content_extraction("Only regular files can be indexed for content search");
    }
    let Ok(canonical_file) = candidate.canonicalize() else {
        return failed_content_extraction("Unable to resolve the file for content extraction");
    };
    if canonical_file == canonical_root || !canonical_file.starts_with(&canonical_root) {
        return failed_content_extraction("The content extraction target is outside File Space");
    }
    extract_file_content(&canonical_file, name)
}

#[derive(Debug)]
struct ContentExtractionJob {
    file_id: String,
    file_name: String,
    relative_path: String,
    indexed_at: i64,
    extraction_version: i64,
}

fn claim_next_content_extraction(
    database: &Database,
) -> Result<Option<ContentExtractionJob>, String> {
    let mut connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Unable to begin content extraction: {error}"))?;
    let job = transaction
        .query_row(
            "SELECT documents.file_id, documents.file_name, files.storage_path,
                    documents.indexed_at, documents.extraction_version
             FROM file_space_search_documents documents
             JOIN files ON files.id = documents.file_id
             WHERE documents.extraction_status = 'pending'
               AND documents.extraction_version = ?1
               AND files.trashed_at IS NULL AND files.storage_path IS NOT NULL
             ORDER BY documents.indexed_at, documents.file_id
             LIMIT 1",
            [EXTRACTION_VERSION],
            |row| {
                Ok(ContentExtractionJob {
                    file_id: row.get(0)?,
                    file_name: row.get(1)?,
                    relative_path: row.get(2)?,
                    indexed_at: row.get(3)?,
                    extraction_version: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("Unable to select content extraction work: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("Unable to start content extraction: {error}"))?;
    Ok(job)
}

fn process_next_content_extraction(database: &Database) -> Result<bool, String> {
    let Some(root) = read_storage_root(database)? else {
        return Ok(false);
    };
    if inspect_root(Some(&root)) != "ready" {
        return Ok(false);
    }
    let Some(job) = claim_next_content_extraction(database)? else {
        return Ok(false);
    };
    let extraction = protected_content_extraction(|| {
        read_searchable_text(&root, &job.relative_path, &job.file_name)
    });
    let updated = {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
        connection
            .execute(
                "UPDATE file_space_search_documents
                 SET body_text = ?1, extraction_status = ?2, extraction_error = ?3
                 WHERE file_id = ?4 AND indexed_at = ?5
                   AND extraction_version = ?6 AND extraction_status = 'pending'",
                params![
                    extraction.text,
                    extraction.status.as_str(),
                    extraction.error,
                    job.file_id,
                    job.indexed_at,
                    job.extraction_version,
                ],
            )
            .map_err(|error| format!("Unable to save extracted file content: {error}"))?
    };
    if updated > 0 {
        crate::semantic_search::schedule_search_documents(database, &[job.file_id])?;
    }
    Ok(true)
}

pub fn start_file_content_extractor(app: tauri::AppHandle) -> Result<(), String> {
    std::thread::Builder::new()
        .name("lumetrace-content-extractor".to_owned())
        .spawn(move || loop {
            let background = app.state::<FileSpaceBackgroundRuntime>();
            if background.is_paused() {
                std::thread::sleep(Duration::from_millis(250));
                continue;
            }
            let database = app.state::<Database>();
            let result = lock_file_space_operations().and_then(|_operation| {
                // Repair old omissions in bounded metadata-only batches. Never
                // put a workspace-wide body scan on startup or search paths.
                let repaired = repair_missed_edit_indexes_batch(database.inner())?;
                let extracted = process_next_content_extraction(database.inner())?;
                Ok(repaired || extracted)
            });
            match result {
                Ok(true) => std::thread::sleep(CONTENT_EXTRACTION_DOCUMENT_PAUSE),
                Ok(false) => std::thread::sleep(Duration::from_millis(500)),
                Err(error) => {
                    eprintln!("Unable to extract Lume Trace file content: {error}");
                    std::thread::sleep(Duration::from_millis(700));
                }
            }
        })
        .map(|_| ())
        .map_err(|error| format!("Unable to start file content extraction: {error}"))
}

fn content_index_status(
    database: &Database,
    paused: bool,
) -> Result<FileSpaceBackgroundPipelineStatus, String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let (total, completed, pending, failed, current_file, error) = connection
        .query_row(
            "SELECT
               COUNT(*),
               COALESCE(SUM(CASE WHEN extraction_status NOT IN ('pending', 'failed') THEN 1 ELSE 0 END), 0),
               COALESCE(SUM(CASE WHEN extraction_status = 'pending' THEN 1 ELSE 0 END), 0),
               COALESCE(SUM(CASE WHEN extraction_status = 'failed' THEN 1 ELSE 0 END), 0),
               (SELECT file_name FROM file_space_search_documents
                WHERE extraction_status = 'pending'
                ORDER BY indexed_at, file_id LIMIT 1),
               (SELECT extraction_error FROM file_space_search_documents
                WHERE extraction_status = 'failed' AND extraction_error IS NOT NULL
                ORDER BY indexed_at DESC, file_id LIMIT 1)
             FROM file_space_search_documents",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            },
        )
        .map_err(|error| format!("Unable to load content indexing status: {error}"))?;
    let state = if paused && pending > 0 {
        "paused"
    } else if pending > 0 {
        "running"
    } else if failed > 0 {
        "failed"
    } else {
        "ready"
    };
    Ok(FileSpaceBackgroundPipelineStatus {
        state: state.to_owned(),
        completed_files: completed,
        total_files: total,
        pending_files: pending,
        failed_files: failed,
        current_file,
        error,
    })
}

fn background_status<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<FileSpaceBackgroundStatus, String> {
    let database = app.state::<Database>();
    let background = app.state::<FileSpaceBackgroundRuntime>();
    let semantic_runtime = app.state::<crate::semantic_search::SemanticSearchRuntime>();
    let paused = background.is_paused();
    let content_index = content_index_status(database.inner(), paused)?;
    let semantic =
        crate::semantic_search::status_record(database.inner(), semantic_runtime.inner());
    let semantic_state = if !semantic.installed {
        if matches!(semantic.state.as_str(), "downloading" | "validating") {
            "preparing"
        } else {
            "disabled"
        }
    } else if paused && semantic.pending_files > 0 {
        "paused"
    } else if semantic.pending_files > 0 || semantic.current_file.is_some() {
        "running"
    } else if semantic.failed_files > 0 {
        "failed"
    } else {
        "ready"
    };
    let semantic_index = FileSpaceBackgroundPipelineStatus {
        state: semantic_state.to_owned(),
        completed_files: semantic.indexed_files,
        total_files: semantic.total_files,
        pending_files: semantic.pending_files,
        failed_files: semantic.failed_files,
        current_file: semantic.current_file,
        error: semantic.error,
    };
    let workspace_id = database.location()?.workspace_id;
    let watcher = background
        .watcher
        .lock()
        .map_err(|_| "Unable to load file watcher status".to_owned())?
        .clone();
    let watcher = if watcher.workspace_id.as_deref() == Some(&workspace_id) {
        FileSpaceWatcherStatus {
            state: watcher.state,
            root_path: watcher.root_path,
            last_checked_at: watcher.last_checked_at,
            last_event_at: watcher.last_event_at,
            error: watcher.error,
        }
    } else {
        FileSpaceWatcherStatus {
            state: "starting".to_owned(),
            root_path: None,
            last_checked_at: None,
            last_event_at: None,
            error: None,
        }
    };
    let state = if paused {
        "paused"
    } else if content_index.failed_files > 0
        || semantic_index.failed_files > 0
        || matches!(watcher.state.as_str(), "failed" | "unavailable")
    {
        "attention"
    } else if content_index.pending_files > 0 || semantic_index.state == "running" {
        "running"
    } else {
        "ready"
    };
    Ok(FileSpaceBackgroundStatus {
        state: state.to_owned(),
        paused,
        content_index,
        semantic_index,
        semantic_model_installed: semantic.installed,
        watcher,
        updated_at: now_millis(),
    })
}

fn retry_background_failures(database: &Database) -> Result<(), String> {
    {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
        connection
            .execute(
                "UPDATE file_space_search_documents
                 SET extraction_status = 'pending', extraction_error = NULL,
                     extraction_version = ?1
                 WHERE extraction_status = 'failed'",
                [EXTRACTION_VERSION],
            )
            .map_err(|error| format!("Unable to retry content extraction: {error}"))?;
    }
    crate::semantic_search::retry_failed_index_jobs(database)
}

#[tauri::command]
pub async fn get_file_space_background_status<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<FileSpaceBackgroundStatus, String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || background_status(&worker_app))
        .await
        .map_err(|error| format!("Unable to inspect background tasks: {error}"))?
}

#[tauri::command]
pub async fn set_file_space_background_paused<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    paused: bool,
) -> Result<FileSpaceBackgroundStatus, String> {
    app.state::<FileSpaceBackgroundRuntime>().set_paused(paused);
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || background_status(&worker_app))
        .await
        .map_err(|error| format!("Unable to update background tasks: {error}"))?
}

#[tauri::command]
pub async fn retry_file_space_background_failures<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<FileSpaceBackgroundStatus, String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let database = worker_app.state::<Database>();
        let _operation = lock_file_space_operations()?;
        retry_background_failures(database.inner())?;
        background_status(&worker_app)
    })
    .await
    .map_err(|error| format!("Unable to retry background tasks: {error}"))?
}

/// Persist index invalidation in the SAME transaction as the file/version edit.
/// This only updates one file's metadata and queue; extraction/embedding remain
/// in the existing throttled background workers. Stale chunks are excluded by
/// their document_indexed_at until the worker replaces them.
fn queue_changed_file_index(
    transaction: &rusqlite::Transaction<'_>,
    file_id: &str,
) -> rusqlite::Result<()> {
    let requested_at = now_millis();
    transaction.execute(
        "INSERT INTO file_space_search_documents
         (file_id, file_name, body_text, extraction_status, extraction_error,
          extraction_version, tag_text, task_text, cell_text, file_updated_at, size_bytes, indexed_at)
         SELECT f.id, f.original_name, '', 'pending', NULL, ?2,
                COALESCE((SELECT group_concat(tag, char(31)) FROM file_space_file_tags WHERE file_id = f.id), ''),
                COALESCE(d.task_text, v.task_title, ''), COALESCE(d.cell_text, v.cell_name, ''),
                f.updated_at, COALESCE(f.size_bytes, 0), ?3
         FROM files f
         LEFT JOIN file_space_search_documents d ON d.file_id = f.id
         LEFT JOIN file_space_artifacts a ON a.file_id = f.id
         LEFT JOIN file_space_artifact_versions v ON v.id = a.current_version_id
         WHERE f.id = ?1 AND f.trashed_at IS NULL AND f.storage_path IS NOT NULL
         ON CONFLICT(file_id) DO UPDATE SET
           file_name = excluded.file_name, body_text = '', extraction_status = 'pending',
           extraction_error = NULL, extraction_version = excluded.extraction_version,
           tag_text = excluded.tag_text, task_text = excluded.task_text, cell_text = excluded.cell_text,
           file_updated_at = excluded.file_updated_at, size_bytes = excluded.size_bytes,
           indexed_at = MAX(file_space_search_documents.indexed_at + 1, excluded.indexed_at)",
        params![file_id, EXTRACTION_VERSION, requested_at],
    )?;
    transaction.execute(
        "INSERT INTO file_space_index_jobs
         (file_id, requested_document_indexed_at, status, retry_count, error,
          requested_at, started_at, completed_at)
         SELECT d.file_id, d.indexed_at, 'pending', 0, NULL, ?2, NULL, NULL
         FROM file_space_search_documents d JOIN files f ON f.id = d.file_id
         WHERE d.file_id = ?1 AND f.trashed_at IS NULL AND f.storage_path IS NOT NULL
         ON CONFLICT(file_id) DO UPDATE SET
           requested_document_indexed_at = excluded.requested_document_indexed_at,
           status = 'pending', retry_count = 0, error = NULL,
           requested_at = excluded.requested_at, started_at = NULL, completed_at = NULL",
        params![file_id, requested_at],
    )?;
    Ok(())
}

/// One-time, resumable repair for files missed by older create/edit handlers.
/// Keyset-page before joining metadata, so even a healthy million-file workspace
/// never has to scan all rows looking for one missing document in a single tick.
fn repair_missed_edit_indexes_batch(database: &Database) -> Result<bool, String> {
    let Some(root) = read_storage_root(database)? else {
        return Ok(false);
    };
    if inspect_root(Some(&root)) != "ready" {
        return Ok(false);
    }
    let mut connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Unable to begin index repair: {error}"))?;
    let cursor: String = transaction
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [EDIT_INDEX_REPAIR_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Unable to read index repair progress: {error}"))?
        .unwrap_or_default();
    if cursor == "done" {
        return Ok(false);
    }
    let candidates = {
        let mut statement = transaction
            .prepare(
                "SELECT f.id, f.trashed_at IS NULL AND f.storage_path IS NOT NULL AND (
                 d.file_id IS NULL OR d.file_name <> f.original_name
                 OR d.file_updated_at <> f.updated_at OR d.size_bytes <> COALESCE(f.size_bytes, 0)
                 OR d.extraction_version <> ?3)
             FROM (SELECT id, original_name, storage_path, trashed_at, updated_at, size_bytes
                   FROM files WHERE id > ?1 ORDER BY id LIMIT ?2) f
             LEFT JOIN file_space_search_documents d ON d.file_id = f.id ORDER BY f.id",
            )
            .map_err(|error| format!("Unable to prepare index repair: {error}"))?;
        statement
            .query_map(
                params![cursor, EDIT_INDEX_REPAIR_BATCH_SIZE, EXTRACTION_VERSION],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)),
            )
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|error| format!("Unable to read index repair candidates: {error}"))?
    };
    for (file_id, stale) in &candidates {
        if *stale {
            queue_changed_file_index(&transaction, file_id)
                .map_err(|error| format!("Unable to repair a file index: {error}"))?;
        }
    }
    let next = if candidates.len() < EDIT_INDEX_REPAIR_BATCH_SIZE {
        "done"
    } else {
        candidates
            .last()
            .map(|row| row.0.as_str())
            .unwrap_or("done")
    };
    transaction
        .execute(
            "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![EDIT_INDEX_REPAIR_KEY, next, now_millis()],
        )
        .map_err(|error| format!("Unable to save index repair progress: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("Unable to commit index repair: {error}"))?;
    Ok(!candidates.is_empty())
}

fn synchronize_file_search_index_for_files(
    database: &Database,
    root: Option<&Path>,
    file_ids: &[String],
) -> Result<(), String> {
    for chunk in file_ids.chunks(320) {
        synchronize_file_search_index_scope(database, root, Some(chunk))?;
    }
    Ok(())
}

fn synchronize_file_search_index_scope(
    database: &Database,
    root: Option<&Path>,
    file_ids: Option<&[String]>,
) -> Result<(), String> {
    let Some(root) = root else {
        return Ok(());
    };
    if inspect_root(Some(root)) != "ready" {
        return Ok(());
    }
    if file_ids.is_some_and(<[String]>::is_empty) {
        return Ok(());
    }
    let candidates = {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
        let mut values = Vec::<Value>::new();
        let id_filter = file_ids
            .map(|ids| {
                let placeholders = ids
                    .iter()
                    .map(|id| push_sql_value(&mut values, id.clone()))
                    .collect::<Vec<_>>();
                format!(" AND f.id IN ({})", placeholders.join(", "))
            })
            .unwrap_or_default();
        let mut statement = connection
            .prepare(&format!(
                "SELECT f.id, f.original_name, f.updated_at,
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
                        search.file_name, search.body_text, search.extraction_status,
                        search.extraction_error, search.extraction_version, search.tag_text,
                        search.task_text, search.cell_text, search.file_updated_at, search.size_bytes
                 FROM files f
                 LEFT JOIN file_space_search_documents search ON search.file_id = f.id
                 WHERE f.trashed_at IS NULL AND f.storage_path IS NOT NULL{id_filter}",
            ))
            .map_err(|error| format!("Unable to prepare File Space search index: {error}"))?;
        statement
            .query_map(params_from_iter(values), |row| {
                Ok(SearchDocumentCandidate {
                    file_id: row.get(0)?,
                    file_name: row.get(1)?,
                    file_updated_at: row.get(2)?,
                    size_bytes: row.get(3)?,
                    tag_text: row.get(4)?,
                    task_text: row.get(5)?,
                    cell_text: row.get(6)?,
                    indexed_file_name: row.get(7)?,
                    indexed_body_text: row.get(8)?,
                    indexed_extraction_status: row.get(9)?,
                    indexed_extraction_error: row.get(10)?,
                    indexed_extraction_version: row.get(11)?,
                    indexed_tag_text: row.get(12)?,
                    indexed_task_text: row.get(13)?,
                    indexed_cell_text: row.get(14)?,
                    indexed_file_updated_at: row.get(15)?,
                    indexed_size_bytes: row.get(16)?,
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
            || candidate.indexed_size_bytes != Some(candidate.size_bytes)
            || candidate.indexed_extraction_version != Some(EXTRACTION_VERSION);
        let metadata_changed = candidate.indexed_tag_text.as_deref()
            != Some(candidate.tag_text.as_str())
            || candidate.indexed_task_text.as_deref() != Some(candidate.task_text.as_str())
            || candidate.indexed_cell_text.as_deref() != Some(candidate.cell_text.as_str());
        if !content_changed && !metadata_changed {
            continue;
        }
        let extraction = if content_changed {
            ContentExtraction {
                text: String::new(),
                status: ExtractionStatus::Pending,
                error: None,
            }
        } else {
            ContentExtraction {
                text: candidate.indexed_body_text.clone().unwrap_or_default(),
                status: match candidate.indexed_extraction_status.as_deref() {
                    Some("pending") => ExtractionStatus::Pending,
                    Some("extracted") => ExtractionStatus::Extracted,
                    Some("empty") => ExtractionStatus::Empty,
                    Some("unsupported") => ExtractionStatus::Unsupported,
                    Some("failed") => ExtractionStatus::Failed,
                    _ => ExtractionStatus::Empty,
                },
                error: candidate.indexed_extraction_error.clone(),
            }
        };
        let should_schedule_semantic = !content_changed
            && !matches!(
                extraction.status,
                ExtractionStatus::Pending | ExtractionStatus::Failed
            );
        updates.push((candidate, extraction, should_schedule_semantic));
    }

    let semantic_file_ids = updates
        .iter()
        .filter(|(_, _, should_schedule)| *should_schedule)
        .map(|(candidate, _, _)| candidate.file_id.clone())
        .collect::<Vec<_>>();
    let mut connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Unable to begin File Space search indexing: {error}"))?;
    if file_ids.is_none() {
        transaction
            .execute(
                "DELETE FROM file_space_search_documents
                 WHERE file_id NOT IN (
                   SELECT id FROM files WHERE trashed_at IS NULL AND storage_path IS NOT NULL
                 )",
                [],
            )
            .map_err(|error| {
                format!("Unable to remove stale File Space search records: {error}")
            })?;
    }
    for (candidate, extraction, _) in updates {
        transaction
            .execute(
                "INSERT INTO file_space_search_documents
                 (file_id, file_name, body_text, extraction_status, extraction_error,
                  extraction_version, tag_text, task_text, cell_text,
                  file_updated_at, size_bytes, indexed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(file_id) DO UPDATE SET
                   file_name = excluded.file_name,
                   body_text = excluded.body_text,
                   extraction_status = excluded.extraction_status,
                   extraction_error = excluded.extraction_error,
                   extraction_version = excluded.extraction_version,
                   tag_text = excluded.tag_text,
                   task_text = excluded.task_text,
                   cell_text = excluded.cell_text,
                   file_updated_at = excluded.file_updated_at,
                   size_bytes = excluded.size_bytes,
                   indexed_at = MAX(file_space_search_documents.indexed_at + 1, excluded.indexed_at)",
                params![
                    candidate.file_id,
                    candidate.file_name,
                    extraction.text,
                    extraction.status.as_str(),
                    extraction.error,
                    EXTRACTION_VERSION,
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
        .map_err(|error| format!("Unable to complete File Space search indexing: {error}"))?;
    drop(connection);
    crate::semantic_search::schedule_search_documents(database, &semantic_file_ids)
}

fn parse_file_tags(value: String) -> Vec<String> {
    if value.is_empty() {
        Vec::new()
    } else {
        value.split('\u{1f}').map(str::to_owned).collect()
    }
}

fn normalized_file_page_sort(sort: &str) -> &'static str {
    match sort {
        "manual" => "manual",
        "updatedAsc" => "updatedAsc",
        "nameAsc" => "nameAsc",
        "nameDesc" => "nameDesc",
        "sizeDesc" => "sizeDesc",
        "typeAsc" => "typeAsc",
        _ => "updatedDesc",
    }
}

fn push_sql_value(values: &mut Vec<Value>, value: impl Into<Value>) -> String {
    values.push(value.into());
    format!("?{}", values.len())
}

fn file_page_cursor(file: &FileSpaceFile, sort: &str) -> FileSpaceFilePageCursor {
    let lower_name = file.name.to_lowercase();
    match sort {
        "manual" => FileSpaceFilePageCursor {
            sort: sort.to_owned(),
            number: Some(file.manual_order),
            text: Some(lower_name),
            secondary_text: None,
            id: file.id.clone(),
        },
        "updatedAsc" | "updatedDesc" => FileSpaceFilePageCursor {
            sort: sort.to_owned(),
            number: Some(file.updated_at),
            text: None,
            secondary_text: None,
            id: file.id.clone(),
        },
        "nameAsc" | "nameDesc" => FileSpaceFilePageCursor {
            sort: sort.to_owned(),
            number: None,
            text: Some(lower_name),
            secondary_text: None,
            id: file.id.clone(),
        },
        "sizeDesc" => FileSpaceFilePageCursor {
            sort: sort.to_owned(),
            number: Some(file.size_bytes),
            text: None,
            secondary_text: None,
            id: file.id.clone(),
        },
        "typeAsc" => FileSpaceFilePageCursor {
            sort: sort.to_owned(),
            number: None,
            text: Some(file.mime_type.clone().unwrap_or_default().to_lowercase()),
            secondary_text: Some(lower_name),
            id: file.id.clone(),
        },
        _ => unreachable!("file page sort is normalized"),
    }
}

fn list_file_space_file_page_record(
    database: &Database,
    request: &FileSpaceFilePageRequest,
) -> Result<FileSpaceFilePage, String> {
    let sort = normalized_file_page_sort(&request.sort);
    if request
        .cursor
        .as_ref()
        .is_some_and(|cursor| cursor.sort != sort)
    {
        return Err("The file page cursor does not match the requested sort".to_owned());
    }
    let limit = request
        .limit
        .unwrap_or(FILE_SPACE_INITIAL_PAGE_LIMIT)
        .clamp(1, FILE_SPACE_FILE_PAGE_MAX_LIMIT);
    let mut conditions = vec![
        "f.trashed_at IS NULL".to_owned(),
        "f.storage_path IS NOT NULL".to_owned(),
    ];
    let mut values = Vec::<Value>::new();
    if let Some(folder_id) = request.folder_id.as_deref() {
        let placeholder = push_sql_value(&mut values, folder_id.to_owned());
        conditions.push(format!("f.folder_id = {placeholder}"));
    }
    if !request.match_ids.is_empty() {
        let mut unique = request.match_ids.clone();
        unique.sort();
        unique.dedup();
        if unique.len() > 500 {
            return Err("A file page can contain at most 500 matched identifiers".to_owned());
        }
        let placeholders = unique
            .into_iter()
            .map(|id| push_sql_value(&mut values, id))
            .collect::<Vec<_>>();
        conditions.push(format!("f.id IN ({})", placeholders.join(", ")));
    }
    match request.type_filter.as_str() {
        "image" => conditions.push("COALESCE(f.mime_type, '') LIKE 'image/%'".to_owned()),
        "sheet" => conditions.push(
            "(lower(COALESCE(f.mime_type, '')) LIKE '%spreadsheet%'
              OR lower(COALESCE(f.mime_type, '')) LIKE '%excel%'
              OR lower(f.original_name) LIKE '%.csv'
              OR lower(f.original_name) LIKE '%.xls'
              OR lower(f.original_name) LIKE '%.xlsx')"
                .to_owned(),
        ),
        "document" => conditions.push(
            "(COALESCE(f.mime_type, '') LIKE 'text/%'
              OR lower(COALESCE(f.mime_type, '')) LIKE '%pdf%'
              OR lower(COALESCE(f.mime_type, '')) LIKE '%word%'
              OR lower(f.original_name) LIKE '%.md'
              OR lower(f.original_name) LIKE '%.markdown'
              OR lower(f.original_name) LIKE '%.txt'
              OR lower(f.original_name) LIKE '%.doc'
              OR lower(f.original_name) LIKE '%.docx'
              OR lower(f.original_name) LIKE '%.pdf'
              OR lower(f.original_name) LIKE '%.ppt'
              OR lower(f.original_name) LIKE '%.pptx')"
                .to_owned(),
        ),
        _ => {}
    }
    if let Some(tag) = request
        .tag_filter
        .as_deref()
        .map(str::trim)
        .filter(|tag| !tag.is_empty() && *tag != "all")
    {
        let placeholder = push_sql_value(&mut values, tag.to_lowercase());
        conditions.push(format!(
            "EXISTS (
               SELECT 1 FROM file_space_file_tags page_tag
               WHERE page_tag.file_id = f.id AND page_tag.normalized_tag = {placeholder}
             )"
        ));
    }
    if let Some(updated_after) = request.updated_after {
        let placeholder = push_sql_value(&mut values, updated_after);
        conditions.push(format!("f.updated_at >= {placeholder}"));
    }
    let base_where = conditions.join(" AND ");
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let total_count = connection
        .query_row(
            &format!("SELECT COUNT(*) FROM files f WHERE {base_where}"),
            params_from_iter(values.clone()),
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("Unable to count File Space files: {error}"))?;

    if let Some(cursor) = request.cursor.as_ref() {
        let id = push_sql_value(&mut values, cursor.id.clone());
        let cursor_condition = match sort {
            "manual" => {
                let number = push_sql_value(
                    &mut values,
                    cursor
                        .number
                        .ok_or_else(|| "The manual-order cursor is incomplete".to_owned())?,
                );
                let text = push_sql_value(
                    &mut values,
                    cursor
                        .text
                        .clone()
                        .ok_or_else(|| "The manual-order cursor is incomplete".to_owned())?,
                );
                format!(
                    "(f.manual_order > {number}
                      OR (f.manual_order = {number} AND lower(f.original_name) > {text})
                      OR (f.manual_order = {number} AND lower(f.original_name) = {text} AND f.id > {id}))"
                )
            }
            "updatedAsc" => {
                let number = push_sql_value(
                    &mut values,
                    cursor
                        .number
                        .ok_or_else(|| "The updated-time cursor is incomplete".to_owned())?,
                );
                format!("(f.updated_at > {number} OR (f.updated_at = {number} AND f.id > {id}))")
            }
            "updatedDesc" => {
                let number = push_sql_value(
                    &mut values,
                    cursor
                        .number
                        .ok_or_else(|| "The updated-time cursor is incomplete".to_owned())?,
                );
                format!("(f.updated_at < {number} OR (f.updated_at = {number} AND f.id > {id}))")
            }
            "nameAsc" => {
                let text = push_sql_value(
                    &mut values,
                    cursor
                        .text
                        .clone()
                        .ok_or_else(|| "The name cursor is incomplete".to_owned())?,
                );
                format!(
                    "(lower(f.original_name) > {text}
                      OR (lower(f.original_name) = {text} AND f.id > {id}))"
                )
            }
            "nameDesc" => {
                let text = push_sql_value(
                    &mut values,
                    cursor
                        .text
                        .clone()
                        .ok_or_else(|| "The name cursor is incomplete".to_owned())?,
                );
                format!(
                    "(lower(f.original_name) < {text}
                      OR (lower(f.original_name) = {text} AND f.id > {id}))"
                )
            }
            "sizeDesc" => {
                let number = push_sql_value(
                    &mut values,
                    cursor
                        .number
                        .ok_or_else(|| "The size cursor is incomplete".to_owned())?,
                );
                format!(
                    "(COALESCE(f.size_bytes, 0) < {number}
                      OR (COALESCE(f.size_bytes, 0) = {number} AND f.id > {id}))"
                )
            }
            "typeAsc" => {
                let text = push_sql_value(
                    &mut values,
                    cursor
                        .text
                        .clone()
                        .ok_or_else(|| "The type cursor is incomplete".to_owned())?,
                );
                let secondary = push_sql_value(
                    &mut values,
                    cursor
                        .secondary_text
                        .clone()
                        .ok_or_else(|| "The type cursor is incomplete".to_owned())?,
                );
                format!(
                    "(lower(COALESCE(f.mime_type, '')) > {text}
                      OR (lower(COALESCE(f.mime_type, '')) = {text} AND lower(f.original_name) > {secondary})
                      OR (lower(COALESCE(f.mime_type, '')) = {text} AND lower(f.original_name) = {secondary} AND f.id > {id}))"
                )
            }
            _ => unreachable!("file page sort is normalized"),
        };
        conditions.push(cursor_condition);
    }
    let order_by = match sort {
        "manual" => "f.manual_order ASC, lower(f.original_name) ASC, f.id ASC",
        "updatedAsc" => "f.updated_at ASC, f.id ASC",
        "nameAsc" => "lower(f.original_name) ASC, f.id ASC",
        "nameDesc" => "lower(f.original_name) DESC, f.id ASC",
        "sizeDesc" => "COALESCE(f.size_bytes, 0) DESC, f.id ASC",
        "typeAsc" => "lower(COALESCE(f.mime_type, '')) ASC, lower(f.original_name) ASC, f.id ASC",
        _ => "f.updated_at DESC, f.id ASC",
    };
    let limit_placeholder = push_sql_value(&mut values, (limit + 1) as i64);
    let sql = format!(
        "SELECT f.id, f.folder_id, f.original_name, f.storage_path, f.mime_type,
                COALESCE(f.size_bytes, 0), f.source_kind, f.manual_order,
                current_version.version_number,
                (SELECT COUNT(*) FROM file_space_artifact_versions page_version
                 WHERE page_version.file_id = f.id),
                COALESCE((
                  SELECT group_concat(tag, char(31))
                  FROM file_space_file_tags page_tags WHERE page_tags.file_id = f.id
                ), ''),
                f.created_at, f.updated_at
         FROM files f
         LEFT JOIN file_space_artifacts artifact ON artifact.file_id = f.id
         LEFT JOIN file_space_artifact_versions current_version
           ON current_version.id = artifact.current_version_id
         WHERE {}
         ORDER BY {order_by}
         LIMIT {limit_placeholder}",
        conditions.join(" AND ")
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| format!("Unable to prepare a File Space file page: {error}"))?;
    let mut files = statement
        .query_map(params_from_iter(values), |row| {
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
        .map_err(|error| format!("Unable to load a File Space file page: {error}"))?;
    let has_more = files.len() > limit;
    if has_more {
        files.pop();
    }
    let next_cursor = if has_more {
        files.last().map(|file| file_page_cursor(file, sort))
    } else {
        None
    };
    Ok(FileSpaceFilePage {
        files,
        total_count,
        next_cursor,
    })
}

pub(crate) fn load_snapshot_record(database: &Database) -> Result<FileSpaceSnapshot, String> {
    recover_pending_file_move(database)?;
    recover_pending_trash_operation(database)?;
    let root = read_storage_root(database)?;
    let status = inspect_root(root.as_deref());
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let mut folder_statement = connection
        .prepare(
            "SELECT folder.id, folder.parent_id, folder.name, folder.relative_path,
                    folder.manual_order,
                    (SELECT COUNT(*) FROM files direct_file
                     WHERE direct_file.folder_id = folder.id
                       AND direct_file.trashed_at IS NULL
                       AND direct_file.storage_path IS NOT NULL),
                    (SELECT COUNT(*) FROM file_space_folders child
                     WHERE child.parent_id = folder.id AND child.trashed_at IS NULL),
                    folder.created_at, folder.updated_at
             FROM file_space_folders folder
             WHERE folder.trashed_at IS NULL
             ORDER BY folder.parent_id, folder.manual_order,
                      folder.name COLLATE NOCASE, folder.id",
        )
        .map_err(|error| format!("Unable to prepare File Space folders: {error}"))?;
    let mut folders = folder_statement
        .query_map([], |row| {
            Ok(FileSpaceFolder {
                id: row.get(0)?,
                parent_id: row.get(1)?,
                name: row.get(2)?,
                relative_path: row.get(3)?,
                manual_order: row.get(4)?,
                direct_file_count: row.get(5)?,
                file_count: row.get(5)?,
                child_folder_count: row.get(6)?,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
            })
        })
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .map_err(|error| format!("Unable to load File Space folders: {error}"))?;
    let folder_positions = folders
        .iter()
        .enumerate()
        .map(|(index, folder)| (folder.id.clone(), index))
        .collect::<HashMap<_, _>>();
    let mut folder_totals = folders
        .iter()
        .map(|folder| (folder.id.clone(), folder.direct_file_count))
        .collect::<HashMap<_, _>>();
    let mut deepest_first = folders.iter().collect::<Vec<_>>();
    deepest_first
        .sort_by_key(|folder| std::cmp::Reverse(folder.relative_path.matches('/').count()));
    for folder in deepest_first {
        let total = folder_totals.get(&folder.id).copied().unwrap_or(0);
        if let Some(parent_id) = folder.parent_id.as_deref() {
            *folder_totals.entry(parent_id.to_owned()).or_insert(0) += total;
        }
    }
    for (folder_id, total) in folder_totals {
        if let Some(index) = folder_positions.get(&folder_id) {
            folders[*index].file_count = total;
        }
    }
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
    let mut trash_statement = connection
        .prepare(
            "SELECT entry.id, COALESCE(entry.root_file_id, entry.root_folder_id),
                    entry.item_type, entry.original_name,
                    entry.original_relative_path,
                    COALESCE((
                      SELECT SUM(COALESCE(file.size_bytes, 0))
                      FROM file_space_trash_entry_files member
                      JOIN files file ON file.id = member.file_id
                      WHERE member.entry_id = entry.id
                    ), 0),
                    (SELECT COUNT(*) FROM file_space_trash_entry_files member
                     WHERE member.entry_id = entry.id),
                    (SELECT COUNT(*) FROM file_space_trash_entry_folders member
                     WHERE member.entry_id = entry.id),
                    (SELECT COUNT(*)
                     FROM file_space_trash_entry_files member
                     JOIN file_space_artifact_versions version ON version.file_id = member.file_id
                     WHERE member.entry_id = entry.id),
                    entry.trashed_at
             FROM file_space_trash_entries entry
             ORDER BY entry.trashed_at DESC, entry.id",
        )
        .map_err(|error| format!("Unable to prepare File Space trash: {error}"))?;
    let trash_items = trash_statement
        .query_map([], |row| {
            Ok(FileSpaceTrashItem {
                id: row.get(0)?,
                root_id: row.get(1)?,
                item_type: row.get(2)?,
                name: row.get(3)?,
                original_relative_path: row.get(4)?,
                size_bytes: row.get(5)?,
                file_count: row.get(6)?,
                folder_count: row.get(7)?,
                version_count: row.get(8)?,
                trashed_at: row.get(9)?,
            })
        })
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .map_err(|error| format!("Unable to load File Space trash: {error}"))?;
    let tags = connection
        .prepare(
            "SELECT MIN(tag)
             FROM file_space_file_tags tag
             JOIN files file ON file.id = tag.file_id
             WHERE file.trashed_at IS NULL AND file.storage_path IS NOT NULL
             GROUP BY tag.normalized_tag
             ORDER BY tag.normalized_tag",
        )
        .and_then(|mut statement| {
            statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|error| format!("Unable to load File Space tags: {error}"))?;
    drop(trash_statement);
    drop(trashed_statement);
    drop(folder_statement);
    drop(connection);
    let initial_page = list_file_space_file_page_record(
        database,
        &FileSpaceFilePageRequest {
            folder_id: None,
            sort: "manual".to_owned(),
            type_filter: "all".to_owned(),
            tag_filter: None,
            updated_after: None,
            match_ids: Vec::new(),
            cursor: None,
            limit: Some(FILE_SPACE_INITIAL_PAGE_LIMIT),
        },
    )?;
    Ok(FileSpaceSnapshot {
        root_path: root
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        root_name: root.as_deref().map(root_name),
        root_status: status,
        file_count: initial_page.total_count,
        tags,
        folders,
        files: initial_page.files,
        trashed_files,
        trash_items,
    })
}

pub(crate) fn load_workspace_snapshot(
    database: &Database,
    artifact_store: &Path,
) -> Result<FileSpaceSnapshot, String> {
    recover_pending_purge_operations_if_root_ready(database, artifact_store)?;
    capture_initial_user_versions(database, artifact_store)?;
    materialize_pending_task_files_unlocked(database)?;
    load_snapshot_record(database)
}

fn filesystem_time_millis(value: std::time::SystemTime) -> i64 {
    value
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or_else(now_millis)
}

fn ignore_existing_import_entry(name: &str, is_directory: bool) -> bool {
    if is_directory {
        name.starts_with('.')
    } else {
        matches!(name, ".DS_Store" | "Thumbs.db" | "desktop.ini")
    }
}

fn scan_existing_directory(
    directory: &Path,
    parent_id: Option<&str>,
    parent_relative_path: Option<&str>,
    folders: &mut Vec<ExistingFolderCandidate>,
    files: &mut Vec<ExistingFileCandidate>,
) -> Result<(), String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| format!("Unable to read {}: {error}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Unable to read {}: {error}", directory.display()))?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("Unable to inspect {}: {error}", path.display()))?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        let name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| format!("An item has an unsupported name: {}", path.display()))
            .and_then(validate_import_name)?;
        if ignore_existing_import_entry(&name, metadata.is_dir()) {
            continue;
        }
        let relative_path = relative_child(parent_relative_path, &name);
        let modified = metadata
            .modified()
            .map(filesystem_time_millis)
            .unwrap_or_else(|_| now_millis());
        let created = metadata
            .created()
            .map(filesystem_time_millis)
            .unwrap_or(modified);

        if metadata.is_dir() {
            let id = Uuid::new_v4().to_string();
            folders.push(ExistingFolderCandidate {
                id: id.clone(),
                parent_id: parent_id.map(str::to_owned),
                name,
                relative_path: relative_path.clone(),
                created_at: created,
                updated_at: modified,
            });
            scan_existing_directory(&path, Some(&id), Some(&relative_path), folders, files)?;
        } else if metadata.is_file() {
            fs::File::open(&path)
                .map_err(|error| format!("Unable to read {}: {error}", path.display()))?;
            let size_bytes = i64::try_from(metadata.len())
                .map_err(|_| format!("The file is too large: {}", path.display()))?;
            files.push(ExistingFileCandidate {
                id: Uuid::new_v4().to_string(),
                folder_id: parent_id.map(str::to_owned),
                name,
                relative_path,
                mime_type: mime_type_for(&path),
                size_bytes,
                created_at: created,
                updated_at: modified,
            });
        } else {
            return Err(format!(
                "The item is not a regular file or folder: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
fn import_existing_storage_root_record(
    database: &Database,
    artifact_store: &Path,
    path: &str,
) -> Result<FileSpaceSnapshot, String> {
    import_existing_storage_root_record_with_progress(database, artifact_store, path, |_, _, _| {})
}

pub(crate) fn import_existing_storage_root_record_with_progress<F>(
    database: &Database,
    artifact_store: &Path,
    path: &str,
    mut progress: F,
) -> Result<FileSpaceSnapshot, String>
where
    F: FnMut(usize, usize, Option<&str>),
{
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

    let mut folders = Vec::new();
    let mut files = Vec::new();
    scan_existing_directory(&root, None, None, &mut folders, &mut files)?;
    let total_files = files.len();
    progress(0, total_files, None);

    let mut connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Unable to begin existing-folder initialization: {error}"))?;
    let managed_count = transaction
        .query_row(
            "SELECT (SELECT COUNT(*) FROM file_space_folders) + (SELECT COUNT(*) FROM files)",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("Unable to inspect the current file index: {error}"))?;
    if managed_count != 0 {
        return Err(
            "Existing-folder initialization requires an empty Lume Trace file index".to_owned(),
        );
    }

    let now = now_millis();
    transaction
        .execute(
            "INSERT INTO app_settings (key, value, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![STORAGE_ROOT_SETTING, root.to_string_lossy(), now],
        )
        .map_err(|error| format!("Unable to save the File Space storage path: {error}"))?;

    for folder in folders {
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
                    folder.id,
                    folder.parent_id,
                    folder.name,
                    folder.relative_path,
                    folder.created_at,
                    folder.updated_at
                ],
            )
            .map_err(|error| format!("Unable to initialize a folder record: {error}"))?;
    }
    for file in files {
        transaction
            .execute(
                "INSERT INTO files
                 (id, original_name, storage_path, mime_type, size_bytes, folder_id,
                  source_kind, manual_order, updated_at, trashed_at, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'existing_import',
                         (SELECT COALESCE(MAX(manual_order), -1) + 1
                          FROM files WHERE folder_id IS ?6 AND trashed_at IS NULL),
                         ?7, NULL, ?8)",
                params![
                    file.id,
                    file.name,
                    file.relative_path,
                    file.mime_type,
                    file.size_bytes,
                    file.folder_id,
                    file.updated_at,
                    file.created_at
                ],
            )
            .map_err(|error| format!("Unable to initialize a file record: {error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("Unable to complete existing-folder initialization: {error}"))?;
    drop(connection);

    capture_initial_user_versions_with_progress(
        database,
        artifact_store,
        |processed, total, name| progress(processed, total, Some(name)),
    )?;
    load_snapshot_record(database)
}

pub(crate) fn configure_storage_root_record(
    database: &Database,
    path: &str,
) -> Result<FileSpaceSnapshot, String> {
    recover_pending_trash_operation(database)?;
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
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let managed_paths = {
        let mut statement = connection
            .prepare(
                "SELECT relative_path, 'folder' AS item_type
                 FROM file_space_folders WHERE trashed_at IS NULL
                 UNION ALL
                 SELECT storage_path, 'file' AS item_type
                 FROM files WHERE trashed_at IS NULL AND storage_path IS NOT NULL
                 UNION ALL
                 SELECT payload_relative_path, item_type
                 FROM file_space_trash_entries",
            )
            .map_err(|error| format!("Unable to inspect existing File Space records: {error}"))?;
        statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|error| format!("Unable to inspect existing File Space paths: {error}"))?
    };
    for (relative_path, item_type) in managed_paths {
        let candidate = physical_path(&root, &relative_path)?;
        let matches = match fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata.file_type().is_symlink() => false,
            Ok(metadata) if item_type == "folder" => metadata.is_dir(),
            Ok(metadata) if item_type == "file" => metadata.is_file(),
            Ok(_) => false,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                return Err(format!(
                    "Unable to inspect an existing managed item in the selected folder: {error}"
                ));
            }
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
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
        direct_file_count: 0,
        file_count: 0,
        child_folder_count: 0,
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

fn normalized_new_text_file_name(name: &str, format: &str) -> Result<String, String> {
    let extension = match format {
        "md" => "md",
        "txt" => "txt",
        _ => return Err("Only Markdown and plain-text files can be created".to_owned()),
    };
    let name = validate_name(name)?;
    match Path::new(&name)
        .extension()
        .and_then(|value| value.to_str())
    {
        None => Ok(format!("{name}.{extension}")),
        Some(current) if current.eq_ignore_ascii_case(extension) => Ok(name),
        Some(_) => Err(format!("The file name must use the .{extension} extension")),
    }
}

fn remove_created_text_file_artifacts(destination: &Path, snapshot_path: &Path) {
    let _ = fs::remove_file(destination);
    let _ = fs::remove_file(snapshot_path);
    if let Some(parent) = snapshot_path.parent() {
        let _ = fs::remove_dir(parent);
    }
}

fn create_text_file_record(
    database: &Database,
    artifact_store: &Path,
    parent_id: Option<&str>,
    name: &str,
    format: &str,
) -> Result<FileSpaceCreatedFileResult, String> {
    let root = require_ready_root(database)?;
    let name = normalized_new_text_file_name(name, format)?;
    let parent_path = {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
        match parent_id {
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
        }
    };
    let relative_path = relative_child(parent_path.as_deref(), &name);
    let destination = physical_path(&root, &relative_path)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                "A file or folder with this name already exists".to_owned()
            } else {
                format!("Unable to create the file on disk: {error}")
            }
        })?;
    if let Err(error) = output.flush().and_then(|_| output.sync_all()) {
        drop(output);
        let _ = fs::remove_file(&destination);
        return Err(format!("Unable to finish creating the file: {error}"));
    }
    drop(output);

    let file_id = Uuid::new_v4().to_string();
    let version_id = Uuid::new_v4().to_string();
    let snapshot_path = artifact_store
        .join(&file_id)
        .join(format!("{version_id}.blob"));
    if let Err(error) = snapshot_file(&destination, &snapshot_path) {
        remove_created_text_file_artifacts(&destination, &snapshot_path);
        return Err(error);
    }
    let (sha256, size_bytes) = match sha256_file(&snapshot_path) {
        Ok(value) => value,
        Err(error) => {
            remove_created_text_file_artifacts(&destination, &snapshot_path);
            return Err(error);
        }
    };
    let mtime_ms = match file_mtime_marker(&destination) {
        Ok(value) => value,
        Err(error) => {
            remove_created_text_file_artifacts(&destination, &snapshot_path);
            return Err(error);
        }
    };
    let now = now_millis();
    let mime_type = mime_type_for(&destination);
    let save_result = (|| -> Result<i64, String> {
        let mut connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
        let transaction = connection
            .transaction()
            .map_err(|error| format!("Unable to begin creating the file: {error}"))?;
        let manual_order = transaction
            .query_row(
                "SELECT COALESCE(MAX(manual_order), -1) + 1 FROM files",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| format!("Unable to choose the new file position: {error}"))?;
        transaction
            .execute(
                "INSERT INTO files
                 (id, original_name, storage_path, mime_type, size_bytes, folder_id,
                  source_kind, manual_order, updated_at, trashed_at, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'user_import', ?7, ?8, NULL, ?8)",
                params![
                    file_id,
                    name,
                    relative_path,
                    mime_type,
                    size_bytes,
                    parent_id,
                    manual_order,
                    now
                ],
            )
            .map_err(|error| format!("Unable to save the new file: {error}"))?;
        transaction
            .execute(
                "INSERT INTO file_space_artifacts
                 (file_id, task_id, logical_key, current_version_id,
                  observed_size_bytes, observed_mtime_ms, observed_sha256,
                  created_at, updated_at)
                 VALUES (?1, 'local', ?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                params![file_id, version_id, size_bytes, mtime_ms, sha256, now],
            )
            .map_err(|error| format!("Unable to create the file version history: {error}"))?;
        transaction
            .execute(
                "INSERT INTO file_space_artifact_versions
                 (id, file_id, version_number, snapshot_path, sha256, size_bytes,
                  produced_name, mime_type, origin, produced_at, created_at)
                 VALUES (?1, ?2, 1, ?3, ?4, ?5, ?6, ?7, 'user_edit', ?8, ?8)",
                params![
                    version_id,
                    file_id,
                    snapshot_path.to_string_lossy(),
                    sha256,
                    size_bytes,
                    name,
                    mime_type,
                    now
                ],
            )
            .map_err(|error| format!("Unable to save the first file version: {error}"))?;
        transaction
            .execute(
                "INSERT INTO file_space_artifact_events
                 (id, file_id, version_id, event_type, actor, details_json, created_at)
                 VALUES (?1, ?2, ?3, 'generated', 'user', ?4, ?5)",
                params![
                    Uuid::new_v4().to_string(),
                    file_id,
                    version_id,
                    json!({ "version": 1, "source": "created" }).to_string(),
                    now
                ],
            )
            .map_err(|error| format!("Unable to save the first file event: {error}"))?;
        queue_changed_file_index(&transaction, &file_id)
            .map_err(|error| format!("Unable to queue the new file for indexing: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("Unable to complete file creation: {error}"))?;
        Ok(manual_order)
    })();
    let manual_order = match save_result {
        Ok(value) => value,
        Err(error) => {
            remove_created_text_file_artifacts(&destination, &snapshot_path);
            return Err(error);
        }
    };
    let file = FileSpaceFile {
        id: file_id,
        folder_id: parent_id.map(str::to_owned),
        name,
        relative_path,
        mime_type,
        size_bytes,
        source_kind: "user_import".to_owned(),
        manual_order,
        current_version: Some(1),
        version_count: 1,
        tags: Vec::new(),
        created_at: now,
        updated_at: now,
    };
    Ok(FileSpaceCreatedFileResult {
        file,
        snapshot: load_snapshot_record(database)?,
    })
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
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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

fn active_file_record(
    database: &Database,
    file_id: &str,
) -> Result<(String, String, Option<String>), String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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

fn file_drag_image(
    path: &Path,
    preview_bytes: Option<Vec<u8>>,
    skip_generated_preview: bool,
) -> (drag::Image, Option<PathBuf>) {
    if let Some(bytes) = preview_bytes.filter(|bytes| valid_file_drag_preview(bytes)) {
        return (drag::Image::Raw(bytes), None);
    }
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if !skip_generated_preview
        && preview_mime_type(file_name, mime_type_for(path).as_deref()).is_some()
    {
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
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
    } else if !is_task_artifact && content != original_content {
        let size_bytes = i64::try_from(content.len())
            .map_err(|_| "The Markdown content is too large".to_owned())?;
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
        let update_result = (|| -> rusqlite::Result<usize> {
            let transaction = connection.unchecked_transaction()?;
            let changed = transaction.execute(
                "UPDATE files SET size_bytes = ?1, updated_at = ?2
                 WHERE id = ?3 AND trashed_at IS NULL",
                params![size_bytes, now_millis(), file_id],
            )?;
            if changed > 0 {
                queue_changed_file_index(&transaction, file_id)?;
            }
            transaction.commit()?;
            Ok(changed)
        })();
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
                .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
                .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
fn managed_item_identity_at(
    directory: &fs::File,
    name: &std::ffi::CStr,
) -> Result<Option<(ManagedFileIdentity, bool)>, String> {
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
        return Err(format!("Unable to inspect a managed item safely: {error}"));
    }
    let metadata = unsafe { metadata.assume_init() };
    let item_type = metadata.st_mode & libc::S_IFMT;
    let is_directory = match item_type {
        libc::S_IFREG => false,
        libc::S_IFDIR => true,
        libc::S_IFLNK => return Err("The managed item is a symbolic link".to_owned()),
        _ => return Err("The managed item is not a regular file or folder".to_owned()),
    };
    Ok(Some((
        ManagedFileIdentity {
            device: metadata.st_dev as u64,
            inode: metadata.st_ino as u64,
        },
        is_directory,
    )))
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "android"))]
fn managed_item_identity(
    root: &Path,
    relative_path: &str,
) -> Result<Option<(ManagedFileIdentity, bool)>, String> {
    let root_directory = open_file_space_root_directory(root)?;
    let (directory, name) = open_managed_parent_directory(&root_directory, relative_path)?;
    managed_item_identity_at(&directory, &name)
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "android")))]
fn managed_item_identity(
    _root: &Path,
    _relative_path: &str,
) -> Result<Option<(ManagedFileIdentity, bool)>, String> {
    Err("Safe managed-item identity checks are not supported on this platform".to_owned())
}

fn pending_trash_expected_identity(
    pending: &PendingTrashOperation,
) -> Result<Option<(ManagedFileIdentity, Option<bool>)>, String> {
    match (pending.source_device, pending.source_inode) {
        (Some(device), Some(inode)) => Ok(Some((
            ManagedFileIdentity { device, inode },
            pending.source_is_directory,
        ))),
        (None, None) => Ok(None),
        _ => Err("The pending trash journal has an incomplete item identity".to_owned()),
    }
}

fn pending_trash_identity_matches(
    actual: (ManagedFileIdentity, bool),
    expected: Option<(ManagedFileIdentity, Option<bool>)>,
) -> bool {
    expected.is_none_or(|(expected_identity, expected_is_directory)| {
        actual.0 == expected_identity
            && expected_is_directory.is_none_or(|expected| expected == actual.1)
    })
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "android"))]
fn atomic_move_managed_item_without_overwrite(
    root: &Path,
    source_relative_path: &str,
    destination_relative_path: &str,
    expected_identity: Option<(ManagedFileIdentity, Option<bool>)>,
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
    let source_identity = managed_item_identity_at(&source_directory, &source_name)
        .map_err(AtomicFileMoveError::NotMoved)?
        .ok_or_else(|| {
            AtomicFileMoveError::NotMoved("The item to move no longer exists".to_owned())
        })?;
    if let Some((expected, expected_is_directory)) = expected_identity {
        if expected != source_identity.0
            || expected_is_directory.is_some_and(|expected| expected != source_identity.1)
        {
            return Err(AtomicFileMoveError::NotMoved(
                "The item changed before it could be moved safely".to_owned(),
            ));
        }
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
    if result != 0 {
        return Err(AtomicFileMoveError::NotMoved(no_replace_move_error(
            std::io::Error::last_os_error(),
        )));
    }

    let destination_identity = managed_item_identity_at(&destination_directory, &destination_name)
        .map_err(AtomicFileMoveError::StateUnknown)?
        .ok_or_else(|| {
            AtomicFileMoveError::StateUnknown(
                "The moved item could not be found at its destination".to_owned(),
            )
        })?;
    if destination_identity != source_identity {
        return Err(AtomicFileMoveError::StateUnknown(
            "The moved item identity changed at its destination".to_owned(),
        ));
    }
    source_directory.sync_all().map_err(|error| {
        AtomicFileMoveError::StateUnknown(format!(
            "Unable to make the move durable at its source: {error}"
        ))
    })?;
    if source_directory.as_raw_fd() != destination_directory.as_raw_fd() {
        destination_directory.sync_all().map_err(|error| {
            AtomicFileMoveError::StateUnknown(format!(
                "Unable to make the move durable at its destination: {error}"
            ))
        })?;
    }
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "android")))]
fn atomic_move_managed_item_without_overwrite(
    _root: &Path,
    _source_relative_path: &str,
    _destination_relative_path: &str,
    _expected_identity: Option<(ManagedFileIdentity, Option<bool>)>,
) -> Result<(), AtomicFileMoveError> {
    Err(AtomicFileMoveError::NotMoved(
        "Safe no-overwrite file and folder movement is not supported on this platform".to_owned(),
    ))
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
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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

#[derive(Debug)]
struct FileMoveCandidate {
    file_id: String,
    name: String,
    current_folder_id: Option<String>,
}

#[derive(Debug)]
struct PreparedFileMoves {
    movable_ids: Vec<String>,
    unchanged_ids: Vec<String>,
}

fn preflight_file_moves(
    database: &Database,
    artifact_store: &Path,
    file_ids: &[String],
    folder_id: Option<&str>,
) -> Result<PreparedFileMoves, String> {
    recover_pending_file_move(database)?;
    if file_ids.is_empty() {
        return Err("No files were supplied for moving".to_owned());
    }
    let requested = file_ids.iter().collect::<HashSet<_>>();
    if requested.len() != file_ids.len() {
        return Err("The file move list contains duplicates".to_owned());
    }

    let root = require_ready_root(database)?;
    let (target_folder_path, candidates, active_paths) = {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
        let target_folder_path = match folder_id {
            Some(folder_id) => Some(
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
            ),
            None => None,
        };

        let mut candidates = Vec::with_capacity(file_ids.len());
        for file_id in file_ids {
            let candidate = connection
                .query_row(
                    "SELECT original_name, folder_id
                     FROM files
                     WHERE id = ?1 AND trashed_at IS NULL AND storage_path IS NOT NULL",
                    [file_id],
                    |row| {
                        Ok(FileMoveCandidate {
                            file_id: file_id.clone(),
                            name: row.get(0)?,
                            current_folder_id: row.get(1)?,
                        })
                    },
                )
                .optional()
                .map_err(|error| format!("Unable to load a selected file: {error}"))?
                .ok_or_else(|| format!("The selected file no longer exists: {file_id}"))?;
            candidates.push(candidate);
        }

        let active_paths = connection
            .prepare(
                "SELECT storage_path FROM files
                 WHERE trashed_at IS NULL AND storage_path IS NOT NULL",
            )
            .map_err(|error| format!("Unable to inspect managed file paths: {error}"))?
            .query_map([], |row| row.get::<_, String>(0))
            .and_then(|rows| rows.collect::<rusqlite::Result<HashSet<_>>>())
            .map_err(|error| format!("Unable to inspect managed file paths: {error}"))?;
        (target_folder_path, candidates, active_paths)
    };

    let destination_directory = match target_folder_path.as_deref() {
        Some(path) => physical_path(&root, path)?,
        None => root.clone(),
    };
    let destination_metadata = fs::symlink_metadata(&destination_directory)
        .map_err(|error| format!("Unable to inspect the destination folder: {error}"))?;
    if destination_metadata.file_type().is_symlink() || !destination_metadata.is_dir() {
        return Err("The destination folder must be a regular folder".to_owned());
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("Unable to resolve the File Space storage path: {error}"))?;
    let canonical_destination = destination_directory
        .canonicalize()
        .map_err(|error| format!("Unable to resolve the destination folder: {error}"))?;
    if !canonical_destination.starts_with(&canonical_root) {
        return Err("The destination folder is outside the File Space storage path".to_owned());
    }

    for candidate in &candidates {
        validate_import_name(&candidate.name)?;
        if task_artifact_exists(database, &candidate.file_id)? {
            reconcile_task_file(database, artifact_store, &candidate.file_id)?;
        }
        active_file_path(database, &candidate.file_id)?;
    }

    let mut destination_paths = HashSet::new();
    let mut movable_ids = Vec::new();
    let mut unchanged_ids = Vec::new();
    for candidate in candidates {
        if candidate.current_folder_id.as_deref() == folder_id {
            unchanged_ids.push(candidate.file_id);
            continue;
        }
        let moved_path = relative_child(target_folder_path.as_deref(), &candidate.name);
        if !destination_paths.insert(moved_path.clone()) {
            return Err(format!(
                "Multiple selected files would have the same destination name: {}",
                candidate.name
            ));
        }
        if active_paths.contains(&moved_path) {
            return Err(format!(
                "A managed file with this name already exists in the destination: {}",
                candidate.name
            ));
        }
        let destination = physical_path(&root, &moved_path)?;
        match fs::symlink_metadata(&destination) {
            Ok(_) => {
                return Err(format!(
                    "A file or folder with this name already exists in the destination: {}",
                    candidate.name
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "Unable to inspect the destination for {}: {error}",
                    candidate.name
                ));
            }
        }
        movable_ids.push(candidate.file_id);
    }

    Ok(PreparedFileMoves {
        movable_ids,
        unchanged_ids,
    })
}

fn move_files_record(
    database: &Database,
    artifact_store: &Path,
    file_ids: &[String],
    folder_id: Option<&str>,
) -> Result<FileSpaceFilesMoveResult, String> {
    let prepared = preflight_file_moves(database, artifact_store, file_ids, folder_id)?;
    let mut latest_snapshot = load_snapshot_record(database)?;
    let mut moved_ids = Vec::with_capacity(prepared.movable_ids.len());
    let mut failed = Vec::new();

    for (index, file_id) in prepared.movable_ids.iter().enumerate() {
        match move_file_record(database, artifact_store, file_id, folder_id) {
            Ok(snapshot) => {
                latest_snapshot = snapshot;
                moved_ids.push(file_id.clone());
            }
            Err(move_error) => {
                let message = match load_snapshot_record(database) {
                    Ok(snapshot) => {
                        latest_snapshot = snapshot;
                        move_error
                    }
                    Err(refresh_error) => format!(
                        "{move_error}. Unable to refresh File Space after the failed move: {refresh_error}"
                    ),
                };
                failed.push(FileSpaceFileMoveFailure {
                    file_id: file_id.clone(),
                    message,
                });
                failed.extend(
                    prepared.movable_ids[index + 1..]
                        .iter()
                        .map(|remaining_id| FileSpaceFileMoveFailure {
                            file_id: remaining_id.clone(),
                            message:
                                "Not attempted because an earlier selected file could not be moved"
                                    .to_owned(),
                        }),
                );
                break;
            }
        }
    }

    Ok(FileSpaceFilesMoveResult {
        snapshot: latest_snapshot,
        moved_ids,
        unchanged_ids: prepared.unchanged_ids,
        failed,
    })
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
                .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "android")))]
fn ensure_managed_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => Err(format!(
            "The managed trash path is not a regular directory: {}",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path)
                .map_err(|error| format!("Unable to create {}: {error}", path.display()))?;
            if let Some(parent) = path.parent() {
                sync_directory(parent)?;
            }
            Ok(())
        }
        Err(error) => Err(format!("Unable to inspect {}: {error}", path.display())),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "android")))]
fn trash_entries_directory(root: &Path) -> Result<PathBuf, String> {
    let trash = root.join(TRASH_DIRECTORY_NAME);
    ensure_managed_directory(&trash)?;
    let entries = trash.join("entries");
    ensure_managed_directory(&entries)?;
    Ok(entries)
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "android"))]
fn open_or_create_directory_at(
    parent: &fs::File,
    name: &str,
    allow_existing: bool,
) -> Result<fs::File, String> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};

    let name = CString::new(name)
        .map_err(|_| "The managed folder name contains an unsupported null byte".to_owned())?;
    let created = if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } == 0 {
        true
    } else {
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::AlreadyExists || !allow_existing {
            return Err(format!(
                "Unable to create the managed Trash folder: {error}"
            ));
        }
        false
    };
    let descriptor = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if descriptor < 0 {
        return Err(format!(
            "Unable to open the managed Trash folder safely: {}",
            std::io::Error::last_os_error()
        ));
    }
    let directory = unsafe { fs::File::from_raw_fd(descriptor) };
    if created {
        parent
            .sync_all()
            .map_err(|error| format!("Unable to save the managed Trash folder: {error}"))?;
    }
    Ok(directory)
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "android"))]
fn prepare_trash_payload(
    root: &Path,
    entry_id: &str,
    name: &str,
) -> Result<(String, String), String> {
    Uuid::parse_str(entry_id).map_err(|_| "The trash entry identity is invalid".to_owned())?;
    let name = validate_import_name(name)?;
    let root_directory = open_file_space_root_directory(root)?;
    let trash = open_or_create_directory_at(&root_directory, TRASH_DIRECTORY_NAME, true)?;
    let entries = open_or_create_directory_at(&trash, "entries", true)?;
    let container = open_or_create_directory_at(&entries, entry_id, false)?;
    container
        .sync_all()
        .map_err(|error| format!("Unable to save the Trash entry container: {error}"))?;
    let container_relative = format!("{TRASH_DIRECTORY_NAME}/entries/{entry_id}");
    Ok((
        container_relative.clone(),
        relative_child(Some(&container_relative), &name),
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "android")))]
fn prepare_trash_payload(
    root: &Path,
    entry_id: &str,
    name: &str,
) -> Result<(String, String), String> {
    Uuid::parse_str(entry_id).map_err(|_| "The trash entry identity is invalid".to_owned())?;
    let name = validate_import_name(name)?;
    let entries = trash_entries_directory(root)?;
    let container = entries.join(entry_id);
    fs::create_dir(&container)
        .map_err(|error| format!("Unable to prepare the trash entry: {error}"))?;
    sync_directory(&entries)?;
    let container_relative = format!("{TRASH_DIRECTORY_NAME}/entries/{entry_id}");
    Ok((
        container_relative.clone(),
        relative_child(Some(&container_relative), &name),
    ))
}

fn remove_empty_trash_entry_container(root: &Path, payload_relative_path: &str) {
    let Some(container_relative) = Path::new(payload_relative_path)
        .parent()
        .and_then(Path::to_str)
    else {
        return;
    };
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "android"))]
    {
        use std::os::fd::AsRawFd;

        let Ok(root_directory) = open_file_space_root_directory(root) else {
            return;
        };
        let Ok((parent, name)) = open_managed_parent_directory(&root_directory, container_relative)
        else {
            return;
        };
        if unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) } == 0 {
            let _ = parent.sync_all();
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "android")))]
    if let Ok(container) = physical_path(root, container_relative) {
        let _ = fs::remove_dir(container);
    }
}

fn write_pending_trash_operation(
    database: &Database,
    pending: &PendingTrashOperation,
) -> Result<String, String> {
    let value = serde_json::to_string(pending)
        .map_err(|error| format!("Unable to encode the pending trash operation: {error}"))?;
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    connection
        .execute(
            "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3)",
            params![PENDING_TRASH_OPERATION_SETTING, value, pending.created_at],
        )
        .map_err(|error| format!("Unable to save the pending trash operation: {error}"))?;
    Ok(value)
}

fn clear_pending_trash_operation(database: &Database, value: &str) -> Result<(), String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let deleted = connection
        .execute(
            "DELETE FROM app_settings WHERE key = ?1 AND value = ?2",
            params![PENDING_TRASH_OPERATION_SETTING, value],
        )
        .map_err(|error| format!("Unable to clear the pending trash operation: {error}"))?;
    if deleted != 1 {
        return Err("The pending trash operation changed unexpectedly".to_owned());
    }
    Ok(())
}

fn rollback_pending_trash_move(
    database: &Database,
    root: &Path,
    pending: &PendingTrashOperation,
    journal_value: &str,
) -> Result<(), String> {
    let journal_is_current = {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
        connection
            .query_row(
                "SELECT value = ?2 FROM app_settings WHERE key = ?1",
                params![PENDING_TRASH_OPERATION_SETTING, journal_value],
                |row| row.get::<_, bool>(0),
            )
            .optional()
            .map_err(|error| format!("Unable to verify the pending trash rollback: {error}"))?
            .unwrap_or(false)
    };
    if !journal_is_current {
        return Err(
            "The trash operation journal is no longer pending, so no filesystem rollback was attempted"
                .to_owned(),
        );
    }
    let expected = pending_trash_expected_identity(pending)?;
    let source = managed_item_identity(root, &pending.source_relative_path)?;
    let destination = managed_item_identity(root, &pending.destination_relative_path)?;
    match (source, destination) {
        (Some(source), None) if pending_trash_identity_matches(source, expected) => {}
        (None, Some(destination)) if pending_trash_identity_matches(destination, expected) => {
            atomic_move_managed_item_without_overwrite(
                root,
                &pending.destination_relative_path,
                &pending.source_relative_path,
                expected,
            )
            .map_err(|error| {
                format!(
                    "Unable to roll back the trash operation: {}",
                    error.into_message()
                )
            })?;
        }
        (Some(_), Some(_)) => {
            return Err(
                "Unable to roll back the trash operation because both paths are occupied; both items and the journal were preserved"
                    .to_owned(),
            );
        }
        (None, None) => {
            return Err(
                "Unable to roll back the trash operation because both paths are missing; the journal was preserved"
                    .to_owned(),
            );
        }
        _ => {
            return Err(
                "Unable to roll back the trash operation because the item identity changed; the item and journal were preserved"
                    .to_owned(),
            );
        }
    }
    clear_pending_trash_operation(database, journal_value)
}

fn recover_pending_trash_operation(database: &Database) -> Result<(), String> {
    let mut connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Unable to begin pending trash recovery: {error}"))?;
    let journal_value = transaction
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [PENDING_TRASH_OPERATION_SETTING],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("Unable to load the pending trash operation: {error}"))?;
    let Some(journal_value) = journal_value else {
        return Ok(());
    };
    let pending = serde_json::from_str::<PendingTrashOperation>(&journal_value)
        .map_err(|error| format!("The pending trash journal is invalid: {error}"))?;
    if pending.version != 1
        || !matches!(pending.action.as_str(), "trash" | "restore")
        || pending.entry_id.trim().is_empty()
        || pending.source_relative_path == pending.destination_relative_path
    {
        return Err("The pending trash journal has unsupported contents".to_owned());
    }
    let root = transaction
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [STORAGE_ROOT_SETTING],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("Unable to load the File Space root for trash recovery: {error}"))?
        .ok_or_else(|| {
            "The File Space root is unavailable while a trash operation requires recovery"
                .to_owned()
        })?;
    let root = PathBuf::from(root);
    let expected = pending_trash_expected_identity(&pending)?;
    let source = managed_item_identity(&root, &pending.source_relative_path)?;
    let destination = managed_item_identity(&root, &pending.destination_relative_path)?;
    match (source, destination) {
        (Some(source), None) if pending_trash_identity_matches(source, expected) => {}
        (None, Some(destination)) if pending_trash_identity_matches(destination, expected) => {
            atomic_move_managed_item_without_overwrite(
                &root,
                &pending.destination_relative_path,
                &pending.source_relative_path,
                expected,
            )
            .map_err(|error| {
                format!(
                    "Unable to recover the pending trash operation: {}",
                    error.into_message()
                )
            })?;
        }
        (Some(_), Some(_)) => {
            return Err(
                "Pending trash recovery is ambiguous because both paths are occupied; both items and the journal were preserved"
                    .to_owned(),
            );
        }
        (None, None) => {
            return Err(
                "Pending trash recovery is ambiguous because both paths are missing; the journal was preserved"
                    .to_owned(),
            );
        }
        _ => {
            return Err(
                "Pending trash recovery stopped because the item identity changed; the item and journal were preserved"
                    .to_owned(),
            );
        }
    }
    let deleted = transaction
        .execute(
            "DELETE FROM app_settings WHERE key = ?1 AND value = ?2",
            params![PENDING_TRASH_OPERATION_SETTING, journal_value],
        )
        .map_err(|error| format!("Unable to clear the recovered trash operation: {error}"))?;
    if deleted != 1 {
        return Err("The pending trash journal changed during recovery".to_owned());
    }
    transaction
        .commit()
        .map_err(|error| format!("Unable to complete pending trash recovery: {error}"))
}

fn trash_file_record(
    database: &Database,
    artifact_store: &Path,
    file_id: &str,
) -> Result<FileSpaceSnapshot, String> {
    recover_pending_trash_operation(database)?;
    let has_artifact = task_artifact_exists(database, file_id)?;
    if has_artifact {
        reconcile_task_file(database, artifact_store, file_id)?;
    }
    let (name, relative_path, folder_id) = active_file_record(database, file_id)?;
    let root = require_ready_root(database)?;
    let source = physical_path(&root, &relative_path)?;
    let metadata = fs::symlink_metadata(&source).map_err(|error| {
        format!("Unable to inspect the file before moving it to Trash: {error}")
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Only regular managed files can be moved to Trash".to_owned());
    }
    let (source_identity, source_is_directory) = managed_item_identity(&root, &relative_path)?
        .ok_or_else(|| "The file to move to Trash no longer exists".to_owned())?;
    if source_is_directory {
        return Err("Only regular managed files can be moved to Trash".to_owned());
    }
    let expected_sha256 = if has_artifact {
        task_file_observed_sha256(database, file_id)?
    } else {
        sha256_file(&source)?.0
    };
    let entry_id = Uuid::new_v4().to_string();
    let (_, payload_relative_path) = prepare_trash_payload(&root, &entry_id, &name)?;
    let destination = physical_path(&root, &payload_relative_path)?;
    let pending = PendingTrashOperation {
        version: 1,
        operation_id: Uuid::new_v4().to_string(),
        entry_id: entry_id.clone(),
        action: "trash".to_owned(),
        source_relative_path: relative_path.clone(),
        destination_relative_path: payload_relative_path.clone(),
        source_device: Some(source_identity.device),
        source_inode: Some(source_identity.inode),
        source_is_directory: Some(false),
        created_at: now_millis(),
    };
    let journal_value = match write_pending_trash_operation(database, &pending) {
        Ok(value) => value,
        Err(error) => {
            remove_empty_trash_entry_container(&root, &payload_relative_path);
            return Err(error);
        }
    };
    match atomic_move_managed_item_without_overwrite(
        &root,
        &relative_path,
        &payload_relative_path,
        Some((source_identity, Some(false))),
    ) {
        Ok(()) => {}
        Err(AtomicFileMoveError::NotMoved(error)) => {
            let _ = clear_pending_trash_operation(database, &journal_value);
            remove_empty_trash_entry_container(&root, &payload_relative_path);
            return Err(format!("Unable to move the file to Trash: {error}"));
        }
        Err(AtomicFileMoveError::StateUnknown(error)) => {
            return Err(format!(
                "Unable to confirm the file move to Trash: {error}. The operation journal was preserved for recovery"
            ));
        }
    }
    let moved_sha256 = sha256_file(&destination).map(|value| value.0);
    if moved_sha256.as_deref() != Ok(expected_sha256.as_str()) {
        let verification_error = moved_sha256
            .err()
            .unwrap_or_else(|| "The file changed while it was being moved to Trash".to_owned());
        let rollback = rollback_pending_trash_move(database, &root, &pending, &journal_value);
        remove_empty_trash_entry_container(&root, &payload_relative_path);
        return match rollback {
            Ok(()) => Err(verification_error),
            Err(rollback_error) => Err(format!("{verification_error}. {rollback_error}")),
        };
    }
    let now = now_millis();
    let database_result = (|| -> Result<(), String> {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
        let transaction = connection
            .unchecked_transaction()
            .map_err(|error| format!("Unable to begin moving the file to Trash: {error}"))?;
        let result = (|| -> rusqlite::Result<()> {
            transaction.execute(
                "INSERT INTO file_space_trash_entries
                 (id, item_type, root_file_id, root_folder_id, original_parent_id,
                  original_name, original_relative_path, payload_relative_path, trashed_at)
                 VALUES (?1, 'file', ?2, NULL, ?3, ?4, ?5, ?6, ?7)",
                params![
                    entry_id,
                    file_id,
                    folder_id,
                    name,
                    relative_path,
                    payload_relative_path,
                    now
                ],
            )?;
            transaction.execute(
                "INSERT INTO file_space_trash_entry_files (entry_id, file_id)
                 VALUES (?1, ?2)",
                params![entry_id, file_id],
            )?;
            let updated = transaction.execute(
                "UPDATE files
                 SET folder_id = NULL, storage_path = ?1, trashed_at = ?2, updated_at = ?2
                 WHERE id = ?3 AND trashed_at IS NULL",
                params![payload_relative_path, now, file_id],
            )?;
            if updated != 1 {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            if has_artifact {
                transaction.execute(
                    "INSERT INTO file_space_artifact_events
                     (id, file_id, event_type, actor, details_json, created_at)
                     VALUES (?1, ?2, 'deleted', 'user', ?3, ?4)",
                    params![
                        Uuid::new_v4().to_string(),
                        file_id,
                        json!({ "originalPath": relative_path }).to_string(),
                        now
                    ],
                )?;
            }
            let cleared = transaction.execute(
                "DELETE FROM app_settings WHERE key = ?1 AND value = ?2",
                params![PENDING_TRASH_OPERATION_SETTING, journal_value],
            )?;
            if cleared != 1 {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            Ok(())
        })();
        result.map_err(|error| format!("Unable to save the file in Trash: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("Unable to complete moving the file to Trash: {error}"))
    })();
    if let Err(error) = database_result {
        let rollback = rollback_pending_trash_move(database, &root, &pending, &journal_value);
        remove_empty_trash_entry_container(&root, &payload_relative_path);
        return match rollback {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(format!("{error}. {rollback_error}")),
        };
    }
    load_snapshot_record(database)
}

fn trash_folder_record(
    database: &Database,
    artifact_store: &Path,
    folder_id: &str,
) -> Result<FileSpaceSnapshot, String> {
    recover_pending_trash_operation(database)?;
    let (name, relative_path, parent_id, folder_ids, file_ids, versioned_file_ids) = {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
        let (name, relative_path, parent_id) = connection
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
        let mut folder_statement = connection
            .prepare(
                "WITH RECURSIVE tree(id) AS (
                   SELECT id FROM file_space_folders
                   WHERE id = ?1 AND trashed_at IS NULL
                   UNION ALL
                   SELECT child.id FROM file_space_folders child
                   JOIN tree ON child.parent_id = tree.id
                   WHERE child.trashed_at IS NULL
                 )
                 SELECT id FROM tree",
            )
            .map_err(|error| format!("Unable to inspect the folder tree: {error}"))?;
        let folder_ids = folder_statement
            .query_map([folder_id], |row| row.get::<_, String>(0))
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|error| format!("Unable to inspect the folder tree: {error}"))?;
        let mut file_statement = connection
            .prepare(
                "WITH RECURSIVE tree(id) AS (
                   SELECT id FROM file_space_folders
                   WHERE id = ?1 AND trashed_at IS NULL
                   UNION ALL
                   SELECT child.id FROM file_space_folders child
                   JOIN tree ON child.parent_id = tree.id
                   WHERE child.trashed_at IS NULL
                 )
                 SELECT file.id, artifact.file_id IS NOT NULL
                 FROM files file
                 JOIN tree ON tree.id = file.folder_id
                 LEFT JOIN file_space_artifacts artifact ON artifact.file_id = file.id
                 WHERE file.trashed_at IS NULL
                 ORDER BY file.storage_path, file.id",
            )
            .map_err(|error| format!("Unable to inspect files in the folder: {error}"))?;
        let file_rows = file_statement
            .query_map([folder_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
            })
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|error| format!("Unable to inspect files in the folder: {error}"))?;
        let file_ids = file_rows
            .iter()
            .map(|(file_id, _)| file_id.clone())
            .collect::<Vec<_>>();
        let versioned_file_ids = file_rows
            .into_iter()
            .filter_map(|(file_id, versioned)| versioned.then_some(file_id))
            .collect::<Vec<_>>();
        (
            name,
            relative_path,
            parent_id,
            folder_ids,
            file_ids,
            versioned_file_ids,
        )
    };
    for file_id in &versioned_file_ids {
        reconcile_task_file(database, artifact_store, file_id)?;
    }
    let observations = {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
        let mut observations = Vec::with_capacity(versioned_file_ids.len());
        for file_id in &versioned_file_ids {
            let (storage_path, expected_sha256) = connection
                .query_row(
                    "SELECT file.storage_path, artifact.observed_sha256
                     FROM files file
                     JOIN file_space_artifacts artifact ON artifact.file_id = file.id
                     WHERE file.id = ?1 AND file.trashed_at IS NULL",
                    [file_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
                )
                .map_err(|error| {
                    format!("Unable to inspect a file before deleting its folder: {error}")
                })?;
            observations.push(FolderTaskFileObservation {
                relative_path: storage_path,
                expected_sha256: expected_sha256
                    .ok_or_else(|| "A managed file observation is missing".to_owned())?,
            });
        }
        observations
    };
    let root = require_ready_root(database)?;
    let source = physical_path(&root, &relative_path)?;
    let metadata = fs::symlink_metadata(&source).map_err(|error| {
        format!("Unable to inspect the folder before moving it to Trash: {error}")
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("Only regular managed folders can be moved to Trash".to_owned());
    }
    let (source_identity, source_is_directory) = managed_item_identity(&root, &relative_path)?
        .ok_or_else(|| "The folder to move to Trash no longer exists".to_owned())?;
    if !source_is_directory {
        return Err("Only regular managed folders can be moved to Trash".to_owned());
    }
    let entry_id = Uuid::new_v4().to_string();
    let (_, payload_relative_path) = prepare_trash_payload(&root, &entry_id, &name)?;
    let destination = physical_path(&root, &payload_relative_path)?;
    let pending = PendingTrashOperation {
        version: 1,
        operation_id: Uuid::new_v4().to_string(),
        entry_id: entry_id.clone(),
        action: "trash".to_owned(),
        source_relative_path: relative_path.clone(),
        destination_relative_path: payload_relative_path.clone(),
        source_device: Some(source_identity.device),
        source_inode: Some(source_identity.inode),
        source_is_directory: Some(true),
        created_at: now_millis(),
    };
    let journal_value = match write_pending_trash_operation(database, &pending) {
        Ok(value) => value,
        Err(error) => {
            remove_empty_trash_entry_container(&root, &payload_relative_path);
            return Err(error);
        }
    };
    match atomic_move_managed_item_without_overwrite(
        &root,
        &relative_path,
        &payload_relative_path,
        Some((source_identity, Some(true))),
    ) {
        Ok(()) => {}
        Err(AtomicFileMoveError::NotMoved(error)) => {
            let _ = clear_pending_trash_operation(database, &journal_value);
            remove_empty_trash_entry_container(&root, &payload_relative_path);
            return Err(format!("Unable to move the folder to Trash: {error}"));
        }
        Err(AtomicFileMoveError::StateUnknown(error)) => {
            return Err(format!(
                "Unable to confirm the folder move to Trash: {error}. The operation journal was preserved for recovery"
            ));
        }
    }
    if let Err(error) =
        verify_pending_folder_task_files(&destination, &relative_path, &observations)
    {
        let rollback = rollback_pending_trash_move(database, &root, &pending, &journal_value);
        remove_empty_trash_entry_container(&root, &payload_relative_path);
        return match rollback {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(format!("{error}. {rollback_error}")),
        };
    }
    let now = now_millis();
    let database_result = (|| -> Result<(), String> {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
        let transaction = connection
            .unchecked_transaction()
            .map_err(|error| format!("Unable to begin moving the folder to Trash: {error}"))?;
        let result = (|| -> rusqlite::Result<()> {
            transaction.execute(
                "INSERT INTO file_space_trash_entries
                 (id, item_type, root_file_id, root_folder_id, original_parent_id,
                  original_name, original_relative_path, payload_relative_path, trashed_at)
                 VALUES (?1, 'folder', NULL, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    entry_id,
                    folder_id,
                    parent_id,
                    name,
                    relative_path,
                    payload_relative_path,
                    now
                ],
            )?;
            for member_folder_id in &folder_ids {
                transaction.execute(
                    "INSERT INTO file_space_trash_entry_folders (entry_id, folder_id)
                     VALUES (?1, ?2)",
                    params![entry_id, member_folder_id],
                )?;
            }
            for member_file_id in &file_ids {
                transaction.execute(
                    "INSERT INTO file_space_trash_entry_files (entry_id, file_id)
                     VALUES (?1, ?2)",
                    params![entry_id, member_file_id],
                )?;
            }
            let updated_folders = transaction.execute(
                "UPDATE file_space_folders
                 SET relative_path = ?1 || substr(relative_path, length(?2) + 1),
                     parent_id = CASE WHEN id = ?3 THEN NULL ELSE parent_id END,
                     trashed_at = ?4, updated_at = ?4
                 WHERE id IN (
                   SELECT folder_id FROM file_space_trash_entry_folders WHERE entry_id = ?5
                 ) AND trashed_at IS NULL",
                params![
                    payload_relative_path,
                    relative_path,
                    folder_id,
                    now,
                    entry_id
                ],
            )?;
            if updated_folders != folder_ids.len() {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            let updated_files = transaction.execute(
                "UPDATE files
                 SET storage_path = ?1 || substr(storage_path, length(?2) + 1),
                     trashed_at = ?3, updated_at = ?3
                 WHERE id IN (
                   SELECT file_id FROM file_space_trash_entry_files WHERE entry_id = ?4
                 ) AND trashed_at IS NULL",
                params![payload_relative_path, relative_path, now, entry_id],
            )?;
            if updated_files != file_ids.len() {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            for member_file_id in &versioned_file_ids {
                transaction.execute(
                    "INSERT INTO file_space_artifact_events
                     (id, file_id, event_type, actor, details_json, created_at)
                     VALUES (?1, ?2, 'deleted', 'user', ?3, ?4)",
                    params![
                        Uuid::new_v4().to_string(),
                        member_file_id,
                        json!({ "folderPath": relative_path }).to_string(),
                        now
                    ],
                )?;
            }
            let cleared = transaction.execute(
                "DELETE FROM app_settings WHERE key = ?1 AND value = ?2",
                params![PENDING_TRASH_OPERATION_SETTING, journal_value],
            )?;
            if cleared != 1 {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            Ok(())
        })();
        result.map_err(|error| format!("Unable to save the folder in Trash: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("Unable to complete moving the folder to Trash: {error}"))
    })();
    if let Err(error) = database_result {
        let rollback = rollback_pending_trash_move(database, &root, &pending, &journal_value);
        remove_empty_trash_entry_container(&root, &payload_relative_path);
        return match rollback {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(format!("{error}. {rollback_error}")),
        };
    }
    load_snapshot_record(database)
}

#[derive(Debug)]
struct TrashEntryRecord {
    id: String,
    item_type: String,
    root_file_id: Option<String>,
    root_folder_id: Option<String>,
    original_parent_id: Option<String>,
    original_name: String,
    original_relative_path: String,
    payload_relative_path: String,
}

fn load_trash_entry(database: &Database, entry_id: &str) -> Result<TrashEntryRecord, String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    connection
        .query_row(
            "SELECT id, item_type, root_file_id, root_folder_id, original_parent_id,
                    original_name, original_relative_path, payload_relative_path
             FROM file_space_trash_entries WHERE id = ?1",
            [entry_id],
            |row| {
                Ok(TrashEntryRecord {
                    id: row.get(0)?,
                    item_type: row.get(1)?,
                    root_file_id: row.get(2)?,
                    root_folder_id: row.get(3)?,
                    original_parent_id: row.get(4)?,
                    original_name: row.get(5)?,
                    original_relative_path: row.get(6)?,
                    payload_relative_path: row.get(7)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("Unable to load the Trash item: {error}"))?
        .ok_or_else(|| "The Trash item no longer exists".to_owned())
}

fn resolve_trash_restore_parent(
    database: &Database,
    root: &Path,
    requested_parent_id: Option<&str>,
) -> Result<(Option<String>, Option<String>, PathBuf), String> {
    let requested_parent = if let Some(parent_id) = requested_parent_id {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
        connection
            .query_row(
                "SELECT relative_path FROM file_space_folders
                 WHERE id = ?1 AND trashed_at IS NULL",
                [parent_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("Unable to load the original restore folder: {error}"))?
            .map(|path| (parent_id.to_owned(), path))
    } else {
        None
    };
    if let Some((parent_id, parent_path)) = requested_parent {
        let directory = physical_path(root, &parent_path)?;
        match fs::symlink_metadata(&directory) {
            Ok(metadata) if !metadata.file_type().is_symlink() && metadata.is_dir() => {
                return Ok((Some(parent_id), Some(parent_path), directory));
            }
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("The original restore folder is a symbolic link".to_owned());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "Unable to inspect the original restore folder: {error}"
                ));
            }
        }
    }
    Ok((None, None, root.to_path_buf()))
}

fn restore_trashed_file_record(
    database: &Database,
    entry: &TrashEntryRecord,
) -> Result<FileSpaceSnapshot, String> {
    let file_id = entry
        .root_file_id
        .as_deref()
        .ok_or_else(|| "The Trash file identity is missing".to_owned())?;
    let root = require_ready_root(database)?;
    let source = physical_path(&root, &entry.payload_relative_path)?;
    let metadata = fs::symlink_metadata(&source)
        .map_err(|error| format!("Unable to inspect the file in Trash: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("The Trash file payload is not a regular file".to_owned());
    }
    let (source_identity, source_is_directory) =
        managed_item_identity(&root, &entry.payload_relative_path)?
            .ok_or_else(|| "The Trash file payload no longer exists".to_owned())?;
    if source_is_directory {
        return Err("The Trash file payload is not a regular file".to_owned());
    }
    let expected_sha256 = {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
        connection
            .query_row(
                "SELECT artifact.observed_sha256 FROM files file
                 LEFT JOIN file_space_artifacts artifact ON artifact.file_id = file.id
                 WHERE file.id = ?1 AND file.trashed_at IS NOT NULL",
                [file_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|error| format!("Unable to load the Trash file observation: {error}"))?
            .ok_or_else(|| "The Trash file record no longer exists".to_owned())?
    };
    let (sha256, size_bytes) = sha256_file(&source)?;
    if expected_sha256
        .as_deref()
        .is_some_and(|expected| expected != sha256)
    {
        return Err(
            "The file in Trash no longer matches its recorded content; it was not restored"
                .to_owned(),
        );
    }
    let (folder_id, folder_path, directory) =
        resolve_trash_restore_parent(database, &root, entry.original_parent_id.as_deref())?;
    let restored_name = unique_file_name(&directory, &entry.original_name);
    let restored_relative_path = relative_child(folder_path.as_deref(), &restored_name);
    let destination = physical_path(&root, &restored_relative_path)?;
    let pending = PendingTrashOperation {
        version: 1,
        operation_id: Uuid::new_v4().to_string(),
        entry_id: entry.id.clone(),
        action: "restore".to_owned(),
        source_relative_path: entry.payload_relative_path.clone(),
        destination_relative_path: restored_relative_path.clone(),
        source_device: Some(source_identity.device),
        source_inode: Some(source_identity.inode),
        source_is_directory: Some(false),
        created_at: now_millis(),
    };
    let journal_value = write_pending_trash_operation(database, &pending)?;
    match atomic_move_managed_item_without_overwrite(
        &root,
        &entry.payload_relative_path,
        &restored_relative_path,
        Some((source_identity, Some(false))),
    ) {
        Ok(()) => {}
        Err(AtomicFileMoveError::NotMoved(error)) => {
            let _ = clear_pending_trash_operation(database, &journal_value);
            return Err(format!("Unable to restore the file from Trash: {error}"));
        }
        Err(AtomicFileMoveError::StateUnknown(error)) => {
            return Err(format!(
                "Unable to confirm the file restore from Trash: {error}. The operation journal was preserved for recovery"
            ));
        }
    }
    let mtime_ms = match file_mtime_marker(&destination) {
        Ok(value) => value,
        Err(error) => {
            let rollback = rollback_pending_trash_move(database, &root, &pending, &journal_value);
            return match rollback {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(format!("{error}. {rollback_error}")),
            };
        }
    };
    let now = now_millis();
    let database_result = (|| -> Result<(), String> {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
        let transaction = connection
            .unchecked_transaction()
            .map_err(|error| format!("Unable to begin restoring the file: {error}"))?;
        let result = (|| -> rusqlite::Result<()> {
            let updated = transaction.execute(
                "UPDATE files
                 SET original_name = ?1, storage_path = ?2, folder_id = ?3,
                     size_bytes = ?4, mime_type = ?5, trashed_at = NULL, updated_at = ?6
                 WHERE id = ?7 AND trashed_at IS NOT NULL",
                params![
                    restored_name,
                    restored_relative_path,
                    folder_id,
                    size_bytes,
                    mime_type_for(&destination),
                    now,
                    file_id
                ],
            )?;
            if updated != 1 {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            let artifact_updated = transaction.execute(
                "UPDATE file_space_artifacts
                 SET observed_size_bytes = ?1, observed_mtime_ms = ?2,
                     observed_sha256 = ?3, updated_at = ?4
                 WHERE file_id = ?5",
                params![size_bytes, mtime_ms, sha256, now, file_id],
            )?;
            if artifact_updated == 1 {
                transaction.execute(
                    "INSERT INTO file_space_artifact_events
                     (id, file_id, event_type, actor, details_json, created_at)
                     VALUES (?1, ?2, 'restored', 'user', ?3, ?4)",
                    params![
                        Uuid::new_v4().to_string(),
                        file_id,
                        json!({
                            "originalPath": entry.original_relative_path,
                            "restoredPath": restored_relative_path
                        })
                        .to_string(),
                        now
                    ],
                )?;
            }
            let deleted = transaction.execute(
                "DELETE FROM file_space_trash_entries WHERE id = ?1",
                [&entry.id],
            )?;
            if deleted != 1 {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            let cleared = transaction.execute(
                "DELETE FROM app_settings WHERE key = ?1 AND value = ?2",
                params![PENDING_TRASH_OPERATION_SETTING, journal_value],
            )?;
            if cleared != 1 {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            Ok(())
        })();
        result.map_err(|error| format!("Unable to save the restored file: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("Unable to complete restoring the file: {error}"))
    })();
    if let Err(error) = database_result {
        let rollback = rollback_pending_trash_move(database, &root, &pending, &journal_value);
        return match rollback {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(format!("{error}. {rollback_error}")),
        };
    }
    remove_empty_trash_entry_container(&root, &entry.payload_relative_path);
    synchronize_file_search_index_for_files(database, Some(&root), &[file_id.to_owned()])?;
    load_snapshot_record(database)
}

fn restore_trashed_folder_record(
    database: &Database,
    entry: &TrashEntryRecord,
) -> Result<FileSpaceSnapshot, String> {
    let folder_id = entry
        .root_folder_id
        .as_deref()
        .ok_or_else(|| "The Trash folder identity is missing".to_owned())?;
    let root = require_ready_root(database)?;
    let source = physical_path(&root, &entry.payload_relative_path)?;
    let metadata = fs::symlink_metadata(&source)
        .map_err(|error| format!("Unable to inspect the folder in Trash: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("The Trash folder payload is not a regular folder".to_owned());
    }
    let (source_identity, source_is_directory) =
        managed_item_identity(&root, &entry.payload_relative_path)?
            .ok_or_else(|| "The Trash folder payload no longer exists".to_owned())?;
    if !source_is_directory {
        return Err("The Trash folder payload is not a regular folder".to_owned());
    }
    let (parent_id, parent_path, directory) =
        resolve_trash_restore_parent(database, &root, entry.original_parent_id.as_deref())?;
    let restored_name = unique_folder_name(&directory, &entry.original_name);
    let restored_relative_path = relative_child(parent_path.as_deref(), &restored_name);
    let versioned_file_ids = {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
        let mut statement = connection
            .prepare(
                "SELECT member.file_id
                 FROM file_space_trash_entry_files member
                 JOIN file_space_artifacts artifact ON artifact.file_id = member.file_id
                 WHERE member.entry_id = ?1",
            )
            .map_err(|error| format!("Unable to inspect Trash folder history: {error}"))?;
        statement
            .query_map([&entry.id], |row| row.get::<_, String>(0))
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|error| format!("Unable to inspect Trash folder history: {error}"))?
    };
    let pending = PendingTrashOperation {
        version: 1,
        operation_id: Uuid::new_v4().to_string(),
        entry_id: entry.id.clone(),
        action: "restore".to_owned(),
        source_relative_path: entry.payload_relative_path.clone(),
        destination_relative_path: restored_relative_path.clone(),
        source_device: Some(source_identity.device),
        source_inode: Some(source_identity.inode),
        source_is_directory: Some(true),
        created_at: now_millis(),
    };
    let journal_value = write_pending_trash_operation(database, &pending)?;
    match atomic_move_managed_item_without_overwrite(
        &root,
        &entry.payload_relative_path,
        &restored_relative_path,
        Some((source_identity, Some(true))),
    ) {
        Ok(()) => {}
        Err(AtomicFileMoveError::NotMoved(error)) => {
            let _ = clear_pending_trash_operation(database, &journal_value);
            return Err(format!("Unable to restore the folder from Trash: {error}"));
        }
        Err(AtomicFileMoveError::StateUnknown(error)) => {
            return Err(format!(
                "Unable to confirm the folder restore from Trash: {error}. The operation journal was preserved for recovery"
            ));
        }
    }
    let now = now_millis();
    let database_result = (|| -> Result<(), String> {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
        let transaction = connection
            .unchecked_transaction()
            .map_err(|error| format!("Unable to begin restoring the folder: {error}"))?;
        let result = (|| -> rusqlite::Result<()> {
            let restored_folders = transaction.execute(
                "UPDATE file_space_folders
                 SET relative_path = ?1 || substr(relative_path, length(?2) + 1),
                     trashed_at = NULL, updated_at = ?3
                 WHERE id IN (
                   SELECT folder_id FROM file_space_trash_entry_folders WHERE entry_id = ?4
                 ) AND trashed_at IS NOT NULL",
                params![
                    restored_relative_path,
                    entry.payload_relative_path,
                    now,
                    entry.id
                ],
            )?;
            let expected_folders = transaction.query_row(
                "SELECT COUNT(*) FROM file_space_trash_entry_folders WHERE entry_id = ?1",
                [&entry.id],
                |row| row.get::<_, usize>(0),
            )?;
            if restored_folders != expected_folders {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            let root_updated = transaction.execute(
                "UPDATE file_space_folders SET name = ?1, parent_id = ?2, updated_at = ?3
                 WHERE id = ?4 AND trashed_at IS NULL",
                params![restored_name, parent_id, now, folder_id],
            )?;
            if root_updated != 1 {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            let restored_files = transaction.execute(
                "UPDATE files
                 SET storage_path = ?1 || substr(storage_path, length(?2) + 1),
                     trashed_at = NULL, updated_at = ?3
                 WHERE id IN (
                   SELECT file_id FROM file_space_trash_entry_files WHERE entry_id = ?4
                 ) AND trashed_at IS NOT NULL",
                params![
                    restored_relative_path,
                    entry.payload_relative_path,
                    now,
                    entry.id
                ],
            )?;
            let expected_files = transaction.query_row(
                "SELECT COUNT(*) FROM file_space_trash_entry_files WHERE entry_id = ?1",
                [&entry.id],
                |row| row.get::<_, usize>(0),
            )?;
            if restored_files != expected_files {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            for member_file_id in &versioned_file_ids {
                transaction.execute(
                    "INSERT INTO file_space_artifact_events
                     (id, file_id, event_type, actor, details_json, created_at)
                     VALUES (?1, ?2, 'restored', 'user', ?3, ?4)",
                    params![
                        Uuid::new_v4().to_string(),
                        member_file_id,
                        json!({
                            "originalPath": entry.original_relative_path,
                            "restoredPath": restored_relative_path
                        })
                        .to_string(),
                        now
                    ],
                )?;
            }
            let deleted = transaction.execute(
                "DELETE FROM file_space_trash_entries WHERE id = ?1",
                [&entry.id],
            )?;
            if deleted != 1 {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            let cleared = transaction.execute(
                "DELETE FROM app_settings WHERE key = ?1 AND value = ?2",
                params![PENDING_TRASH_OPERATION_SETTING, journal_value],
            )?;
            if cleared != 1 {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            Ok(())
        })();
        result.map_err(|error| format!("Unable to save the restored folder: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("Unable to complete restoring the folder: {error}"))
    })();
    if let Err(error) = database_result {
        let rollback = rollback_pending_trash_move(database, &root, &pending, &journal_value);
        return match rollback {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(format!("{error}. {rollback_error}")),
        };
    }
    remove_empty_trash_entry_container(&root, &entry.payload_relative_path);
    synchronize_file_search_index_for_files(database, Some(&root), &versioned_file_ids)?;
    load_snapshot_record(database)
}

fn restore_trash_entry_record(
    database: &Database,
    entry_id: &str,
) -> Result<FileSpaceSnapshot, String> {
    recover_pending_trash_operation(database)?;
    let entry = load_trash_entry(database, entry_id)?;
    match entry.item_type.as_str() {
        "file" => restore_trashed_file_record(database, &entry),
        "folder" => restore_trashed_folder_record(database, &entry),
        _ => Err("The Trash item type is unsupported".to_owned()),
    }
}

fn canonical_uuid(value: &str, label: &str) -> Result<String, String> {
    let parsed = Uuid::parse_str(value).map_err(|_| format!("The {label} identity is invalid"))?;
    let normalized = parsed.to_string();
    if value != normalized {
        return Err(format!("The {label} identity is not canonical"));
    }
    Ok(normalized)
}

fn validate_regular_tree(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "Unable to inspect purge content {}: {error}",
            path.display()
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "Purge content contains a symbolic link: {}",
            path.display()
        ));
    }
    if metadata.is_file() {
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(format!(
            "Purge content is not a regular file or folder: {}",
            path.display()
        ));
    }
    for entry in fs::read_dir(path)
        .map_err(|error| format!("Unable to inspect purge folder {}: {error}", path.display()))?
    {
        let entry = entry.map_err(|error| {
            format!("Unable to inspect purge folder {}: {error}", path.display())
        })?;
        validate_regular_tree(&entry.path())?;
    }
    Ok(())
}

fn trash_entry_container_relative(entry: &TrashEntryRecord) -> Result<String, String> {
    let entry_id = canonical_uuid(&entry.id, "Trash entry")?;
    let name = validate_import_name(&entry.original_name)?;
    let expected_payload = format!("{TRASH_DIRECTORY_NAME}/entries/{entry_id}/{name}");
    if entry.payload_relative_path != expected_payload {
        return Err("The Trash payload path does not match its recorded entry".to_owned());
    }
    Ok(format!("{TRASH_DIRECTORY_NAME}/entries/{entry_id}"))
}

fn validate_artifact_directory(
    artifact_store: &Path,
    file_id: &str,
    versions: &[(String, String, String, i64)],
) -> Result<(String, ManagedFileIdentity), String> {
    let file_id = canonical_uuid(file_id, "file")?;
    let metadata = fs::symlink_metadata(artifact_store)
        .map_err(|error| format!("Unable to inspect the File Space version storage: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("The File Space version storage is not a regular directory".to_owned());
    }
    let canonical_store = artifact_store
        .canonicalize()
        .map_err(|error| format!("Unable to resolve the File Space version storage: {error}"))?;
    let directory = canonical_store.join(&file_id);
    let directory_metadata = fs::symlink_metadata(&directory)
        .map_err(|error| format!("Unable to inspect Task file history: {error}"))?;
    if directory_metadata.file_type().is_symlink() || !directory_metadata.is_dir() {
        return Err("The Task file version directory is invalid".to_owned());
    }
    let mut expected_names = HashSet::with_capacity(versions.len());
    for (version_id, snapshot_path, sha256, size_bytes) in versions {
        let version_id = canonical_uuid(version_id, "version")?;
        let file_name = format!("{version_id}.blob");
        let expected_path = directory.join(&file_name);
        let recorded_path = Path::new(snapshot_path);
        let recorded_metadata = fs::symlink_metadata(recorded_path)
            .map_err(|error| format!("Unable to inspect a Task file version: {error}"))?;
        if recorded_metadata.file_type().is_symlink() || !recorded_metadata.is_file() {
            return Err("A Task file version path is unsafe".to_owned());
        }
        let canonical_recorded = recorded_path
            .canonicalize()
            .map_err(|error| format!("Unable to resolve a Task file version: {error}"))?;
        if canonical_recorded != expected_path {
            return Err("A Task file version path is outside its managed directory".to_owned());
        }
        verify_file_integrity(recorded_path, sha256, *size_bytes)?;
        expected_names.insert(file_name);
    }
    let actual_entries = fs::read_dir(&directory)
        .map_err(|error| format!("Unable to inspect Task file history: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Unable to inspect Task file history: {error}"))?;
    if actual_entries.len() != expected_names.len() {
        return Err("The Task file version directory contains unrecorded content".to_owned());
    }
    for entry in actual_entries {
        let name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| "A Task file version has an unsupported name".to_owned())?
            .to_owned();
        if !expected_names.contains(&name) {
            return Err("The Task file version directory contains unrecorded content".to_owned());
        }
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| format!("Unable to inspect Task file history: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("The Task file version directory contains unsafe content".to_owned());
        }
    }
    let (identity, is_directory) = managed_item_identity(&canonical_store, &file_id)?
        .ok_or_else(|| "The Task file version directory no longer exists".to_owned())?;
    if !is_directory {
        return Err("The Task file version directory is invalid".to_owned());
    }
    Ok((file_id, identity))
}

fn purge_path_identity(
    base: &Path,
    relative_path: &str,
) -> Result<Option<(ManagedFileIdentity, bool)>, String> {
    let path = physical_path(base, relative_path)?;
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "Unable to inspect purge path {}: {error}",
                path.display()
            ))
        }
    };
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "The purge path is a symbolic link: {}",
            path.display()
        ));
    }
    if !metadata.is_file() && !metadata.is_dir() {
        return Err(format!(
            "The purge path is not a regular file or folder: {}",
            path.display()
        ));
    }
    managed_item_identity(base, relative_path)
}

fn purge_path_base<'a>(
    root: &'a Path,
    artifact_store: &'a Path,
    path: &PurgePathRecord,
) -> Result<&'a Path, String> {
    match path.base_kind.as_str() {
        "storage" if path.path_kind == "trash_entry" => Ok(root),
        "artifact_store" if path.path_kind == "artifact_directory" => Ok(artifact_store),
        _ => Err("The purge journal contains an unsupported path scope".to_owned()),
    }
}

fn validate_purge_journal_path(
    operation: &PurgeOperationRecord,
    path: &PurgePathRecord,
) -> Result<(), String> {
    if !path.expected_is_directory {
        return Err("The purge journal path is not a directory".to_owned());
    }
    match (path.base_kind.as_str(), path.path_kind.as_str()) {
        ("storage", "trash_entry") => {
            let expected_original =
                format!("{TRASH_DIRECTORY_NAME}/entries/{}", operation.entry_id);
            let expected_staged = format!(
                "{TRASH_DIRECTORY_NAME}/{PURGE_DIRECTORY_NAME}/{}/entry",
                operation.id
            );
            if path.original_relative_path != expected_original
                || path.staged_relative_path != expected_staged
            {
                return Err("The purge journal Trash path is invalid".to_owned());
            }
        }
        ("artifact_store", "artifact_directory") => {
            let file_id = canonical_uuid(&path.original_relative_path, "file")?;
            let expected_staged = format!(
                "{ARTIFACT_PURGE_DIRECTORY_NAME}/{}/{}",
                operation.id, file_id
            );
            if path.staged_relative_path != expected_staged {
                return Err("The purge journal version path is invalid".to_owned());
            }
        }
        _ => return Err("The purge journal path type is invalid".to_owned()),
    }
    Ok(())
}

fn load_purge_operations(database: &Database) -> Result<Vec<PurgeOperationRecord>, String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let mut statement = connection
        .prepare(
            "SELECT id, entry_id, item_type, phase
             FROM file_space_purge_operations ORDER BY created_at, id",
        )
        .map_err(|error| format!("Unable to prepare purge recovery: {error}"))?;
    statement
        .query_map([], |row| {
            Ok(PurgeOperationRecord {
                id: row.get(0)?,
                entry_id: row.get(1)?,
                item_type: row.get(2)?,
                phase: row.get(3)?,
            })
        })
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .map_err(|error| format!("Unable to load purge recovery: {error}"))
}

fn load_purge_members(
    database: &Database,
    operation_id: &str,
) -> Result<Vec<PurgeMemberRecord>, String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let mut statement = connection
        .prepare(
            "SELECT member_type, member_id, depth
             FROM file_space_purge_members WHERE operation_id = ?1
             ORDER BY member_type, depth DESC, member_id",
        )
        .map_err(|error| format!("Unable to prepare purge members: {error}"))?;
    statement
        .query_map([operation_id], |row| {
            Ok(PurgeMemberRecord {
                member_type: row.get(0)?,
                member_id: row.get(1)?,
                depth: row.get(2)?,
            })
        })
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .map_err(|error| format!("Unable to load purge members: {error}"))
}

fn load_purge_paths(
    database: &Database,
    operation_id: &str,
) -> Result<Vec<PurgePathRecord>, String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let mut statement = connection
        .prepare(
            "SELECT path_kind, base_kind, original_relative_path, staged_relative_path,
                    expected_device, expected_inode, expected_is_directory
             FROM file_space_purge_paths WHERE operation_id = ?1
             ORDER BY CASE path_kind WHEN 'trash_entry' THEN 0 ELSE 1 END,
                      original_relative_path",
        )
        .map_err(|error| format!("Unable to prepare purge paths: {error}"))?;
    let rows = statement
        .query_map([operation_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, bool>(6)?,
            ))
        })
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .map_err(|error| format!("Unable to load purge paths: {error}"))?;
    rows.into_iter()
        .map(
            |(path_kind, base_kind, original, staged, device, inode, is_directory)| {
                Ok(PurgePathRecord {
                    path_kind,
                    base_kind,
                    original_relative_path: original,
                    staged_relative_path: staged,
                    expected_identity: ManagedFileIdentity {
                        device: device.parse::<u64>().map_err(|_| {
                            "The purge journal device identity is invalid".to_owned()
                        })?,
                        inode: inode.parse::<u64>().map_err(|_| {
                            "The purge journal inode identity is invalid".to_owned()
                        })?,
                    },
                    expected_is_directory: is_directory,
                })
            },
        )
        .collect()
}

fn remove_purge_journal(database: &Database, operation_id: &str) -> Result<(), String> {
    let mut connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Unable to begin purge journal cleanup: {error}"))?;
    transaction
        .execute(
            "DELETE FROM file_space_purge_paths WHERE operation_id = ?1",
            [operation_id],
        )
        .map_err(|error| format!("Unable to clear purge paths: {error}"))?;
    transaction
        .execute(
            "DELETE FROM file_space_purge_members WHERE operation_id = ?1",
            [operation_id],
        )
        .map_err(|error| format!("Unable to clear purge members: {error}"))?;
    let deleted = transaction
        .execute(
            "DELETE FROM file_space_purge_operations WHERE id = ?1",
            [operation_id],
        )
        .map_err(|error| format!("Unable to clear purge operation: {error}"))?;
    if deleted != 1 {
        return Err("The purge journal changed unexpectedly".to_owned());
    }
    transaction
        .commit()
        .map_err(|error| format!("Unable to complete purge journal cleanup: {error}"))
}

fn remove_directory_if_empty(path: &Path) {
    if fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_none()) {
        let _ = fs::remove_dir(path);
        if let Some(parent) = path.parent() {
            let _ = sync_directory(parent);
        }
    }
}

fn cleanup_purge_staging_directories(root: &Path, artifact_store: &Path, operation_id: &str) {
    let storage_operation = root
        .join(TRASH_DIRECTORY_NAME)
        .join(PURGE_DIRECTORY_NAME)
        .join(operation_id);
    remove_directory_if_empty(&storage_operation);
    remove_directory_if_empty(&root.join(TRASH_DIRECTORY_NAME).join(PURGE_DIRECTORY_NAME));
    let artifact_operation = artifact_store
        .join(ARTIFACT_PURGE_DIRECTORY_NAME)
        .join(operation_id);
    remove_directory_if_empty(&artifact_operation);
    remove_directory_if_empty(&artifact_store.join(ARTIFACT_PURGE_DIRECTORY_NAME));
}

fn ensure_purge_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => Err(format!(
            "The purge staging path is not a regular directory: {}",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|error| {
                format!("Unable to create purge staging {}: {error}", path.display())
            })?;
            if let Some(parent) = path.parent() {
                sync_directory(parent)?;
            }
            Ok(())
        }
        Err(error) => Err(format!(
            "Unable to inspect purge staging {}: {error}",
            path.display()
        )),
    }
}

fn prepare_purge_staging_directories(
    root: &Path,
    artifact_store: &Path,
    operation_id: &str,
    has_artifacts: bool,
) -> Result<(), String> {
    canonical_uuid(operation_id, "purge operation")?;
    let trash = root.join(TRASH_DIRECTORY_NAME);
    let trash_metadata = fs::symlink_metadata(&trash)
        .map_err(|error| format!("Unable to inspect the managed Trash folder: {error}"))?;
    if trash_metadata.file_type().is_symlink() || !trash_metadata.is_dir() {
        return Err("The managed Trash folder is invalid".to_owned());
    }
    let purging = trash.join(PURGE_DIRECTORY_NAME);
    ensure_purge_directory(&purging)?;
    ensure_purge_directory(&purging.join(operation_id))?;

    if has_artifacts {
        let store_metadata = fs::symlink_metadata(artifact_store).map_err(|error| {
            format!("Unable to inspect the File Space version storage: {error}")
        })?;
        if store_metadata.file_type().is_symlink() || !store_metadata.is_dir() {
            return Err("The File Space version storage is invalid".to_owned());
        }
        let purging = artifact_store.join(ARTIFACT_PURGE_DIRECTORY_NAME);
        ensure_purge_directory(&purging)?;
        ensure_purge_directory(&purging.join(operation_id))?;
    }
    Ok(())
}

fn validate_purge_operation(operation: &PurgeOperationRecord) -> Result<(), String> {
    canonical_uuid(&operation.id, "purge operation")?;
    canonical_uuid(&operation.entry_id, "Trash entry")?;
    if !matches!(operation.item_type.as_str(), "file" | "folder")
        || !matches!(operation.phase.as_str(), "staging" | "database_committed")
    {
        return Err("The purge journal operation is invalid".to_owned());
    }
    Ok(())
}

fn validate_staged_purge_paths(
    root: &Path,
    artifact_store: &Path,
    operation: &PurgeOperationRecord,
    paths: &[PurgePathRecord],
) -> Result<(), String> {
    for path in paths {
        validate_purge_journal_path(operation, path)?;
        let base = purge_path_base(root, artifact_store, path)?;
        if purge_path_identity(base, &path.original_relative_path)?.is_some() {
            return Err("A purge source path is still occupied after staging".to_owned());
        }
        let staged = purge_path_identity(base, &path.staged_relative_path)?;
        if staged != Some((path.expected_identity, path.expected_is_directory)) {
            return Err("A staged purge item identity changed before database commit".to_owned());
        }
        validate_regular_tree(&physical_path(base, &path.staged_relative_path)?)?;
        if purge_path_identity(base, &path.staged_relative_path)?
            != Some((path.expected_identity, path.expected_is_directory))
        {
            return Err("A staged purge item changed during validation".to_owned());
        }
    }
    Ok(())
}

fn rollback_staging_purge_operation(
    database: &Database,
    artifact_store: &Path,
    operation: &PurgeOperationRecord,
) -> Result<(), String> {
    let root = require_ready_root(database)?;
    let paths = load_purge_paths(database, &operation.id)?;
    if paths
        .iter()
        .filter(|path| path.path_kind == "trash_entry")
        .count()
        != 1
    {
        return Err("The purge journal does not contain exactly one Trash entry path".to_owned());
    }
    for path in paths.iter().rev() {
        validate_purge_journal_path(operation, path)?;
        let base = purge_path_base(&root, artifact_store, path)?;
        let expected = (path.expected_identity, path.expected_is_directory);
        let original = purge_path_identity(base, &path.original_relative_path)?;
        let staged = purge_path_identity(base, &path.staged_relative_path)?;
        match (original, staged) {
            (Some(actual), None) if actual == expected => {}
            (None, Some(actual)) if actual == expected => {
                atomic_move_managed_item_without_overwrite(
                    base,
                    &path.staged_relative_path,
                    &path.original_relative_path,
                    Some((path.expected_identity, Some(path.expected_is_directory))),
                )
                .map_err(|error| {
                    format!(
                        "Unable to roll back the pending purge: {}",
                        error.into_message()
                    )
                })?;
            }
            (Some(_), Some(_)) => {
                return Err(
                    "Pending purge recovery is ambiguous because both paths are occupied"
                        .to_owned(),
                )
            }
            (None, None) => {
                return Err(
                    "Pending purge recovery is ambiguous because both paths are missing".to_owned(),
                )
            }
            _ => {
                return Err(
                    "Pending purge recovery stopped because a staged item identity changed"
                        .to_owned(),
                )
            }
        }
    }
    remove_purge_journal(database, &operation.id)?;
    cleanup_purge_staging_directories(&root, artifact_store, &operation.id);
    Ok(())
}

fn finalize_committed_purge_operation(
    database: &Database,
    artifact_store: &Path,
    operation: &PurgeOperationRecord,
) -> Result<(), String> {
    let root = require_ready_root(database)?;
    let paths = load_purge_paths(database, &operation.id)?;
    if paths
        .iter()
        .filter(|path| path.path_kind == "trash_entry")
        .count()
        != 1
    {
        return Err("The purge journal does not contain exactly one Trash entry path".to_owned());
    }
    for path in &paths {
        validate_purge_journal_path(operation, path)?;
        let base = purge_path_base(&root, artifact_store, path)?;
        let original = purge_path_identity(base, &path.original_relative_path)?;
        let staged = purge_path_identity(base, &path.staged_relative_path)?;
        if original.is_some() {
            return Err(
                "Committed purge recovery stopped because an original path is occupied".to_owned(),
            );
        }
        match staged {
            None => continue,
            Some((identity, is_directory))
                if identity == path.expected_identity
                    && is_directory == path.expected_is_directory => {}
            Some(_) => {
                return Err(
                    "Committed purge recovery stopped because a staged item identity changed"
                        .to_owned(),
                )
            }
        }
        let staged_path = physical_path(base, &path.staged_relative_path)?;
        validate_regular_tree(&staged_path)?;
        let identity_after_validation = purge_path_identity(base, &path.staged_relative_path)?;
        if identity_after_validation != Some((path.expected_identity, path.expected_is_directory)) {
            return Err("A staged purge item changed during validation".to_owned());
        }
        fs::remove_dir_all(&staged_path).map_err(|error| {
            format!(
                "Unable to permanently delete staged purge content {}: {error}",
                staged_path.display()
            )
        })?;
        if let Some(parent) = staged_path.parent() {
            sync_directory(parent)?;
        }
    }
    remove_purge_journal(database, &operation.id)?;
    cleanup_purge_staging_directories(&root, artifact_store, &operation.id);
    Ok(())
}

pub(crate) fn recover_pending_purge_operations(
    database: &Database,
    artifact_store: &Path,
) -> Result<(), String> {
    for operation in load_purge_operations(database)? {
        validate_purge_operation(&operation)?;
        let members = load_purge_members(database, &operation.id)?;
        if members.is_empty()
            || members.iter().any(|member| {
                !matches!(member.member_type.as_str(), "file" | "folder")
                    || canonical_uuid(&member.member_id, &member.member_type).is_err()
                    || member.depth < 0
            })
        {
            return Err("The purge journal members are invalid".to_owned());
        }
        match operation.phase.as_str() {
            "staging" => rollback_staging_purge_operation(database, artifact_store, &operation)?,
            "database_committed" => {
                finalize_committed_purge_operation(database, artifact_store, &operation)?
            }
            _ => return Err("The purge journal phase is invalid".to_owned()),
        }
    }
    Ok(())
}

fn recover_pending_purge_operations_if_root_ready(
    database: &Database,
    artifact_store: &Path,
) -> Result<(), String> {
    let root = read_storage_root(database)?;
    if inspect_root(root.as_deref()) == "ready" {
        recover_pending_purge_operations(database, artifact_store)?;
    }
    Ok(())
}

fn prepare_purge_operation(
    database: &Database,
    artifact_store: &Path,
    entry_id: &str,
) -> Result<
    (
        PurgeOperationRecord,
        Vec<PurgeMemberRecord>,
        Vec<PurgePathRecord>,
    ),
    String,
> {
    let entry_id = canonical_uuid(entry_id, "Trash entry")?;
    let entry = load_trash_entry(database, &entry_id)?;
    let root = require_ready_root(database)?;
    let container_relative = trash_entry_container_relative(&entry)?;
    let container_path = physical_path(&root, &container_relative)?;
    validate_regular_tree(&container_path)?;
    let (container_identity, container_is_directory) =
        managed_item_identity(&root, &container_relative)?
            .ok_or_else(|| "The Trash entry container no longer exists".to_owned())?;
    if !container_is_directory {
        return Err("The Trash entry container is not a regular directory".to_owned());
    }
    let payload_path = physical_path(&root, &entry.payload_relative_path)?;
    let payload_metadata = fs::symlink_metadata(&payload_path)
        .map_err(|error| format!("Unable to inspect the Trash payload: {error}"))?;
    let payload_matches = match entry.item_type.as_str() {
        "file" => payload_metadata.is_file() && !payload_metadata.file_type().is_symlink(),
        "folder" => payload_metadata.is_dir() && !payload_metadata.file_type().is_symlink(),
        _ => return Err("The Trash item type is unsupported".to_owned()),
    };
    if !payload_matches {
        return Err("The Trash payload type does not match its record".to_owned());
    }

    let (file_ids, folder_rows) = {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
        let mut file_statement = connection
            .prepare(
                "SELECT member.file_id
                 FROM file_space_trash_entry_files member
                 JOIN files file ON file.id = member.file_id
                 WHERE member.entry_id = ?1 AND file.trashed_at IS NOT NULL
                 ORDER BY member.file_id",
            )
            .map_err(|error| format!("Unable to prepare purge files: {error}"))?;
        let file_ids = file_statement
            .query_map([&entry_id], |row| row.get::<_, String>(0))
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|error| format!("Unable to load purge files: {error}"))?;
        let mut folder_statement = connection
            .prepare(
                "SELECT member.folder_id, folder.relative_path
                 FROM file_space_trash_entry_folders member
                 JOIN file_space_folders folder ON folder.id = member.folder_id
                 WHERE member.entry_id = ?1 AND folder.trashed_at IS NOT NULL
                 ORDER BY folder.relative_path, member.folder_id",
            )
            .map_err(|error| format!("Unable to prepare purge folders: {error}"))?;
        let folder_rows = folder_statement
            .query_map([&entry_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|error| format!("Unable to load purge folders: {error}"))?;
        (file_ids, folder_rows)
    };
    if file_ids.is_empty() && folder_rows.is_empty() {
        return Err("The Trash entry has no recorded members".to_owned());
    }
    if entry.item_type == "file"
        && (file_ids.len() != 1
            || entry.root_file_id.as_deref() != file_ids.first().map(String::as_str)
            || !folder_rows.is_empty())
    {
        return Err("The Trash file membership is invalid".to_owned());
    }
    if entry.item_type == "folder"
        && !folder_rows
            .iter()
            .any(|(folder_id, _)| Some(folder_id.as_str()) == entry.root_folder_id.as_deref())
    {
        return Err("The Trash folder membership is invalid".to_owned());
    }

    let operation_id = Uuid::new_v4().to_string();
    let operation = PurgeOperationRecord {
        id: operation_id.clone(),
        entry_id,
        item_type: entry.item_type,
        phase: "staging".to_owned(),
    };
    let mut members = Vec::with_capacity(file_ids.len() + folder_rows.len());
    for file_id in &file_ids {
        members.push(PurgeMemberRecord {
            member_type: "file".to_owned(),
            member_id: canonical_uuid(file_id, "file")?,
            depth: 0,
        });
    }
    for (folder_id, relative_path) in &folder_rows {
        members.push(PurgeMemberRecord {
            member_type: "folder".to_owned(),
            member_id: canonical_uuid(folder_id, "folder")?,
            depth: relative_path.split('/').count() as i64,
        });
    }

    let mut paths = vec![PurgePathRecord {
        path_kind: "trash_entry".to_owned(),
        base_kind: "storage".to_owned(),
        original_relative_path: container_relative,
        staged_relative_path: format!(
            "{TRASH_DIRECTORY_NAME}/{PURGE_DIRECTORY_NAME}/{operation_id}/entry"
        ),
        expected_identity: container_identity,
        expected_is_directory: true,
    }];
    for file_id in &file_ids {
        let versions = {
            let connection = database
                .0
                .lock()
                .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
            let artifact_exists = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM file_space_artifacts WHERE file_id = ?1)",
                    [file_id],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(|error| format!("Unable to inspect Task file history: {error}"))?;
            if !artifact_exists {
                Vec::new()
            } else {
                let mut statement = connection
                    .prepare(
                        "SELECT id, snapshot_path, sha256, size_bytes
                         FROM file_space_artifact_versions WHERE file_id = ?1
                         ORDER BY version_number, id",
                    )
                    .map_err(|error| format!("Unable to prepare Task file history: {error}"))?;
                let versions = statement
                    .query_map([file_id], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, i64>(3)?,
                        ))
                    })
                    .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
                    .map_err(|error| format!("Unable to load Task file history: {error}"))?;
                if versions.is_empty() {
                    return Err("The Task file history has no recorded versions".to_owned());
                }
                versions
            }
        };
        if versions.is_empty() {
            continue;
        }
        let (relative, identity) = validate_artifact_directory(artifact_store, file_id, &versions)?;
        paths.push(PurgePathRecord {
            path_kind: "artifact_directory".to_owned(),
            base_kind: "artifact_store".to_owned(),
            original_relative_path: relative.clone(),
            staged_relative_path: format!(
                "{ARTIFACT_PURGE_DIRECTORY_NAME}/{operation_id}/{relative}"
            ),
            expected_identity: identity,
            expected_is_directory: true,
        });
    }
    Ok((operation, members, paths))
}

fn persist_purge_journal(
    database: &Database,
    operation: &PurgeOperationRecord,
    members: &[PurgeMemberRecord],
    paths: &[PurgePathRecord],
) -> Result<(), String> {
    let now = now_millis();
    let mut connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Unable to begin purge journal: {error}"))?;
    transaction
        .execute(
            "INSERT INTO file_space_purge_operations
             (id, entry_id, item_type, phase, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'staging', ?4, ?4)",
            params![operation.id, operation.entry_id, operation.item_type, now],
        )
        .map_err(|error| format!("Unable to save purge operation: {error}"))?;
    for member in members {
        transaction
            .execute(
                "INSERT INTO file_space_purge_members
                 (operation_id, member_type, member_id, depth)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    operation.id,
                    member.member_type,
                    member.member_id,
                    member.depth
                ],
            )
            .map_err(|error| format!("Unable to save purge member: {error}"))?;
    }
    for path in paths {
        validate_purge_journal_path(operation, path)?;
        transaction
            .execute(
                "INSERT INTO file_space_purge_paths
                 (operation_id, path_kind, base_kind, original_relative_path,
                  staged_relative_path, expected_device, expected_inode,
                  expected_is_directory)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    operation.id,
                    path.path_kind,
                    path.base_kind,
                    path.original_relative_path,
                    path.staged_relative_path,
                    path.expected_identity.device.to_string(),
                    path.expected_identity.inode.to_string(),
                    path.expected_is_directory
                ],
            )
            .map_err(|error| format!("Unable to save purge path: {error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("Unable to complete purge journal: {error}"))
}

fn commit_purge_database(
    database: &Database,
    operation: &PurgeOperationRecord,
    members: &[PurgeMemberRecord],
) -> Result<(), String> {
    let mut connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Unable to begin permanent Trash deletion: {error}"))?;
    let phase = transaction
        .query_row(
            "SELECT phase FROM file_space_purge_operations WHERE id = ?1",
            [&operation.id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("Unable to verify the purge journal: {error}"))?;
    if phase.as_deref() != Some("staging") {
        return Err("The purge journal is no longer in its staging phase".to_owned());
    }

    for member in members.iter().filter(|member| member.member_type == "file") {
        let deleted = transaction
            .execute(
                "DELETE FROM files WHERE id = ?1 AND trashed_at IS NOT NULL",
                [&member.member_id],
            )
            .map_err(|error| format!("Unable to delete a Trash file record: {error}"))?;
        if deleted != 1 {
            return Err("A Trash file record changed before permanent deletion".to_owned());
        }
    }
    let mut folders = members
        .iter()
        .filter(|member| member.member_type == "folder")
        .collect::<Vec<_>>();
    folders.sort_by(|left, right| {
        right
            .depth
            .cmp(&left.depth)
            .then_with(|| left.member_id.cmp(&right.member_id))
    });
    for member in folders {
        let deleted = transaction
            .execute(
                "DELETE FROM file_space_folders WHERE id = ?1 AND trashed_at IS NOT NULL",
                [&member.member_id],
            )
            .map_err(|error| format!("Unable to delete a Trash folder record: {error}"))?;
        if deleted != 1 {
            return Err("A Trash folder record changed before permanent deletion".to_owned());
        }
    }
    transaction
        .execute(
            "DELETE FROM file_space_trash_entries WHERE id = ?1",
            [&operation.entry_id],
        )
        .map_err(|error| format!("Unable to delete the Trash entry record: {error}"))?;
    let remaining = transaction
        .query_row(
            "SELECT COUNT(*) FROM file_space_trash_entries WHERE id = ?1",
            [&operation.entry_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("Unable to verify Trash entry deletion: {error}"))?;
    if remaining != 0 {
        return Err("The Trash entry record was not deleted".to_owned());
    }
    let updated = transaction
        .execute(
            "UPDATE file_space_purge_operations
             SET phase = 'database_committed', updated_at = ?1
             WHERE id = ?2 AND phase = 'staging'",
            params![now_millis(), operation.id],
        )
        .map_err(|error| format!("Unable to commit the purge journal phase: {error}"))?;
    if updated != 1 {
        return Err("The purge journal phase changed unexpectedly".to_owned());
    }
    transaction
        .commit()
        .map_err(|error| format!("Unable to complete permanent Trash deletion: {error}"))
}

fn purge_operation_phase(
    database: &Database,
    operation_id: &str,
) -> Result<Option<String>, String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    connection
        .query_row(
            "SELECT phase FROM file_space_purge_operations WHERE id = ?1",
            [operation_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("Unable to inspect the purge journal phase: {error}"))
}

fn finalize_committed_purge_with_warning(
    database: &Database,
    artifact_store: &Path,
    operation: PurgeOperationRecord,
) -> Option<String> {
    let operation = PurgeOperationRecord {
        phase: "database_committed".to_owned(),
        ..operation
    };
    finalize_committed_purge_operation(database, artifact_store, &operation)
        .err()
        .map(|error| {
            format!(
                "The Trash record was permanently deleted, but staged disk cleanup is pending and will be retried: {error}"
            )
        })
}

fn purge_trash_entry_record(
    database: &Database,
    artifact_store: &Path,
    entry_id: &str,
) -> Result<Option<String>, String> {
    recover_pending_purge_operations(database, artifact_store)?;
    recover_pending_trash_operation(database)?;
    let root = require_ready_root(database)?;
    let (operation, members, paths) = prepare_purge_operation(database, artifact_store, entry_id)?;
    persist_purge_journal(database, &operation, &members, &paths)?;
    if let Err(error) = prepare_purge_staging_directories(
        &root,
        artifact_store,
        &operation.id,
        paths.iter().any(|path| path.base_kind == "artifact_store"),
    ) {
        return match rollback_staging_purge_operation(database, artifact_store, &operation) {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(format!("{error}. {rollback_error}")),
        };
    }
    for path in &paths {
        let base = purge_path_base(&root, artifact_store, path)?;
        if let Err(error) = atomic_move_managed_item_without_overwrite(
            base,
            &path.original_relative_path,
            &path.staged_relative_path,
            Some((path.expected_identity, Some(path.expected_is_directory))),
        ) {
            let error = format!(
                "Unable to stage permanent deletion: {}",
                error.into_message()
            );
            return match rollback_staging_purge_operation(database, artifact_store, &operation) {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(format!("{error}. {rollback_error}")),
            };
        }
    }
    if let Err(error) = validate_staged_purge_paths(&root, artifact_store, &operation, &paths) {
        return match rollback_staging_purge_operation(database, artifact_store, &operation) {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(format!("{error}. {rollback_error}")),
        };
    }
    if let Err(error) = commit_purge_database(database, &operation, &members) {
        // A failed SQLite COMMIT can leave its durability outcome uncertain. Inspect the
        // journal before deciding whether a rollback is safe: once database_committed is
        // visible, payloads must never be restored into Trash.
        return match purge_operation_phase(database, &operation.id) {
            Ok(Some(phase)) if phase == "database_committed" => Ok(
                finalize_committed_purge_with_warning(database, artifact_store, operation)
                    .or_else(|| {
                        Some(format!(
                            "The Trash record was permanently deleted even though the database commit reported an error: {error}"
                        ))
                    }),
            ),
            Ok(Some(phase)) if phase == "staging" => {
                match rollback_staging_purge_operation(database, artifact_store, &operation) {
                    Ok(()) => Err(error),
                    Err(rollback_error) => Err(format!("{error}. {rollback_error}")),
                }
            }
            Ok(Some(phase)) => Err(format!(
                "{error}. The purge journal has an unsupported phase ({phase}), so no filesystem rollback was attempted"
            )),
            Ok(None) => Err(format!(
                "{error}. The purge journal disappeared, so no filesystem rollback was attempted"
            )),
            Err(phase_error) => Err(format!(
                "{error}. The database outcome could not be determined, so no filesystem rollback was attempted: {phase_error}"
            )),
        };
    }
    Ok(finalize_committed_purge_with_warning(
        database,
        artifact_store,
        operation,
    ))
}

fn purge_entry_ids(
    database: &Database,
    artifact_store: &Path,
    entry_ids: &[String],
) -> PurgeBatchSummary {
    let mut summary = PurgeBatchSummary::default();
    for entry_id in entry_ids {
        match purge_trash_entry_record(database, artifact_store, entry_id) {
            Ok(warning) => {
                summary.purged_count += 1;
                if let Some(warning) = warning {
                    summary
                        .cleanup_warnings
                        .push(format!("{entry_id}: {warning}"));
                }
            }
            Err(error) => {
                summary.failed_count += 1;
                summary
                    .failure_messages
                    .push(format!("{entry_id}: {error}"));
            }
        }
    }
    summary
}

fn validate_requested_trash_entry_ids(
    database: &Database,
    entry_ids: &[String],
) -> Result<Vec<String>, String> {
    let mut requested = HashSet::with_capacity(entry_ids.len());
    let mut validated = Vec::with_capacity(entry_ids.len());
    for entry_id in entry_ids {
        let entry_id = canonical_uuid(entry_id, "Trash entry")?;
        if !requested.insert(entry_id.clone()) {
            return Err("The Trash deletion request contains a duplicate entry".to_owned());
        }
        validated.push(entry_id);
    }
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    for entry_id in &validated {
        let exists = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM file_space_trash_entries WHERE id = ?1)",
                [entry_id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|error| format!("Unable to verify a requested Trash entry: {error}"))?;
        if !exists {
            return Err(format!(
                "The requested Trash entry no longer exists: {entry_id}"
            ));
        }
    }
    Ok(validated)
}

fn expired_trash_entry_ids(
    database: &Database,
    now: i64,
    limit: usize,
) -> Result<Vec<String>, String> {
    let cutoff = now
        .checked_sub(TRASH_RETENTION_MILLIS)
        .ok_or_else(|| "The Trash retention cutoff is out of range".to_owned())?;
    let limit = i64::try_from(limit).map_err(|_| "The purge batch limit is invalid".to_owned())?;
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let mut statement = connection
        .prepare(
            "SELECT id FROM file_space_trash_entries
             WHERE trashed_at <= ?1 ORDER BY trashed_at, id LIMIT ?2",
        )
        .map_err(|error| format!("Unable to prepare expired Trash entries: {error}"))?;
    statement
        .query_map(params![cutoff, limit], |row| row.get::<_, String>(0))
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .map_err(|error| format!("Unable to load expired Trash entries: {error}"))
}

fn purge_expired_trash_entries_record(
    database: &Database,
    artifact_store: &Path,
    now: i64,
) -> Result<PurgeBatchSummary, String> {
    recover_pending_purge_operations(database, artifact_store)?;
    recover_pending_trash_operation(database)?;
    let entry_ids = expired_trash_entry_ids(database, now, EXPIRED_TRASH_PURGE_BATCH_SIZE)?;
    Ok(purge_entry_ids(database, artifact_store, &entry_ids))
}

fn empty_trash_entries_record(
    database: &Database,
    artifact_store: &Path,
    entry_ids: &[String],
) -> Result<PurgeBatchSummary, String> {
    recover_pending_purge_operations(database, artifact_store)?;
    recover_pending_trash_operation(database)?;
    let entry_ids = validate_requested_trash_entry_ids(database, entry_ids)?;
    Ok(purge_entry_ids(database, artifact_store, &entry_ids))
}

fn purge_result(
    database: &Database,
    summary: PurgeBatchSummary,
) -> Result<FileSpaceTrashPurgeResult, String> {
    let mut messages = summary.failure_messages;
    messages.extend(summary.cleanup_warnings);
    Ok(FileSpaceTrashPurgeResult {
        snapshot: load_snapshot_record(database)?,
        purged_count: summary.purged_count,
        failed_count: summary.failed_count,
        failure_message: (!messages.is_empty()).then(|| messages.join("\n")),
    })
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

fn import_destination_record(
    database: &Database,
    folder_id: Option<&str>,
) -> Result<(PathBuf, Option<String>, PathBuf), String> {
    let root = require_ready_root(database)?;
    let folder_path = match folder_id {
        Some(folder_id) => {
            let connection = database
                .0
                .lock()
                .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
    let directory = match folder_path.as_deref() {
        Some(path) => physical_path(&root, path)?,
        None => root.clone(),
    };
    Ok((root, folder_path, directory))
}

fn managed_import_conflict_record(
    database: &Database,
    relative_path: &str,
) -> Result<Option<(String, i64, i64)>, String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    connection
        .query_row(
            "SELECT file.id, COALESCE(version.version_number, 0),
                    COALESCE(file.size_bytes, 0)
             FROM files file
             LEFT JOIN file_space_artifacts artifact ON artifact.file_id = file.id
             LEFT JOIN file_space_artifact_versions version
               ON version.id = artifact.current_version_id
             WHERE file.storage_path = ?1 AND file.trashed_at IS NULL",
            [relative_path],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("Unable to inspect the existing imported file: {error}"))
}

fn inspect_dropped_file_conflicts(
    database: &Database,
    artifact_store: &Path,
    folder_id: Option<&str>,
    files: &[FileSpaceDroppedFile],
) -> Result<
    (
        FileSpaceDroppedFileConflictInspection,
        Vec<FileSpaceVersionCreatedNotification>,
    ),
    String,
> {
    capture_initial_user_versions(database, artifact_store)?;
    let (root, folder_path, directory) = import_destination_record(database, folder_id)?;
    let mut conflicts = Vec::new();
    let mut notifications = Vec::new();
    for file in files {
        let dropped_path = dropped_relative_path(&file.relative_path)?;
        if dropped_path.components().count() != 1 {
            continue;
        }
        let file_name = dropped_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "A dropped file has an unsupported name".to_owned())?
            .to_owned();
        let relative_path = relative_child(folder_path.as_deref(), &file_name);
        let Some((file_id, _, _)) = managed_import_conflict_record(database, &relative_path)?
        else {
            continue;
        };
        if let Some(notification) =
            reconcile_task_file_with_options(database, artifact_store, &file_id, true)?
        {
            notifications.push(notification);
        }
        let Some((existing_file_id, existing_version, existing_size_bytes)) =
            managed_import_conflict_record(database, &relative_path)?
        else {
            continue;
        };
        let existing_path = physical_path(&root, &relative_path)?;
        if !existing_path.starts_with(&directory) {
            return Err("The conflicting file is outside the selected destination".to_owned());
        }
        let existing_sha256 = sha256_file(&existing_path)?.0;
        let incoming_size_bytes = i64::try_from(file.bytes.len())
            .map_err(|_| format!("The dropped file is too large: {file_name}"))?;
        let incoming_sha256 = format!("{:x}", Sha256::digest(&file.bytes));
        conflicts.push(FileSpaceDroppedFileConflict {
            relative_path: file.relative_path.clone(),
            file_name,
            existing_file_id,
            existing_version,
            existing_size_bytes,
            incoming_size_bytes,
            identical: incoming_sha256 == existing_sha256,
        });
    }
    Ok((
        FileSpaceDroppedFileConflictInspection { conflicts },
        notifications,
    ))
}

fn inspect_path_file_conflicts(
    database: &Database,
    artifact_store: &Path,
    folder_id: Option<&str>,
    paths: &[String],
) -> Result<
    (
        FileSpaceDroppedFileConflictInspection,
        Vec<FileSpaceVersionCreatedNotification>,
    ),
    String,
> {
    capture_initial_user_versions(database, artifact_store)?;
    let (root, folder_path, directory) = import_destination_record(database, folder_id)?;
    let import_roots = canonical_import_roots(paths)?;
    let mut conflicts = Vec::new();
    let mut notifications = Vec::new();
    for source in import_roots {
        let metadata = fs::symlink_metadata(&source)
            .map_err(|error| format!("Unable to inspect {}: {error}", source.display()))?;
        if !metadata.is_file() {
            continue;
        }
        let file_name = source
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "A selected file has an unsupported name".to_owned())?
            .to_owned();
        let relative_path = relative_child(folder_path.as_deref(), &file_name);
        let Some((file_id, _, _)) = managed_import_conflict_record(database, &relative_path)?
        else {
            continue;
        };
        if let Some(notification) =
            reconcile_task_file_with_options(database, artifact_store, &file_id, true)?
        {
            notifications.push(notification);
        }
        let Some((existing_file_id, existing_version, existing_size_bytes)) =
            managed_import_conflict_record(database, &relative_path)?
        else {
            continue;
        };
        let existing_path = physical_path(&root, &relative_path)?;
        if !existing_path.starts_with(&directory) {
            return Err("The conflicting file is outside the selected destination".to_owned());
        }
        let existing_sha256 = sha256_file(&existing_path)?.0;
        let (incoming_sha256, incoming_size_bytes) = sha256_file(&source)?;
        conflicts.push(FileSpaceDroppedFileConflict {
            relative_path: source.to_string_lossy().into_owned(),
            file_name,
            existing_file_id,
            existing_version,
            existing_size_bytes,
            incoming_size_bytes,
            identical: incoming_sha256 == existing_sha256,
        });
    }
    Ok((
        FileSpaceDroppedFileConflictInspection { conflicts },
        notifications,
    ))
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
        let metadata = fs::symlink_metadata(&source)
            .map_err(|error| format!("Unable to inspect {}: {error}", source.display()))?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        progress(&source)?;
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

#[derive(Debug)]
struct PendingImportedVersion {
    file_id: String,
    file_name: String,
    mime_type: Option<String>,
    version_id: String,
    version_number: i64,
    snapshot_path: PathBuf,
    sha256: String,
    size_bytes: i64,
    mtime_ms: i64,
    created_at: i64,
    replacement: PendingWorkingCopyReplacement,
}

impl PendingImportedVersion {
    fn save(&self, transaction: &rusqlite::Transaction<'_>) -> Result<(), String> {
        transaction
            .execute(
                "INSERT INTO file_space_artifact_versions
                 (id, file_id, version_number, snapshot_path, sha256, size_bytes,
                  produced_name, mime_type, origin, produced_at, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'user_edit', ?9, ?10)",
                params![
                    self.version_id,
                    self.file_id,
                    self.version_number,
                    self.snapshot_path.to_string_lossy(),
                    self.sha256,
                    self.size_bytes,
                    self.file_name,
                    self.mime_type,
                    self.created_at,
                    self.created_at
                ],
            )
            .map_err(|error| format!("Unable to save the imported file version: {error}"))?;
        transaction
            .execute(
                "UPDATE file_space_artifacts
                 SET current_version_id = ?1, observed_size_bytes = ?2,
                     observed_mtime_ms = ?3, observed_sha256 = ?4, updated_at = ?5
                 WHERE file_id = ?6",
                params![
                    self.version_id,
                    self.size_bytes,
                    self.mtime_ms,
                    self.sha256,
                    self.created_at,
                    self.file_id
                ],
            )
            .map_err(|error| format!("Unable to select the imported file version: {error}"))?;
        transaction
            .execute(
                "UPDATE files SET size_bytes = ?1, updated_at = ?2 WHERE id = ?3",
                params![self.size_bytes, self.created_at, self.file_id],
            )
            .map_err(|error| format!("Unable to update the imported file: {error}"))?;
        transaction
            .execute(
                "INSERT INTO file_space_artifact_events
                 (id, file_id, version_id, event_type, actor, details_json, created_at)
                 VALUES (?1, ?2, ?3, 'modified', 'user', ?4, ?5)",
                params![
                    Uuid::new_v4().to_string(),
                    self.file_id,
                    self.version_id,
                    json!({
                        "version": self.version_number,
                        "source": "drag_import"
                    })
                    .to_string(),
                    self.created_at
                ],
            )
            .map_err(|error| format!("Unable to save the imported file event: {error}"))?;
        Ok(())
    }

    fn commit(self) {
        self.replacement.commit();
    }

    fn rollback(self) -> Result<(), String> {
        let snapshot_path = self.snapshot_path;
        let rollback = self.replacement.rollback();
        let cleanup = fs::remove_file(&snapshot_path);
        match (rollback, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Ok(()), Err(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            (Err(error), Ok(())) => Err(error),
            (Ok(()), Err(error)) => Err(format!(
                "Unable to remove the uncommitted version snapshot at {}: {error}",
                snapshot_path.display()
            )),
            (Err(rollback_error), Err(cleanup_error)) => Err(format!(
                "{rollback_error}. Unable to remove the uncommitted version snapshot at {}: {cleanup_error}",
                snapshot_path.display()
            )),
        }
    }
}

fn prepare_pending_imported_version(
    database: &Database,
    artifact_store: &Path,
    root: &Path,
    source: &Path,
    relative_path: &str,
    file_id: &str,
) -> Result<Option<PendingImportedVersion>, String> {
    reconcile_task_file_with_options(database, artifact_store, file_id, true)?;
    let (file_name, mime_type, observed_sha256, version_number) = {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
        connection
            .query_row(
                "SELECT file.original_name, file.mime_type, artifact.observed_sha256,
                        (SELECT COALESCE(MAX(version_number), 0) + 1
                         FROM file_space_artifact_versions WHERE file_id = file.id)
                 FROM files file
                 JOIN file_space_artifacts artifact ON artifact.file_id = file.id
                 WHERE file.id = ?1 AND file.trashed_at IS NULL",
                [file_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| format!("Unable to load the conflicting file: {error}"))?
            .ok_or_else(|| "The conflicting file no longer exists".to_owned())?
    };
    let observed_sha256 =
        observed_sha256.ok_or_else(|| "The conflicting file observation is missing".to_owned())?;
    let (sha256, size_bytes) = sha256_file(source)?;
    if sha256 == observed_sha256 {
        return Ok(None);
    }
    let destination = physical_path(root, relative_path)?;
    let version_id = Uuid::new_v4().to_string();
    let snapshot_path = artifact_store
        .join(file_id)
        .join(format!("{version_id}.blob"));
    snapshot_file(source, &snapshot_path)?;
    let replacement = match prepare_working_copy_replacement(
        source,
        &destination,
        &sha256,
        size_bytes,
        &observed_sha256,
    ) {
        Ok(replacement) => replacement,
        Err(error) => {
            let _ = fs::remove_file(&snapshot_path);
            return Err(error);
        }
    };
    let mtime_ms = match file_mtime_marker(&destination) {
        Ok(mtime) => mtime,
        Err(error) => {
            let rollback_error = replacement.rollback().err();
            let _ = fs::remove_file(&snapshot_path);
            return Err(match rollback_error {
                Some(rollback_error) => format!("{error}. {rollback_error}"),
                None => error,
            });
        }
    };
    Ok(Some(PendingImportedVersion {
        file_id: file_id.to_owned(),
        file_name,
        mime_type,
        version_id,
        version_number,
        snapshot_path,
        sha256,
        size_bytes,
        mtime_ms,
        created_at: now_millis(),
        replacement,
    }))
}

fn rollback_pending_imported_versions(
    pending_versions: Vec<PendingImportedVersion>,
) -> Result<(), String> {
    let mut errors = Vec::new();
    for pending in pending_versions.into_iter().rev() {
        if let Err(error) = pending.rollback() {
            errors.push(error);
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
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

fn import_files_record_with_progress_policy<F>(
    database: &Database,
    folder_id: Option<&str>,
    paths: &[String],
    artifact_store: Option<&Path>,
    conflict_action: Option<DroppedFileConflictAction>,
    mut progress: F,
) -> Result<FileSpaceSnapshot, String>
where
    F: FnMut(&Path) -> Result<(), String>,
{
    if paths.is_empty() {
        return load_snapshot_record(database);
    }
    let (root, folder_path, directory) = import_destination_record(database, folder_id)?;
    let mut import_roots = canonical_import_roots(paths)?;
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

    let mut pending_versions = Vec::<PendingImportedVersion>::new();
    let mut handled_sources = HashSet::<PathBuf>::new();
    let conflict_plan_result = (|| -> Result<(), String> {
        let Some(conflict_action) = conflict_action else {
            return Ok(());
        };
        let artifact_store =
            artifact_store.ok_or_else(|| "The file version store is unavailable".to_owned())?;
        capture_initial_user_versions(database, artifact_store)?;
        for source in &import_roots {
            let metadata = fs::symlink_metadata(source)
                .map_err(|error| format!("Unable to inspect {}: {error}", source.display()))?;
            if !metadata.is_file() {
                continue;
            }
            let original_name = source
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| "A selected item has an unsupported name".to_owned())?;
            let original_name = validate_import_name(original_name)?;
            let relative_path = relative_child(folder_path.as_deref(), &original_name);
            let Some((file_id, _, _)) = managed_import_conflict_record(database, &relative_path)?
            else {
                continue;
            };
            match conflict_action {
                DroppedFileConflictAction::LatestVersion => {
                    progress(source)?;
                    if let Some(pending) = prepare_pending_imported_version(
                        database,
                        artifact_store,
                        &root,
                        source,
                        &relative_path,
                        &file_id,
                    )? {
                        pending_versions.push(pending);
                    }
                    handled_sources.insert(source.clone());
                }
                DroppedFileConflictAction::Rename => {
                    reconcile_task_file_with_options(database, artifact_store, &file_id, true)?;
                    let incoming_sha256 = sha256_file(source)?.0;
                    if incoming_sha256 == task_file_observed_sha256(database, &file_id)? {
                        progress(source)?;
                        handled_sources.insert(source.clone());
                    }
                }
            }
        }
        Ok(())
    })();
    if let Err(error) = conflict_plan_result {
        return match rollback_pending_imported_versions(pending_versions) {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(format!(
                "{error}. Unable to fully roll back the imported versions: {rollback_error}"
            )),
        };
    }
    import_roots.retain(|source| !handled_sources.contains(source));

    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| format!("Unable to begin the file import: {error}"))?;
    let mut imported_destinations = Vec::new();
    let import_result = (|| -> Result<(), String> {
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
        for pending in &pending_versions {
            pending.save(&transaction)?;
        }
        Ok(())
    })();

    match import_result {
        Ok(()) => {}
        Err(error) => {
            drop(transaction);
            let mut rollback_errors = Vec::new();
            if let Err(rollback_error) = rollback_imported_destinations(&imported_destinations) {
                rollback_errors.push(rollback_error);
            }
            if let Err(rollback_error) = rollback_pending_imported_versions(pending_versions) {
                rollback_errors.push(rollback_error);
            }
            return if rollback_errors.is_empty() {
                Err(error)
            } else {
                Err(format!(
                    "{error}. Unable to fully roll back the imported items: {}",
                    rollback_errors.join("; ")
                ))
            };
        }
    }
    if let Err(error) = transaction.commit() {
        let mut rollback_errors = Vec::new();
        if let Err(rollback_error) = rollback_imported_destinations(&imported_destinations) {
            rollback_errors.push(rollback_error);
        }
        if let Err(rollback_error) = rollback_pending_imported_versions(pending_versions) {
            rollback_errors.push(rollback_error);
        }
        return if rollback_errors.is_empty() {
            Err(format!("Unable to complete the file import: {error}"))
        } else {
            Err(format!(
                "Unable to complete the file import: {error}. Unable to fully roll back the imported items: {}",
                rollback_errors.join("; ")
            ))
        };
    }
    drop(connection);
    let modified_file_ids = pending_versions
        .iter()
        .map(|pending| pending.file_id.clone())
        .collect::<Vec<_>>();
    for pending in pending_versions {
        pending.commit();
    }
    synchronize_file_search_index_for_files(database, Some(&root), &modified_file_ids)?;
    load_snapshot_record(database)
}

#[cfg(test)]
fn import_files_record_with_progress<F>(
    database: &Database,
    folder_id: Option<&str>,
    paths: &[String],
    progress: F,
) -> Result<FileSpaceSnapshot, String>
where
    F: FnMut(&Path) -> Result<(), String>,
{
    import_files_record_with_progress_policy(database, folder_id, paths, None, None, progress)
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
        return Ok(0);
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

pub(crate) fn emit_import_progress<R: tauri::Runtime>(
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

fn search_file_space_matches_with_generation(
    database: &Database,
    semantic_runtime: Option<&crate::semantic_search::SemanticSearchRuntime>,
    request: &FileSpaceSearchRequest,
    search_generation: Option<u64>,
) -> Result<Vec<FileSpaceSearchMatch>, String> {
    let query = request.query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    if search_generation.is_some_and(|generation| {
        FILE_SPACE_SEARCH_GENERATION.load(AtomicOrdering::Acquire) != generation
    }) {
        return Ok(Vec::new());
    }
    let scopes = normalized_search_scopes(&request.scopes);
    let query_weight = query
        .chars()
        .filter(|character| !character.is_whitespace())
        .map(|character| if character.is_ascii() { 1 } else { 2 })
        .sum::<usize>();
    let fts_scopes = scopes
        .iter()
        .copied()
        .filter(|column| query_weight >= SEARCH_CONTENT_MIN_WEIGHT || *column != "body_text")
        .collect::<Vec<_>>();
    let terms = query
        .split_whitespace()
        .filter(|term| !term.is_empty())
        .map(fts_term)
        .collect::<Vec<_>>();
    let mut matches = HashSet::new();
    let mut lexical = Vec::new();
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;

    if !terms.is_empty() && !fts_scopes.is_empty() {
        let expression = fts_scopes
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
                 WHERE file_space_search_fts MATCH ?1
                 ORDER BY bm25(file_space_search_fts, 8.0, 1.0, 3.0, 1.0, 1.0)
                 LIMIT 200",
            )
            .map_err(|error| format!("Unable to prepare File Space full-text search: {error}"))?;
        let rows = statement.query_map([expression], |row| row.get::<_, String>(0));
        if let Ok(rows) = rows {
            for file_id in rows.flatten() {
                if matches.insert(file_id.clone()) {
                    lexical.push(file_id);
                }
            }
        }
    }

    if scopes.contains(&"file_name") && query.chars().count() <= 256 {
        let mut statement = connection
            .prepare(
                "SELECT documents.file_id
                 FROM file_space_search_documents documents
                 JOIN files ON files.id = documents.file_id
                 WHERE instr(lower(documents.file_name), lower(?1)) > 0
                   AND files.trashed_at IS NULL
                   AND files.storage_path IS NOT NULL
                 ORDER BY files.updated_at DESC, documents.file_id
                 LIMIT 200",
            )
            .map_err(|error| format!("Unable to prepare File Space name search: {error}"))?;
        let rows = statement
            .query_map([query], |row| row.get::<_, String>(0))
            .map_err(|error| format!("Unable to search File Space names: {error}"))?;
        for file_id in rows.flatten() {
            if matches.insert(file_id.clone()) {
                lexical.push(file_id);
            }
        }
    }

    drop(connection);
    if search_generation.is_some_and(|generation| {
        FILE_SPACE_SEARCH_GENERATION.load(AtomicOrdering::Acquire) != generation
    }) {
        return Ok(Vec::new());
    }
    let semantic = match semantic_runtime {
        Some(runtime)
            if query_weight >= SEARCH_CONTENT_MIN_WEIGHT
                && !lexical.is_empty()
                && (request.scopes.is_empty()
                    || request.scopes.iter().any(|scope| scope == "content")) =>
        {
            crate::semantic_search::semantic_file_ranks(database, runtime, query, &lexical)
                .unwrap_or_default()
        }
        _ => Vec::new(),
    };
    Ok(
        crate::semantic_search::merge_hybrid_matches(lexical, semantic)
            .into_iter()
            .map(|result| FileSpaceSearchMatch {
                file_id: result.file_id,
                lexical_match: result.lexical_match,
                semantic_similarity: result.semantic_similarity,
            })
            .collect(),
    )
}

#[cfg(test)]
pub(crate) fn search_file_space_matches(
    database: &Database,
    semantic_runtime: Option<&crate::semantic_search::SemanticSearchRuntime>,
    request: &FileSpaceSearchRequest,
) -> Result<Vec<FileSpaceSearchMatch>, String> {
    search_file_space_matches_with_generation(database, semantic_runtime, request, None)
}

#[cfg(test)]
fn search_file_space_records(
    database: &Database,
    semantic_runtime: Option<&crate::semantic_search::SemanticSearchRuntime>,
    request: &FileSpaceSearchRequest,
) -> Result<Vec<String>, String> {
    Ok(
        search_file_space_matches(database, semantic_runtime, request)?
            .into_iter()
            .map(|result| result.file_id)
            .collect(),
    )
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
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
    let root = read_storage_root(database)?;
    synchronize_file_search_index_for_files(database, root.as_deref(), &[file_id.to_owned()])?;
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
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
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
pub async fn get_file_space_snapshot<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<FileSpaceSnapshot, String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let database = worker_app.state::<Database>();
        let _operation = lock_file_space_operations()?;
        let artifact_store = artifact_store_path(&worker_app)?;
        load_workspace_snapshot(database.inner(), &artifact_store)
    })
    .await
    .map_err(|error| format!("Unable to load File Space: {error}"))?
}

#[tauri::command]
pub async fn list_file_space_files<R: tauri::Runtime>(
    request: FileSpaceFilePageRequest,
    app: tauri::AppHandle<R>,
) -> Result<FileSpaceFilePage, String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let database = worker_app.state::<Database>();
        list_file_space_file_page_record(database.inner(), &request)
    })
    .await
    .map_err(|error| format!("Unable to load a File Space file page: {error}"))?
}

#[tauri::command]
pub async fn scan_file_space_external_changes<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<Vec<FileSpaceVersionCreatedNotification>, String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let database = worker_app.state::<Database>();
        let _operation = lock_file_space_operations()?;
        capture_external_version_changes(database.inner(), &artifact_store_path(&worker_app)?, None)
    })
    .await
    .map_err(|error| format!("Unable to scan File Space changes: {error}"))?
}

#[tauri::command]
pub fn drain_file_space_version_notifications(
    notifications: State<'_, FileSpaceVersionNotificationQueue>,
) -> Result<Vec<FileSpaceVersionCreatedNotification>, String> {
    notifications.drain()
}

#[tauri::command]
pub async fn search_file_space_files<R: tauri::Runtime>(
    request: FileSpaceSearchRequest,
    app: tauri::AppHandle<R>,
) -> Result<Vec<FileSpaceSearchMatch>, String> {
    let search_generation = FILE_SPACE_SEARCH_GENERATION.fetch_add(1, AtomicOrdering::AcqRel) + 1;
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let database = worker_app.state::<Database>();
        let semantic_runtime = worker_app.state::<crate::semantic_search::SemanticSearchRuntime>();
        search_file_space_matches_with_generation(
            database.inner(),
            Some(semantic_runtime.inner()),
            &request,
            Some(search_generation),
        )
    })
    .await
    .map_err(|error| format!("Unable to search File Space: {error}"))?
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
pub async fn export_file_space_backup<R: tauri::Runtime>(
    destination_path: String,
    app: tauri::AppHandle<R>,
) -> Result<FileSpaceBackupResult, String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let database = worker_app.state::<Database>();
        let _operation = lock_file_space_operations()?;
        export_file_space_backup_record(
            database.inner(),
            &artifact_store_path(&worker_app)?,
            &destination_path,
        )
    })
    .await
    .map_err(|error| format!("Unable to run the backup export: {error}"))?
}

#[tauri::command]
pub async fn inspect_file_space_backup(path: String) -> Result<FileSpaceBackupInspection, String> {
    tauri::async_runtime::spawn_blocking(move || inspect_file_space_backup_record(&path))
        .await
        .map_err(|error| format!("Unable to inspect the backup: {error}"))?
}

#[tauri::command]
pub async fn restore_file_space_backup<R: tauri::Runtime>(
    backup_path: String,
    destination_directory: String,
    app: tauri::AppHandle<R>,
) -> Result<FileSpaceSnapshot, String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let database = worker_app.state::<Database>();
        let _operation = lock_file_space_operations()?;
        restore_file_space_backup_record(
            database.inner(),
            &artifact_store_path(&worker_app)?,
            &backup_path,
            &destination_directory,
        )
    })
    .await
    .map_err(|error| format!("Unable to run the backup restore: {error}"))?
}

#[tauri::command]
pub async fn import_existing_file_space_root<R: tauri::Runtime>(
    request_id: String,
    path: String,
    app: tauri::AppHandle<R>,
) -> Result<FileSpaceSnapshot, String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        emit_import_progress(&worker_app, &request_id, "scanning", 0, 0, None);
        let database = worker_app.state::<Database>();
        let _operation = lock_file_space_operations()?;
        let snapshot = import_existing_storage_root_record_with_progress(
            database.inner(),
            &artifact_store_path(&worker_app)?,
            &path,
            |processed, total, current_name| {
                emit_import_progress(
                    &worker_app,
                    &request_id,
                    "importing",
                    processed,
                    total,
                    current_name.map(str::to_owned),
                );
            },
        )?;
        let total = snapshot.file_count.max(0) as usize;
        emit_import_progress(&worker_app, &request_id, "completed", total, total, None);
        Ok(snapshot)
    })
    .await
    .map_err(|error| format!("Unable to initialize the existing folder: {error}"))?
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
pub async fn create_file_space_text_file<R: tauri::Runtime>(
    parent_id: Option<String>,
    name: String,
    format: String,
    app: tauri::AppHandle<R>,
) -> Result<FileSpaceCreatedFileResult, String> {
    let location = app.state::<Database>().location()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = lock_file_space_operations()?;
        let artifact_store = location.artifact_store_path.clone();
        let database = crate::database::open_workspace_database(
            location.database_path,
            location.artifact_store_path,
            location.workspace_id,
        )?;
        create_text_file_record(
            &database,
            &artifact_store,
            parent_id.as_deref(),
            &name,
            &format,
        )
    })
    .await
    .map_err(|error| format!("Unable to create the text file: {error}"))?
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
    trash_folder_record(database.inner(), &artifact_store_path(&app)?, &folder_id)
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
pub fn move_file_space_files<R: tauri::Runtime>(
    file_ids: Vec<String>,
    folder_id: Option<String>,
    app: tauri::AppHandle<R>,
    database: State<'_, Database>,
) -> Result<FileSpaceFilesMoveResult, String> {
    let _operation = lock_file_space_operations()?;
    move_files_record(
        database.inner(),
        &artifact_store_path(&app)?,
        &file_ids,
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
    trash_file_record(database.inner(), &artifact_store_path(&app)?, &file_id)
}

#[tauri::command]
pub fn restore_file_space_trash_entry(
    entry_id: String,
    database: State<'_, Database>,
) -> Result<FileSpaceSnapshot, String> {
    let _operation = lock_file_space_operations()?;
    restore_trash_entry_record(database.inner(), &entry_id)
}

#[tauri::command]
pub async fn empty_file_space_trash<R: tauri::Runtime>(
    entry_ids: Vec<String>,
    app: tauri::AppHandle<R>,
) -> Result<FileSpaceTrashPurgeResult, String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let database = worker_app.state::<Database>();
        let _operation = lock_file_space_operations()?;
        let summary = empty_trash_entries_record(
            database.inner(),
            &artifact_store_path(&worker_app)?,
            &entry_ids,
        )?;
        purge_result(database.inner(), summary)
    })
    .await
    .map_err(|error| format!("Unable to run permanent Trash deletion: {error}"))?
}

#[tauri::command]
pub async fn purge_expired_file_space_trash<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<FileSpaceTrashPurgeResult, String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let database = worker_app.state::<Database>();
        let _operation = lock_file_space_operations()?;
        let summary = purge_expired_trash_entries_record(
            database.inner(),
            &artifact_store_path(&worker_app)?,
            now_millis(),
        )?;
        purge_result(database.inner(), summary)
    })
    .await
    .map_err(|error| format!("Unable to run expired Trash cleanup: {error}"))?
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
    skip_generated_preview: Option<bool>,
    window: tauri::Window<R>,
    app: tauri::AppHandle<R>,
    database: State<'_, Database>,
) -> Result<(), String> {
    let path = {
        let _operation = lock_file_space_operations()?;
        reconcile_before_file_action(database.inner(), &app, &file_id)?;
        active_file_path(database.inner(), &file_id)?
    };
    let (drag_image, preview_cleanup) = file_drag_image(
        &path,
        preview_bytes,
        skip_generated_preview.unwrap_or(false),
    );
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
pub async fn get_task_file_timeline<R: tauri::Runtime>(
    file_id: String,
    app: tauri::AppHandle<R>,
) -> Result<TaskFileTimeline, String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let database = worker_app.state::<Database>();
        let _operation = lock_file_space_operations()?;
        reconcile_task_file(
            database.inner(),
            &artifact_store_path(&worker_app)?,
            &file_id,
        )?;
        load_task_file_timeline_record(database.inner(), &file_id)
    })
    .await
    .map_err(|error| format!("Unable to load the file version timeline: {error}"))?
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
pub async fn read_task_file_version<R: tauri::Runtime>(
    file_id: String,
    version_id: String,
    app: tauri::AppHandle<R>,
) -> Result<Vec<u8>, String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let database = worker_app.state::<Database>();
        read_task_file_version_record(database.inner(), &file_id, &version_id)
    })
    .await
    .map_err(|error| format!("Unable to read the file version: {error}"))?
}

#[tauri::command]
pub async fn read_file_space_markdown<R: tauri::Runtime>(
    file_id: String,
    app: tauri::AppHandle<R>,
) -> Result<String, String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let database = worker_app.state::<Database>();
        let started_at = Instant::now();
        // Preview reads are independent from indexing and extraction writes.
        // Waiting for the coarse File Space operation lock here made a tiny
        // Markdown file wait for an unrelated embedding job to finish.
        let result = read_markdown_file_record(database.inner(), &file_id);
        #[cfg(debug_assertions)]
        eprintln!(
            "[preview-timing] markdown file_id={file_id} duration_ms={:.1}",
            started_at.elapsed().as_secs_f64() * 1_000.0,
        );
        result
    })
    .await
    .map_err(|error| format!("Unable to read the Markdown file: {error}"))?
}

#[tauri::command]
pub async fn read_file_space_text<R: tauri::Runtime>(
    file_id: String,
    app: tauri::AppHandle<R>,
) -> Result<String, String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let database = worker_app.state::<Database>();
        read_text_file_record(database.inner(), &file_id)
    })
    .await
    .map_err(|error| format!("Unable to read the text file: {error}"))?
}

#[tauri::command]
pub async fn save_file_space_markdown<R: tauri::Runtime>(
    file_id: String,
    content: String,
    app: tauri::AppHandle<R>,
) -> Result<FileSpaceSnapshot, String> {
    let location = app.state::<Database>().location()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = lock_file_space_operations()?;
        let artifact_store = location.artifact_store_path.clone();
        let database = crate::database::open_workspace_database(
            location.database_path,
            location.artifact_store_path,
            location.workspace_id,
        )?;
        save_markdown_file_record(&database, &artifact_store, &file_id, &content)
    })
    .await
    .map_err(|error| format!("Unable to save the Markdown file: {error}"))?
}

#[tauri::command]
pub fn inspect_file_space_import_conflicts<R: tauri::Runtime>(
    folder_id: Option<String>,
    paths: Vec<String>,
    app: tauri::AppHandle<R>,
    database: State<'_, Database>,
) -> Result<FileSpaceDroppedFileConflictInspection, String> {
    let _operation = lock_file_space_operations()?;
    let (inspection, notifications) = inspect_path_file_conflicts(
        database.inner(),
        &artifact_store_path(&app)?,
        folder_id.as_deref(),
        &paths,
    )?;
    publish_version_notifications(&app, notifications);
    Ok(inspection)
}

#[tauri::command]
pub async fn import_file_space_files<R: tauri::Runtime>(
    request_id: String,
    folder_id: Option<String>,
    paths: Vec<String>,
    conflict_action: Option<String>,
    app: tauri::AppHandle<R>,
) -> Result<FileSpaceSnapshot, String> {
    let conflict_action = DroppedFileConflictAction::parse(conflict_action.as_deref())?;
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
            let artifact_store = artifact_store_path(&worker_app)?;
            let _snapshot = import_files_record_with_progress_policy(
                database.inner(),
                folder_id.as_deref(),
                &paths,
                Some(&artifact_store),
                conflict_action,
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
pub fn inspect_file_space_dropped_conflicts<R: tauri::Runtime>(
    folder_id: Option<String>,
    files: Vec<FileSpaceDroppedFile>,
    app: tauri::AppHandle<R>,
    database: State<'_, Database>,
) -> Result<FileSpaceDroppedFileConflictInspection, String> {
    let _operation = lock_file_space_operations()?;
    let (inspection, notifications) = inspect_dropped_file_conflicts(
        database.inner(),
        &artifact_store_path(&app)?,
        folder_id.as_deref(),
        &files,
    )?;
    publish_version_notifications(&app, notifications);
    Ok(inspection)
}

#[tauri::command]
pub async fn import_file_space_dropped_files<R: tauri::Runtime>(
    request_id: String,
    folder_id: Option<String>,
    files: Vec<FileSpaceDroppedFile>,
    conflict_action: Option<String>,
    app: tauri::AppHandle<R>,
) -> Result<FileSpaceSnapshot, String> {
    let conflict_action = DroppedFileConflictAction::parse(conflict_action.as_deref())?;
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
            let artifact_store = artifact_store_path(&worker_app)?;
            let _snapshot = import_files_record_with_progress_policy(
                database.inner(),
                folder_id.as_deref(),
                &roots,
                Some(&artifact_store),
                conflict_action,
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
#[path = "file_space_edit_index_tests.rs"]
mod edit_index_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use zip::ZipArchive;

    fn test_workspace(label: &str) -> (PathBuf, PathBuf, PathBuf, Database) {
        let root = std::env::temp_dir().join(format!("lumetrace-{label}-{}", Uuid::new_v4()));
        let storage = root.join("storage");
        let versions = root.join("versions");
        fs::create_dir_all(&storage).unwrap();
        let database = crate::database::open_for_test(&root.join("lumetrace.sqlite3")).unwrap();
        configure_storage_root_record(&database, storage.to_string_lossy().as_ref()).unwrap();
        (root, storage, versions, database)
    }

    fn stage_purge_for_test(
        database: &Database,
        storage: &Path,
        versions: &Path,
        entry_id: &str,
    ) -> (
        PurgeOperationRecord,
        Vec<PurgeMemberRecord>,
        Vec<PurgePathRecord>,
    ) {
        let (operation, members, paths) =
            prepare_purge_operation(database, versions, entry_id).unwrap();
        persist_purge_journal(database, &operation, &members, &paths).unwrap();
        prepare_purge_staging_directories(
            storage,
            versions,
            &operation.id,
            paths.iter().any(|path| path.base_kind == "artifact_store"),
        )
        .unwrap();
        for path in &paths {
            let base = purge_path_base(storage, versions, path).unwrap();
            atomic_move_managed_item_without_overwrite(
                base,
                &path.original_relative_path,
                &path.staged_relative_path,
                Some((path.expected_identity, Some(path.expected_is_directory))),
            )
            .unwrap();
        }
        validate_staged_purge_paths(storage, versions, &operation, &paths).unwrap();
        (operation, members, paths)
    }

    fn purge_journal_count(database: &Database) -> i64 {
        let connection = database.0.lock().unwrap();
        connection
            .query_row(
                "SELECT COUNT(*) FROM file_space_purge_operations",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[test]
    fn markdown_preview_read_is_independent_from_background_operation_lock() {
        let (root, _storage, _versions, database) = test_workspace("markdown-preview-read");
        let source = root.join("brief.md");
        fs::write(
            &source,
            b"# Immediate preview\n\nSixteen kilobytes should not wait for indexing.",
        )
        .unwrap();
        let snapshot =
            import_files_record(&database, None, &[source.to_string_lossy().into_owned()]).unwrap();
        let file_id = snapshot.files[0].id.clone();

        let background_operation = lock_file_space_operations().unwrap();
        assert_eq!(
            read_markdown_file_record(&database, &file_id).unwrap(),
            "# Immediate preview\n\nSixteen kilobytes should not wait for indexing."
        );
        drop(background_operation);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn creates_versioned_markdown_and_text_files_without_overwriting() {
        let (root, storage, versions, database) = test_workspace("create-text-file");
        let notes = create_folder_record(&database, None, "Notes").unwrap();

        let markdown =
            create_text_file_record(&database, &versions, Some(&notes.id), "Meeting notes", "md")
                .unwrap();
        assert_eq!(markdown.file.name, "Meeting notes.md");
        assert_eq!(markdown.file.folder_id.as_deref(), Some(notes.id.as_str()));
        assert_eq!(markdown.file.current_version, Some(1));
        assert_eq!(markdown.file.version_count, 1);
        assert_eq!(
            fs::read(storage.join("Notes/Meeting notes.md")).unwrap(),
            b""
        );
        let timeline = load_task_file_timeline_record(&database, &markdown.file.id).unwrap();
        assert_eq!(timeline.versions.len(), 1);
        assert_eq!(timeline.versions[0].version_number, 1);
        assert_eq!(
            read_task_file_version_record(&database, &markdown.file.id, &timeline.versions[0].id,)
                .unwrap(),
            b""
        );

        let duplicate = create_text_file_record(
            &database,
            &versions,
            Some(&notes.id),
            "Meeting notes.md",
            "md",
        )
        .unwrap_err();
        assert!(duplicate.contains("already exists"));

        let text =
            create_text_file_record(&database, &versions, None, "Scratch.txt", "txt").unwrap();
        assert_eq!(text.file.name, "Scratch.txt");
        assert!(storage.join("Scratch.txt").is_file());
        assert_eq!(text.snapshot.file_count, 2);

        assert!(
            create_text_file_record(&database, &versions, None, "Wrong.pdf", "md")
                .unwrap_err()
                .contains(".md extension")
        );
        assert!(
            create_text_file_record(&database, &versions, None, "Script", "js")
                .unwrap_err()
                .contains("Only Markdown and plain-text")
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn snapshot_and_cursor_pages_keep_large_file_lists_bounded() {
        let (root, _storage, _versions, database) = test_workspace("file-pages");
        let file_count = FILE_SPACE_INITIAL_PAGE_LIMIT * 2 + 27;
        {
            let mut connection = database.0.lock().unwrap();
            let transaction = connection.transaction().unwrap();
            for index in 0..file_count {
                let id = format!("paged-file-{index:04}");
                let name = format!("document-{index:04}.txt");
                transaction
                    .execute(
                        "INSERT INTO files
                         (id, original_name, storage_path, mime_type, size_bytes, folder_id,
                          source_kind, manual_order, updated_at, trashed_at, created_at)
                         VALUES (?1, ?2, ?2, 'text/plain', ?3, NULL,
                                 'user_import', ?4, ?5, NULL, ?5)",
                        params![id, name, index as i64, index as i64, 10_000 + index as i64],
                    )
                    .unwrap();
            }
            transaction.commit().unwrap();
        }

        let snapshot = load_snapshot_record(&database).unwrap();
        assert_eq!(snapshot.file_count, file_count as i64);
        assert_eq!(snapshot.files.len(), FILE_SPACE_INITIAL_PAGE_LIMIT);

        let mut cursor = None;
        let mut loaded_ids = Vec::new();
        loop {
            let page = list_file_space_file_page_record(
                &database,
                &FileSpaceFilePageRequest {
                    folder_id: None,
                    sort: "updatedDesc".to_owned(),
                    type_filter: "all".to_owned(),
                    tag_filter: None,
                    updated_after: None,
                    match_ids: Vec::new(),
                    cursor,
                    limit: Some(73),
                },
            )
            .unwrap();
            assert!(page.files.len() <= 73);
            assert_eq!(page.total_count, file_count as i64);
            loaded_ids.extend(page.files.into_iter().map(|file| file.id));
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }

        assert_eq!(loaded_ids.len(), file_count);
        assert_eq!(loaded_ids.iter().collect::<HashSet<_>>().len(), file_count);
        assert_eq!(
            loaded_ids.first().map(String::as_str),
            Some("paged-file-0346")
        );
        assert_eq!(
            loaded_ids.last().map(String::as_str),
            Some("paged-file-0000")
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn backup_export_contains_workspace_versions_database_and_manifest() {
        let (root, _storage, versions, database) = test_workspace("backup-export");
        database
            .0
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO app_settings (key, value, updated_at) VALUES ('ai.cloud_api_key', 'sk-local-only', 1)",
                [],
            )
            .unwrap();
        let source = root.join("brief.md");
        fs::write(&source, b"traceable backup").unwrap();
        import_files_record(&database, None, &[source.to_string_lossy().into_owned()]).unwrap();
        capture_initial_user_versions(&database, &versions).unwrap();
        let export_directory = root.join("exports");
        fs::create_dir_all(&export_directory).unwrap();
        let destination = export_directory.join("Lume Trace 2026-08-27.lumetrace");

        let result = export_file_space_backup_record(
            &database,
            &versions,
            destination.to_string_lossy().as_ref(),
        )
        .unwrap();

        assert_eq!(
            result.path,
            destination.canonicalize().unwrap().to_string_lossy()
        );
        assert_eq!(result.workspace_file_count, 1);
        assert_eq!(
            result.archive_size_bytes,
            fs::metadata(&destination).unwrap().len()
        );
        let mut archive = ZipArchive::new(fs::File::open(&destination).unwrap()).unwrap();
        let names = archive.file_names().map(str::to_owned).collect::<Vec<_>>();
        assert!(names.contains(&"workspace/brief.md".to_owned()));
        assert!(names.contains(&"metadata/lumetrace.sqlite3".to_owned()));
        assert!(names.contains(&"manifest.json".to_owned()));
        assert!(names
            .iter()
            .any(|name| name.starts_with("versions/") && name.ends_with(".blob")));

        let mut workspace_file = String::new();
        archive
            .by_name("workspace/brief.md")
            .unwrap()
            .read_to_string(&mut workspace_file)
            .unwrap();
        assert_eq!(workspace_file, "traceable backup");
        let mut manifest = String::new();
        archive
            .by_name("manifest.json")
            .unwrap()
            .read_to_string(&mut manifest)
            .unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&manifest).unwrap();
        assert_eq!(manifest["formatVersion"], 1);
        assert_eq!(manifest["product"], "Lume Trace");
        assert_eq!(manifest["workspace"]["fileCount"], 1);

        let exported_database = root.join("exported-lumetrace.sqlite3");
        {
            let mut archived_database = archive.by_name("metadata/lumetrace.sqlite3").unwrap();
            let mut output = fs::File::create(&exported_database).unwrap();
            std::io::copy(&mut archived_database, &mut output).unwrap();
        }
        let exported_connection = Connection::open(exported_database).unwrap();
        let derived_counts = exported_connection
            .query_row(
                "SELECT
                   (SELECT COUNT(*) FROM file_space_search_chunks),
                   (SELECT COUNT(*) FROM file_space_index_jobs),
                   (SELECT COUNT(*) FROM file_space_search_documents)",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(derived_counts, (0, 0, 1));
        let exported_cloud_key_count = exported_connection
            .query_row(
                "SELECT COUNT(*) FROM app_settings WHERE key = 'ai.cloud_api_key'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(exported_cloud_key_count, 0);

        drop(archive);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn backup_export_rejects_a_destination_inside_the_workspace() {
        let (root, storage, versions, database) = test_workspace("backup-inside-workspace");
        let destination = storage.join("unsafe.lumetrace");

        let error = export_file_space_backup_record(
            &database,
            &versions,
            destination.to_string_lossy().as_ref(),
        )
        .unwrap_err();

        assert!(error.contains("outside the current File Space folder"));
        assert!(!destination.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn backup_restore_switches_to_a_verified_new_workspace_without_deleting_the_old_one() {
        let (source_root, source_storage, source_versions, source_database) =
            test_workspace("backup-restore-source");
        let source = source_root.join("brief.md");
        fs::write(&source, b"version one").unwrap();
        let imported = import_files_record(
            &source_database,
            None,
            &[source.to_string_lossy().into_owned()],
        )
        .unwrap();
        let restored_file_id = imported.files[0].id.clone();
        capture_initial_user_versions(&source_database, &source_versions).unwrap();
        fs::write(source_storage.join("brief.md"), b"version two").unwrap();
        reconcile_task_file(&source_database, &source_versions, &restored_file_id).unwrap();
        set_file_tags_record(
            &source_database,
            &restored_file_id,
            vec!["important".to_owned()],
        )
        .unwrap();
        let backup_directory = source_root.join("exports");
        fs::create_dir_all(&backup_directory).unwrap();
        let backup = backup_directory.join("source.lumetrace");
        export_file_space_backup_record(
            &source_database,
            &source_versions,
            backup.to_string_lossy().as_ref(),
        )
        .unwrap();

        let (current_root, current_storage, current_versions, current_database) =
            test_workspace("backup-restore-current");
        let current_source = current_root.join("current.txt");
        fs::write(&current_source, b"keep this physical file").unwrap();
        import_files_record(
            &current_database,
            None,
            &[current_source.to_string_lossy().into_owned()],
        )
        .unwrap();
        capture_initial_user_versions(&current_database, &current_versions).unwrap();
        let restore_parent = current_root.join("restored");
        fs::create_dir_all(&restore_parent).unwrap();

        let snapshot = restore_file_space_backup_record(
            &current_database,
            &current_versions,
            backup.to_string_lossy().as_ref(),
            restore_parent.to_string_lossy().as_ref(),
        )
        .unwrap();

        let restored_root = PathBuf::from(snapshot.root_path.clone().unwrap());
        assert!(restored_root.starts_with(restore_parent.canonicalize().unwrap()));
        assert_eq!(
            fs::read(restored_root.join("brief.md")).unwrap(),
            b"version two"
        );
        assert_eq!(
            fs::read(current_storage.join("current.txt")).unwrap(),
            b"keep this physical file"
        );
        assert!(!snapshot.files.iter().any(|file| file.name == "current.txt"));
        let restored_file = snapshot
            .files
            .iter()
            .find(|file| file.name == "brief.md")
            .unwrap();
        assert_eq!(restored_file.version_count, 2);
        assert_eq!(restored_file.tags, vec!["important"]);
        let timeline =
            load_task_file_timeline_record(&current_database, &restored_file.id).unwrap();
        assert_eq!(timeline.versions.len(), 2);
        assert!(timeline
            .versions
            .iter()
            .all(|version| Path::new(&current_versions)
                .join(&restored_file.id)
                .join(format!("{}.blob", version.id))
                .is_file()));

        fs::remove_dir_all(source_root).unwrap();
        fs::remove_dir_all(current_root).unwrap();
    }

    #[test]
    fn backup_restore_rolls_back_files_and_versions_when_index_replacement_fails() {
        let (source_root, _source_storage, source_versions, source_database) =
            test_workspace("backup-restore-broken-source");
        let source = source_root.join("broken.md");
        fs::write(&source, b"broken source").unwrap();
        import_files_record(
            &source_database,
            None,
            &[source.to_string_lossy().into_owned()],
        )
        .unwrap();
        capture_initial_user_versions(&source_database, &source_versions).unwrap();
        source_database
            .0
            .lock()
            .unwrap()
            .execute_batch("DROP TABLE file_space_search_documents")
            .unwrap();
        let backup_directory = source_root.join("exports");
        fs::create_dir_all(&backup_directory).unwrap();
        let backup = backup_directory.join("broken.lumetrace");
        export_file_space_backup_record(
            &source_database,
            &source_versions,
            backup.to_string_lossy().as_ref(),
        )
        .unwrap();

        let (current_root, current_storage, current_versions, current_database) =
            test_workspace("backup-restore-rollback");
        let current_source = current_root.join("current.txt");
        fs::write(&current_source, b"current workspace").unwrap();
        let current_snapshot = import_files_record(
            &current_database,
            None,
            &[current_source.to_string_lossy().into_owned()],
        )
        .unwrap();
        capture_initial_user_versions(&current_database, &current_versions).unwrap();
        let current_file_id = current_snapshot.files[0].id.clone();
        let current_timeline =
            load_task_file_timeline_record(&current_database, &current_file_id).unwrap();
        let current_version_path = current_versions
            .join(&current_file_id)
            .join(format!("{}.blob", current_timeline.current_version_id));
        let restore_parent = current_root.join("restored");
        fs::create_dir_all(&restore_parent).unwrap();

        let error = restore_file_space_backup_record(
            &current_database,
            &current_versions,
            backup.to_string_lossy().as_ref(),
            restore_parent.to_string_lossy().as_ref(),
        )
        .unwrap_err();

        assert!(error.contains("file_space_search_documents"));
        assert_eq!(
            fs::read(current_storage.join("current.txt")).unwrap(),
            b"current workspace"
        );
        assert!(current_version_path.is_file());
        let snapshot = load_snapshot_record(&current_database).unwrap();
        assert_eq!(snapshot.files.len(), 1);
        assert_eq!(snapshot.files[0].id, current_file_id);
        assert_eq!(fs::read_dir(&restore_parent).unwrap().count(), 0);

        fs::remove_dir_all(source_root).unwrap();
        fs::remove_dir_all(current_root).unwrap();
    }

    #[test]
    fn batch_file_move_moves_every_file_in_input_order() {
        let (root, storage, versions, database) = test_workspace("batch-move-success");
        let destination = create_folder_record(&database, None, "Destination").unwrap();
        let alpha_source = root.join("alpha.txt");
        let beta_source = root.join("beta.txt");
        fs::write(&alpha_source, b"alpha").unwrap();
        fs::write(&beta_source, b"beta").unwrap();
        let imported = import_files_record(
            &database,
            None,
            &[
                alpha_source.to_string_lossy().into_owned(),
                beta_source.to_string_lossy().into_owned(),
            ],
        )
        .unwrap();
        let alpha_id = imported
            .files
            .iter()
            .find(|file| file.name == "alpha.txt")
            .unwrap()
            .id
            .clone();
        let beta_id = imported
            .files
            .iter()
            .find(|file| file.name == "beta.txt")
            .unwrap()
            .id
            .clone();
        let requested = vec![beta_id.clone(), alpha_id.clone()];

        let result =
            move_files_record(&database, &versions, &requested, Some(&destination.id)).unwrap();

        assert_eq!(result.moved_ids, requested);
        assert!(result.unchanged_ids.is_empty());
        assert!(result.failed.is_empty());
        assert_eq!(
            fs::read(storage.join("Destination/alpha.txt")).unwrap(),
            b"alpha"
        );
        assert_eq!(
            fs::read(storage.join("Destination/beta.txt")).unwrap(),
            b"beta"
        );
        assert!(!storage.join("alpha.txt").exists());
        assert!(!storage.join("beta.txt").exists());
        assert!(result
            .snapshot
            .files
            .iter()
            .all(|file| { file.folder_id.as_deref() == Some(destination.id.as_str()) }));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn batch_file_move_restores_a_missing_versioned_working_file_before_preflight() {
        let (root, storage, versions, database) = test_workspace("batch-move-restore");
        let destination = create_folder_record(&database, None, "Destination").unwrap();
        let source = root.join("recoverable.txt");
        fs::write(&source, b"recoverable content").unwrap();
        let imported =
            import_files_record(&database, None, &[source.to_string_lossy().into_owned()]).unwrap();
        capture_initial_user_versions(&database, &versions).unwrap();
        let file_id = imported.files[0].id.clone();
        let original_timeline = load_task_file_timeline_record(&database, &file_id).unwrap();

        fs::remove_file(storage.join("recoverable.txt")).unwrap();

        let result = move_files_record(
            &database,
            &versions,
            std::slice::from_ref(&file_id),
            Some(&destination.id),
        )
        .unwrap();

        assert_eq!(result.moved_ids, vec![file_id.clone()]);
        assert!(result.unchanged_ids.is_empty());
        assert!(result.failed.is_empty());
        assert_eq!(
            fs::read(storage.join("Destination/recoverable.txt")).unwrap(),
            b"recoverable content"
        );
        assert!(!storage.join("recoverable.txt").exists());
        let moved_timeline = load_task_file_timeline_record(&database, &file_id).unwrap();
        assert_eq!(
            moved_timeline.current_version_id,
            original_timeline.current_version_id
        );
        assert_eq!(
            moved_timeline.versions.len(),
            original_timeline.versions.len()
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn batch_file_move_reports_files_already_in_the_destination_as_unchanged() {
        let (root, storage, versions, database) = test_workspace("batch-move-unchanged");
        let destination = create_folder_record(&database, None, "Destination").unwrap();
        let source = root.join("already-there.txt");
        fs::write(&source, b"unchanged").unwrap();
        let imported = import_files_record(
            &database,
            Some(&destination.id),
            &[source.to_string_lossy().into_owned()],
        )
        .unwrap();
        let file_id = imported.files[0].id.clone();

        let result = move_files_record(
            &database,
            &versions,
            std::slice::from_ref(&file_id),
            Some(&destination.id),
        )
        .unwrap();

        assert!(result.moved_ids.is_empty());
        assert_eq!(result.unchanged_ids, vec![file_id]);
        assert!(result.failed.is_empty());
        assert_eq!(
            fs::read(storage.join("Destination/already-there.txt")).unwrap(),
            b"unchanged"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn batch_file_move_rejects_duplicate_destination_names_before_moving_anything() {
        let (root, storage, versions, database) = test_workspace("batch-move-duplicate-name");
        let first_folder = create_folder_record(&database, None, "First").unwrap();
        let second_folder = create_folder_record(&database, None, "Second").unwrap();
        let destination = create_folder_record(&database, None, "Destination").unwrap();
        let first_source_directory = root.join("outside-first");
        let second_source_directory = root.join("outside-second");
        fs::create_dir_all(&first_source_directory).unwrap();
        fs::create_dir_all(&second_source_directory).unwrap();
        let first_source = first_source_directory.join("same.txt");
        let second_source = second_source_directory.join("same.txt");
        fs::write(&first_source, b"first").unwrap();
        fs::write(&second_source, b"second").unwrap();
        let first_import = import_files_record(
            &database,
            Some(&first_folder.id),
            &[first_source.to_string_lossy().into_owned()],
        )
        .unwrap();
        let second_import = import_files_record(
            &database,
            Some(&second_folder.id),
            &[second_source.to_string_lossy().into_owned()],
        )
        .unwrap();
        let file_ids = vec![
            first_import.files[0].id.clone(),
            second_import
                .files
                .iter()
                .find(|file| file.folder_id.as_deref() == Some(second_folder.id.as_str()))
                .unwrap()
                .id
                .clone(),
        ];

        let error =
            move_files_record(&database, &versions, &file_ids, Some(&destination.id)).unwrap_err();

        assert!(error.contains("same destination name"));
        assert_eq!(fs::read(storage.join("First/same.txt")).unwrap(), b"first");
        assert_eq!(
            fs::read(storage.join("Second/same.txt")).unwrap(),
            b"second"
        );
        assert!(!storage.join("Destination/same.txt").exists());
        let snapshot = load_snapshot_record(&database).unwrap();
        assert!(snapshot
            .files
            .iter()
            .all(|file| file.folder_id.as_deref() != Some(destination.id.as_str())));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn batch_file_move_rejects_an_occupied_destination_before_moving_anything() {
        let (root, storage, versions, database) = test_workspace("batch-move-occupied");
        let source_folder = create_folder_record(&database, None, "Source").unwrap();
        let destination = create_folder_record(&database, None, "Destination").unwrap();
        let source_directory = root.join("outside-source");
        let occupied_directory = root.join("outside-occupied");
        fs::create_dir_all(&source_directory).unwrap();
        fs::create_dir_all(&occupied_directory).unwrap();
        let source = source_directory.join("same.txt");
        let occupied = occupied_directory.join("same.txt");
        fs::write(&source, b"source").unwrap();
        fs::write(&occupied, b"occupied").unwrap();
        let source_import = import_files_record(
            &database,
            Some(&source_folder.id),
            &[source.to_string_lossy().into_owned()],
        )
        .unwrap();
        import_files_record(
            &database,
            Some(&destination.id),
            &[occupied.to_string_lossy().into_owned()],
        )
        .unwrap();
        let source_id = source_import.files[0].id.clone();

        let error = move_files_record(
            &database,
            &versions,
            std::slice::from_ref(&source_id),
            Some(&destination.id),
        )
        .unwrap_err();

        assert!(error.contains("already exists in the destination"));
        assert_eq!(
            fs::read(storage.join("Source/same.txt")).unwrap(),
            b"source"
        );
        assert_eq!(
            fs::read(storage.join("Destination/same.txt")).unwrap(),
            b"occupied"
        );
        let snapshot = load_snapshot_record(&database).unwrap();
        assert_eq!(
            snapshot
                .files
                .iter()
                .find(|file| file.id == source_id)
                .unwrap()
                .folder_id
                .as_deref(),
            Some(source_folder.id.as_str())
        );

        fs::remove_dir_all(root).unwrap();
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
    fn dropped_same_name_file_can_be_added_as_the_latest_version() {
        let (root, storage, versions, database) = test_workspace("drop-latest-version");
        let original_directory = root.join("original");
        let incoming_directory = root.join("incoming");
        fs::create_dir_all(&original_directory).unwrap();
        fs::create_dir_all(&incoming_directory).unwrap();
        let original = original_directory.join("common.md");
        let incoming = incoming_directory.join("common.md");
        fs::write(&original, b"first version").unwrap();
        fs::write(&incoming, b"second version from drop").unwrap();
        import_files_record(&database, None, &[original.to_string_lossy().into_owned()]).unwrap();
        capture_initial_user_versions(&database, &versions).unwrap();
        let original_file = load_snapshot_record(&database).unwrap().files.remove(0);

        let inspection = inspect_dropped_file_conflicts(
            &database,
            &versions,
            None,
            &[FileSpaceDroppedFile {
                relative_path: "common.md".to_owned(),
                bytes: b"second version from drop".to_vec(),
            }],
        )
        .unwrap()
        .0;
        assert_eq!(inspection.conflicts.len(), 1);
        assert!(!inspection.conflicts[0].identical);
        assert_eq!(inspection.conflicts[0].existing_file_id, original_file.id);
        let path_inspection = inspect_path_file_conflicts(
            &database,
            &versions,
            None,
            &[incoming.to_string_lossy().into_owned()],
        )
        .unwrap()
        .0;
        assert_eq!(path_inspection.conflicts.len(), 1);
        assert!(!path_inspection.conflicts[0].identical);

        let snapshot = import_files_record_with_progress_policy(
            &database,
            None,
            &[incoming.to_string_lossy().into_owned()],
            Some(&versions),
            Some(DroppedFileConflictAction::LatestVersion),
            |_| Ok(()),
        )
        .unwrap();

        assert_eq!(snapshot.files.len(), 1);
        assert_eq!(snapshot.files[0].id, original_file.id);
        assert_eq!(snapshot.files[0].current_version, Some(2));
        assert_eq!(snapshot.files[0].version_count, 2);
        assert_eq!(
            fs::read(storage.join("common.md")).unwrap(),
            b"second version from drop"
        );
        let timeline = load_task_file_timeline_record(&database, &original_file.id).unwrap();
        assert!(timeline.events.iter().any(|event| {
            event.details.get("source").and_then(|value| value.as_str()) == Some("drag_import")
        }));
        assert!(
            reconcile_task_file_with_options(&database, &versions, &original_file.id, true,)
                .unwrap()
                .is_none()
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn identical_same_name_drop_is_skipped_without_creating_a_version() {
        let (root, _storage, versions, database) = test_workspace("drop-identical");
        let original_directory = root.join("original");
        let incoming_directory = root.join("incoming");
        fs::create_dir_all(&original_directory).unwrap();
        fs::create_dir_all(&incoming_directory).unwrap();
        let original = original_directory.join("common.md");
        let incoming = incoming_directory.join("common.md");
        fs::write(&original, b"same content").unwrap();
        fs::write(&incoming, b"same content").unwrap();
        import_files_record(&database, None, &[original.to_string_lossy().into_owned()]).unwrap();
        capture_initial_user_versions(&database, &versions).unwrap();

        let inspection = inspect_dropped_file_conflicts(
            &database,
            &versions,
            None,
            &[FileSpaceDroppedFile {
                relative_path: "common.md".to_owned(),
                bytes: b"same content".to_vec(),
            }],
        )
        .unwrap()
        .0;
        assert!(inspection.conflicts[0].identical);

        let snapshot = import_files_record_with_progress_policy(
            &database,
            None,
            &[incoming.to_string_lossy().into_owned()],
            Some(&versions),
            Some(DroppedFileConflictAction::LatestVersion),
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(snapshot.files.len(), 1);
        assert_eq!(snapshot.files[0].current_version, Some(1));
        assert_eq!(snapshot.files[0].version_count, 1);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn same_name_drop_can_still_be_kept_as_an_independent_file() {
        let (root, storage, versions, database) = test_workspace("drop-rename");
        let original_directory = root.join("original");
        let incoming_directory = root.join("incoming");
        fs::create_dir_all(&original_directory).unwrap();
        fs::create_dir_all(&incoming_directory).unwrap();
        let original = original_directory.join("common.md");
        let incoming = incoming_directory.join("common.md");
        fs::write(&original, b"first file").unwrap();
        fs::write(&incoming, b"second independent file").unwrap();
        import_files_record(&database, None, &[original.to_string_lossy().into_owned()]).unwrap();
        capture_initial_user_versions(&database, &versions).unwrap();

        import_files_record_with_progress_policy(
            &database,
            None,
            &[incoming.to_string_lossy().into_owned()],
            Some(&versions),
            Some(DroppedFileConflictAction::Rename),
            |_| Ok(()),
        )
        .unwrap();
        capture_initial_user_versions(&database, &versions).unwrap();
        let snapshot = load_snapshot_record(&database).unwrap();

        assert_eq!(snapshot.files.len(), 2);
        assert!(snapshot.files.iter().any(|file| file.name == "common.md"));
        assert!(snapshot
            .files
            .iter()
            .any(|file| file.name == "common (2).md"));
        assert_eq!(fs::read(storage.join("common.md")).unwrap(), b"first file");
        assert_eq!(
            fs::read(storage.join("common (2).md")).unwrap(),
            b"second independent file"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn automatic_tracking_returns_one_notification_for_a_stable_external_edit() {
        let (root, storage, versions, database) = test_workspace("automatic-version-tracking");
        let source = root.join("automatic.md");
        fs::write(&source, b"first version").unwrap();
        import_files_record(&database, None, &[source.to_string_lossy().into_owned()]).unwrap();
        capture_initial_user_versions(&database, &versions).unwrap();
        let file = load_snapshot_record(&database).unwrap().files.remove(0);
        let working_file = storage.join("automatic.md");

        fs::write(&working_file, b"second version from another app").unwrap();
        let changed_paths = HashSet::from([working_file.clone()]);
        let managed_path =
            physical_path(&require_ready_root(&database).unwrap(), &file.relative_path).unwrap();
        assert_eq!(managed_path, working_file.canonicalize().unwrap());
        assert!(watcher_paths_match_file(
            &require_ready_root(&database).unwrap(),
            &managed_path,
            &changed_paths,
        ));
        let notifications =
            capture_external_version_changes(&database, &versions, Some(&changed_paths)).unwrap();

        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].file_id, file.id);
        assert_eq!(notifications[0].file_name, "automatic.md");
        assert_eq!(notifications[0].version_number, 2);
        assert!(
            capture_external_version_changes(&database, &versions, Some(&changed_paths),)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            load_task_file_timeline_record(&database, &file.id)
                .unwrap()
                .versions
                .len(),
            2
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn automatic_version_notification_queue_deduplicates_versions() {
        let queue = FileSpaceVersionNotificationQueue::default();
        let notification = FileSpaceVersionCreatedNotification {
            version_id: "version-2".to_owned(),
            file_id: "file-1".to_owned(),
            file_name: "brief.md".to_owned(),
            version_number: 2,
            created_at: 2,
        };
        queue.push(notification.clone()).unwrap();
        queue.push(notification.clone()).unwrap();
        assert_eq!(queue.drain().unwrap(), vec![notification]);
        assert!(queue.drain().unwrap().is_empty());
    }

    #[test]
    fn a_panicking_extractor_is_failed_without_stopping_later_files() {
        let failed = protected_content_extraction(|| panic!("malformed document"));
        assert_eq!(failed.status, ExtractionStatus::Failed);
        assert!(failed
            .error
            .as_deref()
            .is_some_and(|error| error.contains("other files will continue")));

        let extracted = protected_content_extraction(|| ContentExtraction {
            text: "next document".to_owned(),
            status: ExtractionStatus::Extracted,
            error: None,
        });
        assert_eq!(extracted.status, ExtractionStatus::Extracted);
        assert_eq!(extracted.text, "next document");
    }

    #[test]
    fn import_returns_with_pending_content_then_background_extraction_enables_search() {
        let (root, _storage, versions, database) = test_workspace("search");
        let source = root.join("research.txt");
        fs::write(&source, "Lume Trace keeps quoted evidence searchable.").unwrap();

        import_files_record(&database, None, &[source.to_string_lossy().into_owned()]).unwrap();
        capture_initial_user_versions(&database, &versions).unwrap();
        let snapshot = load_snapshot_record(&database).unwrap();
        let file_id = snapshot.files[0].id.clone();
        {
            let connection = database.0.lock().unwrap();
            let queued = connection
                .query_row(
                    "SELECT extraction_status, body_text
                     FROM file_space_search_documents WHERE file_id = ?1",
                    [&file_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .unwrap();
            assert_eq!(queued, ("pending".to_owned(), String::new()));
        }
        let before_extraction = search_file_space_records(
            &database,
            None,
            &FileSpaceSearchRequest {
                query: "evidence".to_owned(),
                scopes: vec!["content".to_owned()],
            },
        )
        .unwrap();
        assert!(before_extraction.is_empty());

        assert!(process_next_content_extraction(&database).unwrap());
        {
            let connection = database.0.lock().unwrap();
            let extracted = connection
                .query_row(
                    "SELECT extraction_status, body_text
                     FROM file_space_search_documents WHERE file_id = ?1",
                    [&file_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .unwrap();
            assert_eq!(extracted.0, "extracted");
            assert!(extracted.1.contains("quoted evidence"));
        }
        let matches = search_file_space_records(
            &database,
            None,
            &FileSpaceSearchRequest {
                query: "evidence".to_owned(),
                scopes: vec!["content".to_owned()],
            },
        )
        .unwrap();

        assert_eq!(matches, vec![file_id]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn chinese_filename_search_supports_bounded_substring_matches() {
        let (root, _storage, _versions, database) = test_workspace("chinese-name-search");
        let source = root.join("产品日报.md");
        fs::write(&source, "今天完成了文件检索优化。".as_bytes()).unwrap();
        let imported =
            import_files_record(&database, None, &[source.to_string_lossy().into_owned()]).unwrap();
        capture_initial_user_versions(&database, &_versions).unwrap();
        let file_id = imported.files[0].id.clone();

        let matches = search_file_space_records(
            &database,
            None,
            &FileSpaceSearchRequest {
                query: "日报".to_owned(),
                scopes: vec!["name".to_owned()],
            },
        )
        .unwrap();

        assert_eq!(matches, vec![file_id]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pending_background_content_survives_database_reopen_and_resumes() {
        let (root, _storage, versions, database) = test_workspace("background-reopen");
        let source = root.join("resume.txt");
        fs::write(&source, "resume background extraction after restart").unwrap();
        import_files_record(&database, None, &[source.to_string_lossy().into_owned()]).unwrap();
        capture_initial_user_versions(&database, &versions).unwrap();
        let location = database.location().unwrap();
        assert_eq!(
            content_index_status(&database, false)
                .unwrap()
                .pending_files,
            1
        );
        drop(database);

        let reopened = crate::database::open_workspace_database(
            location.database_path,
            location.artifact_store_path,
            location.workspace_id,
        )
        .unwrap();
        assert_eq!(
            content_index_status(&reopened, false)
                .unwrap()
                .pending_files,
            1
        );
        assert!(process_next_content_extraction(&reopened).unwrap());
        let resumed = content_index_status(&reopened, false).unwrap();
        assert_eq!(resumed.pending_files, 0);
        assert_eq!(resumed.completed_files, 1);
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn search_never_rebuilds_the_full_document_index_on_the_read_path() {
        let (root, _storage, _versions, database) = test_workspace("search-read-only-index");
        let source = root.join("already-imported.txt");
        fs::write(&source, "bounded search").unwrap();
        let imported =
            import_files_record(&database, None, &[source.to_string_lossy().into_owned()]).unwrap();
        let file_id = imported.files[0].id.clone();
        {
            let connection = database.0.lock().unwrap();
            connection
                .execute(
                    "DELETE FROM file_space_search_documents WHERE file_id = ?1",
                    [&file_id],
                )
                .unwrap();
        }

        let matches = search_file_space_records(
            &database,
            None,
            &FileSpaceSearchRequest {
                query: "already-imported".to_owned(),
                scopes: vec!["name".to_owned()],
            },
        )
        .unwrap();

        assert!(matches.is_empty());
        let connection = database.0.lock().unwrap();
        let indexed: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM file_space_search_documents WHERE file_id = ?1",
                [&file_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(indexed, 0);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn single_ascii_character_search_does_not_scan_document_bodies() {
        let (root, _storage, versions, database) = test_workspace("search-short-content");
        let source = root.join("notes.txt");
        fs::write(&source, "single character body match").unwrap();
        import_files_record(&database, None, &[source.to_string_lossy().into_owned()]).unwrap();
        capture_initial_user_versions(&database, &versions).unwrap();
        assert!(process_next_content_extraction(&database).unwrap());

        let matches = search_file_space_records(
            &database,
            None,
            &FileSpaceSearchRequest {
                query: "s".to_owned(),
                scopes: vec!["content".to_owned()],
            },
        )
        .unwrap();

        assert!(matches.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn existing_folder_initialization_preserves_hierarchy_and_creates_versions() {
        let root =
            std::env::temp_dir().join(format!("lumetrace-existing-folder-{}", Uuid::new_v4()));
        let storage = root.join("existing-files");
        let versions = root.join("versions");
        fs::create_dir_all(storage.join("Projects/Notes")).unwrap();
        fs::create_dir_all(storage.join("Empty Folder")).unwrap();
        fs::create_dir_all(storage.join(".virelume-trash")).unwrap();
        fs::write(storage.join("overview.md"), b"existing root file").unwrap();
        fs::write(
            storage.join("Projects/Notes/decision.txt"),
            b"nested existing file",
        )
        .unwrap();
        fs::write(storage.join(".DS_Store"), b"system metadata").unwrap();
        fs::write(storage.join(".virelume-trash/deleted.txt"), b"deleted").unwrap();
        let database = crate::database::open_for_test(&root.join("lumetrace.sqlite3")).unwrap();

        let snapshot = import_existing_storage_root_record(
            &database,
            &versions,
            storage.to_string_lossy().as_ref(),
        )
        .unwrap();

        assert_eq!(
            snapshot.root_path.as_deref(),
            storage.canonicalize().unwrap().to_str()
        );
        assert_eq!(snapshot.folders.len(), 3);
        assert_eq!(snapshot.files.len(), 2);
        assert!(snapshot
            .folders
            .iter()
            .any(|folder| folder.relative_path == "Projects/Notes"));
        let nested = snapshot
            .files
            .iter()
            .find(|file| file.relative_path == "Projects/Notes/decision.txt")
            .unwrap();
        assert_eq!(nested.source_kind, "existing_import");
        assert_eq!(nested.current_version, Some(1));
        assert_eq!(nested.version_count, 1);
        assert_eq!(
            fs::read(storage.join("Projects/Notes/decision.txt")).unwrap(),
            b"nested existing file"
        );
        assert!(snapshot
            .files
            .iter()
            .all(|file| !file.relative_path.contains(".virelume-trash")));
        assert!(snapshot.files.iter().all(|file| file.name != ".DS_Store"));

        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "android"))]
    #[test]
    fn existing_folder_initialization_skips_symbolic_links_without_following_them() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "lumetrace-existing-folder-symlink-{}",
            Uuid::new_v4()
        ));
        let storage = root.join("Obsidian Vault");
        let versions = root.join("versions");
        let outside_file = root.join("outside.md");
        let outside_directory = root.join("outside-directory");
        fs::create_dir_all(&storage).unwrap();
        fs::create_dir_all(&outside_directory).unwrap();
        let obsidian_plugins = storage.join(".obsidian/plugins/example/node_modules/.bin");
        let arbitrary_hidden_directory = storage.join(".custom-tool/cache");
        fs::create_dir_all(&obsidian_plugins).unwrap();
        fs::create_dir_all(&arbitrary_hidden_directory).unwrap();
        fs::write(storage.join("note.md"), b"managed note").unwrap();
        fs::write(&outside_file, b"must not be imported").unwrap();
        fs::write(outside_directory.join("secret.md"), b"must stay outside").unwrap();
        fs::write(storage.join(".obsidian/app.json"), b"{}").unwrap();
        fs::write(
            arbitrary_hidden_directory.join("generated.txt"),
            b"hidden tool data",
        )
        .unwrap();
        symlink(&outside_file, storage.join("linked-note.md")).unwrap();
        symlink(&outside_directory, storage.join("linked-directory")).unwrap();
        symlink(&outside_file, obsidian_plugins.join("acorn")).unwrap();
        let database = crate::database::open_for_test(&root.join("lumetrace.sqlite3")).unwrap();

        let mut progress = Vec::new();
        let snapshot = import_existing_storage_root_record_with_progress(
            &database,
            &versions,
            storage.to_string_lossy().as_ref(),
            |processed, total, current_name| {
                progress.push((processed, total, current_name.map(str::to_owned)));
            },
        )
        .unwrap();

        assert_eq!(snapshot.files.len(), 1);
        assert_eq!(snapshot.files[0].relative_path, "note.md");
        assert!(snapshot.folders.is_empty());
        assert_eq!(
            progress,
            vec![(0, 1, None), (1, 1, Some("note.md".to_owned())),]
        );
        assert_eq!(fs::read(&outside_file).unwrap(), b"must not be imported");
        assert_eq!(
            fs::read(outside_directory.join("secret.md")).unwrap(),
            b"must stay outside"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn trash_and_restore_file_preserves_identity_versions_tags_and_avoids_overwrite() {
        let (root, storage, versions, database) = test_workspace("trash-file-restore");
        let source = root.join("decision.md");
        fs::write(&source, b"first version").unwrap();
        let imported =
            import_files_record(&database, None, &[source.to_string_lossy().into_owned()]).unwrap();
        capture_initial_user_versions(&database, &versions).unwrap();
        let file_id = imported.files[0].id.clone();
        set_file_tags_record(
            &database,
            &file_id,
            vec!["important".to_owned(), "review".to_owned()],
        )
        .unwrap();
        fs::write(storage.join("decision.md"), b"latest external edit").unwrap();

        let trashed = trash_file_record(&database, &versions, &file_id).unwrap();

        assert!(trashed.files.is_empty());
        assert_eq!(trashed.trash_items.len(), 1);
        assert_eq!(trashed.trash_items[0].item_type, "file");
        assert_eq!(trashed.trash_items[0].root_id, file_id);
        assert_eq!(trashed.trash_items[0].version_count, 2);
        assert!(!storage.join("decision.md").exists());
        let hidden_from_search = search_file_space_records(
            &database,
            None,
            &FileSpaceSearchRequest {
                query: "external".to_owned(),
                scopes: vec!["content".to_owned()],
            },
        )
        .unwrap();
        assert!(hidden_from_search.is_empty());
        let payload = physical_path(
            &storage,
            &load_trash_entry(&database, &trashed.trash_items[0].id)
                .unwrap()
                .payload_relative_path,
        )
        .unwrap();
        assert_eq!(fs::read(&payload).unwrap(), b"latest external edit");

        fs::write(storage.join("decision.md"), b"new occupant").unwrap();
        let restored = restore_trash_entry_record(&database, &trashed.trash_items[0].id).unwrap();

        assert!(restored.trash_items.is_empty());
        let restored_file = restored
            .files
            .iter()
            .find(|file| file.id == file_id)
            .unwrap();
        assert_eq!(restored_file.name, "decision (2).md");
        assert_eq!(restored_file.version_count, 2);
        assert_eq!(restored_file.tags, vec!["important", "review"]);
        assert_eq!(
            fs::read(storage.join("decision.md")).unwrap(),
            b"new occupant"
        );
        assert_eq!(
            fs::read(storage.join("decision (2).md")).unwrap(),
            b"latest external edit"
        );
        let timeline = load_task_file_timeline_record(&database, &file_id).unwrap();
        assert!(timeline
            .events
            .iter()
            .any(|event| event.event_type == "deleted"));
        assert!(timeline
            .events
            .iter()
            .any(|event| event.event_type == "restored"));
        assert!(process_next_content_extraction(&database).unwrap());
        let restored_search = search_file_space_records(
            &database,
            None,
            &FileSpaceSearchRequest {
                query: "external".to_owned(),
                scopes: vec!["content".to_owned()],
            },
        )
        .unwrap();
        assert_eq!(restored_search, vec![file_id]);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn trash_and_restore_folder_preserves_nested_and_unindexed_content() {
        let (root, storage, versions, database) = test_workspace("trash-folder-restore");
        let project = create_folder_record(&database, None, "Project").unwrap();
        let notes = create_folder_record(&database, Some(&project.id), "Notes").unwrap();
        let outside = root.join("brief.txt");
        fs::write(&outside, b"indexed content").unwrap();
        let imported = import_files_record(
            &database,
            Some(&notes.id),
            &[outside.to_string_lossy().into_owned()],
        )
        .unwrap();
        capture_initial_user_versions(&database, &versions).unwrap();
        let file_id = imported.files[0].id.clone();
        fs::write(storage.join("Project/unindexed.txt"), b"finder content").unwrap();
        fs::create_dir(storage.join("Project/Empty Folder")).unwrap();

        let trashed = trash_folder_record(&database, &versions, &project.id).unwrap();

        assert!(trashed.folders.is_empty());
        assert!(trashed.files.is_empty());
        assert_eq!(trashed.trash_items.len(), 1);
        assert_eq!(trashed.trash_items[0].item_type, "folder");
        assert_eq!(trashed.trash_items[0].root_id, project.id);
        assert_eq!(trashed.trash_items[0].file_count, 1);
        assert_eq!(trashed.trash_items[0].folder_count, 2);
        assert!(!storage.join("Project").exists());
        let trash_entry_id = trashed.trash_items[0].id.clone();
        let payload = physical_path(
            &storage,
            &load_trash_entry(&database, &trash_entry_id)
                .unwrap()
                .payload_relative_path,
        )
        .unwrap();
        assert_eq!(
            fs::read(payload.join("unindexed.txt")).unwrap(),
            b"finder content"
        );
        assert!(payload.join("Empty Folder").is_dir());

        create_folder_record(&database, None, "Project").unwrap();
        let restored = restore_trash_entry_record(&database, &trash_entry_id).unwrap();

        assert!(restored.trash_items.is_empty());
        let restored_project = restored
            .folders
            .iter()
            .find(|folder| folder.id == project.id)
            .unwrap();
        assert_eq!(restored_project.name, "Project (2)");
        let restored_notes = restored
            .folders
            .iter()
            .find(|folder| folder.id == notes.id)
            .unwrap();
        assert_eq!(restored_notes.relative_path, "Project (2)/Notes");
        let restored_file = restored
            .files
            .iter()
            .find(|file| file.id == file_id)
            .unwrap();
        assert_eq!(restored_file.relative_path, "Project (2)/Notes/brief.txt");
        assert_eq!(restored_file.version_count, 1);
        assert_eq!(
            fs::read(storage.join("Project (2)/Notes/brief.txt")).unwrap(),
            b"indexed content"
        );
        assert_eq!(
            fs::read(storage.join("Project (2)/unindexed.txt")).unwrap(),
            b"finder content"
        );
        assert!(storage.join("Project (2)/Empty Folder").is_dir());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn storage_root_switch_requires_every_trash_payload() {
        let (root, storage, versions, database) = test_workspace("trash-root-switch");
        let outside = root.join("keep-me.txt");
        fs::write(&outside, b"recoverable content").unwrap();
        let imported =
            import_files_record(&database, None, &[outside.to_string_lossy().into_owned()])
                .unwrap();
        capture_initial_user_versions(&database, &versions).unwrap();
        let file_id = imported.files[0].id.clone();
        let trashed = trash_file_record(&database, &versions, &file_id).unwrap();
        let payload = physical_path(
            &storage,
            &load_trash_entry(&database, &trashed.trash_items[0].id)
                .unwrap()
                .payload_relative_path,
        )
        .unwrap();
        let replacement = root.join("replacement-storage");
        fs::create_dir(&replacement).unwrap();

        let error =
            configure_storage_root_record(&database, replacement.to_string_lossy().as_ref())
                .unwrap_err();

        assert!(error.contains("does not contain the existing managed item"));
        assert_eq!(fs::read(payload).unwrap(), b"recoverable content");
        let snapshot = load_snapshot_record(&database).unwrap();
        assert_eq!(
            snapshot.root_path.as_deref(),
            storage.canonicalize().unwrap().to_str()
        );
        assert_eq!(snapshot.trash_items.len(), 1);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn atomic_trash_move_never_overwrites_an_occupied_destination() {
        let (root, storage, _versions, _database) = test_workspace("trash-no-overwrite");
        fs::write(storage.join("source.txt"), b"source").unwrap();
        fs::write(storage.join("destination.txt"), b"destination").unwrap();
        let (source_identity, source_is_directory) = managed_item_identity(&storage, "source.txt")
            .unwrap()
            .unwrap();

        let error = atomic_move_managed_item_without_overwrite(
            &storage,
            "source.txt",
            "destination.txt",
            Some((source_identity, Some(source_is_directory))),
        )
        .unwrap_err()
        .into_message();

        assert!(error.contains("already exists"));
        assert_eq!(fs::read(storage.join("source.txt")).unwrap(), b"source");
        assert_eq!(
            fs::read(storage.join("destination.txt")).unwrap(),
            b"destination"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pending_trash_recovery_rejects_a_replaced_item() {
        let (root, storage, _versions, database) = test_workspace("trash-journal-identity");
        fs::write(storage.join("identity.txt"), b"original").unwrap();
        let (source_identity, source_is_directory) =
            managed_item_identity(&storage, "identity.txt")
                .unwrap()
                .unwrap();
        let entry_id = Uuid::new_v4().to_string();
        let (_, payload_relative_path) =
            prepare_trash_payload(&storage, &entry_id, "identity.txt").unwrap();
        let pending = PendingTrashOperation {
            version: 1,
            operation_id: Uuid::new_v4().to_string(),
            entry_id,
            action: "trash".to_owned(),
            source_relative_path: "identity.txt".to_owned(),
            destination_relative_path: payload_relative_path.clone(),
            source_device: Some(source_identity.device),
            source_inode: Some(source_identity.inode),
            source_is_directory: Some(source_is_directory),
            created_at: now_millis(),
        };
        write_pending_trash_operation(&database, &pending).unwrap();
        atomic_move_managed_item_without_overwrite(
            &storage,
            "identity.txt",
            &payload_relative_path,
            Some((source_identity, Some(source_is_directory))),
        )
        .unwrap();
        let payload = physical_path(&storage, &payload_relative_path).unwrap();
        fs::remove_file(&payload).unwrap();
        fs::write(&payload, b"replacement").unwrap();

        let error = load_snapshot_record(&database).unwrap_err();

        assert!(error.contains("identity changed"));
        assert!(!storage.join("identity.txt").exists());
        assert_eq!(fs::read(&payload).unwrap(), b"replacement");
        let connection = database.0.lock().unwrap();
        let journal_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM app_settings WHERE key = ?1",
                [PENDING_TRASH_OPERATION_SETTING],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(journal_count, 1);
        drop(connection);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pending_trash_move_is_rolled_back_before_snapshot_loading() {
        let (root, storage, _versions, database) = test_workspace("trash-journal-recovery");
        let outside = root.join("recover-me.txt");
        fs::write(&outside, b"journal protected content").unwrap();
        let imported =
            import_files_record(&database, None, &[outside.to_string_lossy().into_owned()])
                .unwrap();
        let file_id = imported.files[0].id.clone();
        let entry_id = Uuid::new_v4().to_string();
        let (_, payload_relative_path) =
            prepare_trash_payload(&storage, &entry_id, "recover-me.txt").unwrap();
        let (source_identity, source_is_directory) =
            managed_item_identity(&storage, "recover-me.txt")
                .unwrap()
                .unwrap();
        let pending = PendingTrashOperation {
            version: 1,
            operation_id: Uuid::new_v4().to_string(),
            entry_id,
            action: "trash".to_owned(),
            source_relative_path: "recover-me.txt".to_owned(),
            destination_relative_path: payload_relative_path.clone(),
            source_device: Some(source_identity.device),
            source_inode: Some(source_identity.inode),
            source_is_directory: Some(source_is_directory),
            created_at: now_millis(),
        };
        write_pending_trash_operation(&database, &pending).unwrap();
        fs::rename(
            storage.join("recover-me.txt"),
            physical_path(&storage, &payload_relative_path).unwrap(),
        )
        .unwrap();

        let recovered = load_snapshot_record(&database).unwrap();

        assert!(recovered.files.iter().any(|file| file.id == file_id));
        assert!(recovered.trash_items.is_empty());
        assert_eq!(
            fs::read(storage.join("recover-me.txt")).unwrap(),
            b"journal protected content"
        );
        assert!(!physical_path(&storage, &payload_relative_path)
            .unwrap()
            .exists());
        let connection = database.0.lock().unwrap();
        let pending_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM app_settings WHERE key = ?1",
                [PENDING_TRASH_OPERATION_SETTING],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(pending_count, 0);
        drop(connection);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn permanent_file_purge_removes_payload_versions_and_related_database_rows() {
        let (root, storage, versions, database) = test_workspace("purge-file-complete");
        let source = root.join("evidence.md");
        fs::write(&source, b"traceable evidence").unwrap();
        let imported =
            import_files_record(&database, None, &[source.to_string_lossy().into_owned()]).unwrap();
        let file_id = imported.files[0].id.clone();
        capture_initial_user_versions(&database, &versions).unwrap();
        set_file_tags_record(
            &database,
            &file_id,
            vec!["important".to_owned(), "source".to_owned()],
        )
        .unwrap();
        assert!(process_next_content_extraction(&database).unwrap());
        let search_hits = search_file_space_records(
            &database,
            None,
            &FileSpaceSearchRequest {
                query: "traceable".to_owned(),
                scopes: vec!["content".to_owned()],
            },
        )
        .unwrap();
        assert_eq!(search_hits, vec![file_id.clone()]);
        let trashed = trash_file_record(&database, &versions, &file_id).unwrap();
        let entry_id = trashed.trash_items[0].id.clone();
        let container = storage
            .join(TRASH_DIRECTORY_NAME)
            .join("entries")
            .join(&entry_id);
        let artifact_directory = versions.join(&file_id);
        assert!(container.is_dir());
        assert!(artifact_directory.is_dir());

        let warning = purge_trash_entry_record(&database, &versions, &entry_id).unwrap();

        assert!(warning.is_none());
        assert!(!container.exists());
        assert!(!artifact_directory.exists());
        assert_eq!(purge_journal_count(&database), 0);
        let connection = database.0.lock().unwrap();
        for (table, column) in [
            ("files", "id"),
            ("file_space_artifacts", "file_id"),
            ("file_space_artifact_versions", "file_id"),
            ("file_space_artifact_events", "file_id"),
            ("file_space_file_tags", "file_id"),
            ("file_space_search_documents", "file_id"),
        ] {
            let count: i64 = connection
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE {column} = ?1"),
                    [&file_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 0, "{table} retained the permanently deleted file");
        }
        let trash_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM file_space_trash_entries WHERE id = ?1",
                [&entry_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(trash_count, 0);
        drop(connection);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn permanent_folder_and_multi_entry_purge_removes_nested_unindexed_content() {
        let (root, storage, versions, database) = test_workspace("purge-folder-multi");
        let project = create_folder_record(&database, None, "Project").unwrap();
        let notes = create_folder_record(&database, Some(&project.id), "Notes").unwrap();
        let nested_source = root.join("nested.txt");
        let loose_source = root.join("loose.txt");
        fs::write(&nested_source, b"nested indexed content").unwrap();
        fs::write(&loose_source, b"loose indexed content").unwrap();
        let nested = import_files_record(
            &database,
            Some(&notes.id),
            &[nested_source.to_string_lossy().into_owned()],
        )
        .unwrap();
        let loose = import_files_record(
            &database,
            None,
            &[loose_source.to_string_lossy().into_owned()],
        )
        .unwrap();
        let nested_id = nested.files[0].id.clone();
        let loose_id = loose
            .files
            .iter()
            .find(|file| file.name == "loose.txt")
            .unwrap()
            .id
            .clone();
        capture_initial_user_versions(&database, &versions).unwrap();
        fs::write(
            storage.join("Project/unindexed.txt"),
            b"Finder-only content",
        )
        .unwrap();
        fs::create_dir(storage.join("Project/Empty Folder")).unwrap();
        let folder_trash = trash_folder_record(&database, &versions, &project.id).unwrap();
        let folder_entry_id = folder_trash.trash_items[0].id.clone();
        let loose_trash = trash_file_record(&database, &versions, &loose_id).unwrap();
        let loose_entry_id = loose_trash
            .trash_items
            .iter()
            .find(|item| item.root_id == loose_id)
            .unwrap()
            .id
            .clone();

        let summary = empty_trash_entries_record(
            &database,
            &versions,
            &[folder_entry_id.clone(), loose_entry_id.clone()],
        )
        .unwrap();

        assert_eq!(summary.purged_count, 2);
        assert_eq!(summary.failed_count, 0);
        assert!(summary.cleanup_warnings.is_empty());
        assert!(load_snapshot_record(&database)
            .unwrap()
            .trash_items
            .is_empty());
        assert!(!versions.join(&nested_id).exists());
        assert!(!versions.join(&loose_id).exists());
        assert!(!storage
            .join(TRASH_DIRECTORY_NAME)
            .join("entries")
            .join(folder_entry_id)
            .exists());
        assert!(!storage
            .join(TRASH_DIRECTORY_NAME)
            .join("entries")
            .join(loose_entry_id)
            .exists());
        let connection = database.0.lock().unwrap();
        let folder_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM file_space_folders", [], |row| {
                row.get(0)
            })
            .unwrap();
        let file_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
            .unwrap();
        assert_eq!(folder_count, 0);
        assert_eq!(file_count, 0);
        drop(connection);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn manual_empty_only_purges_the_confirmed_entry_ids() {
        let (root, _storage, versions, database) = test_workspace("purge-confirmed-only");
        let first_source = root.join("first.txt");
        let second_source = root.join("second.txt");
        fs::write(&first_source, b"first").unwrap();
        fs::write(&second_source, b"second").unwrap();
        let imported = import_files_record(
            &database,
            None,
            &[
                first_source.to_string_lossy().into_owned(),
                second_source.to_string_lossy().into_owned(),
            ],
        )
        .unwrap();
        let first_id = imported
            .files
            .iter()
            .find(|file| file.name == "first.txt")
            .unwrap()
            .id
            .clone();
        let second_id = imported
            .files
            .iter()
            .find(|file| file.name == "second.txt")
            .unwrap()
            .id
            .clone();
        let first_trash = trash_file_record(&database, &versions, &first_id).unwrap();
        let confirmed_entry_id = first_trash.trash_items[0].id.clone();
        let second_trash = trash_file_record(&database, &versions, &second_id).unwrap();
        let later_entry_id = second_trash
            .trash_items
            .iter()
            .find(|item| item.root_id == second_id)
            .unwrap()
            .id
            .clone();

        let duplicate_error = empty_trash_entries_record(
            &database,
            &versions,
            &[confirmed_entry_id.clone(), confirmed_entry_id.clone()],
        )
        .unwrap_err();
        assert!(duplicate_error.contains("duplicate"));
        assert_eq!(
            load_snapshot_record(&database).unwrap().trash_items.len(),
            2
        );
        let stale_error = empty_trash_entries_record(
            &database,
            &versions,
            &[confirmed_entry_id.clone(), Uuid::new_v4().to_string()],
        )
        .unwrap_err();
        assert!(stale_error.contains("no longer exists"));
        assert_eq!(
            load_snapshot_record(&database).unwrap().trash_items.len(),
            2
        );

        let summary = empty_trash_entries_record(
            &database,
            &versions,
            std::slice::from_ref(&confirmed_entry_id),
        )
        .unwrap();

        assert_eq!(summary.purged_count, 1);
        assert_eq!(summary.failed_count, 0);
        let remaining = load_snapshot_record(&database).unwrap().trash_items;
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, later_entry_id);
        assert_eq!(remaining[0].root_id, second_id);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn manual_empty_returns_a_partial_result_and_continues_after_one_entry_fails() {
        let (root, storage, versions, database) = test_workspace("purge-partial-result");
        let broken_source = root.join("broken.txt");
        let healthy_source = root.join("healthy.txt");
        fs::write(&broken_source, b"broken").unwrap();
        fs::write(&healthy_source, b"healthy").unwrap();
        let imported = import_files_record(
            &database,
            None,
            &[
                broken_source.to_string_lossy().into_owned(),
                healthy_source.to_string_lossy().into_owned(),
            ],
        )
        .unwrap();
        let broken_id = imported
            .files
            .iter()
            .find(|file| file.name == "broken.txt")
            .unwrap()
            .id
            .clone();
        let healthy_id = imported
            .files
            .iter()
            .find(|file| file.name == "healthy.txt")
            .unwrap()
            .id
            .clone();
        let broken_trash = trash_file_record(&database, &versions, &broken_id).unwrap();
        let broken_entry_id = broken_trash.trash_items[0].id.clone();
        let healthy_trash = trash_file_record(&database, &versions, &healthy_id).unwrap();
        let healthy_entry_id = healthy_trash
            .trash_items
            .iter()
            .find(|item| item.root_id == healthy_id)
            .unwrap()
            .id
            .clone();
        let broken_entry = load_trash_entry(&database, &broken_entry_id).unwrap();
        fs::remove_file(physical_path(&storage, &broken_entry.payload_relative_path).unwrap())
            .unwrap();

        let summary = empty_trash_entries_record(
            &database,
            &versions,
            &[broken_entry_id.clone(), healthy_entry_id],
        )
        .unwrap();

        assert_eq!(summary.purged_count, 1);
        assert_eq!(summary.failed_count, 1);
        assert_eq!(summary.failure_messages.len(), 1);
        assert!(summary.failure_messages[0].contains(&broken_entry_id));
        let remaining = load_snapshot_record(&database).unwrap().trash_items;
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, broken_entry_id);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn expired_trash_purge_uses_the_exact_thirty_day_boundary() {
        let (root, _storage, versions, database) = test_workspace("purge-retention-boundary");
        let mut entries = Vec::new();
        for name in ["almost.txt", "exact.txt", "future.txt"] {
            let source = root.join(name);
            fs::write(&source, name.as_bytes()).unwrap();
            let imported =
                import_files_record(&database, None, &[source.to_string_lossy().into_owned()])
                    .unwrap();
            let file_id = imported
                .files
                .iter()
                .find(|file| file.name == name)
                .unwrap()
                .id
                .clone();
            let trashed = trash_file_record(&database, &versions, &file_id).unwrap();
            let entry_id = trashed
                .trash_items
                .iter()
                .find(|item| item.root_id == file_id)
                .unwrap()
                .id
                .clone();
            entries.push((name.to_owned(), entry_id));
        }
        let now = 2_000_000_000_000_i64;
        let cutoff = now - TRASH_RETENTION_MILLIS;
        let connection = database.0.lock().unwrap();
        connection
            .execute(
                "UPDATE file_space_trash_entries SET trashed_at = ?1 WHERE id = ?2",
                params![cutoff + 60_000, entries[0].1],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE file_space_trash_entries SET trashed_at = ?1 WHERE id = ?2",
                params![cutoff, entries[1].1],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE file_space_trash_entries SET trashed_at = ?1 WHERE id = ?2",
                params![now + 60_000, entries[2].1],
            )
            .unwrap();
        drop(connection);

        let summary = purge_expired_trash_entries_record(&database, &versions, now).unwrap();

        assert_eq!(summary.purged_count, 1);
        assert_eq!(summary.failed_count, 0);
        let remaining = load_snapshot_record(&database)
            .unwrap()
            .trash_items
            .into_iter()
            .map(|item| item.id)
            .collect::<HashSet<_>>();
        assert!(remaining.contains(&entries[0].1));
        assert!(!remaining.contains(&entries[1].1));
        assert!(remaining.contains(&entries[2].1));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn expired_trash_selection_is_bounded_to_twenty_entries_per_batch() {
        let (root, _storage, _versions, database) = test_workspace("purge-batch-limit");
        let now = 2_000_000_000_000_i64;
        let trashed_at = now - TRASH_RETENTION_MILLIS - 1;
        let connection = database.0.lock().unwrap();
        for index in 0..21 {
            let file_id = Uuid::new_v4().to_string();
            let entry_id = Uuid::new_v4().to_string();
            let name = format!("expired-{index}.txt");
            let payload = format!("{TRASH_DIRECTORY_NAME}/entries/{entry_id}/{name}");
            connection
                .execute(
                    "INSERT INTO files
                     (id, original_name, storage_path, mime_type, size_bytes, folder_id,
                      source_kind, manual_order, updated_at, trashed_at, created_at)
                     VALUES (?1, ?2, ?3, 'text/plain', 1, NULL,
                             'user_import', ?4, ?5, ?5, ?5)",
                    params![file_id, name, payload, index, trashed_at],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO file_space_trash_entries
                     (id, item_type, root_file_id, root_folder_id, original_parent_id,
                      original_name, original_relative_path, payload_relative_path, trashed_at)
                     VALUES (?1, 'file', ?2, NULL, NULL, ?3, ?3, ?4, ?5)",
                    params![entry_id, file_id, name, payload, trashed_at],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO file_space_trash_entry_files (entry_id, file_id)
                     VALUES (?1, ?2)",
                    params![entry_id, file_id],
                )
                .unwrap();
        }
        drop(connection);

        let selected =
            expired_trash_entry_ids(&database, now, EXPIRED_TRASH_PURGE_BATCH_SIZE).unwrap();

        assert_eq!(EXPIRED_TRASH_PURGE_BATCH_SIZE, 20);
        assert_eq!(selected.len(), 20);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn staging_purge_recovery_rolls_payload_and_versions_back_without_database_loss() {
        let (root, storage, versions, database) = test_workspace("purge-staging-recovery");
        let source = root.join("recover-staging.txt");
        fs::write(&source, b"recover staging").unwrap();
        let imported =
            import_files_record(&database, None, &[source.to_string_lossy().into_owned()]).unwrap();
        let file_id = imported.files[0].id.clone();
        capture_initial_user_versions(&database, &versions).unwrap();
        let trashed = trash_file_record(&database, &versions, &file_id).unwrap();
        let entry_id = trashed.trash_items[0].id.clone();
        let (operation, _members, paths) =
            stage_purge_for_test(&database, &storage, &versions, &entry_id);
        assert_eq!(purge_journal_count(&database), 1);

        recover_pending_purge_operations(&database, &versions).unwrap();

        assert_eq!(purge_journal_count(&database), 0);
        for path in &paths {
            let base = purge_path_base(&storage, &versions, path).unwrap();
            assert_eq!(
                purge_path_identity(base, &path.original_relative_path).unwrap(),
                Some((path.expected_identity, path.expected_is_directory))
            );
            assert!(purge_path_identity(base, &path.staged_relative_path)
                .unwrap()
                .is_none());
        }
        assert!(!storage
            .join(TRASH_DIRECTORY_NAME)
            .join(PURGE_DIRECTORY_NAME)
            .join(operation.id)
            .exists());
        let snapshot = load_snapshot_record(&database).unwrap();
        assert_eq!(snapshot.trash_items.len(), 1);
        assert_eq!(snapshot.trash_items[0].id, entry_id);
        assert!(versions.join(file_id).is_dir());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn database_committed_purge_recovery_only_finishes_disk_cleanup() {
        let (root, storage, versions, database) = test_workspace("purge-committed-recovery");
        let source = root.join("recover-committed.txt");
        fs::write(&source, b"recover committed").unwrap();
        let imported =
            import_files_record(&database, None, &[source.to_string_lossy().into_owned()]).unwrap();
        let file_id = imported.files[0].id.clone();
        capture_initial_user_versions(&database, &versions).unwrap();
        let trashed = trash_file_record(&database, &versions, &file_id).unwrap();
        let entry_id = trashed.trash_items[0].id.clone();
        let (operation, members, paths) =
            stage_purge_for_test(&database, &storage, &versions, &entry_id);
        commit_purge_database(&database, &operation, &members).unwrap();
        assert!(paths.iter().all(|path| {
            let base = purge_path_base(&storage, &versions, path).unwrap();
            purge_path_identity(base, &path.original_relative_path)
                .unwrap()
                .is_none()
                && purge_path_identity(base, &path.staged_relative_path)
                    .unwrap()
                    .is_some()
        }));

        recover_pending_purge_operations(&database, &versions).unwrap();

        assert_eq!(purge_journal_count(&database), 0);
        for path in &paths {
            let base = purge_path_base(&storage, &versions, path).unwrap();
            assert!(purge_path_identity(base, &path.original_relative_path)
                .unwrap()
                .is_none());
            assert!(purge_path_identity(base, &path.staged_relative_path)
                .unwrap()
                .is_none());
        }
        let connection = database.0.lock().unwrap();
        let file_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM files WHERE id = ?1",
                [&file_id],
                |row| row.get(0),
            )
            .unwrap();
        let entry_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM file_space_trash_entries WHERE id = ?1",
                [&entry_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(file_count, 0);
        assert_eq!(entry_count, 0);
        drop(connection);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn committed_purge_waits_while_storage_root_is_offline_then_recovers_when_ready() {
        let (root, storage, versions, database) = test_workspace("purge-offline-root");
        let source = root.join("offline.txt");
        fs::write(&source, b"offline cleanup").unwrap();
        let imported =
            import_files_record(&database, None, &[source.to_string_lossy().into_owned()]).unwrap();
        let file_id = imported.files[0].id.clone();
        let trashed = trash_file_record(&database, &versions, &file_id).unwrap();
        let entry_id = trashed.trash_items[0].id.clone();
        let (operation, members, _paths) =
            stage_purge_for_test(&database, &storage, &versions, &entry_id);
        commit_purge_database(&database, &operation, &members).unwrap();
        let offline_storage = root.join("storage-offline");
        fs::rename(&storage, &offline_storage).unwrap();

        recover_pending_purge_operations_if_root_ready(&database, &versions).unwrap();
        let missing_snapshot = load_snapshot_record(&database).unwrap();

        assert_eq!(missing_snapshot.root_status, "missing");
        assert_eq!(purge_journal_count(&database), 1);

        fs::rename(&offline_storage, &storage).unwrap();
        recover_pending_purge_operations_if_root_ready(&database, &versions).unwrap();
        assert_eq!(purge_journal_count(&database), 0);

        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "android"))]
    #[test]
    fn permanent_purge_rejects_symbolic_links_inside_trash_content() {
        use std::os::unix::fs::symlink;

        let (root, storage, versions, database) = test_workspace("purge-symlink-content");
        let source = root.join("symlink.txt");
        let outside = root.join("must-survive.txt");
        fs::write(&source, b"trash payload").unwrap();
        fs::write(&outside, b"must survive").unwrap();
        let imported =
            import_files_record(&database, None, &[source.to_string_lossy().into_owned()]).unwrap();
        let file_id = imported.files[0].id.clone();
        let trashed = trash_file_record(&database, &versions, &file_id).unwrap();
        let entry_id = trashed.trash_items[0].id.clone();
        let container = storage
            .join(TRASH_DIRECTORY_NAME)
            .join("entries")
            .join(&entry_id);
        symlink(&outside, container.join("unsafe-link")).unwrap();

        let error = purge_trash_entry_record(&database, &versions, &entry_id).unwrap_err();

        assert!(error.contains("symbolic link"));
        assert_eq!(fs::read(&outside).unwrap(), b"must survive");
        assert!(container.join("unsafe-link").exists());
        assert_eq!(purge_journal_count(&database), 0);
        let snapshot = load_snapshot_record(&database).unwrap();
        assert_eq!(snapshot.trash_items.len(), 1);
        assert_eq!(snapshot.trash_items[0].id, entry_id);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn purge_journal_rejects_path_traversal() {
        let (root, storage, versions, database) = test_workspace("purge-path-traversal");
        let source = root.join("safe.txt");
        fs::write(&source, b"safe").unwrap();
        let imported =
            import_files_record(&database, None, &[source.to_string_lossy().into_owned()]).unwrap();
        let file_id = imported.files[0].id.clone();
        let trashed = trash_file_record(&database, &versions, &file_id).unwrap();
        let entry_id = trashed.trash_items[0].id.clone();
        let (operation, _members, mut paths) =
            prepare_purge_operation(&database, &versions, &entry_id).unwrap();
        paths[0].original_relative_path = "../outside".to_owned();

        let error = validate_purge_journal_path(&operation, &paths[0]).unwrap_err();

        assert!(error.contains("invalid"));
        assert_eq!(fs::read(root.join("safe.txt")).unwrap(), b"safe");
        assert!(storage
            .join(TRASH_DIRECTORY_NAME)
            .join("entries")
            .join(entry_id)
            .is_dir());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn purge_staging_never_overwrites_an_occupied_target() {
        let (root, storage, versions, database) = test_workspace("purge-target-occupied");
        let source = root.join("occupied.txt");
        fs::write(&source, b"original payload").unwrap();
        let imported =
            import_files_record(&database, None, &[source.to_string_lossy().into_owned()]).unwrap();
        let file_id = imported.files[0].id.clone();
        let trashed = trash_file_record(&database, &versions, &file_id).unwrap();
        let entry_id = trashed.trash_items[0].id.clone();
        let (operation, members, paths) =
            prepare_purge_operation(&database, &versions, &entry_id).unwrap();
        persist_purge_journal(&database, &operation, &members, &paths).unwrap();
        prepare_purge_staging_directories(&storage, &versions, &operation.id, false).unwrap();
        let path = &paths[0];
        let occupied = physical_path(&storage, &path.staged_relative_path).unwrap();
        fs::create_dir(&occupied).unwrap();
        fs::write(occupied.join("marker"), b"do not overwrite").unwrap();

        let error = atomic_move_managed_item_without_overwrite(
            &storage,
            &path.original_relative_path,
            &path.staged_relative_path,
            Some((path.expected_identity, Some(true))),
        )
        .unwrap_err()
        .into_message();

        assert!(error.contains("already exists"));
        assert_eq!(
            fs::read(occupied.join("marker")).unwrap(),
            b"do not overwrite"
        );
        assert_eq!(
            purge_path_identity(&storage, &path.original_relative_path)
                .unwrap()
                .unwrap()
                .0,
            path.expected_identity
        );
        fs::remove_dir_all(&occupied).unwrap();
        rollback_staging_purge_operation(&database, &versions, &operation).unwrap();
        assert_eq!(purge_journal_count(&database), 0);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn staging_validation_refuses_an_identity_replacement_before_database_commit() {
        let (root, storage, versions, database) = test_workspace("purge-staged-replaced");
        let source = root.join("replace-staged.txt");
        fs::write(&source, b"original payload").unwrap();
        let imported =
            import_files_record(&database, None, &[source.to_string_lossy().into_owned()]).unwrap();
        let file_id = imported.files[0].id.clone();
        let trashed = trash_file_record(&database, &versions, &file_id).unwrap();
        let entry_id = trashed.trash_items[0].id.clone();
        let (operation, _members, paths) =
            stage_purge_for_test(&database, &storage, &versions, &entry_id);
        let path = &paths[0];
        let staged = physical_path(&storage, &path.staged_relative_path).unwrap();
        fs::remove_dir_all(&staged).unwrap();
        fs::create_dir(&staged).unwrap();
        fs::write(staged.join("replacement"), b"must survive").unwrap();

        let error =
            validate_staged_purge_paths(&storage, &versions, &operation, &paths).unwrap_err();

        assert!(error.contains("identity changed"));
        assert_eq!(
            fs::read(staged.join("replacement")).unwrap(),
            b"must survive"
        );
        assert_eq!(purge_journal_count(&database), 1);
        let connection = database.0.lock().unwrap();
        let file_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM files WHERE id = ?1",
                [&file_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(file_count, 1);
        drop(connection);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn committed_recovery_refuses_an_identity_replacement_without_resurrecting_data() {
        let (root, storage, versions, database) = test_workspace("purge-committed-replaced");
        let source = root.join("replace-committed.txt");
        fs::write(&source, b"original payload").unwrap();
        let imported =
            import_files_record(&database, None, &[source.to_string_lossy().into_owned()]).unwrap();
        let file_id = imported.files[0].id.clone();
        let trashed = trash_file_record(&database, &versions, &file_id).unwrap();
        let entry_id = trashed.trash_items[0].id.clone();
        let (operation, members, paths) =
            stage_purge_for_test(&database, &storage, &versions, &entry_id);
        commit_purge_database(&database, &operation, &members).unwrap();
        let path = &paths[0];
        let staged = physical_path(&storage, &path.staged_relative_path).unwrap();
        fs::remove_dir_all(&staged).unwrap();
        fs::create_dir(&staged).unwrap();
        fs::write(staged.join("replacement"), b"must survive").unwrap();

        let error = recover_pending_purge_operations(&database, &versions).unwrap_err();

        assert!(error.contains("identity changed"));
        assert_eq!(
            fs::read(staged.join("replacement")).unwrap(),
            b"must survive"
        );
        assert_eq!(purge_journal_count(&database), 1);
        assert!(purge_path_identity(&storage, &path.original_relative_path)
            .unwrap()
            .is_none());
        let connection = database.0.lock().unwrap();
        let file_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM files WHERE id = ?1",
                [&file_id],
                |row| row.get(0),
            )
            .unwrap();
        let trash_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM file_space_trash_entries WHERE id = ?1",
                [&entry_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(file_count, 0);
        assert_eq!(trash_count, 0);
        drop(connection);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn background_status_counts_only_index_metadata_and_retry_requeues_failures() {
        let root =
            std::env::temp_dir().join(format!("lumetrace-background-status-{}", Uuid::new_v4()));
        let database = crate::database::open_database(root.join("lumetrace.sqlite3")).unwrap();
        {
            let connection = database.0.lock().unwrap();
            connection
                .execute_batch(
                    "INSERT INTO files
                       (id, original_name, storage_path, source_kind, updated_at, created_at)
                     VALUES
                       ('ready-file', 'ready.md', 'ready.md', 'user_import', 1, 1),
                       ('pending-file', 'pending.md', 'pending.md', 'user_import', 2, 2),
                       ('failed-file', 'failed.md', 'failed.md', 'user_import', 3, 3);
                     INSERT INTO file_space_search_documents
                       (file_id, file_name, body_text, extraction_status, extraction_error,
                        extraction_version, file_updated_at, size_bytes, indexed_at)
                     VALUES
                       ('ready-file', 'ready.md', 'body must never be returned', 'extracted', NULL, 1, 1, 27, 10),
                       ('pending-file', 'pending.md', '', 'pending', NULL, 1, 2, 18, 20),
                       ('failed-file', 'failed.md', '', 'failed', 'cannot read', 1, 3, 21, 30);
                     INSERT INTO file_space_index_jobs
                       (file_id, requested_document_indexed_at, status, retry_count, error, requested_at)
                     VALUES ('failed-file', 30, 'failed', 1, 'cannot embed', 30);",
                )
                .unwrap();
        }

        let running = content_index_status(&database, false).unwrap();
        assert_eq!(running.state, "running");
        assert_eq!(running.total_files, 3);
        assert_eq!(running.completed_files, 1);
        assert_eq!(running.pending_files, 1);
        assert_eq!(running.failed_files, 1);
        assert_eq!(running.current_file.as_deref(), Some("pending.md"));
        assert_eq!(running.error.as_deref(), Some("cannot read"));
        assert!(!format!("{running:?}").contains("body must never be returned"));

        let paused = content_index_status(&database, true).unwrap();
        assert_eq!(paused.state, "paused");

        retry_background_failures(&database).unwrap();
        let connection = database.0.lock().unwrap();
        let extraction_status: String = connection
            .query_row(
                "SELECT extraction_status FROM file_space_search_documents WHERE file_id = 'failed-file'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let semantic_status: String = connection
            .query_row(
                "SELECT status FROM file_space_index_jobs WHERE file_id = 'failed-file'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(extraction_status, "pending");
        assert_eq!(semantic_status, "failed");
        drop(connection);

        database
            .0
            .lock()
            .unwrap()
            .execute(
                "UPDATE file_space_search_documents
                 SET extraction_status = 'extracted', extraction_error = NULL
                 WHERE file_id = 'failed-file'",
                [],
            )
            .unwrap();
        crate::semantic_search::schedule_search_documents(&database, &["failed-file".to_owned()])
            .unwrap();
        let semantic_status: String = database
            .0
            .lock()
            .unwrap()
            .query_row(
                "SELECT status FROM file_space_index_jobs WHERE file_id = 'failed-file'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(semantic_status, "pending");
        drop(database);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn background_status_queue_lookup_uses_the_extraction_index() {
        let root = std::env::temp_dir().join(format!(
            "lumetrace-background-query-plan-{}",
            Uuid::new_v4()
        ));
        let database = crate::database::open_database(root.join("lumetrace.sqlite3")).unwrap();
        let connection = database.0.lock().unwrap();
        let details = connection
            .prepare(
                "EXPLAIN QUERY PLAN
                 SELECT file_name FROM file_space_search_documents
                 WHERE extraction_status = 'pending'
                 ORDER BY indexed_at, file_id LIMIT 1",
            )
            .unwrap()
            .query_map([], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(details
            .iter()
            .any(|detail| { detail.contains("idx_file_space_search_documents_extraction") }));
        drop(connection);
        drop(database);
        fs::remove_dir_all(root).unwrap();
    }
}
