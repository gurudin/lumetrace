use crate::{
    ai_service::{cloud_ai_is_configured, CLOUD_AI_API_KEY_KEY, CLOUD_AI_SETTINGS_KEY},
    database::{self, Database, DatabaseLocation},
    file_space::{
        self, configure_storage_root_record, emit_import_progress,
        import_existing_storage_root_record_with_progress, lock_file_space_operations,
        FileSpaceSnapshot, FileSpaceVersionNotificationQueue,
    },
};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

const REGISTRY_FILE_NAME: &str = "workspaces.json";
const REGISTRY_VERSION: u32 = 1;
const SHARED_SETTING_KEYS: [&str; 6] = [
    "ai.agent_cli",
    CLOUD_AI_SETTINGS_KEY,
    CLOUD_AI_API_KEY_KEY,
    "ai.local_llm",
    "ai.service_mode",
    "semantic_search.model_id",
];
type SharedSettingRecord = (String, String, i64);
type CloudSettingPair = [SharedSettingRecord; 2];

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StoredWorkspace {
    id: String,
    name: String,
    kind: String,
    root_path: Option<String>,
    database_path: PathBuf,
    artifact_store_path: PathBuf,
    member_count: usize,
    created_at: i64,
    last_opened_at: i64,
}

impl StoredWorkspace {
    fn location(&self) -> DatabaseLocation {
        DatabaseLocation {
            workspace_id: self.id.clone(),
            database_path: self.database_path.clone(),
            artifact_store_path: self.artifact_store_path.clone(),
        }
    }

    fn summary(&self, current_workspace_id: &str) -> FileSpaceWorkspace {
        FileSpaceWorkspace {
            id: self.id.clone(),
            name: self.name.clone(),
            kind: self.kind.clone(),
            root_path: self.root_path.clone(),
            member_count: self.member_count,
            current: self.id == current_workspace_id,
            created_at: self.created_at,
            last_opened_at: self.last_opened_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegistryDocument {
    version: u32,
    current_workspace_id: String,
    workspaces: Vec<StoredWorkspace>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSpaceWorkspace {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub root_path: Option<String>,
    pub member_count: usize,
    pub current: bool,
    pub created_at: i64,
    pub last_opened_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSpaceWorkspaceDirectory {
    pub current_workspace_id: String,
    pub workspaces: Vec<FileSpaceWorkspace>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSpaceWorkspaceMutation {
    pub directory: FileSpaceWorkspaceDirectory,
    pub snapshot: FileSpaceSnapshot,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFileSpaceWorkspaceRequest {
    request_id: String,
    name: String,
    path: String,
    mode: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveFileSpaceWorkspaceRequest {
    workspace_id: String,
    #[serde(default)]
    delete_workspace_data: bool,
}

pub struct WorkspaceRegistry {
    registry_path: PathBuf,
    data_directory: PathBuf,
    document: Mutex<RegistryDocument>,
}

fn read_storage_root(database: &Database) -> Result<Option<String>, String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    connection
        .query_row(
            "SELECT value FROM app_settings WHERE key = 'file_space.storage_root'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("Unable to load the workspace storage path: {error}"))
}

fn default_workspace_name(root_path: Option<&str>) -> String {
    root_path
        .and_then(|path| Path::new(path).file_name())
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("Lume Trace")
        .to_owned()
}

fn validate_workspace_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Workspace name cannot be empty".to_owned());
    }
    if name.chars().count() > 80 {
        return Err("Workspace name cannot exceed 80 characters".to_owned());
    }
    if name.chars().any(char::is_control) {
        return Err("Workspace name contains unsupported characters".to_owned());
    }
    Ok(name.to_owned())
}

fn write_registry(path: &Path, document: &RegistryDocument) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Unable to resolve the workspace registry directory".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Unable to create the workspace registry directory: {error}"))?;
    let bytes = serde_json::to_vec_pretty(document)
        .map_err(|error| format!("Unable to encode the workspace registry: {error}"))?;
    let temporary = parent.join(format!(".workspaces-{}.tmp", Uuid::new_v4()));
    fs::write(&temporary, bytes)
        .map_err(|error| format!("Unable to save the workspace registry: {error}"))?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(format!(
            "Unable to finish saving the workspace registry: {error}"
        ));
    }
    Ok(())
}

impl WorkspaceRegistry {
    fn load_or_create(data_directory: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&data_directory)
            .map_err(|error| format!("Unable to create application data directory: {error}"))?;
        let registry_path = data_directory.join(REGISTRY_FILE_NAME);
        let document = if registry_path.is_file() {
            let bytes = fs::read(&registry_path)
                .map_err(|error| format!("Unable to read the workspace registry: {error}"))?;
            let mut document: RegistryDocument = serde_json::from_slice(&bytes)
                .map_err(|error| format!("Unable to decode the workspace registry: {error}"))?;
            if document.version != REGISTRY_VERSION {
                return Err(format!(
                    "Unsupported workspace registry version: {}",
                    document.version
                ));
            }
            if document.workspaces.is_empty() {
                return Err("The workspace registry does not contain a workspace".to_owned());
            }
            if !document
                .workspaces
                .iter()
                .any(|workspace| workspace.id == document.current_workspace_id)
            {
                document.current_workspace_id = document.workspaces[0].id.clone();
                write_registry(&registry_path, &document)?;
            }
            document
        } else {
            let id = Uuid::new_v4().to_string();
            let database_path = data_directory.join("lumetrace.sqlite3");
            let artifact_store_path = data_directory.join("file-space-versions");
            let database = database::open_workspace_database(
                database_path.clone(),
                artifact_store_path.clone(),
                id.clone(),
            )?;
            let root_path = read_storage_root(&database)?;
            let now = now_millis();
            let document = RegistryDocument {
                version: REGISTRY_VERSION,
                current_workspace_id: id.clone(),
                workspaces: vec![StoredWorkspace {
                    id,
                    name: default_workspace_name(root_path.as_deref()),
                    kind: "local".to_owned(),
                    root_path,
                    database_path,
                    artifact_store_path,
                    member_count: 0,
                    created_at: now,
                    last_opened_at: now,
                }],
            };
            write_registry(&registry_path, &document)?;
            document
        };
        Ok(Self {
            registry_path,
            data_directory,
            document: Mutex::new(document),
        })
    }

    fn current_workspace(&self) -> Result<StoredWorkspace, String> {
        let document = self
            .document
            .lock()
            .map_err(|_| "Unable to access the workspace registry".to_owned())?;
        document
            .workspaces
            .iter()
            .find(|workspace| workspace.id == document.current_workspace_id)
            .cloned()
            .ok_or_else(|| "The current workspace no longer exists".to_owned())
    }

    fn workspaces(&self) -> Result<Vec<StoredWorkspace>, String> {
        self.document
            .lock()
            .map(|document| document.workspaces.clone())
            .map_err(|_| "Unable to access the workspace registry".to_owned())
    }

    fn workspace(&self, workspace_id: &str) -> Result<StoredWorkspace, String> {
        let document = self
            .document
            .lock()
            .map_err(|_| "Unable to access the workspace registry".to_owned())?;
        document
            .workspaces
            .iter()
            .find(|workspace| workspace.id == workspace_id)
            .cloned()
            .ok_or_else(|| "The selected workspace no longer exists".to_owned())
    }

    fn directory(&self, database: &Database) -> Result<FileSpaceWorkspaceDirectory, String> {
        let current_root = read_storage_root(database)?;
        let mut document = self
            .document
            .lock()
            .map_err(|_| "Unable to access the workspace registry".to_owned())?;
        let current_id = document.current_workspace_id.clone();
        let mut changed = false;
        if let Some(current) = document
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.id == current_id)
        {
            if current.root_path != current_root {
                current.root_path = current_root;
                changed = true;
            }
        }
        if changed {
            write_registry(&self.registry_path, &document)?;
        }
        Ok(FileSpaceWorkspaceDirectory {
            current_workspace_id: current_id.clone(),
            workspaces: document
                .workspaces
                .iter()
                .map(|workspace| workspace.summary(&current_id))
                .collect(),
        })
    }

    fn set_current(&self, workspace_id: &str) -> Result<(), String> {
        let mut document = self
            .document
            .lock()
            .map_err(|_| "Unable to access the workspace registry".to_owned())?;
        if !document
            .workspaces
            .iter()
            .any(|workspace| workspace.id == workspace_id)
        {
            return Err("The selected workspace no longer exists".to_owned());
        }
        let opened_at = now_millis();
        document.current_workspace_id = workspace_id.to_owned();
        if let Some(workspace) = document
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.id == workspace_id)
        {
            workspace.last_opened_at = opened_at;
        }
        write_registry(&self.registry_path, &document)
    }

    fn add_and_set_current(&self, workspace: StoredWorkspace) -> Result<(), String> {
        let mut document = self
            .document
            .lock()
            .map_err(|_| "Unable to access the workspace registry".to_owned())?;
        if document
            .workspaces
            .iter()
            .any(|candidate| candidate.id == workspace.id)
        {
            return Err("The workspace is already registered".to_owned());
        }
        document.current_workspace_id = workspace.id.clone();
        document.workspaces.push(workspace);
        write_registry(&self.registry_path, &document)
    }

    fn rename(&self, workspace_id: &str, name: String) -> Result<(), String> {
        let mut document = self
            .document
            .lock()
            .map_err(|_| "Unable to access the workspace registry".to_owned())?;
        let workspace = document
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.id == workspace_id)
            .ok_or_else(|| "The selected workspace no longer exists".to_owned())?;
        workspace.name = name;
        write_registry(&self.registry_path, &document)
    }

    fn remove(&self, workspace_id: &str) -> Result<(), String> {
        let mut document = self
            .document
            .lock()
            .map_err(|_| "Unable to access the workspace registry".to_owned())?;
        if document.current_workspace_id == workspace_id {
            return Err("The current workspace cannot be removed from the list".to_owned());
        }
        let previous_len = document.workspaces.len();
        document
            .workspaces
            .retain(|workspace| workspace.id != workspace_id);
        if document.workspaces.len() == previous_len {
            return Err("The selected workspace no longer exists".to_owned());
        }
        write_registry(&self.registry_path, &document)
    }

    fn new_workspace_paths(&self, workspace_id: &str) -> (PathBuf, PathBuf) {
        let workspace_directory = self.data_directory.join("workspaces").join(workspace_id);
        (
            workspace_directory.join("lumetrace.sqlite3"),
            workspace_directory.join("file-space-versions"),
        )
    }
}

fn copy_shared_settings(source: &Database, destination: &Database) -> Result<(), String> {
    let settings = {
        let connection = source
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
        let mut statement = connection
            .prepare(
                "SELECT key, value, updated_at FROM app_settings
                 WHERE key IN (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .map_err(|error| format!("Unable to prepare shared settings: {error}"))?;
        statement
            .query_map(
                params![
                    SHARED_SETTING_KEYS[0],
                    SHARED_SETTING_KEYS[1],
                    SHARED_SETTING_KEYS[2],
                    SHARED_SETTING_KEYS[3],
                    SHARED_SETTING_KEYS[4],
                    SHARED_SETTING_KEYS[5]
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|error| format!("Unable to load shared settings: {error}"))?
    };
    let mut connection = destination
        .0
        .lock()
        .map_err(|_| "Unable to access the new workspace database".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Unable to begin copying shared settings: {error}"))?;
    transaction
        .execute(
            "DELETE FROM app_settings WHERE key IN (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                SHARED_SETTING_KEYS[0],
                SHARED_SETTING_KEYS[1],
                SHARED_SETTING_KEYS[2],
                SHARED_SETTING_KEYS[3],
                SHARED_SETTING_KEYS[4],
                SHARED_SETTING_KEYS[5]
            ],
        )
        .map_err(|error| format!("Unable to synchronize shared settings: {error}"))?;
    for (key, value, updated_at) in settings {
        transaction
            .execute(
                "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                params![key, value, updated_at],
            )
            .map_err(|error| format!("Unable to copy a shared setting: {error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("Unable to finish copying shared settings: {error}"))
}

fn cloud_setting_pair(database: &Database) -> Result<Option<CloudSettingPair>, String> {
    if !cloud_ai_is_configured(database)? {
        return Ok(None);
    }
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let mut statement = connection
        .prepare(
            "SELECT key, value, updated_at FROM app_settings
             WHERE key IN (?1, ?2)",
        )
        .map_err(|error| format!("Unable to prepare cloud AI settings: {error}"))?;
    let records = statement
        .query_map(
            params![CLOUD_AI_SETTINGS_KEY, CLOUD_AI_API_KEY_KEY],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .map_err(|error| format!("Unable to load cloud AI settings: {error}"))?;
    if records.len() != 2 {
        return Ok(None);
    }
    let cloud = records
        .iter()
        .find(|(key, _, _)| key == CLOUD_AI_SETTINGS_KEY)
        .cloned();
    let api_key = records
        .iter()
        .find(|(key, _, _)| key == CLOUD_AI_API_KEY_KEY)
        .cloned();
    Ok(cloud.zip(api_key).map(|(cloud, api_key)| [cloud, api_key]))
}

fn cloud_setting_pair_updated_at(settings: &CloudSettingPair) -> i64 {
    settings
        .iter()
        .map(|(_, _, updated_at)| *updated_at)
        .max()
        .unwrap_or_default()
}

fn restore_cloud_settings(database: &Database, settings: CloudSettingPair) -> Result<(), String> {
    let mut connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Unable to begin restoring cloud AI settings: {error}"))?;
    for (key, value, updated_at) in settings {
        transaction
            .execute(
                "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                params![key, value, updated_at],
            )
            .map_err(|error| format!("Unable to restore a cloud AI setting: {error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("Unable to finish restoring cloud AI settings: {error}"))
}

fn restore_missing_cloud_settings(
    database: &Database,
    fallback: &Database,
) -> Result<bool, String> {
    if cloud_setting_pair(database)?.is_some() {
        return Ok(false);
    }
    let Some(settings) = cloud_setting_pair(fallback)? else {
        return Ok(false);
    };
    restore_cloud_settings(database, settings)?;
    Ok(true)
}

fn canonical_existing_path(
    path: &Path,
    expected_directory: bool,
) -> Result<Option<PathBuf>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Unable to inspect {}: {error}", path.display())),
    };
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "Refusing to delete a symbolic link: {}",
            path.display()
        ));
    }
    if expected_directory != metadata.is_dir() {
        return Err(format!(
            "The deletion target has an unexpected type: {}",
            path.display()
        ));
    }
    path.canonicalize()
        .map(Some)
        .map_err(|error| format!("Unable to resolve {}: {error}", path.display()))
}

fn canonical_app_data_directory(app_data_directory: &Path) -> Result<PathBuf, String> {
    app_data_directory
        .canonicalize()
        .map_err(|error| format!("Unable to resolve the Lume Trace data directory: {error}"))
}

fn validate_internal_file_target(
    path: &Path,
    app_data_directory: &Path,
) -> Result<Option<PathBuf>, String> {
    let Some(target) = canonical_existing_path(path, false)? else {
        return Ok(None);
    };
    let app_data = canonical_app_data_directory(app_data_directory)?;
    if target == app_data || !target.starts_with(&app_data) {
        return Err(
            "The workspace database is outside the managed Lume Trace data directory".to_owned(),
        );
    }
    Ok(Some(target))
}

fn validate_internal_directory_target(
    path: &Path,
    app_data_directory: &Path,
) -> Result<Option<PathBuf>, String> {
    let Some(target) = canonical_existing_path(path, true)? else {
        return Ok(None);
    };
    let app_data = canonical_app_data_directory(app_data_directory)?;
    if target == app_data || !target.starts_with(&app_data) {
        return Err(
            "The version directory is outside the managed Lume Trace data directory".to_owned(),
        );
    }
    Ok(Some(target))
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

fn sqlite_sidecar_path(database_path: &Path, suffix: &str) -> PathBuf {
    let mut path = OsString::from(database_path.as_os_str());
    path.push(suffix);
    PathBuf::from(path)
}

fn remove_file_if_present(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Unable to delete {}: {error}", path.display())),
    }
}

fn remove_semantic_ann_temporary_files(database_path: &Path) -> Result<(), String> {
    let Some(directory) = database_path.parent() else {
        return Ok(());
    };
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "Unable to inspect the workspace semantic index directory: {error}"
            ))
        }
    };
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!("Unable to inspect a workspace semantic index file: {error}")
        })?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with(".semantic-search.usearch.") && name.ends_with(".tmp") {
            remove_file_if_present(&entry.path())?;
        }
    }
    Ok(())
}

fn remove_empty_workspace_container(
    workspace: &StoredWorkspace,
    app_data_directory: &Path,
) -> Result<(), String> {
    let Some(workspace_directory) = workspace.database_path.parent() else {
        return Ok(());
    };
    if workspace.artifact_store_path.parent() != Some(workspace_directory)
        || workspace_directory
            .file_name()
            .and_then(|name| name.to_str())
            != Some(workspace.id.as_str())
    {
        return Ok(());
    }
    let managed_workspaces_directory = app_data_directory.join("workspaces");
    if workspace_directory.parent() != Some(managed_workspaces_directory.as_path()) {
        return Ok(());
    }
    match fs::remove_dir(workspace_directory) {
        Ok(()) => Ok(()),
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                || error.kind() == std::io::ErrorKind::DirectoryNotEmpty =>
        {
            Ok(())
        }
        Err(error) => Err(format!(
            "Unable to remove the empty workspace data directory: {error}"
        )),
    }
}

fn remove_workspace_with_options(
    registry: &WorkspaceRegistry,
    workspace: &StoredWorkspace,
    request: &RemoveFileSpaceWorkspaceRequest,
    app_data_directory: &Path,
) -> Result<(), String> {
    if registry.current_workspace()?.id == workspace.id {
        return Err("The current workspace cannot be removed from the list".to_owned());
    }
    let all_workspaces = registry.workspaces()?;
    let database_target = if request.delete_workspace_data {
        validate_internal_file_target(&workspace.database_path, app_data_directory)?
    } else {
        None
    };
    let version_target = if request.delete_workspace_data {
        validate_internal_directory_target(&workspace.artifact_store_path, app_data_directory)?
    } else {
        None
    };
    let semantic_index_target = if request.delete_workspace_data {
        validate_internal_file_target(
            &crate::semantic_search::semantic_ann_index_path_for_database(&workspace.database_path),
            app_data_directory,
        )?
    } else {
        None
    };

    for other in all_workspaces
        .iter()
        .filter(|candidate| candidate.id != workspace.id)
    {
        if request.delete_workspace_data && other.database_path == workspace.database_path {
            return Err(
                "The workspace database is also registered by another workspace".to_owned(),
            );
        }
        if request.delete_workspace_data
            && paths_overlap(&other.artifact_store_path, &workspace.artifact_store_path)
        {
            return Err("The version directory overlaps another registered workspace".to_owned());
        }
    }

    if let Some(target) = version_target {
        fs::remove_dir_all(&target)
            .map_err(|error| format!("Unable to delete the workspace version history: {error}"))?;
    }
    if request.delete_workspace_data {
        if let Some(target) = semantic_index_target.as_deref() {
            remove_file_if_present(target)?;
        }
        if let Some(target) = database_target.as_deref() {
            remove_semantic_ann_temporary_files(target)?;
            remove_file_if_present(target)?;
            remove_file_if_present(&sqlite_sidecar_path(target, "-wal"))?;
            remove_file_if_present(&sqlite_sidecar_path(target, "-shm"))?;
            remove_file_if_present(&sqlite_sidecar_path(target, "-journal"))?;
        }
        remove_empty_workspace_container(workspace, app_data_directory)?;
    }
    registry.remove(&workspace.id)
}

pub fn initialize(app: &AppHandle) -> Result<(Database, WorkspaceRegistry), String> {
    let data_directory = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Unable to resolve application data directory: {error}"))?;
    let registry = WorkspaceRegistry::load_or_create(data_directory)?;
    let current = registry.current_workspace()?;
    let mut newest_cloud_settings: Option<CloudSettingPair> = None;
    for workspace in registry.workspaces()? {
        if workspace.id == current.id {
            continue;
        }
        let inactive_database = database::open_workspace_database(
            workspace.database_path,
            workspace.artifact_store_path,
            workspace.id,
        )?;
        database::recover_interrupted_ai_turns(&inactive_database)?;
        if let Some(settings) = cloud_setting_pair(&inactive_database)? {
            let is_newer = newest_cloud_settings
                .as_ref()
                .is_none_or(|current_settings| {
                    cloud_setting_pair_updated_at(&settings)
                        > cloud_setting_pair_updated_at(current_settings)
                });
            if is_newer {
                newest_cloud_settings = Some(settings);
            }
        }
    }
    let database = database::open_workspace_database(
        current.database_path.clone(),
        current.artifact_store_path.clone(),
        current.id.clone(),
    )?;
    database::recover_interrupted_ai_turns(&database)?;
    if cloud_setting_pair(&database)?.is_none() {
        if let Some(settings) = newest_cloud_settings {
            restore_cloud_settings(&database, settings)?;
        }
    }
    Ok((database, registry))
}

fn switch_database(
    database: &Database,
    registry: &WorkspaceRegistry,
    workspace: &StoredWorkspace,
) -> Result<(), String> {
    let previous = database.location()?;
    let target = database::open_workspace_database(
        workspace.database_path.clone(),
        workspace.artifact_store_path.clone(),
        workspace.id.clone(),
    )?;
    restore_missing_cloud_settings(database, &target)?;
    copy_shared_settings(database, &target)?;
    drop(target);
    database.switch_workspace(workspace.location())?;
    if let Err(error) = registry.set_current(&workspace.id) {
        let _ = database.switch_workspace(previous);
        return Err(error);
    }
    Ok(())
}

fn mutation_result(
    database: &Database,
    registry: &WorkspaceRegistry,
) -> Result<FileSpaceWorkspaceMutation, String> {
    let snapshot = file_space::load_workspace_snapshot(database, &database.artifact_store_path()?)?;
    let directory = registry.directory(database)?;
    Ok(FileSpaceWorkspaceMutation {
        directory,
        snapshot,
    })
}

#[tauri::command]
pub fn get_file_space_workspaces(
    database: State<'_, Database>,
    registry: State<'_, WorkspaceRegistry>,
) -> Result<FileSpaceWorkspaceDirectory, String> {
    registry.directory(database.inner())
}

#[tauri::command]
pub async fn switch_file_space_workspace<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    workspace_id: String,
) -> Result<FileSpaceWorkspaceMutation, String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let database = worker_app.state::<Database>();
        let registry = worker_app.state::<WorkspaceRegistry>();
        let _operation = lock_file_space_operations()?;
        let workspace = registry.workspace(&workspace_id)?;
        switch_database(database.inner(), registry.inner(), &workspace)?;
        worker_app
            .state::<FileSpaceVersionNotificationQueue>()
            .clear()?;
        mutation_result(database.inner(), registry.inner())
    })
    .await
    .map_err(|error| format!("Unable to switch the workspace: {error}"))?
}

#[tauri::command]
pub async fn create_file_space_workspace<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    request: CreateFileSpaceWorkspaceRequest,
) -> Result<FileSpaceWorkspaceMutation, String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let database = worker_app.state::<Database>();
        let registry = worker_app.state::<WorkspaceRegistry>();
        let _operation = lock_file_space_operations()?;
        let name = validate_workspace_name(&request.name)?;
        if request.mode != "new" && request.mode != "import" {
            return Err("Unsupported workspace creation mode".to_owned());
        }
        let id = Uuid::new_v4().to_string();
        let (database_path, artifact_store_path) = registry.new_workspace_paths(&id);
        let new_database = database::open_workspace_database(
            database_path.clone(),
            artifact_store_path.clone(),
            id.clone(),
        )?;
        copy_shared_settings(database.inner(), &new_database)?;
        let snapshot = if request.mode == "import" {
            emit_import_progress(&worker_app, &request.request_id, "scanning", 0, 0, None);
            let snapshot = import_existing_storage_root_record_with_progress(
                &new_database,
                &artifact_store_path,
                &request.path,
                |processed, total, current_name| {
                    emit_import_progress(
                        &worker_app,
                        &request.request_id,
                        "importing",
                        processed,
                        total,
                        current_name.map(str::to_owned),
                    );
                },
            )?;
            let total = snapshot.file_count.max(0) as usize;
            emit_import_progress(
                &worker_app,
                &request.request_id,
                "completed",
                total,
                total,
                None,
            );
            snapshot
        } else {
            configure_storage_root_record(&new_database, &request.path)?
        };
        let now = now_millis();
        let workspace = StoredWorkspace {
            id: id.clone(),
            name,
            kind: "local".to_owned(),
            root_path: snapshot.root_path.clone(),
            database_path,
            artifact_store_path,
            member_count: 0,
            created_at: now,
            last_opened_at: now,
        };
        let previous = database.location()?;
        database.switch_workspace(workspace.location())?;
        if let Err(error) = registry.add_and_set_current(workspace) {
            let _ = database.switch_workspace(previous);
            return Err(error);
        }
        worker_app
            .state::<FileSpaceVersionNotificationQueue>()
            .clear()?;
        mutation_result(database.inner(), registry.inner())
    })
    .await
    .map_err(|error| format!("Unable to create the workspace: {error}"))?
}

#[tauri::command]
pub fn rename_file_space_workspace(
    workspace_id: String,
    name: String,
    database: State<'_, Database>,
    registry: State<'_, WorkspaceRegistry>,
) -> Result<FileSpaceWorkspaceDirectory, String> {
    let _operation = lock_file_space_operations()?;
    let name = validate_workspace_name(&name)?;
    registry.rename(&workspace_id, name)?;
    registry.directory(database.inner())
}

#[tauri::command]
pub async fn remove_file_space_workspace<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    request: RemoveFileSpaceWorkspaceRequest,
) -> Result<FileSpaceWorkspaceDirectory, String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let database = worker_app.state::<Database>();
        let registry = worker_app.state::<WorkspaceRegistry>();
        let _operation = lock_file_space_operations()?;
        let workspace = registry.workspace(&request.workspace_id)?;
        let app_data_directory = worker_app
            .path()
            .app_data_dir()
            .map_err(|error| format!("Unable to resolve the Lume Trace data directory: {error}"))?;
        remove_workspace_with_options(registry.inner(), &workspace, &request, &app_data_directory)?;
        registry.directory(database.inner())
    })
    .await
    .map_err(|error| format!("Unable to remove the workspace: {error}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn register_inactive_workspace(
        registry: &WorkspaceRegistry,
        app_data_directory: &Path,
        root_path: &Path,
    ) -> StoredWorkspace {
        let current = registry.current_workspace().unwrap();
        let id = Uuid::new_v4().to_string();
        let workspace_directory = app_data_directory.join("workspaces").join(&id);
        let database_path = workspace_directory.join("lumetrace.sqlite3");
        let artifact_store_path = workspace_directory.join("file-space-versions");
        let database = database::open_workspace_database(
            database_path.clone(),
            artifact_store_path.clone(),
            id.clone(),
        )
        .unwrap();
        drop(database);
        fs::create_dir_all(&artifact_store_path).unwrap();
        fs::write(
            artifact_store_path.join("snapshot.bin"),
            b"version snapshot",
        )
        .unwrap();
        let now = now_millis();
        let workspace = StoredWorkspace {
            id,
            name: "Removable workspace".to_owned(),
            kind: "local".to_owned(),
            root_path: Some(root_path.to_string_lossy().to_string()),
            database_path,
            artifact_store_path,
            member_count: 0,
            created_at: now,
            last_opened_at: now,
        };
        registry.add_and_set_current(workspace.clone()).unwrap();
        registry.set_current(&current.id).unwrap();
        workspace
    }

    #[test]
    fn removing_only_the_workspace_record_preserves_all_files() {
        let root =
            std::env::temp_dir().join(format!("lumetrace-remove-record-only-{}", Uuid::new_v4()));
        let app_data_directory = root.join("app-data");
        let physical_root = root.join("user-files");
        fs::create_dir_all(&physical_root).unwrap();
        let physical_file = physical_root.join("proposal.md");
        fs::write(&physical_file, b"original file").unwrap();
        let registry = WorkspaceRegistry::load_or_create(app_data_directory.clone()).unwrap();
        let workspace = register_inactive_workspace(&registry, &app_data_directory, &physical_root);
        let semantic_index =
            crate::semantic_search::semantic_ann_index_path_for_database(&workspace.database_path);
        fs::write(&semantic_index, b"derived index").unwrap();
        let semantic_temporary_index = workspace
            .database_path
            .parent()
            .unwrap()
            .join(".semantic-search.usearch.interrupted.tmp");
        fs::write(&semantic_temporary_index, b"interrupted derived index").unwrap();

        remove_workspace_with_options(
            &registry,
            &workspace,
            &RemoveFileSpaceWorkspaceRequest {
                workspace_id: workspace.id.clone(),
                delete_workspace_data: false,
            },
            &app_data_directory,
        )
        .unwrap();

        assert!(registry.workspace(&workspace.id).is_err());
        assert!(workspace.database_path.is_file());
        assert!(semantic_index.is_file());
        assert!(semantic_temporary_index.is_file());
        assert!(workspace.artifact_store_path.join("snapshot.bin").is_file());
        assert!(physical_file.is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn deleting_managed_workspace_data_preserves_the_physical_file_root() {
        let root =
            std::env::temp_dir().join(format!("lumetrace-delete-managed-data-{}", Uuid::new_v4()));
        let app_data_directory = root.join("app-data");
        let physical_root = root.join("user-files");
        fs::create_dir_all(&physical_root).unwrap();
        let physical_file = physical_root.join("proposal.md");
        fs::write(&physical_file, b"original file").unwrap();
        let registry = WorkspaceRegistry::load_or_create(app_data_directory.clone()).unwrap();
        let workspace = register_inactive_workspace(&registry, &app_data_directory, &physical_root);
        let semantic_index =
            crate::semantic_search::semantic_ann_index_path_for_database(&workspace.database_path);
        fs::write(&semantic_index, b"derived index").unwrap();
        let semantic_temporary_index = workspace
            .database_path
            .parent()
            .unwrap()
            .join(".semantic-search.usearch.interrupted.tmp");
        fs::write(&semantic_temporary_index, b"interrupted derived index").unwrap();

        remove_workspace_with_options(
            &registry,
            &workspace,
            &RemoveFileSpaceWorkspaceRequest {
                workspace_id: workspace.id.clone(),
                delete_workspace_data: true,
            },
            &app_data_directory,
        )
        .unwrap();

        assert!(registry.workspace(&workspace.id).is_err());
        assert!(!workspace.database_path.exists());
        assert!(!semantic_index.exists());
        assert!(!semantic_temporary_index.exists());
        assert!(!workspace.artifact_store_path.exists());
        assert!(!workspace.database_path.parent().unwrap().exists());
        assert!(physical_root.is_dir());
        assert!(physical_file.is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn current_workspace_cannot_be_removed_or_have_its_data_deleted() {
        let root =
            std::env::temp_dir().join(format!("lumetrace-remove-current-{}", Uuid::new_v4()));
        let app_data_directory = root.join("app-data");
        let registry = WorkspaceRegistry::load_or_create(app_data_directory.clone()).unwrap();
        let current = registry.current_workspace().unwrap();

        let error = remove_workspace_with_options(
            &registry,
            &current,
            &RemoveFileSpaceWorkspaceRequest {
                workspace_id: current.id.clone(),
                delete_workspace_data: true,
            },
            &app_data_directory,
        )
        .unwrap_err();

        assert!(error.contains("current workspace"));
        assert!(registry.workspace(&current.id).is_ok());
        assert!(current.database_path.is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn managed_data_outside_app_data_is_refused() {
        let root =
            std::env::temp_dir().join(format!("lumetrace-refuse-external-data-{}", Uuid::new_v4()));
        let app_data_directory = root.join("app-data");
        let external_directory = root.join("external-managed-data");
        fs::create_dir_all(&external_directory).unwrap();
        let registry = WorkspaceRegistry::load_or_create(app_data_directory.clone()).unwrap();
        let current = registry.current_workspace().unwrap();
        let id = Uuid::new_v4().to_string();
        let database_path = external_directory.join("lumetrace.sqlite3");
        fs::write(&database_path, b"external database").unwrap();
        let artifact_store_path = app_data_directory
            .join("workspaces")
            .join(&id)
            .join("file-space-versions");
        fs::create_dir_all(&artifact_store_path).unwrap();
        let workspace = StoredWorkspace {
            id,
            name: "Unsafe workspace".to_owned(),
            kind: "local".to_owned(),
            root_path: Some(root.join("user-files").to_string_lossy().to_string()),
            database_path: database_path.clone(),
            artifact_store_path: artifact_store_path.clone(),
            member_count: 0,
            created_at: now_millis(),
            last_opened_at: now_millis(),
        };
        registry.add_and_set_current(workspace.clone()).unwrap();
        registry.set_current(&current.id).unwrap();

        let error = remove_workspace_with_options(
            &registry,
            &workspace,
            &RemoveFileSpaceWorkspaceRequest {
                workspace_id: workspace.id.clone(),
                delete_workspace_data: true,
            },
            &app_data_directory,
        )
        .unwrap_err();

        assert!(error.contains("outside the managed Lume Trace data directory"));
        assert!(registry.workspace(&workspace.id).is_ok());
        assert!(database_path.is_file());
        assert!(artifact_store_path.is_dir());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shared_managed_paths_are_refused() {
        let root =
            std::env::temp_dir().join(format!("lumetrace-refuse-shared-data-{}", Uuid::new_v4()));
        let app_data_directory = root.join("app-data");
        let registry = WorkspaceRegistry::load_or_create(app_data_directory.clone()).unwrap();
        let current = registry.current_workspace().unwrap();
        let id = Uuid::new_v4().to_string();
        let artifact_store_path = app_data_directory
            .join("workspaces")
            .join(&id)
            .join("file-space-versions");
        fs::create_dir_all(&artifact_store_path).unwrap();
        let workspace = StoredWorkspace {
            id,
            name: "Shared database workspace".to_owned(),
            kind: "local".to_owned(),
            root_path: None,
            database_path: current.database_path.clone(),
            artifact_store_path: artifact_store_path.clone(),
            member_count: 0,
            created_at: now_millis(),
            last_opened_at: now_millis(),
        };
        registry.add_and_set_current(workspace.clone()).unwrap();
        registry.set_current(&current.id).unwrap();

        let error = remove_workspace_with_options(
            &registry,
            &workspace,
            &RemoveFileSpaceWorkspaceRequest {
                workspace_id: workspace.id.clone(),
                delete_workspace_data: true,
            },
            &app_data_directory,
        )
        .unwrap_err();

        assert!(error.contains("also registered by another workspace"));
        assert!(registry.workspace(&workspace.id).is_ok());
        assert!(current.database_path.is_file());
        assert!(artifact_store_path.is_dir());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_database_becomes_the_first_workspace_without_moving_data() {
        let root = std::env::temp_dir().join(format!("lumetrace-workspaces-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let database_path = root.join("lumetrace.sqlite3");
        let database = database::open_database(database_path.clone()).unwrap();
        database
            .0
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO app_settings (key, value, updated_at) VALUES ('file_space.storage_root', ?1, 1)",
                [root.to_string_lossy().to_string()],
            )
            .unwrap();
        drop(database);

        let registry = WorkspaceRegistry::load_or_create(root.clone()).unwrap();
        let current = registry.current_workspace().unwrap();
        assert_eq!(current.database_path, database_path);
        assert_eq!(
            current.root_path.as_deref(),
            Some(root.to_string_lossy().as_ref())
        );
        assert!(root.join(REGISTRY_FILE_NAME).is_file());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn database_switch_keeps_workspace_records_isolated() {
        let root =
            std::env::temp_dir().join(format!("lumetrace-database-switch-{}", Uuid::new_v4()));
        let first_path = root.join("first.sqlite3");
        let second_path = root.join("second.sqlite3");
        let database = database::open_workspace_database(
            first_path.clone(),
            root.join("first-versions"),
            "first".to_owned(),
        )
        .unwrap();
        database
            .0
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO app_settings (key, value, updated_at) VALUES ('only.first', 'yes', 1)",
                [],
            )
            .unwrap();
        database
            .switch_workspace(DatabaseLocation {
                workspace_id: "second".to_owned(),
                database_path: second_path,
                artifact_store_path: root.join("second-versions"),
            })
            .unwrap();
        let missing = database
            .0
            .lock()
            .unwrap()
            .query_row(
                "SELECT value FROM app_settings WHERE key = 'only.first'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .unwrap();
        assert_eq!(missing, None);
        database
            .switch_workspace(DatabaseLocation {
                workspace_id: "first".to_owned(),
                database_path: first_path,
                artifact_store_path: root.join("first-versions"),
            })
            .unwrap();
        let value: String = database
            .0
            .lock()
            .unwrap()
            .query_row(
                "SELECT value FROM app_settings WHERE key = 'only.first'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(value, "yes");
        drop(database);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shared_ai_service_settings_are_copied_between_workspaces() {
        let root = std::env::temp_dir().join(format!(
            "lumetrace-shared-ai-service-settings-{}",
            Uuid::new_v4()
        ));
        let source = database::open_for_test(&root.join("source.sqlite3")).unwrap();
        let destination = database::open_for_test(&root.join("destination.sqlite3")).unwrap();
        {
            let connection = source.0.lock().unwrap();
            for (key, value) in [
                ("ai.agent_cli", "agent"),
                ("ai.cloud", "cloud"),
                ("ai.cloud_api_key", "cloud-key"),
                ("ai.local_llm", "local"),
                ("ai.service_mode", "local"),
                ("semantic_search.model_id", "semantic"),
                ("workspace.only", "private"),
            ] {
                connection
                    .execute(
                        "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, 1)",
                        rusqlite::params![key, value],
                    )
                    .unwrap();
            }
        }

        copy_shared_settings(&source, &destination).unwrap();

        let copied = {
            let connection = destination.0.lock().unwrap();
            let mut statement = connection
                .prepare("SELECT key, value FROM app_settings ORDER BY key")
                .unwrap();
            statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        assert_eq!(
            copied,
            vec![
                ("ai.agent_cli".to_owned(), "agent".to_owned()),
                ("ai.cloud".to_owned(), "cloud".to_owned()),
                ("ai.cloud_api_key".to_owned(), "cloud-key".to_owned()),
                ("ai.local_llm".to_owned(), "local".to_owned()),
                ("ai.service_mode".to_owned(), "local".to_owned()),
                ("semantic_search.model_id".to_owned(), "semantic".to_owned()),
            ]
        );
        drop(source);
        drop(destination);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_cloud_settings_are_restored_before_workspace_sync() {
        let root = std::env::temp_dir().join(format!(
            "lumetrace-restore-cloud-settings-{}",
            Uuid::new_v4()
        ));
        let current = database::open_for_test(&root.join("current.sqlite3")).unwrap();
        let fallback = database::open_for_test(&root.join("fallback.sqlite3")).unwrap();
        current
            .0
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO app_settings (key, value, updated_at) VALUES ('ai.service_mode', 'cloud', 7)",
                [],
            )
            .unwrap();
        {
            let connection = fallback.0.lock().unwrap();
            for (key, value) in [
                (
                    "ai.cloud",
                    r#"{"provider":"openai","baseUrl":"https://example.test/v1","model":"test-model"}"#,
                ),
                ("ai.cloud_api_key", "test-cloud-key"),
            ] {
                connection
                    .execute(
                        "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, 7)",
                        params![key, value],
                    )
                    .unwrap();
            }
        }

        assert!(restore_missing_cloud_settings(&current, &fallback).unwrap());
        copy_shared_settings(&current, &fallback).unwrap();

        for database in [&current, &fallback] {
            let connection = database.0.lock().unwrap();
            let cloud_count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM app_settings WHERE key IN ('ai.cloud', 'ai.cloud_api_key')",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(cloud_count, 2);
        }
        drop(current);
        drop(fallback);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pinned_database_handle_stays_on_the_original_workspace_after_switch() {
        let root =
            std::env::temp_dir().join(format!("lumetrace-pinned-workspace-{}", Uuid::new_v4()));
        let first_path = root.join("first.sqlite3");
        let second_path = root.join("second.sqlite3");
        let database = database::open_workspace_database(
            first_path.clone(),
            root.join("first-versions"),
            "first".to_owned(),
        )
        .unwrap();
        let pinned_location = database.location().unwrap();
        let pinned = database::open_workspace_database(
            pinned_location.database_path,
            pinned_location.artifact_store_path,
            pinned_location.workspace_id,
        )
        .unwrap();

        database
            .switch_workspace(DatabaseLocation {
                workspace_id: "second".to_owned(),
                database_path: second_path,
                artifact_store_path: root.join("second-versions"),
            })
            .unwrap();
        pinned
            .0
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO app_settings (key, value, updated_at) VALUES ('pinned.write', 'first', 1)",
                [],
            )
            .unwrap();

        let current_value = database
            .0
            .lock()
            .unwrap()
            .query_row(
                "SELECT value FROM app_settings WHERE key = 'pinned.write'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .unwrap();
        assert_eq!(current_value, None);
        let pinned_value: String = pinned
            .0
            .lock()
            .unwrap()
            .query_row(
                "SELECT value FROM app_settings WHERE key = 'pinned.write'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(pinned_value, "first");

        drop(pinned);
        drop(database);
        fs::remove_dir_all(root).unwrap();
    }
}
