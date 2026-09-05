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
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, Manager, State};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};
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
const SEMANTIC_ANN_INDEX_FILE: &str = "semantic-search.usearch";
const SEMANTIC_ANN_BUILD_BATCH_SIZE: usize = 256;
const SEMANTIC_ANN_SEARCH_LIMIT: usize = 256;
const SEMANTIC_ANN_CONNECTIVITY: usize = 16;
const SEMANTIC_ANN_EXPANSION_ADD: usize = 64;
const SEMANTIC_ANN_EXPANSION_SEARCH: usize = 96;
const AI_DENSE_SEARCH_LIMIT: usize = 48;
const AI_LEXICAL_SEARCH_LIMIT: usize = 48;
const AI_FUSED_CANDIDATE_LIMIT: usize = 20;
const AI_CONTEXT_RESULT_LIMIT: usize = 6;
const AI_CONTEXT_PER_FILE_LIMIT: usize = 3;
const AI_RRF_K: f32 = 60.0;
const AI_DENSE_SCORE_WINDOW: f32 = 0.035;
const AI_CHUNK_FTS_BACKFILL_BATCH_SIZE: usize = 256;
const AI_CHUNK_FTS_BACKFILL_CURSOR_KEY: &str = "search_chunks_fts.backfill_cursor.v1";
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
    "https://huggingface.co/Xenova/multilingual-e5-small/resolve",
    "https://hf-mirror.com/Xenova/multilingual-e5-small/resolve",
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

struct SemanticAnnSnapshot {
    workspace_id: String,
    data_revision: i64,
    index: Arc<Index>,
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
    ann_snapshot: Mutex<Option<SemanticAnnSnapshot>>,
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
            ann_snapshot: Mutex::new(None),
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

    fn clear_ann_snapshot(&self) {
        if let Ok(mut snapshot) = self.ann_snapshot.lock() {
            *snapshot = None;
        }
    }

    fn set_ann_snapshot(&self, snapshot: SemanticAnnSnapshot) -> Result<(), String> {
        let mut current = self
            .ann_snapshot
            .lock()
            .map_err(|_| "Unable to access the local semantic ANN index".to_owned())?;
        *current = Some(snapshot);
        Ok(())
    }

    fn ann_snapshot_for(
        &self,
        workspace_id: &str,
        data_revision: i64,
    ) -> Result<Option<Arc<Index>>, String> {
        let current = self
            .ann_snapshot
            .lock()
            .map_err(|_| "Unable to access the local semantic ANN index".to_owned())?;
        Ok(current
            .as_ref()
            .filter(|snapshot| {
                snapshot.workspace_id == workspace_id && snapshot.data_revision == data_revision
            })
            .map(|snapshot| Arc::clone(&snapshot.index)))
    }

    fn apply_ann_mutation(
        &self,
        workspace_id: &str,
        revision_before: i64,
        revision_after: i64,
        removed_keys: &[u64],
        added_vectors: &[(u64, Vec<f32>)],
    ) {
        let Ok(mut current) = self.ann_snapshot.lock() else {
            return;
        };
        let Some(snapshot) = current.as_mut().filter(|snapshot| {
            snapshot.workspace_id == workspace_id && snapshot.data_revision == revision_before
        }) else {
            return;
        };
        let update_result = (|| -> Result<(), String> {
            if !added_vectors.is_empty() {
                snapshot
                    .index
                    .reserve(snapshot.index.size().saturating_add(added_vectors.len()))
                    .map_err(|error| format!("Unable to grow the semantic ANN index: {error}"))?;
            }
            for key in removed_keys {
                snapshot
                    .index
                    .remove(*key)
                    .map_err(|error| format!("Unable to remove a semantic ANN vector: {error}"))?;
            }
            for (key, vector) in added_vectors {
                snapshot
                    .index
                    .add(*key, vector)
                    .map_err(|error| format!("Unable to add a semantic ANN vector: {error}"))?;
            }
            Ok(())
        })();
        if update_result.is_ok() {
            snapshot.data_revision = revision_after;
        } else {
            *current = None;
        }
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

struct SemanticAnnMutation {
    workspace_id: String,
    revision_before: i64,
    revision_after: i64,
    removed_keys: Vec<u64>,
    added_vectors: Vec<(u64, Vec<f32>)>,
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

pub(crate) fn semantic_ann_index_path_for_database(database_path: &Path) -> PathBuf {
    database_path.with_file_name(SEMANTIC_ANN_INDEX_FILE)
}

fn semantic_ann_revisions(database: &Database) -> Result<(i64, i64), String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    connection
        .query_row(
            "SELECT data_revision, file_revision
             FROM file_space_semantic_ann_state WHERE model_id = ?1",
            [SEMANTIC_MODEL_ID],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| format!("Unable to read semantic ANN state: {error}"))
}

fn semantic_ann_vector_count(database: &Database) -> Result<usize, String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    connection
        .query_row(
            "SELECT COUNT(*)
             FROM file_space_semantic_ann_keys ann
             JOIN file_space_semantic_embeddings embeddings
               ON embeddings.chunk_id = ann.chunk_id
             WHERE embeddings.model_id = ?1
               AND embeddings.dimensions = ?2",
            params![SEMANTIC_MODEL_ID, SEMANTIC_MODEL_DIMENSIONS as i64],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count.max(0) as usize)
        .map_err(|error| format!("Unable to count semantic ANN vectors: {error}"))
}

fn backfill_semantic_ann_keys(database: &Database) -> Result<(), String> {
    loop {
        let changed = {
            let mut connection = database
                .0
                .lock()
                .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
            let transaction = connection
                .transaction()
                .map_err(|error| format!("Unable to begin semantic ANN migration: {error}"))?;
            let changed = transaction
                .execute(
                    "INSERT OR IGNORE INTO file_space_semantic_ann_keys (chunk_id)
                     SELECT embeddings.chunk_id
                     FROM file_space_semantic_embeddings embeddings
                     LEFT JOIN file_space_semantic_ann_keys ann
                       ON ann.chunk_id = embeddings.chunk_id
                     WHERE embeddings.model_id = ?1 AND ann.chunk_id IS NULL
                     ORDER BY embeddings.rowid
                     LIMIT ?2",
                    params![SEMANTIC_MODEL_ID, SEMANTIC_ANN_BUILD_BATCH_SIZE as i64],
                )
                .map_err(|error| format!("Unable to migrate semantic ANN keys: {error}"))?;
            transaction
                .commit()
                .map_err(|error| format!("Unable to finish semantic ANN migration: {error}"))?;
            changed
        };
        if changed == 0 {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn new_semantic_ann_index(capacity: usize) -> Result<Index, String> {
    let index = Index::new(&IndexOptions {
        dimensions: SEMANTIC_MODEL_DIMENSIONS,
        metric: MetricKind::Cos,
        quantization: ScalarKind::BF16,
        connectivity: SEMANTIC_ANN_CONNECTIVITY,
        expansion_add: SEMANTIC_ANN_EXPANSION_ADD,
        expansion_search: SEMANTIC_ANN_EXPANSION_SEARCH,
        multi: false,
    })
    .map_err(|error| format!("Unable to create the semantic ANN index: {error}"))?;
    index
        .reserve_capacity_and_threads(capacity.max(1), 1)
        .map_err(|error| format!("Unable to reserve the semantic ANN index: {error}"))?;
    Ok(index)
}

fn save_semantic_ann_index(index: &Index, destination: &Path) -> Result<(), String> {
    let parent = destination
        .parent()
        .ok_or_else(|| "Unable to resolve the semantic ANN directory".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Unable to create the semantic ANN directory: {error}"))?;
    let temporary = parent.join(format!(".{SEMANTIC_ANN_INDEX_FILE}.{}.tmp", Uuid::new_v4()));
    let temporary_text = temporary
        .to_str()
        .ok_or_else(|| "The semantic ANN path is not valid UTF-8".to_owned())?;
    if let Err(error) = index.save(temporary_text) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("Unable to save the semantic ANN index: {error}"));
    }
    #[cfg(target_os = "windows")]
    if destination.exists() {
        fs::remove_file(destination)
            .map_err(|error| format!("Unable to replace the semantic ANN index: {error}"))?;
    }
    if let Err(error) = fs::rename(&temporary, destination) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("Unable to publish the semantic ANN index: {error}"));
    }
    Ok(())
}

fn mark_semantic_ann_file_revision(
    database: &Database,
    expected_data_revision: i64,
) -> Result<bool, String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    connection
        .execute(
            "UPDATE file_space_semantic_ann_state
             SET file_revision = ?2
             WHERE model_id = ?1 AND data_revision = ?2",
            params![SEMANTIC_MODEL_ID, expected_data_revision],
        )
        .map(|changed| changed == 1)
        .map_err(|error| format!("Unable to save semantic ANN state: {error}"))
}

fn current_workspace_is(database: &Database, workspace_id: &str) -> bool {
    database
        .location()
        .is_ok_and(|location| location.workspace_id == workspace_id)
}

fn build_semantic_ann_index(
    database: &Database,
    workspace_id: &str,
    expected_data_revision: i64,
) -> Result<Option<Index>, String> {
    let vector_count = semantic_ann_vector_count(database)?;
    let index = new_semantic_ann_index(vector_count)?;
    let mut last_key = 0_i64;
    loop {
        if !current_workspace_is(database, workspace_id)
            || semantic_ann_revisions(database)?.0 != expected_data_revision
        {
            return Ok(None);
        }
        let started_at = Instant::now();
        let rows = {
            let connection = database
                .0
                .lock()
                .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
            let mut statement = connection
                .prepare(
                    "SELECT ann.ann_key, embeddings.vector, embeddings.dimensions
                     FROM file_space_semantic_ann_keys ann
                     JOIN file_space_semantic_embeddings embeddings
                       ON embeddings.chunk_id = ann.chunk_id
                     WHERE embeddings.model_id = ?1 AND ann.ann_key > ?2
                     ORDER BY ann.ann_key
                     LIMIT ?3",
                )
                .map_err(|error| format!("Unable to prepare semantic ANN rebuild: {error}"))?;
            statement
                .query_map(
                    params![
                        SEMANTIC_MODEL_ID,
                        last_key,
                        SEMANTIC_ANN_BUILD_BATCH_SIZE as i64
                    ],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, Vec<u8>>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    },
                )
                .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
                .map_err(|error| format!("Unable to read semantic ANN vectors: {error}"))?
        };
        if rows.is_empty() {
            break;
        }
        for (ann_key, blob, dimensions) in &rows {
            last_key = *ann_key;
            if *dimensions != SEMANTIC_MODEL_DIMENSIONS as i64 {
                continue;
            }
            let Some(vector) = vector_from_blob(blob, SEMANTIC_MODEL_DIMENSIONS) else {
                continue;
            };
            index
                .add(*ann_key as u64, &vector)
                .map_err(|error| format!("Unable to rebuild a semantic ANN vector: {error}"))?;
        }
        let pause = semantic_index_cooldown(started_at.elapsed());
        std::thread::sleep(pause);
    }
    Ok(Some(index))
}

fn load_semantic_ann_index(path: &Path, expected_size: usize) -> Result<Index, String> {
    let path = path
        .to_str()
        .ok_or_else(|| "The semantic ANN path is not valid UTF-8".to_owned())?;
    let index = Index::restore(path)
        .map_err(|error| format!("Unable to load the semantic ANN index: {error}"))?;
    if index.dimensions() != SEMANTIC_MODEL_DIMENSIONS || index.size() != expected_size {
        return Err(
            "The saved semantic ANN index does not match the workspace database".to_owned(),
        );
    }
    Ok(index)
}

fn reconcile_semantic_ann_index(
    database: &Database,
    runtime: &SemanticSearchRuntime,
) -> Result<(), String> {
    let location = database.location()?;
    backfill_semantic_ann_keys(database)?;
    if !current_workspace_is(database, &location.workspace_id) {
        return Ok(());
    }
    let (data_revision, file_revision) = semantic_ann_revisions(database)?;
    let index_path = semantic_ann_index_path_for_database(&location.database_path);
    if let Some(index) = runtime.ann_snapshot_for(&location.workspace_id, data_revision)? {
        if file_revision != data_revision {
            save_semantic_ann_index(&index, &index_path)?;
            let _ = mark_semantic_ann_file_revision(database, data_revision)?;
        }
        return Ok(());
    }

    let expected_size = semantic_ann_vector_count(database)?;
    if file_revision == data_revision && index_path.is_file() {
        if let Ok(index) = load_semantic_ann_index(&index_path, expected_size) {
            runtime.set_ann_snapshot(SemanticAnnSnapshot {
                workspace_id: location.workspace_id,
                data_revision,
                index: Arc::new(index),
            })?;
            return Ok(());
        }
    }

    let Some(index) = build_semantic_ann_index(database, &location.workspace_id, data_revision)?
    else {
        return Ok(());
    };
    save_semantic_ann_index(&index, &index_path)?;
    if !mark_semantic_ann_file_revision(database, data_revision)? {
        return Ok(());
    }
    runtime.set_ann_snapshot(SemanticAnnSnapshot {
        workspace_id: location.workspace_id,
        data_revision,
        index: Arc::new(index),
    })
}

fn semantic_ann_is_ready(database: &Database, runtime: &SemanticSearchRuntime) -> bool {
    let Ok(location) = database.location() else {
        return false;
    };
    let Ok((data_revision, _)) = semantic_ann_revisions(database) else {
        return false;
    };
    runtime
        .ann_snapshot_for(&location.workspace_id, data_revision)
        .is_ok_and(|snapshot| snapshot.is_some())
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
    let workspace_id = database.location()?.workspace_id;
    let index_generation = runtime.index_generation.load(AtomicOrdering::SeqCst);
    let result = (|| -> Result<Option<SemanticAnnMutation>, String> {
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
            return Ok(None);
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
            return Ok(None);
        }
        let revision_before = transaction
            .query_row(
                "SELECT data_revision FROM file_space_semantic_ann_state WHERE model_id = ?1",
                [SEMANTIC_MODEL_ID],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| format!("Unable to read semantic ANN revision: {error}"))?;
        let removed_keys = {
            let mut statement = transaction
                .prepare(
                    "SELECT ann.ann_key
                     FROM file_space_semantic_ann_keys ann
                     JOIN file_space_search_chunks chunks ON chunks.id = ann.chunk_id
                     WHERE chunks.file_id = ?1",
                )
                .map_err(|error| format!("Unable to prepare semantic ANN replacement: {error}"))?;
            statement
                .query_map([&document.file_id], |row| row.get::<_, i64>(0))
                .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
                .map_err(|error| format!("Unable to read replaced semantic ANN keys: {error}"))?
                .into_iter()
                .map(|key| key as u64)
                .collect::<Vec<_>>()
        };
        transaction
            .execute(
                "DELETE FROM file_space_search_chunks WHERE file_id = ?1",
                [&document.file_id],
            )
            .map_err(|error| format!("Unable to replace semantic chunks: {error}"))?;
        let created_at = now_millis();
        let mut added_vectors = Vec::with_capacity(embedded_chunks.len());
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
            transaction
                .execute(
                    "INSERT INTO file_space_semantic_ann_keys (chunk_id) VALUES (?1)",
                    [&chunk_id],
                )
                .map_err(|error| format!("Unable to save a semantic ANN key: {error}"))?;
            added_vectors.push((transaction.last_insert_rowid() as u64, vector));
        }
        transaction
            .execute(
                "UPDATE file_space_index_jobs
                 SET status = 'ready', error = NULL, completed_at = ?2
                 WHERE file_id = ?1",
                params![document.file_id, now_millis()],
            )
            .map_err(|error| format!("Unable to finish semantic indexing: {error}"))?;
        let revision_after = transaction
            .query_row(
                "SELECT data_revision FROM file_space_semantic_ann_state WHERE model_id = ?1",
                [SEMANTIC_MODEL_ID],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| format!("Unable to read updated semantic ANN revision: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("Unable to finish semantic indexing: {error}"))?;
        Ok(Some(SemanticAnnMutation {
            workspace_id,
            revision_before,
            revision_after,
            removed_keys,
            added_vectors,
        }))
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
    if let Ok(Some(mutation)) = &result {
        runtime.apply_ann_mutation(
            &mutation.workspace_id,
            mutation.revision_before,
            mutation.revision_after,
            &mutation.removed_keys,
            &mutation.added_vectors,
        );
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

#[cfg(test)]
pub(crate) fn index_one_document_with_installed_test_model(
    database: &Database,
    runtime: &SemanticSearchRuntime,
) -> Result<bool, String> {
    // Opt-in tests use existing model assets but an isolated workspace database.
    // No downloads, configured AI services, or user documents are accessed.
    ensure_model_loaded(runtime)?;
    process_next_document(database, runtime)
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
    let ann_ready = semantic_ann_is_ready(database, runtime);
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
    } else if counts.2 > 0 || counts.1 < counts.0 || !ann_ready {
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

fn backfill_ai_chunk_fts_batch(database: &Database) -> Result<bool, String> {
    let mut connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let cursor = connection
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [AI_CHUNK_FTS_BACKFILL_CURSOR_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("Unable to inspect AI passage search migration: {error}"))?
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    let rows = {
        let mut statement = connection
            .prepare(
                "SELECT rowid, id, file_id, body_text
                 FROM file_space_search_chunks
                 WHERE rowid > ?1
                 ORDER BY rowid
                 LIMIT ?2",
            )
            .map_err(|error| format!("Unable to prepare AI passage search migration: {error}"))?;
        statement
            .query_map(
                params![cursor, AI_CHUNK_FTS_BACKFILL_BATCH_SIZE as i64],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|error| format!("Unable to read AI passage search migration: {error}"))?
    };
    let Some(last_rowid) = rows.last().map(|row| row.0) else {
        return Ok(false);
    };
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Unable to begin AI passage search migration: {error}"))?;
    for (rowid, chunk_id, file_id, body_text) in rows {
        transaction
            .execute(
                "INSERT OR REPLACE INTO file_space_search_chunks_fts
                   (rowid, chunk_id, file_id, body_text)
                 VALUES (?1, ?2, ?3, ?4)",
                params![rowid, chunk_id, file_id, body_text],
            )
            .map_err(|error| format!("Unable to index an AI passage: {error}"))?;
    }
    transaction
        .execute(
            "INSERT INTO app_settings (key, value, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![
                AI_CHUNK_FTS_BACKFILL_CURSOR_KEY,
                last_rowid.to_string(),
                now_millis()
            ],
        )
        .map_err(|error| format!("Unable to save AI passage search migration: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("Unable to finish AI passage search migration: {error}"))?;
    Ok(true)
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
                let lexical_work_started_at = Instant::now();
                match lock_file_space_operations()
                    .and_then(|_operation| backfill_ai_chunk_fts_batch(database.inner()))
                {
                    Ok(true) => {
                        std::thread::sleep(semantic_index_cooldown(
                            lexical_work_started_at.elapsed(),
                        ));
                        continue;
                    }
                    Ok(false) => {}
                    Err(error) => {
                        eprintln!("Unable to update the AI passage search index: {error}");
                    }
                }
                let runtime = app.state::<SemanticSearchRuntime>();
                if !runtime.is_installed() || runtime.download_running.load(AtomicOrdering::Relaxed)
                {
                    std::thread::sleep(SEMANTIC_IDLE_PAUSE);
                    continue;
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
                            reconciled_workspace_id = Some(workspace_id.clone());
                        } else if let Err(error) =
                            reconcile_semantic_ann_index(database.inner(), runtime.inner())
                        {
                            runtime.set_progress(
                                "failed",
                                model_total_bytes(),
                                None,
                                Some(error.clone()),
                            );
                            eprintln!("Unable to reconcile the semantic ANN index: {error}");
                        } else {
                            runtime.set_progress("installed", model_total_bytes(), None, None);
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

#[derive(Debug, Clone)]
struct StoredAiChunk {
    chunk_id: String,
    file_id: String,
    file_name: String,
    relative_path: String,
    version_id: Option<String>,
    version_number: Option<i64>,
    ordinal: i64,
    body_text: String,
    vector_blob: Option<Vec<u8>>,
    dimensions: Option<i64>,
}

#[derive(Debug)]
struct RankedAiContextChunk {
    chunk: AiContextChunk,
    score: f32,
    dense_rank: Option<usize>,
    lexical_rank: Option<usize>,
    ordinal: i64,
}

fn stored_ai_chunk_from_row(
    row: &rusqlite::Row<'_>,
    offset: usize,
) -> rusqlite::Result<StoredAiChunk> {
    Ok(StoredAiChunk {
        chunk_id: row.get(offset)?,
        file_id: row.get(offset + 1)?,
        file_name: row.get(offset + 2)?,
        relative_path: row.get(offset + 3)?,
        version_id: row.get(offset + 4)?,
        version_number: row.get(offset + 5)?,
        ordinal: row.get(offset + 6)?,
        body_text: row.get(offset + 7)?,
        vector_blob: row.get(offset + 8)?,
        dimensions: row.get(offset + 9)?,
    })
}

fn semantic_ai_chunk_rows(
    database: &Database,
    ann_keys: &[u64],
) -> Result<Vec<(usize, StoredAiChunk)>, String> {
    if ann_keys.is_empty() {
        return Ok(Vec::new());
    }
    let candidate_values = ann_keys
        .iter()
        .enumerate()
        .map(|(rank, _)| format!("(?{}, {rank})", rank + 1))
        .collect::<Vec<_>>()
        .join(", ");
    let model_parameter = ann_keys.len() + 1;
    let sql = format!(
        "WITH candidates(ann_key, candidate_rank) AS (VALUES {candidate_values})
         SELECT candidates.candidate_rank,
                chunks.id, chunks.file_id, documents.file_name, files.storage_path,
                chunks.version_id, versions.version_number, chunks.ordinal,
                chunks.body_text, embeddings.vector, embeddings.dimensions
         FROM candidates
         JOIN file_space_semantic_ann_keys ann ON ann.ann_key = candidates.ann_key
         JOIN file_space_semantic_embeddings embeddings ON embeddings.chunk_id = ann.chunk_id
         JOIN file_space_search_chunks chunks ON chunks.id = ann.chunk_id
         JOIN file_space_search_documents documents
           ON documents.file_id = chunks.file_id
          AND documents.indexed_at = chunks.document_indexed_at
          AND documents.extraction_status = 'extracted'
         JOIN files ON files.id = chunks.file_id
         LEFT JOIN file_space_artifact_versions versions ON versions.id = chunks.version_id
         WHERE embeddings.model_id = ?{model_parameter}
           AND files.trashed_at IS NULL AND files.storage_path IS NOT NULL
         ORDER BY candidates.candidate_rank"
    );
    let mut parameters = ann_keys
        .iter()
        .map(|key| Value::Integer(*key as i64))
        .collect::<Vec<_>>();
    parameters.push(Value::Text(SEMANTIC_MODEL_ID.to_owned()));
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| format!("Unable to prepare AI semantic passage recall: {error}"))?;
    statement
        .query_map(params_from_iter(parameters), |row| {
            Ok((
                row.get::<_, i64>(0)?.max(0) as usize,
                stored_ai_chunk_from_row(row, 1)?,
            ))
        })
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .map_err(|error| format!("Unable to read AI semantic passages: {error}"))
}

fn dense_ai_context_chunks(
    database: &Database,
    runtime: &SemanticSearchRuntime,
    query: &str,
) -> Result<Vec<(StoredAiChunk, f32)>, String> {
    if !runtime.is_installed() || query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut fused = HashMap::<String, (StoredAiChunk, f32, f32)>::new();
    for variant in semantic_query_variants(query) {
        let Some(query_vector) = runtime.embed(&format!("query: {variant}"))? else {
            continue;
        };
        for (rank, (chunk, similarity)) in
            dense_ai_context_chunks_for_vector(database, runtime, &query_vector)?
                .into_iter()
                .enumerate()
        {
            let reciprocal_rank = 1.0 / (AI_RRF_K + rank as f32 + 1.0);
            match fused.entry(chunk.chunk_id.clone()) {
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    let (_, best_similarity, score) = entry.get_mut();
                    *best_similarity = best_similarity.max(similarity);
                    *score += reciprocal_rank;
                }
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert((chunk, similarity, reciprocal_rank));
                }
            }
        }
    }
    let mut ranked = fused.into_values().collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(Ordering::Equal)
            .then_with(|| right.2.partial_cmp(&left.2).unwrap_or(Ordering::Equal))
            .then_with(|| left.0.chunk_id.cmp(&right.0.chunk_id))
    });
    ranked.truncate(AI_DENSE_SEARCH_LIMIT);
    Ok(ranked
        .into_iter()
        .map(|(chunk, similarity, _)| (chunk, similarity))
        .collect())
}

fn is_cjk_character(character: char) -> bool {
    matches!(
        character,
        '\u{3400}'..='\u{4dbf}'
            | '\u{4e00}'..='\u{9fff}'
            | '\u{f900}'..='\u{faff}'
            | '\u{3040}'..='\u{30ff}'
            | '\u{ac00}'..='\u{d7af}'
    )
}

fn semantic_query_variants(query: &str) -> Vec<String> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let mut variants = vec![query.to_owned()];
    let Some(primary_clause) = query
        .split(|character| {
            matches!(
                character,
                ',' | '，' | '.' | '。' | '!' | '！' | '?' | '？' | ';' | '；' | '\n'
            )
        })
        .map(str::trim)
        .filter(|clause| !clause.is_empty())
        .max_by_key(|clause| clause.chars().count())
    else {
        return variants;
    };
    let focus = if primary_clause.chars().any(is_cjk_character) {
        let characters = primary_clause.chars().collect::<Vec<_>>();
        let focus_length = if characters.len() <= 6 {
            characters.len()
        } else if characters.len() <= 8 {
            (characters.len() * 2 / 3).max(4)
        } else {
            ((characters.len() * 2 + 2) / 3).clamp(4, 18)
        };
        characters[characters.len().saturating_sub(focus_length)..]
            .iter()
            .collect::<String>()
    } else {
        let words = primary_clause.split_whitespace().collect::<Vec<_>>();
        let focus_length = if words.len() <= 4 {
            words.len()
        } else {
            ((words.len() * 2 + 2) / 3).clamp(3, 12)
        };
        words[words.len().saturating_sub(focus_length)..].join(" ")
    };
    let focus = focus.trim();
    if !focus.is_empty() && focus != query {
        variants.push(focus.to_owned());
    }
    variants
}

fn dense_ai_context_chunks_for_vector(
    database: &Database,
    runtime: &SemanticSearchRuntime,
    query_vector: &[f32],
) -> Result<Vec<(StoredAiChunk, f32)>, String> {
    let location = database.location()?;
    let (data_revision, _) = semantic_ann_revisions(database)?;
    let Some(index) = runtime.ann_snapshot_for(&location.workspace_id, data_revision)? else {
        return Ok(Vec::new());
    };
    if index.size() == 0 {
        return Ok(Vec::new());
    }
    let matches = index
        .search(&query_vector, SEMANTIC_ANN_SEARCH_LIMIT.min(index.size()))
        .map_err(|error| format!("Unable to search AI semantic passages: {error}"))?;
    let rows = semantic_ai_chunk_rows(database, &matches.keys)?;
    let mut ranked = rows
        .into_iter()
        .filter_map(|(ann_rank, chunk)| {
            let similarity = match (&chunk.vector_blob, chunk.dimensions) {
                (Some(blob), Some(dimensions))
                    if dimensions == SEMANTIC_MODEL_DIMENSIONS as i64 =>
                {
                    vector_from_blob(blob, SEMANTIC_MODEL_DIMENSIONS)
                        .map(|vector| cosine_similarity(&query_vector, &vector))
                        .filter(|similarity| similarity.is_finite())
                }
                _ => None,
            }?;
            (similarity >= SEMANTIC_MIN_SIMILARITY).then_some((chunk, similarity, ann_rank))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.2.cmp(&right.2))
            .then_with(|| left.0.chunk_id.cmp(&right.0.chunk_id))
    });
    if let Some(best_similarity) = ranked.first().map(|result| result.1) {
        let relative_cutoff =
            (best_similarity - AI_DENSE_SCORE_WINDOW).max(SEMANTIC_MIN_SIMILARITY);
        ranked.retain(|result| result.1 >= relative_cutoff);
    }
    ranked.truncate(AI_DENSE_SEARCH_LIMIT);
    Ok(ranked
        .into_iter()
        .map(|(chunk, similarity, _)| (chunk, similarity))
        .collect())
}

fn lexical_ai_query_terms(queries: &[String]) -> Vec<String> {
    let mut terms = Vec::new();
    let mut seen = HashSet::new();
    for query in queries {
        for token in query
            .to_lowercase()
            .split(|character: char| !character.is_alphanumeric())
            .filter(|token| !token.is_empty())
        {
            let characters = token.chars().collect::<Vec<_>>();
            if characters.len() < 3 {
                continue;
            }
            if characters.iter().all(|character| character.is_ascii()) {
                if seen.insert(token.to_owned()) {
                    terms.push(token.to_owned());
                }
            } else {
                for window in characters.windows(3) {
                    let term = window.iter().collect::<String>();
                    if seen.insert(term.clone()) {
                        terms.push(term);
                    }
                    if terms.len() >= 48 {
                        break;
                    }
                }
            }
            if terms.len() >= 48 {
                break;
            }
        }
        if terms.len() >= 48 {
            break;
        }
    }
    terms
}

fn lexical_ai_context_chunks(
    database: &Database,
    queries: &[String],
) -> Result<Vec<StoredAiChunk>, String> {
    let terms = lexical_ai_query_terms(queries);
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let expression = format!(
        "body_text : ({})",
        terms
            .iter()
            .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" OR ")
    );
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let mut statement = connection
        .prepare(
            "SELECT chunks.id, chunks.file_id, documents.file_name, files.storage_path,
                    chunks.version_id, versions.version_number, chunks.ordinal,
                    chunks.body_text, embeddings.vector, embeddings.dimensions
             FROM file_space_search_chunks_fts
             JOIN file_space_search_chunks chunks
               ON chunks.rowid = file_space_search_chunks_fts.rowid
              AND chunks.id = file_space_search_chunks_fts.chunk_id
             JOIN file_space_search_documents documents
               ON documents.file_id = chunks.file_id
              AND documents.indexed_at = chunks.document_indexed_at
              AND documents.extraction_status = 'extracted'
             JOIN files ON files.id = chunks.file_id
             LEFT JOIN file_space_artifact_versions versions ON versions.id = chunks.version_id
             LEFT JOIN file_space_semantic_embeddings embeddings
               ON embeddings.chunk_id = chunks.id AND embeddings.model_id = ?2
             WHERE file_space_search_chunks_fts MATCH ?1
               AND files.trashed_at IS NULL AND files.storage_path IS NOT NULL
             ORDER BY bm25(file_space_search_chunks_fts)
             LIMIT ?3",
        )
        .map_err(|error| format!("Unable to prepare AI lexical passage recall: {error}"))?;
    statement
        .query_map(
            params![
                expression,
                SEMANTIC_MODEL_ID,
                AI_LEXICAL_SEARCH_LIMIT as i64
            ],
            |row| stored_ai_chunk_from_row(row, 0),
        )
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .map_err(|error| format!("Unable to read AI lexical passages: {error}"))
}

fn into_ai_context_chunk(
    chunk: StoredAiChunk,
    lexical_match: bool,
    semantic_similarity: Option<f32>,
) -> AiContextChunk {
    AiContextChunk {
        file_id: chunk.file_id,
        file_name: chunk.file_name,
        relative_path: chunk.relative_path,
        version_id: chunk.version_id,
        version_number: chunk.version_number,
        body_text: chunk.body_text.trim().to_owned(),
        lexical_match,
        semantic_similarity,
    }
}

pub(crate) fn retrieve_ai_context_chunks(
    database: &Database,
    runtime: &SemanticSearchRuntime,
    query: &str,
    lexical_queries: &[String],
    preferred_file_ids: &[String],
) -> Result<Vec<AiContextChunk>, String> {
    let started_at = Instant::now();
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let dense = dense_ai_context_chunks(database, runtime, query)?;
    let lexical = lexical_ai_context_chunks(database, lexical_queries)?;
    let dense_count = dense.len();
    let lexical_count = lexical.len();
    let preferred_files = preferred_file_ids.iter().collect::<HashSet<_>>();
    let mut fused = HashMap::<String, RankedAiContextChunk>::new();
    for (rank, (chunk, similarity)) in dense.into_iter().enumerate() {
        let chunk_id = chunk.chunk_id.clone();
        let ordinal = chunk.ordinal;
        let preferred_boost = if preferred_files.contains(&chunk.file_id) {
            1.0 / (AI_RRF_K + 1.0)
        } else {
            0.0
        };
        fused.insert(
            chunk_id,
            RankedAiContextChunk {
                chunk: into_ai_context_chunk(chunk, false, Some(similarity)),
                score: 1.0 / (AI_RRF_K + rank as f32 + 1.0) + preferred_boost,
                dense_rank: Some(rank),
                lexical_rank: None,
                ordinal,
            },
        );
    }
    for (rank, chunk) in lexical.into_iter().enumerate() {
        let chunk_id = chunk.chunk_id.clone();
        if let Some(existing) = fused.get_mut(&chunk_id) {
            existing.score += 1.0 / (AI_RRF_K + rank as f32 + 1.0);
            existing.lexical_rank = Some(rank);
            existing.chunk.lexical_match = true;
        } else {
            let ordinal = chunk.ordinal;
            let preferred_boost = if preferred_files.contains(&chunk.file_id) {
                1.0 / (AI_RRF_K + 1.0)
            } else {
                0.0
            };
            fused.insert(
                chunk_id,
                RankedAiContextChunk {
                    chunk: into_ai_context_chunk(chunk, true, None),
                    score: 1.0 / (AI_RRF_K + rank as f32 + 1.0) + preferred_boost,
                    dense_rank: None,
                    lexical_rank: Some(rank),
                    ordinal,
                },
            );
        }
    }
    let mut ranked = fused.into_values().collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| {
                left.dense_rank
                    .unwrap_or(usize::MAX)
                    .cmp(&right.dense_rank.unwrap_or(usize::MAX))
            })
            .then_with(|| {
                left.lexical_rank
                    .unwrap_or(usize::MAX)
                    .cmp(&right.lexical_rank.unwrap_or(usize::MAX))
            })
            .then_with(|| left.ordinal.cmp(&right.ordinal))
    });
    ranked.truncate(AI_FUSED_CANDIDATE_LIMIT);
    let mut per_file = HashMap::<String, usize>::new();
    let mut selected = Vec::new();
    for result in ranked {
        if selected.len() >= AI_CONTEXT_RESULT_LIMIT {
            break;
        }
        let count = per_file.entry(result.chunk.file_id.clone()).or_default();
        if *count >= AI_CONTEXT_PER_FILE_LIMIT {
            continue;
        }
        *count += 1;
        selected.push(result.chunk);
    }
    eprintln!(
        "Lume Trace AI retrieval: dense={dense_count}, lexical={lexical_count}, selected={}, elapsed_ms={}",
        selected.len(),
        started_at.elapsed().as_millis()
    );
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
    runtime.clear_ann_snapshot();
    if let Ok(location) = database.location() {
        let ann_index_path = semantic_ann_index_path_for_database(&location.database_path);
        if let Err(error) = fs::remove_file(&ann_index_path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                return Err(format!("Unable to remove the semantic ANN index: {error}"));
            }
        }
    }
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
    fn semantic_model_download_prefers_the_official_source() {
        assert_eq!(
            MODEL_SOURCES,
            [
                "https://huggingface.co/Xenova/multilingual-e5-small/resolve",
                "https://hf-mirror.com/Xenova/multilingual-e5-small/resolve",
            ]
        );
    }

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
    fn natural_language_queries_add_a_focused_semantic_variant() {
        let question = "帮我总结一下最近关于断流后恢复策略的调整，不要只列文件名。";
        let variants = semantic_query_variants(question);
        assert_eq!(variants.first().map(String::as_str), Some(question));
        assert_eq!(variants.len(), 2);
        assert!(variants[1].contains("断流后恢复策略"));
        assert!(!variants[1].contains("帮我总结一下"));

        assert_eq!(
            semantic_query_variants("找一下日报文件"),
            vec!["找一下日报文件".to_owned(), "日报文件".to_owned()]
        );
        assert_eq!(
            semantic_query_variants("Please find the latest daily report file"),
            vec![
                "Please find the latest daily report file".to_owned(),
                "the latest daily report file".to_owned()
            ]
        );
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
    fn ann_recall_preserves_ranked_chunks_instead_of_collapsing_files() {
        let root = std::env::temp_dir().join(format!("lumetrace-ann-recall-{}", Uuid::new_v4()));
        {
            let database = crate::database::open_for_test(&root.join("lumetrace.sqlite3")).unwrap();
            let mut near_vector = vec![0.0_f32; SEMANTIC_MODEL_DIMENSIONS];
            near_vector[0] = 1.0;
            let mut far_vector = vec![0.0_f32; SEMANTIC_MODEL_DIMENSIONS];
            far_vector[1] = 1.0;
            let connection = database.0.lock().unwrap();
            connection
                .execute_batch(
                    "INSERT INTO files
                       (id, original_name, storage_path, size_bytes, source_kind,
                        updated_at, created_at)
                     VALUES
                       ('near-file', 'strategy.md', 'strategy.md', 10, 'user_import', 10, 10),
                       ('far-file', 'unrelated.md', 'unrelated.md', 10, 'user_import', 10, 10);
                     INSERT INTO file_space_search_documents
                       (file_id, file_name, body_text, extraction_status, extraction_version,
                        tag_text, task_text, cell_text, file_updated_at, size_bytes, indexed_at)
                     VALUES
                       ('near-file', 'strategy.md', 'source body', 'extracted', 1,
                        '', '', '', 10, 10, 11),
                       ('far-file', 'unrelated.md', 'other body', 'extracted', 1,
                        '', '', '', 10, 10, 11);
                     INSERT INTO file_space_search_chunks
                       (id, file_id, document_indexed_at, content_hash, ordinal,
                        start_character, end_character, body_text, character_count, created_at)
                     VALUES
                       ('near-chunk', 'near-file', 11, 'near', 0, 0, 11, 'source body', 11, 11),
                       ('far-chunk', 'far-file', 11, 'far', 0, 0, 10, 'other body', 10, 11);",
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO file_space_semantic_embeddings
                       (chunk_id, model_id, dimensions, vector, created_at)
                     VALUES ('near-chunk', ?1, ?2, ?3, 11)",
                    params![
                        SEMANTIC_MODEL_ID,
                        SEMANTIC_MODEL_DIMENSIONS as i64,
                        vector_to_blob(&near_vector)
                    ],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO file_space_semantic_embeddings
                       (chunk_id, model_id, dimensions, vector, created_at)
                     VALUES ('far-chunk', ?1, ?2, ?3, 11)",
                    params![
                        SEMANTIC_MODEL_ID,
                        SEMANTIC_MODEL_DIMENSIONS as i64,
                        vector_to_blob(&far_vector)
                    ],
                )
                .unwrap();
            drop(connection);

            let runtime = SemanticSearchRuntime::new(&root);
            reconcile_semantic_ann_index(&database, &runtime).unwrap();
            let ranked =
                dense_ai_context_chunks_for_vector(&database, &runtime, &near_vector).unwrap();
            assert_eq!(ranked.len(), 1);
            assert_eq!(ranked[0].0.chunk_id, "near-chunk");
            assert_eq!(ranked[0].0.file_id, "near-file");
            assert!((ranked[0].1 - 1.0).abs() < 0.0001);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn chunk_fts_backfill_restores_existing_passages_in_bounded_batches() {
        let root =
            std::env::temp_dir().join(format!("lumetrace-chunk-fts-backfill-{}", Uuid::new_v4()));
        {
            let database = crate::database::open_for_test(&root.join("lumetrace.sqlite3")).unwrap();
            let connection = database.0.lock().unwrap();
            connection
                .execute_batch(
                    "INSERT INTO files
                       (id, original_name, storage_path, size_bytes, source_kind,
                        updated_at, created_at)
                     VALUES ('file-1', 'weekly.md', 'weekly.md', 32, 'user_import', 10, 10);
                     INSERT INTO file_space_search_documents
                       (file_id, file_name, body_text, extraction_status, extraction_version,
                        tag_text, task_text, cell_text, file_updated_at, size_bytes, indexed_at)
                     VALUES ('file-1', 'weekly.md', '断流恢复策略已调整', 'extracted', 1,
                             '', '', '', 10, 32, 11);
                     INSERT INTO file_space_search_chunks
                       (id, file_id, document_indexed_at, content_hash, ordinal,
                        start_character, end_character, body_text, character_count, created_at)
                     VALUES ('chunk-1', 'file-1', 11, 'hash', 0, 0, 9,
                             '断流恢复策略已调整', 9, 11);
                     DELETE FROM file_space_search_chunks_fts;",
                )
                .unwrap();
            let missing = connection
                .query_row(
                    "SELECT COUNT(*) FROM file_space_search_chunks_fts",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap();
            assert_eq!(missing, 0);
            drop(connection);

            assert!(backfill_ai_chunk_fts_batch(&database).unwrap());
            assert!(!backfill_ai_chunk_fts_batch(&database).unwrap());
            let restored = lexical_ai_context_chunks(&database, &["断流恢复".to_owned()]).unwrap();
            assert_eq!(restored.len(), 1);
            assert_eq!(restored[0].chunk_id, "chunk-1");
        }
        fs::remove_dir_all(root).unwrap();
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
            connection
                .execute(
                    "INSERT INTO file_space_search_chunks
                     (id, file_id, document_indexed_at, content_hash, ordinal,
                      start_character, end_character, body_text, character_count, created_at)
                     VALUES ('chunk-1', 'file-1', 11, 'chunk-1', 0, 0, 31,
                             'The release decision is Friday.', 31, 11)",
                    [],
                )
                .unwrap();
            drop(connection);
            let runtime = SemanticSearchRuntime::new(&root);
            let chunks = retrieve_ai_context_chunks(
                &database,
                &runtime,
                "release decision",
                &["release decision".to_owned()],
                &[],
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
    fn ai_context_returns_relevant_passages_without_scanning_candidate_files() {
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
                transaction
                    .execute(
                        "INSERT INTO file_space_search_chunks
                         (id, file_id, document_indexed_at, content_hash, ordinal,
                          start_character, end_character, body_text, character_count, created_at)
                         VALUES (?1, ?2, 11, ?1, 0, 0, ?4, ?3, ?4, 11)",
                        params![
                            format!("chunk-{index}"),
                            &file_id,
                            &body,
                            body.chars().count() as i64
                        ],
                    )
                    .unwrap();
            }
            transaction.commit().unwrap();
            drop(connection);

            let runtime = SemanticSearchRuntime::new(&root);
            let chunks = retrieve_ai_context_chunks(
                &database,
                &runtime,
                "断流恢复",
                &["断流恢复".to_owned()],
                &[],
            )
            .unwrap();
            let file_ids = chunks
                .iter()
                .map(|chunk| chunk.file_id.as_str())
                .collect::<HashSet<_>>();
            assert_eq!(file_ids.len(), 4);
            assert_eq!(chunks.len(), 4);
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
            let chunks = retrieve_ai_context_chunks(
                &database,
                &runtime,
                "断流恢复",
                &["断流恢复".to_owned()],
                &[],
            )
            .unwrap();
            assert_eq!(chunks.len(), 1);
            assert!(chunks[0].body_text.contains("必须参与综合分析"));
            assert!(chunks[0].lexical_match);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ai_context_does_not_scan_full_documents_while_chunk_indexing_is_pending() {
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
            let chunks = retrieve_ai_context_chunks(
                &database,
                &runtime,
                "断流恢复",
                &["断流恢复".to_owned()],
                &[],
            )
            .unwrap();
            assert!(chunks.is_empty());
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ai_context_has_hard_result_and_per_file_limits() {
        let root =
            std::env::temp_dir().join(format!("lumetrace-ai-context-limit-{}", Uuid::new_v4()));
        {
            let database = crate::database::open_for_test(&root.join("lumetrace.sqlite3")).unwrap();
            let mut connection = database.0.lock().unwrap();
            let transaction = connection.transaction().unwrap();
            for file_index in 0..4 {
                let file_id = format!("file-{file_index}");
                let file_name = format!("file-{file_index}.md");
                transaction
                    .execute(
                        "INSERT INTO files
                         (id, original_name, storage_path, size_bytes, source_kind,
                          updated_at, created_at)
                         VALUES (?1, ?2, ?2, 100, 'user_import', 10, 10)",
                        params![&file_id, &file_name],
                    )
                    .unwrap();
                transaction
                    .execute(
                        "INSERT INTO file_space_search_documents
                         (file_id, file_name, body_text, tag_text, task_text, cell_text,
                          extraction_status, extraction_version, file_updated_at,
                          size_bytes, indexed_at)
                         VALUES (?1, ?2, '断流恢复', '', '', '', 'extracted', 1, 10, 100, 11)",
                        params![&file_id, &file_name],
                    )
                    .unwrap();
                for ordinal in 0..4 {
                    let chunk_id = format!("chunk-{file_index}-{ordinal}");
                    let body = format!("断流恢复的第 {file_index}-{ordinal} 条证据");
                    transaction
                        .execute(
                            "INSERT INTO file_space_search_chunks
                             (id, file_id, document_indexed_at, content_hash, ordinal,
                              start_character, end_character, body_text, character_count, created_at)
                             VALUES (?1, ?2, 11, ?1, ?3, 0, ?4, ?5, ?4, 11)",
                            params![
                                chunk_id,
                                &file_id,
                                ordinal,
                                body.chars().count() as i64,
                                body
                            ],
                        )
                        .unwrap();
                }
            }
            transaction.commit().unwrap();
            drop(connection);

            let runtime = SemanticSearchRuntime::new(&root);
            let chunks = retrieve_ai_context_chunks(
                &database,
                &runtime,
                "断流恢复",
                &["断流恢复".to_owned()],
                &[],
            )
            .unwrap();
            assert!(chunks.len() <= AI_CONTEXT_RESULT_LIMIT);
            let mut per_file = HashMap::<String, usize>::new();
            for chunk in chunks {
                *per_file.entry(chunk.file_id).or_default() += 1;
            }
            assert!(per_file
                .values()
                .all(|count| *count <= AI_CONTEXT_PER_FILE_LIMIT));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ai_context_never_uses_stale_chunks_as_document_content() {
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
            let runtime = SemanticSearchRuntime::new(&root);
            let chunks = retrieve_ai_context_chunks(
                &database,
                &runtime,
                "confidential.xlsx",
                &["confidential.xlsx".to_owned()],
                &[],
            )
            .unwrap();
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
