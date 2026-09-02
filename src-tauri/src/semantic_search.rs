use crate::{
    database::Database,
    file_space::{lock_file_space_operations, FileSpaceBackgroundRuntime},
};
use fastembed::{
    InitOptionsUserDefined, Pooling, QuantizationMode, TextEmbedding, TokenizerFiles,
    UserDefinedEmbeddingModel,
};
use reqwest::blocking::Client;
use rusqlite::{params, params_from_iter, types::Value, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering},
        Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, Manager, State};
use uuid::Uuid;

pub const SEMANTIC_MODEL_ID: &str = "multilingual-e5-small-int8-v1";
const SEMANTIC_MODEL_NAME: &str = "Multilingual E5 Small";
const SEMANTIC_MODEL_REVISION: &str = "761b726dd34fb83930e26aab4e9ac3899aa1fa78";
const SEMANTIC_MODEL_DIMENSIONS: usize = 384;
const SEMANTIC_MODEL_DIRECTORY: &str = "semantic-models/multilingual-e5-small-int8-v1";
const SEMANTIC_MODEL_MANIFEST: &str = "manifest.json";
const SEMANTIC_MODEL_SETTING: &str = "semantic_search.model_id";
const SEMANTIC_EVENT: &str = "semantic-search-status";
const SEMANTIC_CHUNK_CHARACTERS: usize = 480;
const SEMANTIC_CHUNK_OVERLAP: usize = 80;
const SEMANTIC_MIN_SIMILARITY: f32 = 0.72;
const SEMANTIC_RESULT_LIMIT: usize = 80;
const SEMANTIC_CANDIDATE_FILE_LIMIT: usize = 200;
const SEMANTIC_CHUNKS_PER_FILE_LIMIT: usize = 64;
const SEMANTIC_VECTOR_SCAN_LIMIT: usize = 4_096;
const AI_CONTEXT_CANDIDATE_FILE_LIMIT: usize = 40;
const AI_CONTEXT_STORED_CHUNK_LIMIT: usize = 64;
const AI_CONTEXT_LEXICAL_CHUNK_LIMIT: usize = 8;
const SEMANTIC_INTRA_THREADS: usize = 1;
const SEMANTIC_COOLDOWN_MULTIPLIER: u32 = 2;
const SEMANTIC_MIN_COOLDOWN: Duration = Duration::from_millis(80);
const SEMANTIC_MAX_COOLDOWN: Duration = Duration::from_secs(2);
const SEMANTIC_IDLE_PAUSE: Duration = Duration::from_millis(700);
const SEMANTIC_STATUS_INTERVAL: Duration = Duration::from_secs(1);
const OUTDATED_DOCUMENT_BATCH_SIZE: usize = 256;
const OUTDATED_DOCUMENT_BATCH_PAUSE: Duration = Duration::from_millis(5);
const OUTDATED_DOCUMENT_BATCH_QUERY: &str = "SELECT documents.file_id,
            CASE WHEN jobs.file_id IS NULL
               OR jobs.requested_document_indexed_at <> documents.indexed_at
               OR (documents.extraction_status = 'extracted' AND NOT EXISTS (
                 SELECT 1 FROM file_space_search_chunks chunks
                 WHERE chunks.file_id = documents.file_id
                   AND chunks.document_indexed_at = documents.indexed_at
                   AND EXISTS (
                     SELECT 1 FROM file_space_semantic_embeddings embeddings
                     WHERE embeddings.chunk_id = chunks.id
                       AND embeddings.model_id = ?1
                   )
               ))
            THEN 1 ELSE 0 END AS needs_indexing
     FROM file_space_search_documents documents
     LEFT JOIN file_space_index_jobs jobs ON jobs.file_id = documents.file_id
     WHERE documents.extraction_status IN ('extracted', 'empty', 'unsupported')
       AND (?2 IS NULL OR documents.file_id > ?2)
     ORDER BY documents.file_id
     LIMIT ?3";

#[derive(Clone, Copy)]
struct ModelFile {
    remote_path: &'static str,
    local_name: &'static str,
    size_bytes: u64,
    sha256: &'static str,
}

const MODEL_FILES: [ModelFile; 5] = [
    ModelFile {
        remote_path: "onnx/model_quantized.onnx",
        local_name: "model.onnx",
        size_bytes: 118_308_185,
        sha256: "f80102d3f2a1229f387d3c81909990d8945513e347b0eab049f7de3c6f98c193",
    },
    ModelFile {
        remote_path: "tokenizer.json",
        local_name: "tokenizer.json",
        size_bytes: 17_082_730,
        sha256: "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39",
    },
    ModelFile {
        remote_path: "config.json",
        local_name: "config.json",
        size_bytes: 658,
        sha256: "cb99455288675345e1a4f411438d5d0adbba5fbd3a67ea4fb03c015433b996c1",
    },
    ModelFile {
        remote_path: "special_tokens_map.json",
        local_name: "special_tokens_map.json",
        size_bytes: 167,
        sha256: "d05497f1da52c5e09554c0cd874037a083e1dc1b9cfd48034d1c717f1afc07a7",
    },
    ModelFile {
        remote_path: "tokenizer_config.json",
        local_name: "tokenizer_config.json",
        size_bytes: 443,
        sha256: "a1d6bc8734a6f635dc158508bef000f8e2e5a759c7d92f984b2c86e5ff53425b",
    },
];

const MODEL_SOURCES: [&str; 2] = [
    "https://hf-mirror.com/Xenova/multilingual-e5-small/resolve",
    "https://huggingface.co/Xenova/multilingual-e5-small/resolve",
];

fn model_total_bytes() -> u64 {
    MODEL_FILES.iter().map(|file| file.size_bytes).sum()
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn semantic_index_cooldown(work_duration: Duration) -> Duration {
    work_duration
        .saturating_mul(SEMANTIC_COOLDOWN_MULTIPLIER)
        .clamp(SEMANTIC_MIN_COOLDOWN, SEMANTIC_MAX_COOLDOWN)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticSearchStatus {
    pub state: String,
    pub installed: bool,
    pub model_id: String,
    pub model_name: String,
    pub dimensions: usize,
    pub downloaded_bytes: u64,
    pub total_download_bytes: u64,
    pub current_file: Option<String>,
    pub indexed_files: i64,
    pub total_files: i64,
    pub pending_files: i64,
    pub failed_files: i64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstalledModelManifest {
    model_id: String,
    model_name: String,
    revision: String,
    dimensions: usize,
    size_bytes: u64,
    installed_at: i64,
}

#[derive(Debug, Clone)]
struct RuntimeProgress {
    phase: String,
    downloaded_bytes: u64,
    current_file: Option<String>,
    error: Option<String>,
}

impl Default for RuntimeProgress {
    fn default() -> Self {
        Self {
            phase: "notInstalled".to_owned(),
            downloaded_bytes: 0,
            current_file: None,
            error: None,
        }
    }
}

pub struct SemanticSearchRuntime {
    model_directory: PathBuf,
    model: Mutex<Option<TextEmbedding>>,
    progress: Mutex<RuntimeProgress>,
    download_running: AtomicBool,
    cancel_download: AtomicBool,
    index_generation: AtomicU64,
}

impl SemanticSearchRuntime {
    pub fn new(app_data_directory: &Path) -> Self {
        Self {
            model_directory: app_data_directory.join(SEMANTIC_MODEL_DIRECTORY),
            model: Mutex::new(None),
            progress: Mutex::new(RuntimeProgress::default()),
            download_running: AtomicBool::new(false),
            cancel_download: AtomicBool::new(false),
            index_generation: AtomicU64::new(0),
        }
    }

    fn is_installed(&self) -> bool {
        self.model_directory.join(SEMANTIC_MODEL_MANIFEST).is_file()
            && MODEL_FILES
                .iter()
                .all(|file| self.model_directory.join(file.local_name).is_file())
    }

    fn set_progress(
        &self,
        phase: &str,
        downloaded_bytes: u64,
        current_file: Option<String>,
        error: Option<String>,
    ) {
        if let Ok(mut progress) = self.progress.lock() {
            progress.phase = phase.to_owned();
            progress.downloaded_bytes = downloaded_bytes;
            progress.current_file = current_file;
            progress.error = error;
        }
    }

    fn set_model(&self, model: Option<TextEmbedding>) -> Result<(), String> {
        let mut current = self
            .model
            .lock()
            .map_err(|_| "Unable to access the local semantic model".to_owned())?;
        *current = model;
        Ok(())
    }

    fn embed(&self, text: &str) -> Result<Option<Vec<f32>>, String> {
        let mut model = self
            .model
            .lock()
            .map_err(|_| "Unable to access the local semantic model".to_owned())?;
        let Some(model) = model.as_mut() else {
            return Ok(None);
        };
        let mut embeddings = model
            .embed([text], None)
            .map_err(|error| format!("Unable to generate a local semantic embedding: {error}"))?;
        let embedding = embeddings
            .pop()
            .ok_or_else(|| "The local semantic model returned no embedding".to_owned())?;
        if embedding.len() != SEMANTIC_MODEL_DIMENSIONS {
            return Err(format!(
                "The local semantic model returned {} dimensions instead of {}",
                embedding.len(),
                SEMANTIC_MODEL_DIMENSIONS
            ));
        }
        Ok(Some(embedding))
    }
}

#[derive(Debug)]
struct SearchChunk {
    body_text: String,
    start_character: usize,
    end_character: usize,
}

#[derive(Debug)]
struct SearchDocument {
    file_id: String,
    file_name: String,
    body_text: String,
    extraction_status: String,
    extraction_error: Option<String>,
    tag_text: String,
    task_text: String,
    cell_text: String,
    version_id: Option<String>,
    indexed_at: i64,
}

fn split_search_text(text: &str) -> Vec<SearchChunk> {
    let characters = text.chars().collect::<Vec<_>>();
    if characters.is_empty() {
        return Vec::new();
    }
    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < characters.len() {
        let maximum_end = (start + SEMANTIC_CHUNK_CHARACTERS).min(characters.len());
        let minimum_end = (start + SEMANTIC_CHUNK_CHARACTERS / 2).min(maximum_end);
        let mut end = maximum_end;
        if maximum_end < characters.len() {
            if let Some(relative) = characters[minimum_end..maximum_end]
                .iter()
                .rposition(|character| *character == '\n')
            {
                end = minimum_end + relative + 1;
            }
        }
        let body_text = characters[start..end].iter().collect::<String>();
        chunks.push(SearchChunk {
            body_text,
            start_character: start,
            end_character: end,
        });
        if end >= characters.len() {
            break;
        }
        let next = end.saturating_sub(SEMANTIC_CHUNK_OVERLAP);
        start = next.max(start + 1);
    }
    chunks
}

fn embedding_input(document: &SearchDocument, chunk: &SearchChunk) -> String {
    let mut metadata = vec![format!("File: {}", document.file_name)];
    if !document.tag_text.trim().is_empty() {
        metadata.push(format!(
            "Tags: {}",
            document.tag_text.replace('\u{1f}', ", ")
        ));
    }
    if !document.task_text.trim().is_empty() {
        metadata.push(format!("Tasks: {}", document.task_text));
    }
    if !document.cell_text.trim().is_empty() {
        metadata.push(format!("Cells: {}", document.cell_text));
    }
    format!("passage: {}\n{}", metadata.join("\n"), chunk.body_text)
}

fn content_hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn vector_to_blob(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vector.len() * std::mem::size_of::<f32>());
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn vector_from_blob(bytes: &[u8], dimensions: usize) -> Option<Vec<f32>> {
    if bytes.len() != dimensions.checked_mul(std::mem::size_of::<f32>())? {
        return None;
    }
    Some(
        bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect(),
    )
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut left_norm = 0.0f32;
    let mut right_norm = 0.0f32;
    for (left, right) in left.iter().zip(right) {
        dot += left * right;
        left_norm += left * left;
        right_norm += right * right;
    }
    let denominator = left_norm.sqrt() * right_norm.sqrt();
    if denominator <= f32::EPSILON {
        0.0
    } else {
        dot / denominator
    }
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("Unable to open {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("Unable to read {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn verify_model_file(directory: &Path, descriptor: ModelFile) -> Result<(), String> {
    let path = directory.join(descriptor.local_name);
    let metadata = fs::metadata(&path)
        .map_err(|error| format!("Unable to inspect {}: {error}", descriptor.local_name))?;
    if metadata.len() != descriptor.size_bytes {
        return Err(format!(
            "{} has an unexpected size ({} bytes)",
            descriptor.local_name,
            metadata.len()
        ));
    }
    let actual = sha256_file(&path)?;
    if actual != descriptor.sha256 {
        return Err(format!(
            "{} failed its integrity check",
            descriptor.local_name
        ));
    }
    Ok(())
}

fn load_model(directory: &Path) -> Result<TextEmbedding, String> {
    for descriptor in MODEL_FILES {
        verify_model_file(directory, descriptor)?;
    }
    let read = |name: &str| {
        fs::read(directory.join(name))
            .map_err(|error| format!("Unable to read the local model file {name}: {error}"))
    };
    let tokenizer_files = TokenizerFiles {
        tokenizer_file: read("tokenizer.json")?,
        config_file: read("config.json")?,
        special_tokens_map_file: read("special_tokens_map.json")?,
        tokenizer_config_file: read("tokenizer_config.json")?,
    };
    let model = UserDefinedEmbeddingModel::new(read("model.onnx")?, tokenizer_files)
        .with_pooling(Pooling::Mean)
        .with_quantization(QuantizationMode::Dynamic);
    TextEmbedding::try_new_from_user_defined(
        model,
        InitOptionsUserDefined::new()
            .with_max_length(512)
            .with_intra_threads(SEMANTIC_INTRA_THREADS),
    )
    .map_err(|error| format!("Unable to load the local semantic model: {error}"))
}

fn model_url(source: &str, descriptor: ModelFile) -> String {
    format!(
        "{source}/{SEMANTIC_MODEL_REVISION}/{}",
        descriptor.remote_path
    )
}

fn download_model_file(
    app: &tauri::AppHandle,
    database: &Database,
    runtime: &SemanticSearchRuntime,
    client: &Client,
    staging_directory: &Path,
    descriptor: ModelFile,
    completed_bytes: u64,
) -> Result<(), String> {
    let final_path = staging_directory.join(descriptor.local_name);
    let partial_path = staging_directory.join(format!("{}.part", descriptor.local_name));
    let mut last_error = None;
    for source in MODEL_SOURCES {
        if runtime.cancel_download.load(AtomicOrdering::Relaxed) {
            return Err("The model download was cancelled".to_owned());
        }
        let _ = fs::remove_file(&partial_path);
        let attempt = (|| -> Result<(), String> {
            let mut response = client
                .get(model_url(source, descriptor))
                .send()
                .and_then(|response| response.error_for_status())
                .map_err(|error| {
                    format!("Unable to download {}: {error}", descriptor.local_name)
                })?;
            let mut output = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&partial_path)
                .map_err(|error| format!("Unable to create the temporary model file: {error}"))?;
            let mut downloaded = 0u64;
            let mut buffer = [0u8; 128 * 1024];
            loop {
                if runtime.cancel_download.load(AtomicOrdering::Relaxed) {
                    return Err("The model download was cancelled".to_owned());
                }
                let read = response
                    .read(&mut buffer)
                    .map_err(|error| format!("Unable to read the model download: {error}"))?;
                if read == 0 {
                    break;
                }
                output
                    .write_all(&buffer[..read])
                    .map_err(|error| format!("Unable to save the local model: {error}"))?;
                downloaded += read as u64;
                runtime.set_progress(
                    "downloading",
                    completed_bytes + downloaded,
                    Some(descriptor.local_name.to_owned()),
                    None,
                );
                if downloaded % (2 * 1024 * 1024) < buffer.len() as u64 {
                    emit_status(app, database, runtime);
                }
            }
            output
                .sync_all()
                .map_err(|error| format!("Unable to finish saving the local model: {error}"))?;
            if downloaded != descriptor.size_bytes {
                return Err(format!(
                    "{} downloaded {} bytes instead of {}",
                    descriptor.local_name, downloaded, descriptor.size_bytes
                ));
            }
            let actual = sha256_file(&partial_path)?;
            if actual != descriptor.sha256 {
                return Err(format!(
                    "{} failed its integrity check",
                    descriptor.local_name
                ));
            }
            fs::rename(&partial_path, &final_path)
                .map_err(|error| format!("Unable to install {}: {error}", descriptor.local_name))?;
            Ok(())
        })();
        match attempt {
            Ok(()) => return Ok(()),
            Err(error) if error == "The model download was cancelled" => {
                let _ = fs::remove_file(&partial_path);
                return Err(error);
            }
            Err(error) => {
                let _ = fs::remove_file(&partial_path);
                last_error = Some(error);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| "Unable to download the local semantic model".to_owned()))
}

fn install_model(
    app: &tauri::AppHandle,
    database: &Database,
    runtime: &SemanticSearchRuntime,
) -> Result<TextEmbedding, String> {
    let parent = runtime
        .model_directory
        .parent()
        .ok_or_else(|| "Unable to resolve the local model directory".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Unable to create the local model directory: {error}"))?;
    let staging_directory = parent.join(format!(".installing-{}", Uuid::new_v4()));
    fs::create_dir(&staging_directory)
        .map_err(|error| format!("Unable to prepare the local model download: {error}"))?;
    let result = (|| {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(300))
            .user_agent("Lume-Trace/0.1 semantic-model-installer")
            .build()
            .map_err(|error| format!("Unable to initialize the model downloader: {error}"))?;
        let mut completed = 0u64;
        for descriptor in MODEL_FILES {
            download_model_file(
                app,
                database,
                runtime,
                &client,
                &staging_directory,
                descriptor,
                completed,
            )?;
            completed += descriptor.size_bytes;
        }
        runtime.set_progress("validating", completed, None, None);
        emit_status(app, database, runtime);
        let mut model = load_model(&staging_directory)?;
        let probe = model
            .embed(["query: Lume Trace local semantic search"], None)
            .map_err(|error| format!("The local semantic model could not be verified: {error}"))?;
        if probe.first().map(Vec::len) != Some(SEMANTIC_MODEL_DIMENSIONS) {
            return Err("The local semantic model returned an unexpected result".to_owned());
        }
        let manifest = InstalledModelManifest {
            model_id: SEMANTIC_MODEL_ID.to_owned(),
            model_name: SEMANTIC_MODEL_NAME.to_owned(),
            revision: SEMANTIC_MODEL_REVISION.to_owned(),
            dimensions: SEMANTIC_MODEL_DIMENSIONS,
            size_bytes: model_total_bytes(),
            installed_at: now_millis(),
        };
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|error| format!("Unable to create the local model manifest: {error}"))?;
        fs::write(
            staging_directory.join(SEMANTIC_MODEL_MANIFEST),
            manifest_bytes,
        )
        .map_err(|error| format!("Unable to save the local model manifest: {error}"))?;
        if runtime.model_directory.exists() {
            fs::remove_dir_all(&runtime.model_directory)
                .map_err(|error| format!("Unable to replace the local model: {error}"))?;
        }
        fs::rename(&staging_directory, &runtime.model_directory)
            .map_err(|error| format!("Unable to finish installing the local model: {error}"))?;
        Ok(model)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging_directory);
    }
    result
}

fn save_installed_model_setting(database: &Database, installed: bool) -> Result<(), String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    if installed {
        connection
            .execute(
                "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                params![SEMANTIC_MODEL_SETTING, SEMANTIC_MODEL_ID, now_millis()],
            )
            .map_err(|error| format!("Unable to save the semantic model setting: {error}"))?;
    } else {
        connection
            .execute(
                "DELETE FROM app_settings WHERE key = ?1",
                [SEMANTIC_MODEL_SETTING],
            )
            .map_err(|error| format!("Unable to remove the semantic model setting: {error}"))?;
    }
    Ok(())
}

fn schedule_search_documents_in_transaction(
    transaction: &Transaction<'_>,
    file_ids: &[String],
    requested_at: i64,
) -> Result<(), String> {
    for file_id in file_ids {
        transaction
            .execute(
                "INSERT INTO file_space_index_jobs
                 (file_id, requested_document_indexed_at, status, retry_count, error,
                  requested_at, started_at, completed_at)
                 SELECT file_id, indexed_at, 'pending', 0, NULL, ?2, NULL, NULL
                 FROM file_space_search_documents
                 WHERE file_id = ?1
                   AND extraction_status IN ('extracted', 'empty', 'unsupported')
                 ON CONFLICT(file_id) DO UPDATE SET
                   requested_document_indexed_at = excluded.requested_document_indexed_at,
                   status = 'pending', retry_count = 0, error = NULL,
                   requested_at = excluded.requested_at, started_at = NULL, completed_at = NULL",
                params![file_id, requested_at],
            )
            .map_err(|error| format!("Unable to queue semantic indexing: {error}"))?;
    }
    Ok(())
}

pub fn schedule_search_documents(database: &Database, file_ids: &[String]) -> Result<(), String> {
    if file_ids.is_empty() {
        return Ok(());
    }
    let mut connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Unable to start semantic indexing: {error}"))?;
    schedule_search_documents_in_transaction(&transaction, file_ids, now_millis())?;
    transaction
        .commit()
        .map_err(|error| format!("Unable to queue semantic indexing: {error}"))
}

#[derive(Debug, PartialEq, Eq)]
struct OutdatedDocumentBatch {
    last_file_id: Option<String>,
    scanned: usize,
    scheduled: usize,
}

fn schedule_outdated_document_batch(
    database: &Database,
    after_file_id: Option<&str>,
    batch_size: usize,
) -> Result<OutdatedDocumentBatch, String> {
    let mut connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Unable to inspect the semantic index: {error}"))?;
    let rows = {
        let mut statement = transaction
            .prepare(OUTDATED_DOCUMENT_BATCH_QUERY)
            .map_err(|error| format!("Unable to inspect the semantic index: {error}"))?;
        statement
            .query_map(
                params![SEMANTIC_MODEL_ID, after_file_id, batch_size.max(1) as i64],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? != 0)),
            )
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|error| format!("Unable to inspect the semantic index: {error}"))?
    };
    let file_ids = rows
        .iter()
        .filter(|(_, needs_indexing)| *needs_indexing)
        .map(|(file_id, _)| file_id.clone())
        .collect::<Vec<_>>();
    schedule_search_documents_in_transaction(&transaction, &file_ids, now_millis())?;
    transaction
        .commit()
        .map_err(|error| format!("Unable to queue semantic indexing: {error}"))?;
    Ok(OutdatedDocumentBatch {
        last_file_id: rows.last().map(|(file_id, _)| file_id.clone()),
        scanned: rows.len(),
        scheduled: file_ids.len(),
    })
}

pub(crate) fn schedule_outdated_documents(database: &Database) -> Result<(), String> {
    let mut after_file_id = None;
    loop {
        let batch = schedule_outdated_document_batch(
            database,
            after_file_id.as_deref(),
            OUTDATED_DOCUMENT_BATCH_SIZE,
        )?;
        if batch.scanned == 0 {
            return Ok(());
        }
        after_file_id = batch.last_file_id;
        if batch.scanned < OUTDATED_DOCUMENT_BATCH_SIZE {
            return Ok(());
        }
        std::thread::sleep(OUTDATED_DOCUMENT_BATCH_PAUSE);
    }
}

fn reset_interrupted_jobs(database: &Database) -> Result<(), String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    connection
        .execute(
            "UPDATE file_space_index_jobs
             SET status = 'pending', started_at = NULL, completed_at = NULL,
                 error = 'Indexing was interrupted and will resume'
             WHERE status IN ('extracting', 'embedding')",
            [],
        )
        .map_err(|error| format!("Unable to recover semantic indexing: {error}"))?;
    Ok(())
}

fn read_next_document(database: &Database) -> Result<Option<SearchDocument>, String> {
    let mut connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Unable to begin semantic indexing: {error}"))?;
    let file_id = transaction
        .query_row(
            "SELECT jobs.file_id FROM file_space_index_jobs jobs
             JOIN file_space_search_documents documents ON documents.file_id = jobs.file_id
             WHERE jobs.status = 'pending'
               AND documents.extraction_status IN ('extracted', 'empty', 'unsupported')
             ORDER BY jobs.requested_at, jobs.file_id LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("Unable to select semantic indexing work: {error}"))?;
    let Some(file_id) = file_id else {
        transaction
            .commit()
            .map_err(|error| format!("Unable to finish semantic indexing: {error}"))?;
        return Ok(None);
    };
    transaction
        .execute(
            "UPDATE file_space_index_jobs
             SET status = 'extracting', started_at = ?2, completed_at = NULL, error = NULL
             WHERE file_id = ?1 AND status = 'pending'",
            params![file_id, now_millis()],
        )
        .map_err(|error| format!("Unable to start semantic indexing: {error}"))?;
    let document = transaction
        .query_row(
            "SELECT documents.file_id, documents.file_name, documents.body_text,
                    documents.extraction_status, documents.extraction_error,
                    documents.tag_text, documents.task_text, documents.cell_text,
                    artifacts.current_version_id, documents.indexed_at
             FROM file_space_search_documents documents
             LEFT JOIN file_space_artifacts artifacts ON artifacts.file_id = documents.file_id
             WHERE documents.file_id = ?1",
            [&file_id],
            |row| {
                Ok(SearchDocument {
                    file_id: row.get(0)?,
                    file_name: row.get(1)?,
                    body_text: row.get(2)?,
                    extraction_status: row.get(3)?,
                    extraction_error: row.get(4)?,
                    tag_text: row.get(5)?,
                    task_text: row.get(6)?,
                    cell_text: row.get(7)?,
                    version_id: row.get(8)?,
                    indexed_at: row.get(9)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("Unable to read a file for semantic indexing: {error}"))?;
    if document.is_none() {
        transaction
            .execute(
                "DELETE FROM file_space_index_jobs WHERE file_id = ?1",
                [&file_id],
            )
            .map_err(|error| format!("Unable to remove stale semantic indexing work: {error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("Unable to start semantic indexing: {error}"))?;
    Ok(document)
}

fn mark_document_failed(database: &Database, file_id: &str, error: &str) {
    if let Ok(connection) = database.0.lock() {
        let _ = connection.execute(
            "UPDATE file_space_index_jobs
             SET status = 'failed', retry_count = retry_count + 1,
                 error = ?2, completed_at = ?3
             WHERE file_id = ?1",
            params![file_id, error, now_millis()],
        );
    }
}

fn reschedule_document(database: &Database, file_id: &str) {
    if let Ok(connection) = database.0.lock() {
        let _ = connection.execute(
            "UPDATE file_space_index_jobs
             SET status = 'pending', retry_count = 0, error = NULL,
                 requested_at = ?2, started_at = NULL, completed_at = NULL
             WHERE file_id = ?1",
            params![file_id, now_millis()],
        );
    }
}

fn process_next_document(
    database: &Database,
    runtime: &SemanticSearchRuntime,
) -> Result<bool, String> {
    let Some(document) = read_next_document(database)? else {
        return Ok(false);
    };
    let index_generation = runtime.index_generation.load(AtomicOrdering::SeqCst);
    let result = (|| -> Result<(), String> {
        if document.extraction_status == "failed" {
            return Err(document
                .extraction_error
                .clone()
                .unwrap_or_else(|| "File content extraction failed".to_owned()));
        }
        let chunks = split_search_text(document.body_text.trim());
        {
            let connection = database
                .0
                .lock()
                .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
            connection
                .execute(
                    "UPDATE file_space_index_jobs SET status = 'embedding' WHERE file_id = ?1",
                    [&document.file_id],
                )
                .map_err(|error| format!("Unable to update semantic indexing: {error}"))?;
        }
        let mut embedded_chunks = Vec::with_capacity(chunks.len());
        for chunk in chunks {
            let input = embedding_input(&document, &chunk);
            let vector = runtime
                .embed(&input)?
                .ok_or_else(|| "The local semantic model is not available".to_owned())?;
            embedded_chunks.push((chunk, content_hash(&input), vector));
        }
        let mut connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
        let transaction = connection
            .transaction()
            .map_err(|error| format!("Unable to save semantic indexing: {error}"))?;
        if runtime.index_generation.load(AtomicOrdering::SeqCst) != index_generation
            || !runtime.is_installed()
        {
            transaction
                .execute(
                    "UPDATE file_space_index_jobs
                     SET status = 'pending', retry_count = 0, error = NULL,
                         requested_at = ?2, started_at = NULL, completed_at = NULL
                     WHERE file_id = ?1",
                    params![document.file_id, now_millis()],
                )
                .map_err(|error| format!("Unable to pause semantic indexing: {error}"))?;
            transaction
                .commit()
                .map_err(|error| format!("Unable to pause semantic indexing: {error}"))?;
            return Ok(());
        }
        let current_indexed_at = transaction
            .query_row(
                "SELECT indexed_at FROM file_space_search_documents WHERE file_id = ?1",
                [&document.file_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| format!("Unable to validate semantic indexing: {error}"))?;
        if current_indexed_at != Some(document.indexed_at) {
            transaction
                .execute(
                    "UPDATE file_space_index_jobs
                     SET requested_document_indexed_at = COALESCE(?2, requested_document_indexed_at),
                         status = 'pending', retry_count = 0, error = NULL,
                         requested_at = ?3, started_at = NULL, completed_at = NULL
                     WHERE file_id = ?1",
                    params![document.file_id, current_indexed_at, now_millis()],
                )
                .map_err(|error| format!("Unable to reschedule semantic indexing: {error}"))?;
            transaction
                .commit()
                .map_err(|error| format!("Unable to reschedule semantic indexing: {error}"))?;
            return Ok(());
        }
        transaction
            .execute(
                "DELETE FROM file_space_search_chunks WHERE file_id = ?1",
                [&document.file_id],
            )
            .map_err(|error| format!("Unable to replace semantic chunks: {error}"))?;
        let created_at = now_millis();
        for (ordinal, (chunk, hash, vector)) in embedded_chunks.into_iter().enumerate() {
            let chunk_id = Uuid::new_v4().to_string();
            transaction
                .execute(
                    "INSERT INTO file_space_search_chunks
                     (id, file_id, version_id, document_indexed_at, content_hash, ordinal,
                      start_character, end_character, body_text, character_count, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        chunk_id,
                        document.file_id,
                        document.version_id,
                        document.indexed_at,
                        hash,
                        ordinal as i64,
                        chunk.start_character as i64,
                        chunk.end_character as i64,
                        chunk.body_text,
                        chunk.end_character.saturating_sub(chunk.start_character) as i64,
                        created_at,
                    ],
                )
                .map_err(|error| format!("Unable to save a semantic chunk: {error}"))?;
            transaction
                .execute(
                    "INSERT INTO file_space_semantic_embeddings
                     (chunk_id, model_id, dimensions, vector, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        chunk_id,
                        SEMANTIC_MODEL_ID,
                        SEMANTIC_MODEL_DIMENSIONS as i64,
                        vector_to_blob(&vector),
                        created_at,
                    ],
                )
                .map_err(|error| format!("Unable to save a semantic embedding: {error}"))?;
        }
        transaction
            .execute(
                "UPDATE file_space_index_jobs
                 SET status = 'ready', error = NULL, completed_at = ?2
                 WHERE file_id = ?1",
                params![document.file_id, now_millis()],
            )
            .map_err(|error| format!("Unable to finish semantic indexing: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("Unable to finish semantic indexing: {error}"))
    })();
    if let Err(error) = &result {
        if runtime.index_generation.load(AtomicOrdering::SeqCst) != index_generation
            || !runtime.is_installed()
        {
            reschedule_document(database, &document.file_id);
        } else {
            mark_document_failed(database, &document.file_id, error);
        }
    }
    result.map(|_| true)
}

fn ensure_model_loaded(runtime: &SemanticSearchRuntime) -> Result<(), String> {
    let has_model = runtime
        .model
        .lock()
        .map_err(|_| "Unable to access the local semantic model".to_owned())?
        .is_some();
    if has_model {
        return Ok(());
    }
    runtime.set_progress("loading", model_total_bytes(), None, None);
    let model = load_model(&runtime.model_directory)?;
    runtime.set_model(Some(model))?;
    runtime.set_progress("installed", model_total_bytes(), None, None);
    Ok(())
}

pub(crate) fn status_record(
    database: &Database,
    runtime: &SemanticSearchRuntime,
) -> SemanticSearchStatus {
    let progress = runtime
        .progress
        .lock()
        .map(|value| value.clone())
        .unwrap_or_default();
    let counts = database
        .0
        .lock()
        .ok()
        .and_then(|connection| {
            connection
                .query_row(
                    "SELECT
                       COALESCE(SUM(CASE WHEN documents.extraction_status IN ('extracted', 'empty', 'unsupported') THEN 1 ELSE 0 END), 0),
                       COALESCE(SUM(CASE WHEN documents.extraction_status IN ('extracted', 'empty', 'unsupported') AND jobs.status = 'ready' THEN 1 ELSE 0 END), 0),
                       COALESCE(SUM(CASE WHEN documents.extraction_status IN ('extracted', 'empty', 'unsupported') AND jobs.status IN ('pending', 'extracting', 'embedding') THEN 1 ELSE 0 END), 0),
                       COALESCE(SUM(CASE WHEN documents.extraction_status IN ('extracted', 'empty', 'unsupported') AND jobs.status = 'failed' THEN 1 ELSE 0 END), 0),
                       (SELECT failed_jobs.error
                        FROM file_space_index_jobs failed_jobs
                        JOIN file_space_search_documents failed_documents
                          ON failed_documents.file_id = failed_jobs.file_id
                        WHERE failed_jobs.status = 'failed' AND failed_jobs.error IS NOT NULL
                          AND failed_documents.extraction_status IN ('extracted', 'empty', 'unsupported')
                        ORDER BY failed_jobs.completed_at DESC LIMIT 1),
                       (SELECT documents.file_name
                        FROM file_space_index_jobs active_jobs
                        JOIN file_space_search_documents documents
                          ON documents.file_id = active_jobs.file_id
                        WHERE active_jobs.status IN ('extracting', 'embedding')
                        ORDER BY active_jobs.started_at, active_jobs.file_id LIMIT 1)
                     FROM file_space_search_documents documents
                     LEFT JOIN file_space_index_jobs jobs ON jobs.file_id = documents.file_id",
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
                .ok()
        })
        .unwrap_or((0, 0, 0, 0, None, None));
    let installed = runtime.is_installed();
    let state = if runtime.download_running.load(AtomicOrdering::Relaxed) {
        if progress.phase == "validating" {
            "validating"
        } else {
            "downloading"
        }
    } else if !installed {
        if progress.phase == "failed" {
            "failed"
        } else {
            "notInstalled"
        }
    } else if progress.phase == "failed" {
        "failed"
    } else if counts.3 > 0 && counts.2 == 0 {
        "failed"
    } else if counts.2 > 0 || counts.1 < counts.0 {
        "indexing"
    } else {
        "ready"
    };
    SemanticSearchStatus {
        state: state.to_owned(),
        installed,
        model_id: SEMANTIC_MODEL_ID.to_owned(),
        model_name: SEMANTIC_MODEL_NAME.to_owned(),
        dimensions: SEMANTIC_MODEL_DIMENSIONS,
        downloaded_bytes: if installed {
            model_total_bytes()
        } else {
            progress.downloaded_bytes.min(model_total_bytes())
        },
        total_download_bytes: model_total_bytes(),
        current_file: counts.5.or(progress.current_file),
        indexed_files: counts.1,
        total_files: counts.0,
        pending_files: counts.2,
        failed_files: counts.3,
        error: progress.error.or(counts.4),
    }
}

fn emit_status(app: &tauri::AppHandle, database: &Database, runtime: &SemanticSearchRuntime) {
    let _ = app.emit(SEMANTIC_EVENT, status_record(database, runtime));
}

pub fn start_semantic_indexer(app: tauri::AppHandle) -> Result<(), String> {
    std::thread::Builder::new()
        .name("lumetrace-semantic-indexer".to_owned())
        .spawn(move || {
            let mut active_workspace_id: Option<String> = None;
            let mut reconciled_workspace_id: Option<String> = None;
            let mut last_status_emit = Instant::now()
                .checked_sub(SEMANTIC_STATUS_INTERVAL)
                .unwrap_or_else(Instant::now);
            loop {
                let background = app.state::<FileSpaceBackgroundRuntime>();
                if background.is_paused() {
                    std::thread::sleep(Duration::from_millis(250));
                    continue;
                }
                let runtime = app.state::<SemanticSearchRuntime>();
                if !runtime.is_installed() || runtime.download_running.load(AtomicOrdering::Relaxed)
                {
                    std::thread::sleep(SEMANTIC_IDLE_PAUSE);
                    continue;
                }
                let database = app.state::<Database>();
                let workspace_id = match database.location() {
                    Ok(location) => location.workspace_id,
                    Err(error) => {
                        eprintln!("Unable to inspect the semantic index workspace: {error}");
                        std::thread::sleep(SEMANTIC_IDLE_PAUSE);
                        continue;
                    }
                };
                if active_workspace_id.as_deref() != Some(&workspace_id) {
                    if let Err(error) = reset_interrupted_jobs(database.inner()) {
                        eprintln!("Unable to recover semantic indexing: {error}");
                    }
                    active_workspace_id = Some(workspace_id.clone());
                }
                if let Err(error) = ensure_model_loaded(runtime.inner()) {
                    runtime.set_progress("failed", model_total_bytes(), None, Some(error.clone()));
                    emit_status(&app, database.inner(), runtime.inner());
                    eprintln!("Unable to load the Lume Trace semantic model: {error}");
                    std::thread::sleep(Duration::from_secs(5));
                    continue;
                }
                let work_started_at = Instant::now();
                let result = lock_file_space_operations().and_then(|_operation| {
                    process_next_document(database.inner(), runtime.inner())
                });
                let work_duration = work_started_at.elapsed();
                match result {
                    Ok(true) => {
                        if last_status_emit.elapsed() >= SEMANTIC_STATUS_INTERVAL {
                            emit_status(&app, database.inner(), runtime.inner());
                            last_status_emit = Instant::now();
                        }
                        std::thread::sleep(semantic_index_cooldown(work_duration));
                    }
                    Ok(false) => {
                        if reconciled_workspace_id.as_deref() != Some(&workspace_id) {
                            if let Err(error) = schedule_outdated_documents(database.inner()) {
                                eprintln!("Unable to reconcile semantic indexing: {error}");
                            }
                            reconciled_workspace_id = Some(workspace_id);
                        }
                        if last_status_emit.elapsed() >= SEMANTIC_STATUS_INTERVAL {
                            emit_status(&app, database.inner(), runtime.inner());
                            last_status_emit = Instant::now();
                        }
                        std::thread::sleep(SEMANTIC_IDLE_PAUSE);
                    }
                    Err(error) => {
                        eprintln!("Unable to update the Lume Trace semantic index: {error}");
                        if last_status_emit.elapsed() >= SEMANTIC_STATUS_INTERVAL {
                            emit_status(&app, database.inner(), runtime.inner());
                            last_status_emit = Instant::now();
                        }
                        std::thread::sleep(SEMANTIC_IDLE_PAUSE);
                    }
                }
            }
        })
        .map_err(|error| format!("Unable to start semantic indexing: {error}"))?;
    Ok(())
}

fn semantic_candidate_query(candidate_count: usize) -> (String, usize, usize) {
    let candidate_values = (0..candidate_count)
        .map(|rank| format!("(?{}, {rank})", rank + 2))
        .collect::<Vec<_>>()
        .join(", ");
    let per_file_limit_parameter = candidate_count + 2;
    let total_limit_parameter = candidate_count + 3;
    let sql = format!(
        "WITH candidate_files(file_id, candidate_rank) AS (VALUES {candidate_values}),
         ranked_embeddings AS (
           SELECT chunks.file_id, embeddings.vector, embeddings.dimensions,
                  candidates.candidate_rank, chunks.ordinal,
                  ROW_NUMBER() OVER (
                    PARTITION BY chunks.file_id ORDER BY chunks.ordinal
                  ) AS chunk_rank
           FROM candidate_files candidates
           CROSS JOIN file_space_search_chunks chunks
             INDEXED BY idx_file_space_search_chunks_file
             ON chunks.file_id = candidates.file_id
           CROSS JOIN file_space_semantic_embeddings embeddings
             ON embeddings.chunk_id = chunks.id
           CROSS JOIN file_space_search_documents documents
             ON documents.file_id = chunks.file_id
            AND documents.indexed_at = chunks.document_indexed_at
            AND documents.extraction_status = 'extracted'
           CROSS JOIN files ON files.id = chunks.file_id
           WHERE embeddings.model_id = ?1
             AND files.trashed_at IS NULL AND files.storage_path IS NOT NULL
         )
         SELECT file_id, vector, dimensions
         FROM ranked_embeddings
         WHERE chunk_rank <= ?{per_file_limit_parameter}
         ORDER BY candidate_rank, ordinal
         LIMIT ?{total_limit_parameter}"
    );
    (sql, per_file_limit_parameter, total_limit_parameter)
}

pub fn semantic_file_ranks(
    database: &Database,
    runtime: &SemanticSearchRuntime,
    query: &str,
    candidate_file_ids: &[String],
) -> Result<Vec<(String, f32)>, String> {
    if !runtime.is_installed() || candidate_file_ids.is_empty() {
        return Ok(Vec::new());
    }
    let Some(query_vector) = runtime.embed(&format!("query: {}", query.trim()))? else {
        return Ok(Vec::new());
    };
    let candidate_file_ids = candidate_file_ids
        .iter()
        .take(SEMANTIC_CANDIDATE_FILE_LIMIT)
        .collect::<Vec<_>>();
    let (sql, _, _) = semantic_candidate_query(candidate_file_ids.len());
    let mut parameters = Vec::with_capacity(candidate_file_ids.len() + 3);
    parameters.push(Value::Text(SEMANTIC_MODEL_ID.to_owned()));
    parameters.extend(
        candidate_file_ids
            .iter()
            .map(|file_id| Value::Text((*file_id).clone())),
    );
    parameters.push(Value::Integer(SEMANTIC_CHUNKS_PER_FILE_LIMIT as i64));
    parameters.push(Value::Integer(SEMANTIC_VECTOR_SCAN_LIMIT as i64));
    let embeddings = {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
        let mut statement = connection
            .prepare(&sql)
            .map_err(|error| format!("Unable to prepare semantic search: {error}"))?;
        statement
            .query_map(params_from_iter(parameters), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|error| format!("Unable to read the semantic index: {error}"))?
    };
    let mut file_scores = HashMap::<String, f32>::new();
    for (file_id, blob, dimensions) in embeddings {
        if dimensions != SEMANTIC_MODEL_DIMENSIONS as i64 {
            continue;
        }
        let Some(vector) = vector_from_blob(&blob, SEMANTIC_MODEL_DIMENSIONS) else {
            continue;
        };
        let similarity = cosine_similarity(&query_vector, &vector);
        if !similarity.is_finite() || similarity < SEMANTIC_MIN_SIMILARITY {
            continue;
        }
        file_scores
            .entry(file_id)
            .and_modify(|score| *score = score.max(similarity))
            .or_insert(similarity);
    }
    let mut ranked = file_scores.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.0.cmp(&right.0))
    });
    ranked.truncate(SEMANTIC_RESULT_LIMIT);
    Ok(ranked)
}

#[derive(Debug, Clone, PartialEq)]
pub struct HybridSearchMatch {
    pub file_id: String,
    pub lexical_match: bool,
    pub semantic_similarity: Option<f32>,
}

pub fn merge_hybrid_matches(
    lexical: Vec<String>,
    semantic: Vec<(String, f32)>,
) -> Vec<HybridSearchMatch> {
    let mut matches = HashMap::<String, (f32, bool, Option<f32>)>::new();
    for (rank, file_id) in lexical.iter().enumerate() {
        let entry = matches.entry(file_id.clone()).or_default();
        entry.0 += 0.62 / (60.0 + rank as f32 + 1.0);
        entry.1 = true;
    }
    for (rank, (file_id, similarity)) in semantic.iter().enumerate() {
        let confidence = ((*similarity - SEMANTIC_MIN_SIMILARITY)
            / (1.0 - SEMANTIC_MIN_SIMILARITY))
            .clamp(0.0, 1.0);
        let entry = matches.entry(file_id.clone()).or_default();
        entry.0 += (0.28 + confidence * 0.10) / (60.0 + rank as f32 + 1.0);
        entry.2 = Some(
            entry
                .2
                .map(|current| current.max(*similarity))
                .unwrap_or(*similarity),
        );
    }
    let mut ranked = matches.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|(left_id, left), (right_id, right)| {
        right
            .0
            .partial_cmp(&left.0)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left_id.cmp(right_id))
    });
    ranked
        .into_iter()
        .map(
            |(file_id, (_, lexical_match, semantic_similarity))| HybridSearchMatch {
                file_id,
                lexical_match,
                semantic_similarity,
            },
        )
        .collect()
}

const AI_CONTEXT_RESULT_LIMIT: usize = 8;
const AI_CONTEXT_PER_FILE_LIMIT: usize = 3;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AiContextChunk {
    pub file_id: String,
    pub file_name: String,
    pub relative_path: String,
    pub version_id: Option<String>,
    pub version_number: Option<i64>,
    pub body_text: String,
    pub lexical_match: bool,
    pub semantic_similarity: Option<f32>,
}

#[derive(Debug)]
struct RankedAiContextChunk {
    chunk: AiContextChunk,
    score: f32,
    candidate_rank: usize,
    ordinal: i64,
}

type StoredAiChunk = (
    String,
    String,
    String,
    Option<String>,
    Option<i64>,
    i64,
    String,
    Option<Vec<u8>>,
    Option<i64>,
);

fn lexical_terms(query: &str) -> Vec<String> {
    query
        .to_lowercase()
        .split(|character: char| character.is_whitespace() || character.is_ascii_punctuation())
        .filter(|term| term.chars().count() >= 2)
        .map(str::to_owned)
        .collect()
}

fn lexical_relevance(value: &str, query: &str, terms: &[String]) -> f32 {
    let value = value.to_lowercase();
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return 0.0;
    }
    let exact = value.contains(&query) as u8 as f32;
    if terms.is_empty() {
        return exact;
    }
    let matches = terms
        .iter()
        .filter(|term| value.contains(term.as_str()))
        .count();
    exact.max(matches as f32 / terms.len() as f32)
}

fn stored_ai_chunks_for_file(
    database: &Database,
    file_id: &str,
) -> Result<Vec<StoredAiChunk>, String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let mut statement = connection
        .prepare(
            "SELECT chunks.id, documents.file_name, files.storage_path,
                    chunks.version_id, versions.version_number, chunks.ordinal,
                    chunks.body_text, embeddings.vector, embeddings.dimensions
             FROM file_space_search_chunks chunks
             JOIN file_space_search_documents documents ON documents.file_id = chunks.file_id
             JOIN files ON files.id = chunks.file_id
             LEFT JOIN file_space_artifact_versions versions ON versions.id = chunks.version_id
             LEFT JOIN file_space_semantic_embeddings embeddings
               ON embeddings.chunk_id = chunks.id AND embeddings.model_id = ?2
             WHERE chunks.file_id = ?1
               AND chunks.document_indexed_at = documents.indexed_at
               AND documents.extraction_status = 'extracted'
               AND files.trashed_at IS NULL AND files.storage_path IS NOT NULL
             ORDER BY chunks.ordinal
             LIMIT ?3",
        )
        .map_err(|error| format!("Unable to prepare AI context retrieval: {error}"))?;
    statement
        .query_map(
            params![
                file_id,
                SEMANTIC_MODEL_ID,
                AI_CONTEXT_STORED_CHUNK_LIMIT as i64
            ],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                ))
            },
        )
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .map_err(|error| format!("Unable to read AI context: {error}"))
}

fn lexical_needles(queries: &[String]) -> Vec<String> {
    let mut needles = Vec::new();
    let mut seen = HashSet::new();
    for query in queries {
        let query = query.trim().to_lowercase();
        if query.chars().count() >= 2 && seen.insert(query.clone()) {
            needles.push(query.clone());
        }
        for term in lexical_terms(&query) {
            if seen.insert(term.clone()) {
                needles.push(term);
            }
        }
        if needles.len() >= 16 {
            break;
        }
    }
    needles.truncate(16);
    needles
}

fn lexical_ai_chunks_for_file(
    database: &Database,
    file_id: &str,
    needles: &[String],
) -> Result<Vec<StoredAiChunk>, String> {
    if needles.is_empty() {
        return Ok(Vec::new());
    }
    let conditions = needles
        .iter()
        .enumerate()
        .map(|(index, _)| format!("instr(lower(chunks.body_text), ?{}) > 0", index + 3))
        .collect::<Vec<_>>();
    let score = conditions
        .iter()
        .enumerate()
        .map(|(index, condition)| {
            format!(
                "CASE WHEN {condition} THEN {} ELSE 0 END",
                needles.len() - index
            )
        })
        .collect::<Vec<_>>()
        .join(" + ");
    let limit_parameter = needles.len() + 3;
    let sql = format!(
        "SELECT chunks.id, documents.file_name, files.storage_path,
                chunks.version_id, versions.version_number, chunks.ordinal,
                chunks.body_text, embeddings.vector, embeddings.dimensions
         FROM file_space_search_chunks chunks
         JOIN file_space_search_documents documents ON documents.file_id = chunks.file_id
         JOIN files ON files.id = chunks.file_id
         LEFT JOIN file_space_artifact_versions versions ON versions.id = chunks.version_id
         LEFT JOIN file_space_semantic_embeddings embeddings
           ON embeddings.chunk_id = chunks.id AND embeddings.model_id = ?2
         WHERE chunks.file_id = ?1
           AND chunks.document_indexed_at = documents.indexed_at
           AND documents.extraction_status = 'extracted'
           AND files.trashed_at IS NULL AND files.storage_path IS NOT NULL
           AND ({})
         ORDER BY ({score}) DESC, chunks.ordinal
         LIMIT ?{limit_parameter}",
        conditions.join(" OR ")
    );
    let mut parameters = Vec::with_capacity(needles.len() + 3);
    parameters.push(Value::Text(file_id.to_owned()));
    parameters.push(Value::Text(SEMANTIC_MODEL_ID.to_owned()));
    parameters.extend(needles.iter().cloned().map(Value::Text));
    parameters.push(Value::Integer(AI_CONTEXT_LEXICAL_CHUNK_LIMIT as i64));
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| format!("Unable to prepare lexical AI context retrieval: {error}"))?;
    statement
        .query_map(params_from_iter(parameters), |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
            ))
        })
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .map_err(|error| format!("Unable to read lexical AI context: {error}"))
}

fn lexical_document_excerpt_for_file(
    database: &Database,
    file_id: &str,
    needles: &[String],
) -> Result<Option<StoredAiChunk>, String> {
    if needles.is_empty() {
        return Ok(None);
    }
    let expression = format!(
        "body_text : ({})",
        needles
            .iter()
            .map(|needle| format!("\"{}\"*", needle.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" OR ")
    );
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    connection
        .query_row(
            "SELECT documents.file_name, files.storage_path,
                    artifacts.current_version_id, versions.version_number,
                    snippet(file_space_search_fts, 1, '', '', ' … ', 64)
             FROM file_space_search_fts
             JOIN file_space_search_documents documents
               ON documents.rowid = file_space_search_fts.rowid
             JOIN files ON files.id = documents.file_id
             LEFT JOIN file_space_artifacts artifacts ON artifacts.file_id = documents.file_id
             LEFT JOIN file_space_artifact_versions versions
               ON versions.id = artifacts.current_version_id
             WHERE documents.file_id = ?1
               AND documents.extraction_status = 'extracted'
               AND files.trashed_at IS NULL AND files.storage_path IS NOT NULL
               AND file_space_search_fts MATCH ?2",
            params![file_id, expression],
            |row| {
                Ok((
                    format!("{file_id}:fts-snippet"),
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    -1,
                    row.get(4)?,
                    None,
                    None,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("Unable to read lexical document excerpt: {error}"))
}

fn fallback_ai_chunks_for_file(
    database: &Database,
    file_id: &str,
) -> Result<Vec<StoredAiChunk>, String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let document = connection
        .query_row(
            "SELECT documents.file_name, files.storage_path,
                    artifacts.current_version_id, versions.version_number,
                    documents.body_text
             FROM file_space_search_documents documents
             JOIN files ON files.id = documents.file_id
             LEFT JOIN file_space_artifacts artifacts ON artifacts.file_id = documents.file_id
             LEFT JOIN file_space_artifact_versions versions
               ON versions.id = artifacts.current_version_id
             WHERE documents.file_id = ?1
               AND documents.extraction_status = 'extracted'
               AND files.trashed_at IS NULL AND files.storage_path IS NOT NULL",
            [file_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("Unable to read AI fallback context: {error}"))?;
    let Some((file_name, relative_path, version_id, version_number, body_text)) = document else {
        return Ok(Vec::new());
    };
    let bounded_body = body_text
        .chars()
        .take(AI_CONTEXT_STORED_CHUNK_LIMIT * (SEMANTIC_CHUNK_CHARACTERS - SEMANTIC_CHUNK_OVERLAP))
        .collect::<String>();
    let chunks = split_search_text(bounded_body.trim());
    if chunks.is_empty() {
        return Ok(Vec::new());
    }
    Ok(chunks
        .into_iter()
        .enumerate()
        .map(|(ordinal, chunk)| {
            (
                format!("{file_id}:fallback:{ordinal}"),
                file_name.clone(),
                relative_path.clone(),
                version_id.clone(),
                version_number,
                ordinal as i64,
                chunk.body_text,
                None,
                None,
            )
        })
        .collect())
}

pub(crate) fn retrieve_ai_context_chunks(
    database: &Database,
    runtime: &SemanticSearchRuntime,
    query: &str,
    lexical_queries: &[String],
    candidates: &[HybridSearchMatch],
) -> Result<Vec<AiContextChunk>, String> {
    let query = query.trim();
    if query.is_empty() || candidates.is_empty() {
        return Ok(Vec::new());
    }
    let query_vector = if runtime.is_installed() {
        runtime.embed(&format!("query: {query}"))?
    } else {
        None
    };
    let lexical_profiles = lexical_queries
        .iter()
        .map(|query| (query.trim().to_owned(), lexical_terms(query)))
        .filter(|(query, _)| !query.is_empty())
        .collect::<Vec<_>>();
    let needles = lexical_needles(lexical_queries);
    let mut ranked = Vec::new();
    for (candidate_rank, candidate) in candidates
        .iter()
        .take(AI_CONTEXT_CANDIDATE_FILE_LIMIT)
        .enumerate()
    {
        let mut chunks = stored_ai_chunks_for_file(database, &candidate.file_id)?;
        if chunks.is_empty() {
            chunks = fallback_ai_chunks_for_file(database, &candidate.file_id)?;
        }
        if candidate.lexical_match {
            let mut stored_ids = chunks
                .iter()
                .map(|chunk| chunk.0.clone())
                .collect::<HashSet<_>>();
            let mut lexical_chunks =
                lexical_ai_chunks_for_file(database, &candidate.file_id, &needles)?;
            let existing_chunk_matches = chunks.iter().any(|chunk| {
                lexical_profiles
                    .iter()
                    .any(|(query, terms)| lexical_relevance(&chunk.6, query, terms) > 0.0)
            });
            if lexical_chunks.is_empty() && !existing_chunk_matches {
                if let Some(chunk) =
                    lexical_document_excerpt_for_file(database, &candidate.file_id, &needles)?
                {
                    lexical_chunks.push(chunk);
                }
            }
            for chunk in lexical_chunks {
                if stored_ids.insert(chunk.0.clone()) {
                    chunks.push(chunk);
                }
            }
        }
        for (
            _chunk_id,
            file_name,
            relative_path,
            version_id,
            version_number,
            ordinal,
            body_text,
            vector_blob,
            dimensions,
        ) in chunks
        {
            let metadata = format!("{file_name}\n{relative_path}");
            let body_lexical = lexical_profiles
                .iter()
                .map(|(query, terms)| lexical_relevance(&body_text, query, terms))
                .fold(0.0, f32::max);
            let metadata_lexical = lexical_profiles
                .iter()
                .map(|(query, terms)| lexical_relevance(&metadata, query, terms))
                .fold(0.0, f32::max);
            let lexical_score = body_lexical.max(metadata_lexical);
            let lexical_match = candidate.lexical_match && lexical_score > 0.0;
            let semantic_similarity = match (&query_vector, vector_blob, dimensions) {
                (Some(query_vector), Some(blob), Some(dimensions))
                    if dimensions == SEMANTIC_MODEL_DIMENSIONS as i64 =>
                {
                    vector_from_blob(&blob, SEMANTIC_MODEL_DIMENSIONS)
                        .map(|vector| cosine_similarity(query_vector, &vector))
                        .filter(|similarity| similarity.is_finite())
                }
                _ => None,
            };
            let semantic_match =
                semantic_similarity.is_some_and(|similarity| similarity >= SEMANTIC_MIN_SIMILARITY);
            if !lexical_match && !semantic_match {
                continue;
            }
            let lexical_component = if lexical_match {
                0.16 + lexical_score * 0.24
            } else {
                0.0
            };
            let semantic_component = semantic_similarity.unwrap_or(0.0) * 0.64;
            let rank_component = 0.04 / (candidate_rank as f32 + 1.0);
            ranked.push(RankedAiContextChunk {
                chunk: AiContextChunk {
                    file_id: candidate.file_id.clone(),
                    file_name,
                    relative_path,
                    version_id,
                    version_number,
                    body_text: body_text.trim().to_owned(),
                    lexical_match,
                    semantic_similarity,
                },
                score: lexical_component + semantic_component + rank_component,
                candidate_rank,
                ordinal,
            });
        }
    }
    ranked.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.candidate_rank.cmp(&right.candidate_rank))
            .then_with(|| left.ordinal.cmp(&right.ordinal))
    });
    let mut seen_files = HashSet::new();
    let mut first_per_file = Vec::new();
    let mut additional = Vec::new();
    for result in ranked {
        if seen_files.insert(result.chunk.file_id.clone()) {
            first_per_file.push(result);
        } else {
            additional.push(result);
        }
    }
    let mut per_file = HashMap::<String, usize>::new();
    let mut selected = Vec::new();
    for result in first_per_file.into_iter().take(AI_CONTEXT_RESULT_LIMIT) {
        per_file.insert(result.chunk.file_id.clone(), 1);
        selected.push(result.chunk);
    }
    for result in additional {
        if selected.len() >= AI_CONTEXT_RESULT_LIMIT {
            break;
        }
        let Some(count) = per_file.get_mut(&result.chunk.file_id) else {
            continue;
        };
        if *count >= AI_CONTEXT_PER_FILE_LIMIT {
            continue;
        }
        *count += 1;
        selected.push(result.chunk);
    }
    Ok(selected)
}

#[tauri::command]
pub fn get_semantic_search_status(
    database: State<'_, Database>,
    runtime: State<'_, SemanticSearchRuntime>,
) -> SemanticSearchStatus {
    status_record(database.inner(), runtime.inner())
}

#[tauri::command]
pub fn install_semantic_search_model(
    app: tauri::AppHandle,
    database: State<'_, Database>,
    runtime: State<'_, SemanticSearchRuntime>,
) -> Result<SemanticSearchStatus, String> {
    if runtime.is_installed() {
        schedule_outdated_documents(database.inner())?;
        return Ok(status_record(database.inner(), runtime.inner()));
    }
    if runtime
        .download_running
        .compare_exchange(false, true, AtomicOrdering::SeqCst, AtomicOrdering::SeqCst)
        .is_err()
    {
        return Ok(status_record(database.inner(), runtime.inner()));
    }
    runtime
        .cancel_download
        .store(false, AtomicOrdering::Relaxed);
    runtime.set_progress("downloading", 0, None, None);
    emit_status(&app, database.inner(), runtime.inner());
    let worker_app = app.clone();
    std::thread::Builder::new()
        .name("lumetrace-model-installer".to_owned())
        .spawn(move || {
            let runtime = worker_app.state::<SemanticSearchRuntime>();
            let database = worker_app.state::<Database>();
            let result = install_model(&worker_app, database.inner(), runtime.inner());
            match result {
                Ok(model) => {
                    if let Err(error) = runtime
                        .set_model(Some(model))
                        .and_then(|_| save_installed_model_setting(database.inner(), true))
                        .and_then(|_| schedule_outdated_documents(database.inner()))
                    {
                        runtime.set_progress("failed", model_total_bytes(), None, Some(error));
                    } else {
                        runtime.set_progress("installed", model_total_bytes(), None, None);
                    }
                }
                Err(error) if error == "The model download was cancelled" => {
                    runtime.set_progress("notInstalled", 0, None, None);
                }
                Err(error) => {
                    runtime.set_progress("failed", 0, None, Some(error));
                }
            }
            runtime
                .download_running
                .store(false, AtomicOrdering::SeqCst);
            emit_status(&worker_app, database.inner(), runtime.inner());
        })
        .map_err(|error| {
            runtime
                .download_running
                .store(false, AtomicOrdering::SeqCst);
            format!("Unable to start the model download: {error}")
        })?;
    Ok(status_record(database.inner(), runtime.inner()))
}

#[tauri::command]
pub fn cancel_semantic_search_model_download(
    database: State<'_, Database>,
    runtime: State<'_, SemanticSearchRuntime>,
) -> SemanticSearchStatus {
    runtime.cancel_download.store(true, AtomicOrdering::Relaxed);
    status_record(database.inner(), runtime.inner())
}

#[tauri::command]
pub fn remove_semantic_search_model(
    database: State<'_, Database>,
    runtime: State<'_, SemanticSearchRuntime>,
) -> Result<SemanticSearchStatus, String> {
    if runtime.download_running.load(AtomicOrdering::Relaxed) {
        return Err("Cancel the model download before removing it".to_owned());
    }
    runtime
        .index_generation
        .fetch_add(1, AtomicOrdering::SeqCst);
    runtime.set_model(None)?;
    if runtime.model_directory.exists() {
        fs::remove_dir_all(&runtime.model_directory)
            .map_err(|error| format!("Unable to remove the local semantic model: {error}"))?;
    }
    {
        let mut connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
        let transaction = connection
            .transaction()
            .map_err(|error| format!("Unable to reset the semantic index: {error}"))?;
        transaction
            .execute("DELETE FROM file_space_search_chunks", [])
            .map_err(|error| format!("Unable to reset the semantic index: {error}"))?;
        transaction
            .execute(
                "UPDATE file_space_index_jobs
                 SET status = 'pending', retry_count = 0, error = NULL,
                     requested_at = ?1, started_at = NULL, completed_at = NULL",
                [now_millis()],
            )
            .map_err(|error| format!("Unable to reset semantic indexing: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("Unable to reset the semantic index: {error}"))?;
    }
    save_installed_model_setting(database.inner(), false)?;
    runtime.set_progress("notInstalled", 0, None, None);
    Ok(status_record(database.inner(), runtime.inner()))
}

#[tauri::command]
pub fn retry_semantic_search_index(
    database: State<'_, Database>,
    runtime: State<'_, SemanticSearchRuntime>,
) -> Result<SemanticSearchStatus, String> {
    retry_failed_index_jobs(database.inner())?;
    runtime.set_progress("installed", model_total_bytes(), None, None);
    Ok(status_record(database.inner(), runtime.inner()))
}

pub(crate) fn retry_failed_index_jobs(database: &Database) -> Result<(), String> {
    {
        let connection = database
            .0
            .lock()
            .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
        connection
            .execute(
                "UPDATE file_space_index_jobs
                 SET status = 'pending', retry_count = 0, error = NULL,
                     requested_at = ?1, started_at = NULL, completed_at = NULL
                 WHERE status = 'failed'
                   AND file_id IN (
                     SELECT file_id FROM file_space_search_documents
                     WHERE extraction_status IN ('extracted', 'empty', 'unsupported')
                   )",
                [now_millis()],
            )
            .map_err(|error| format!("Unable to retry semantic indexing: {error}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunking_preserves_content_with_overlap() {
        let text = (0..1_100)
            .map(|index| char::from(b'a' + (index % 26) as u8))
            .collect::<String>();
        let chunks = split_search_text(&text);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].start_character, 0);
        assert_eq!(chunks[0].end_character, 480);
        assert_eq!(chunks[1].start_character, 400);
        assert_eq!(chunks[2].end_character, 1_100);
        assert_eq!(chunks[0].body_text.chars().count(), 480);
    }

    #[test]
    fn vector_blob_round_trip_is_exact() {
        let vector = vec![0.25, -1.5, 3.75, 0.0];
        let blob = vector_to_blob(&vector);
        assert_eq!(vector_from_blob(&blob, vector.len()), Some(vector));
        assert!(vector_from_blob(&blob, 3).is_none());
    }

    #[test]
    fn cosine_similarity_handles_normalized_and_empty_vectors() {
        assert!((cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 0.0001);
        assert_eq!(cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]), 0.0);
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
    }

    #[test]
    fn semantic_index_cooldown_enforces_a_bounded_cpu_budget() {
        assert_eq!(
            semantic_index_cooldown(Duration::from_millis(1)),
            SEMANTIC_MIN_COOLDOWN
        );
        assert_eq!(
            semantic_index_cooldown(Duration::from_millis(250)),
            Duration::from_millis(500)
        );
        assert_eq!(
            semantic_index_cooldown(Duration::from_secs(5)),
            SEMANTIC_MAX_COOLDOWN
        );
    }

    #[test]
    fn hybrid_results_keep_lexical_precision_and_add_semantic_recall() {
        let matches = merge_hybrid_matches(
            vec!["lexical".to_owned(), "both".to_owned()],
            vec![("both".to_owned(), 0.93), ("semantic".to_owned(), 0.9)],
        );
        assert_eq!(matches[0].file_id, "both");
        assert!(matches[0].lexical_match);
        assert_eq!(matches[0].semantic_similarity, Some(0.93));
        let lexical = matches
            .iter()
            .find(|result| result.file_id == "lexical")
            .unwrap();
        assert!(lexical.lexical_match);
        assert_eq!(lexical.semantic_similarity, None);
        let semantic = matches
            .iter()
            .find(|result| result.file_id == "semantic")
            .unwrap();
        assert!(!semantic.lexical_match);
        assert_eq!(semantic.semantic_similarity, Some(0.9));
    }

    #[test]
    fn ai_context_uses_only_real_active_file_records() {
        let root = std::env::temp_dir().join(format!("lumetrace-ai-context-{}", Uuid::new_v4()));
        {
            let database = crate::database::open_for_test(&root.join("lumetrace.sqlite3")).unwrap();
            let connection = database.0.lock().unwrap();
            connection
                .execute(
                    "INSERT INTO files
                     (id, original_name, storage_path, size_bytes, source_kind, updated_at, created_at)
                     VALUES ('file-1', 'decisions.md', 'project/decisions.md', 24, 'user_import', 10, 10)",
                    [],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO file_space_search_documents
                     (file_id, file_name, body_text, tag_text, task_text, cell_text,
                      extraction_status, extraction_version, file_updated_at, size_bytes, indexed_at)
                     VALUES ('file-1', 'decisions.md', 'The release decision is Friday.', '', '', '',
                             'extracted', 1, 10, 24, 11)",
                    [],
                )
                .unwrap();
            drop(connection);
            let runtime = SemanticSearchRuntime::new(&root);
            let candidates = vec![
                HybridSearchMatch {
                    file_id: "file-1".to_owned(),
                    lexical_match: true,
                    semantic_similarity: None,
                },
                HybridSearchMatch {
                    file_id: "model-invented-file".to_owned(),
                    lexical_match: true,
                    semantic_similarity: None,
                },
            ];
            let chunks = retrieve_ai_context_chunks(
                &database,
                &runtime,
                "release decision",
                &["release decision".to_owned()],
                &candidates,
            )
            .unwrap();
            assert_eq!(chunks.len(), 1);
            assert_eq!(chunks[0].file_id, "file-1");
            assert_eq!(chunks[0].relative_path, "project/decisions.md");
            assert!(chunks[0].body_text.contains("Friday"));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ai_context_reserves_evidence_for_each_relevant_file_before_extra_chunks() {
        let root =
            std::env::temp_dir().join(format!("lumetrace-ai-context-diversity-{}", Uuid::new_v4()));
        {
            let database = crate::database::open_for_test(&root.join("lumetrace.sqlite3")).unwrap();
            let mut connection = database.0.lock().unwrap();
            let transaction = connection.transaction().unwrap();
            for index in 1..=4 {
                let file_id = format!("file-{index}");
                let file_name = format!("related-{index}.md");
                let body = format!(
                    "断流恢复 文件 {index} 的独立改进证据。{}",
                    "断流恢复的补充细节。".repeat(180)
                );
                transaction
                    .execute(
                        "INSERT INTO files
                         (id, original_name, storage_path, size_bytes, source_kind,
                          updated_at, created_at)
                         VALUES (?1, ?2, ?2, ?3, 'user_import', 10, 10)",
                        params![&file_id, &file_name, body.len() as i64],
                    )
                    .unwrap();
                transaction
                    .execute(
                        "INSERT INTO file_space_search_documents
                         (file_id, file_name, body_text, tag_text, task_text, cell_text,
                          extraction_status, extraction_version, file_updated_at,
                          size_bytes, indexed_at)
                         VALUES (?1, ?2, ?3, '', '', '', 'extracted', 1, 10, ?4, 11)",
                        params![&file_id, &file_name, &body, body.len() as i64],
                    )
                    .unwrap();
            }
            transaction.commit().unwrap();
            drop(connection);

            let runtime = SemanticSearchRuntime::new(&root);
            let candidates = (1..=4)
                .map(|index| HybridSearchMatch {
                    file_id: format!("file-{index}"),
                    lexical_match: true,
                    semantic_similarity: None,
                })
                .collect::<Vec<_>>();
            let chunks = retrieve_ai_context_chunks(
                &database,
                &runtime,
                "断流恢复",
                &["断流恢复".to_owned()],
                &candidates,
            )
            .unwrap();
            let file_ids = chunks
                .iter()
                .map(|chunk| chunk.file_id.as_str())
                .collect::<HashSet<_>>();
            assert_eq!(file_ids.len(), 4);
            assert_eq!(chunks.len(), AI_CONTEXT_RESULT_LIMIT);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ai_context_retrieves_a_late_lexical_match_in_a_large_file() {
        let root = std::env::temp_dir().join(format!(
            "lumetrace-ai-late-lexical-context-{}",
            Uuid::new_v4()
        ));
        {
            let database = crate::database::open_for_test(&root.join("lumetrace.sqlite3")).unwrap();
            let mut connection = database.0.lock().unwrap();
            connection
                .execute(
                    "INSERT INTO files
                     (id, original_name, storage_path, size_bytes, source_kind, updated_at, created_at)
                     VALUES ('file-1', 'plan.md', 'plan.md', 100000, 'user_import', 10, 10)",
                    [],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO file_space_search_documents
                     (file_id, file_name, body_text, tag_text, task_text, cell_text,
                      extraction_status, extraction_version, file_updated_at, size_bytes, indexed_at)
                     VALUES ('file-1', 'plan.md', 'full indexed document', '', '', '',
                             'extracted', 1, 10, 100000, 11)",
                    [],
                )
                .unwrap();
            let transaction = connection.transaction().unwrap();
            for ordinal in 0..70 {
                let body = if ordinal == 69 {
                    "断流恢复机制位于大文件后部，必须参与综合分析。"
                } else {
                    "与当前问题无关的早期记录。"
                };
                transaction
                    .execute(
                        "INSERT INTO file_space_search_chunks
                         (id, file_id, document_indexed_at, content_hash, ordinal,
                          start_character, end_character, body_text, character_count, created_at)
                         VALUES (?1, 'file-1', 11, ?1, ?2, ?3, ?4, ?5, ?6, 11)",
                        params![
                            format!("chunk-{ordinal}"),
                            ordinal,
                            ordinal * 480,
                            ordinal * 480 + body.chars().count(),
                            body,
                            body.chars().count(),
                        ],
                    )
                    .unwrap();
            }
            transaction.commit().unwrap();
            drop(connection);

            let runtime = SemanticSearchRuntime::new(&root);
            let candidates = vec![HybridSearchMatch {
                file_id: "file-1".to_owned(),
                lexical_match: true,
                semantic_similarity: None,
            }];
            let chunks = retrieve_ai_context_chunks(
                &database,
                &runtime,
                "断流恢复",
                &["断流恢复".to_owned()],
                &candidates,
            )
            .unwrap();
            assert_eq!(chunks.len(), 1);
            assert!(chunks[0].body_text.contains("必须参与综合分析"));
            assert!(chunks[0].lexical_match);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ai_context_uses_the_full_text_excerpt_while_semantic_indexing_is_pending() {
        let root = std::env::temp_dir().join(format!(
            "lumetrace-ai-pending-index-context-{}",
            Uuid::new_v4()
        ));
        {
            let database = crate::database::open_for_test(&root.join("lumetrace.sqlite3")).unwrap();
            let body = format!(
                "{}断流恢复机制位于文件后部，也必须提供给 AI。",
                "无关的早期记录。\n".repeat(4_000)
            );
            let connection = database.0.lock().unwrap();
            connection
                .execute(
                    "INSERT INTO files
                     (id, original_name, storage_path, size_bytes, source_kind, updated_at, created_at)
                     VALUES ('file-1', 'pending.md', 'pending.md', ?1, 'user_import', 10, 10)",
                    [body.len() as i64],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO file_space_search_documents
                     (file_id, file_name, body_text, tag_text, task_text, cell_text,
                      extraction_status, extraction_version, file_updated_at, size_bytes, indexed_at)
                     VALUES ('file-1', 'pending.md', ?1, '', '', '',
                             'extracted', 1, 10, ?2, 11)",
                    params![body, body.len() as i64],
                )
                .unwrap();
            drop(connection);

            let runtime = SemanticSearchRuntime::new(&root);
            let candidates = vec![HybridSearchMatch {
                file_id: "file-1".to_owned(),
                lexical_match: true,
                semantic_similarity: None,
            }];
            let chunks = retrieve_ai_context_chunks(
                &database,
                &runtime,
                "断流恢复",
                &["断流恢复".to_owned()],
                &candidates,
            )
            .unwrap();
            assert_eq!(chunks.len(), 1);
            assert!(chunks[0].body_text.contains("也必须提供给 AI"));
            assert!(chunks[0].lexical_match);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ai_fallback_context_has_a_hard_per_file_chunk_limit() {
        let root =
            std::env::temp_dir().join(format!("lumetrace-ai-context-limit-{}", Uuid::new_v4()));
        {
            let database = crate::database::open_for_test(&root.join("lumetrace.sqlite3")).unwrap();
            let oversized_body = "bounded local context ".repeat(4_000);
            let connection = database.0.lock().unwrap();
            connection
                .execute(
                    "INSERT INTO files
                     (id, original_name, storage_path, size_bytes, source_kind, updated_at, created_at)
                     VALUES ('file-1', 'large.txt', 'large.txt', ?1, 'user_import', 10, 10)",
                    [oversized_body.len() as i64],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO file_space_search_documents
                     (file_id, file_name, body_text, tag_text, task_text, cell_text,
                      extraction_status, extraction_version, file_updated_at, size_bytes, indexed_at)
                     VALUES ('file-1', 'large.txt', ?1, '', '', '',
                             'extracted', 1, 10, ?2, 11)",
                    params![oversized_body, oversized_body.len() as i64],
                )
                .unwrap();
            drop(connection);

            let chunks = fallback_ai_chunks_for_file(&database, "file-1").unwrap();
            assert!(!chunks.is_empty());
            assert!(chunks.len() <= AI_CONTEXT_STORED_CHUNK_LIMIT);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ai_fallback_never_uses_a_filename_as_document_content() {
        let root =
            std::env::temp_dir().join(format!("lumetrace-ai-empty-content-{}", Uuid::new_v4()));
        {
            let database = crate::database::open_for_test(&root.join("lumetrace.sqlite3")).unwrap();
            let connection = database.0.lock().unwrap();
            connection
                .execute(
                    "INSERT INTO files
                     (id, original_name, storage_path, size_bytes, source_kind, updated_at, created_at)
                     VALUES ('file-1', 'confidential.xlsx', 'confidential.xlsx', 24, 'user_import', 10, 10)",
                    [],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO file_space_search_documents
                     (file_id, file_name, body_text, extraction_status, extraction_version,
                      tag_text, task_text, cell_text, file_updated_at, size_bytes, indexed_at)
                     VALUES ('file-1', 'confidential.xlsx', '', 'empty', 1, '', '', '', 10, 24, 11)",
                    [],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO file_space_search_chunks
                     (id, file_id, document_indexed_at, content_hash, ordinal,
                      start_character, end_character, body_text, character_count, created_at)
                     VALUES ('stale-chunk', 'file-1', 9, 'stale', 0, 0, 17,
                             'confidential.xlsx', 17, 9)",
                    [],
                )
                .unwrap();
            drop(connection);
            let stored = stored_ai_chunks_for_file(&database, "file-1").unwrap();
            assert!(stored.is_empty());
            let chunks = fallback_ai_chunks_for_file(&database, "file-1").unwrap();
            assert!(chunks.is_empty());
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scheduling_a_changed_document_invalidates_previous_work() {
        let root =
            std::env::temp_dir().join(format!("lumetrace-semantic-schedule-{}", Uuid::new_v4()));
        {
            let database = crate::database::open_for_test(&root.join("lumetrace.sqlite3")).unwrap();
            let connection = database.0.lock().unwrap();
            connection
                .execute(
                    "INSERT INTO files
                     (id, original_name, storage_path, size_bytes, source_kind, updated_at, created_at)
                     VALUES ('file-1', 'notes.md', 'notes.md', 12, 'user_import', 10, 10)",
                    [],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO file_space_search_documents
                     (file_id, file_name, body_text, tag_text, task_text, cell_text,
                      file_updated_at, size_bytes, indexed_at)
                     VALUES ('file-1', 'notes.md', 'first', '', '', '', 10, 12, 11)",
                    [],
                )
                .unwrap();
            drop(connection);
            schedule_search_documents(&database, &["file-1".to_owned()]).unwrap();
            assert!(read_next_document(&database).unwrap().is_none());
            {
                let connection = database.0.lock().unwrap();
                connection
                    .execute(
                        "INSERT INTO file_space_index_jobs
                         (file_id, requested_document_indexed_at, status, retry_count,
                          error, requested_at)
                         VALUES ('file-1', 11, 'failed', 2, 'old', 11)",
                        [],
                    )
                    .unwrap();
                connection
                    .execute(
                        "UPDATE file_space_search_documents
                         SET body_text = 'second', extraction_status = 'extracted', indexed_at = 22
                         WHERE file_id = 'file-1'",
                        [],
                    )
                    .unwrap();
            }
            schedule_search_documents(&database, &["file-1".to_owned()]).unwrap();
            let connection = database.0.lock().unwrap();
            let job = connection
                .query_row(
                    "SELECT requested_document_indexed_at, status, retry_count, error
                     FROM file_space_index_jobs WHERE file_id = 'file-1'",
                    [],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, Option<String>>(3)?,
                        ))
                    },
                )
                .unwrap();
            assert_eq!(job, (22, "pending".to_owned(), 0, None));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn outdated_document_reconciliation_is_bounded_and_cursor_based() {
        let root = std::env::temp_dir().join(format!(
            "lumetrace-semantic-reconciliation-{}",
            Uuid::new_v4()
        ));
        {
            let database = crate::database::open_for_test(&root.join("lumetrace.sqlite3")).unwrap();
            let mut connection = database.0.lock().unwrap();
            let transaction = connection.transaction().unwrap();
            for index in 0..600 {
                let file_id = format!("file-{index:04}");
                let file_name = format!("note-{index:04}.md");
                transaction
                    .execute(
                        "INSERT INTO files
                         (id, original_name, storage_path, size_bytes, source_kind,
                          updated_at, created_at)
                         VALUES (?1, ?2, ?2, 10, 'user_import', 10, 10)",
                        params![&file_id, &file_name],
                    )
                    .unwrap();
                transaction
                    .execute(
                        "INSERT INTO file_space_search_documents
                         (file_id, file_name, body_text, extraction_status, extraction_version,
                          tag_text, task_text, cell_text, file_updated_at, size_bytes, indexed_at)
                         VALUES (?1, ?2, '', 'empty', 1, '', '', '', 10, 10, 11)",
                        params![&file_id, &file_name],
                    )
                    .unwrap();
            }
            transaction.commit().unwrap();
            drop(connection);

            let first = schedule_outdated_document_batch(&database, None, 128).unwrap();
            assert_eq!(first.scanned, 128);
            assert_eq!(first.scheduled, 128);
            assert_eq!(first.last_file_id.as_deref(), Some("file-0127"));

            let second =
                schedule_outdated_document_batch(&database, first.last_file_id.as_deref(), 128)
                    .unwrap();
            assert_eq!(second.scanned, 128);
            assert_eq!(second.scheduled, 128);
            assert_eq!(second.last_file_id.as_deref(), Some("file-0255"));

            let queued = database
                .0
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM file_space_index_jobs", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap();
            assert_eq!(queued, 256);

            schedule_outdated_documents(&database).unwrap();
            let queued = database
                .0
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM file_space_index_jobs", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap();
            assert_eq!(queued, 600);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reconciliation_skips_current_embeddings_and_schedules_changed_documents() {
        let root =
            std::env::temp_dir().join(format!("lumetrace-semantic-current-{}", Uuid::new_v4()));
        {
            let database = crate::database::open_for_test(&root.join("lumetrace.sqlite3")).unwrap();
            let connection = database.0.lock().unwrap();
            connection
                .execute_batch(&format!(
                    "INSERT INTO files
                       (id, original_name, storage_path, size_bytes, source_kind,
                        updated_at, created_at)
                     VALUES ('file-1', 'notes.md', 'notes.md', 10, 'user_import', 10, 10);
                     INSERT INTO file_space_search_documents
                       (file_id, file_name, body_text, extraction_status, extraction_version,
                        tag_text, task_text, cell_text, file_updated_at, size_bytes, indexed_at)
                     VALUES ('file-1', 'notes.md', 'body', 'extracted', 1,
                             '', '', '', 10, 10, 11);
                     INSERT INTO file_space_index_jobs
                       (file_id, requested_document_indexed_at, status, requested_at)
                     VALUES ('file-1', 11, 'ready', 11);
                     INSERT INTO file_space_search_chunks
                       (id, file_id, document_indexed_at, content_hash, ordinal,
                        start_character, end_character, body_text, character_count, created_at)
                     VALUES ('chunk-1', 'file-1', 11, 'hash', 0, 0, 4, 'body', 4, 11);
                     INSERT INTO file_space_semantic_embeddings
                       (chunk_id, model_id, dimensions, vector, created_at)
                     VALUES ('chunk-1', '{SEMANTIC_MODEL_ID}', 384, X'00000000', 11);"
                ))
                .unwrap();
            drop(connection);

            let current = schedule_outdated_document_batch(&database, None, 10).unwrap();
            assert_eq!(current.scanned, 1);
            assert_eq!(current.scheduled, 0);

            database
                .0
                .lock()
                .unwrap()
                .execute(
                    "UPDATE file_space_search_documents SET indexed_at = 12 WHERE file_id = 'file-1'",
                    [],
                )
                .unwrap();
            let changed = schedule_outdated_document_batch(&database, None, 10).unwrap();
            assert_eq!(changed.scheduled, 1);
            let job = database
                .0
                .lock()
                .unwrap()
                .query_row(
                    "SELECT requested_document_indexed_at, status
                     FROM file_space_index_jobs WHERE file_id = 'file-1'",
                    [],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
                )
                .unwrap();
            assert_eq!(job, (12, "pending".to_owned()));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reconciliation_query_looks_up_embeddings_by_chunk_instead_of_scanning_the_model() {
        let root =
            std::env::temp_dir().join(format!("lumetrace-semantic-query-plan-{}", Uuid::new_v4()));
        {
            let database = crate::database::open_for_test(&root.join("lumetrace.sqlite3")).unwrap();
            let connection = database.0.lock().unwrap();
            let mut statement = connection
                .prepare(&format!(
                    "EXPLAIN QUERY PLAN {OUTDATED_DOCUMENT_BATCH_QUERY}"
                ))
                .unwrap();
            let plan = statement
                .query_map(
                    params![SEMANTIC_MODEL_ID, Option::<String>::None, 256_i64],
                    |row| row.get::<_, String>(3),
                )
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            assert!(plan.iter().any(|step| step.contains("SEARCH chunks")));
            assert!(plan.iter().any(|step| step.contains("SEARCH embeddings")));
            assert!(!plan.iter().any(|step| step.contains("SCAN embeddings")));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn semantic_search_query_starts_from_bounded_candidate_files() {
        let root = std::env::temp_dir().join(format!(
            "lumetrace-semantic-candidate-plan-{}",
            Uuid::new_v4()
        ));
        {
            let database = crate::database::open_for_test(&root.join("lumetrace.sqlite3")).unwrap();
            let connection = database.0.lock().unwrap();
            let (sql, _, _) = semantic_candidate_query(2);
            let mut statement = connection
                .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                .unwrap();
            let plan = statement
                .query_map(
                    params![
                        SEMANTIC_MODEL_ID,
                        "candidate-a",
                        "candidate-b",
                        SEMANTIC_CHUNKS_PER_FILE_LIMIT as i64,
                        SEMANTIC_VECTOR_SCAN_LIMIT as i64,
                    ],
                    |row| row.get::<_, String>(3),
                )
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            let embedding_step = plan
                .iter()
                .find(|step| step.contains("SEARCH embeddings") || step.contains("SCAN embeddings"))
                .expect("semantic query plan must access embeddings");
            assert!(embedding_step.contains("SEARCH embeddings"));
            assert!(embedding_step.contains("chunk_id=?"));
            assert!(!plan.iter().any(|step| step.contains("SCAN embeddings")));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pending_content_extraction_is_hidden_from_semantic_progress() {
        let root = std::env::temp_dir().join(format!(
            "lumetrace-hidden-content-extraction-{}",
            Uuid::new_v4()
        ));
        {
            let database = crate::database::open_for_test(&root.join("lumetrace.sqlite3")).unwrap();
            let connection = database.0.lock().unwrap();
            connection
                .execute(
                    "INSERT INTO files
                     (id, original_name, storage_path, size_bytes, source_kind, updated_at, created_at)
                     VALUES ('file-1', 'large.pdf', 'large.pdf', 24, 'user_import', 10, 10)",
                    [],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO file_space_search_documents
                     (file_id, file_name, body_text, extraction_status, extraction_version,
                      tag_text, task_text, cell_text, file_updated_at, size_bytes, indexed_at)
                     VALUES ('file-1', 'large.pdf', '', 'pending', 1, '', '', '', 10, 24, 11)",
                    [],
                )
                .unwrap();
            drop(connection);
            let runtime = SemanticSearchRuntime::new(&root);
            let status = status_record(&database, &runtime);
            assert_eq!(status.total_files, 0);
            assert_eq!(status.pending_files, 0);
            assert_eq!(status.indexed_files, 0);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_content_extraction_is_not_reported_or_retried_as_semantic_failure() {
        let root = std::env::temp_dir().join(format!(
            "lumetrace-hidden-content-failure-{}",
            Uuid::new_v4()
        ));
        {
            let database = crate::database::open_for_test(&root.join("lumetrace.sqlite3")).unwrap();
            let connection = database.0.lock().unwrap();
            connection
                .execute_batch(
                    "INSERT INTO files
                       (id, original_name, storage_path, size_bytes, source_kind,
                        updated_at, created_at)
                     VALUES ('file-1', 'broken.pdf', 'broken.pdf', 24,
                             'user_import', 10, 10);
                     INSERT INTO file_space_search_documents
                       (file_id, file_name, body_text, extraction_status, extraction_error,
                        extraction_version, tag_text, task_text, cell_text,
                        file_updated_at, size_bytes, indexed_at)
                     VALUES ('file-1', 'broken.pdf', '', 'failed', 'PDF parser failed', 1,
                             '', '', '', 10, 24, 11);
                     INSERT INTO file_space_index_jobs
                       (file_id, requested_document_indexed_at, status, retry_count, error,
                        requested_at, completed_at)
                     VALUES ('file-1', 11, 'failed', 1, 'Content extraction failed', 11, 12);",
                )
                .unwrap();
            drop(connection);

            let runtime = SemanticSearchRuntime::new(&root);
            let status = status_record(&database, &runtime);
            assert_eq!(status.total_files, 0);
            assert_eq!(status.failed_files, 0);
            assert!(status.error.is_none());

            retry_failed_index_jobs(&database).unwrap();
            let job_status = database
                .0
                .lock()
                .unwrap()
                .query_row(
                    "SELECT status FROM file_space_index_jobs WHERE file_id = 'file-1'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap();
            assert_eq!(job_status, "failed");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "requires the pinned multilingual model files"]
    fn real_quantized_model_generates_multilingual_embeddings() {
        let directory = std::env::var("LUMETRACE_TEST_MODEL_DIR")
            .expect("set LUMETRACE_TEST_MODEL_DIR to the verified model directory");
        let mut model = load_model(Path::new(&directory)).unwrap();
        let embed =
            |model: &mut TextEmbedding, text: &str| model.embed([text], None).unwrap().remove(0);
        let query = embed(&mut model, "query: Lume Trace 如何追踪文件版本？");
        let relevant = embed(
            &mut model,
            "passage: Lume Trace 会记录文件版本时间线，并保留可追溯的历史。",
        );
        let unrelated = embed(
            &mut model,
            "passage: The weather is sunny and suitable for a walk.",
        );
        assert_eq!(query.len(), SEMANTIC_MODEL_DIMENSIONS);
        assert_eq!(relevant.len(), SEMANTIC_MODEL_DIMENSIONS);
        assert!(cosine_similarity(&query, &relevant) > cosine_similarity(&query, &unrelated));
    }
}
