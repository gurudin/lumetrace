//! Shared local indexing engine for caller-owned, isolated workspace replicas.
//! Transport, source revisions and authorization stay with the caller. No worker
//! is started here and no active local workspace or registration is changed.
pub use super::SemanticSearchStatus as Status;
pub const MODEL_ID: &str = SEMANTIC_MODEL_ID;
use super::*;
use crate::content_extractor::{self, ContentExtraction, ExtractionStatus};
pub use crate::file_space::{
    FileSpaceSearchMatch as SearchMatch, FileSpaceSearchRequest as SearchRequest,
};

#[derive(Clone, Debug)]
pub struct Source {
    pub id: String,
    pub name: String,
    /// Monotonic caller-owned revision, covering bytes, name, tags and lineage.
    pub revision: i64,
    pub updated_at: i64,
    pub size: i64,
    pub tags: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(revision: i64) -> Source {
        Source {
            id: "one".into(),
            name: "notes.md".into(),
            revision,
            updated_at: 1,
            size: 42,
            tags: "review".into(),
        }
    }
    fn fixture(root: &Path, shared: &SemanticSearchRuntime) -> Workspace {
        let w = Workspace::with_runtime(
            &root.join("index.sqlite3"),
            root.display().to_string(),
            shared,
        )
        .unwrap();
        w.database.0.lock().unwrap().execute("INSERT OR IGNORE INTO files(id,original_name,storage_path,created_at) VALUES('one','notes.md','notes.md',1)",[]).unwrap();
        w
    }
    fn supply(w: &Workspace, revision: i64, text: &str) -> bool {
        w.store_extraction(
            "one",
            revision,
            ContentExtraction {
                text: text.into(),
                status: ExtractionStatus::Extracted,
                error: None,
            },
        )
        .unwrap()
    }
    fn query(w: &Workspace, q: &str) -> Vec<SearchMatch> {
        w.search(&SearchRequest {
            query: q.into(),
            scopes: vec!["content".into()],
        })
        .unwrap()
    }
    fn mixed_sources(w: &Workspace) -> Vec<Source> {
        let mut sources = vec![source(1)];
        for n in 0..468 {
            let mut s = source(n + 2);
            s.id = format!("file-{n:03}");
            s.name = if n == 467 {
                "second.txt".into()
            } else {
                format!("image-{n:03}.png")
            };
            w.database.0.lock().unwrap().execute(
                "INSERT INTO files(id,original_name,storage_path,created_at) VALUES(?1,?2,?2,1)",
                params![s.id,s.name]).unwrap();
            sources.push(s);
        }
        sources
    }
    #[test]
    fn hundreds_of_metadata_only_files_complete_without_download_or_model_jobs() {
        let root = std::env::temp_dir().join(format!("lumetrace-skip-index-{}", Uuid::new_v4()));
        let w = fixture(&root, &SemanticSearchRuntime::new(&root));
        let mut sources = mixed_sources(&w);
        w.prepare(&sources).unwrap();
        let status = w.status().unwrap();
        assert_eq!(
            (
                status.indexed_files,
                status.total_files,
                status.pending_files
            ),
            (467, 469, 2)
        );
        assert_eq!(w.next_source().unwrap(), Some(("one".into(), 1)));
        // Upgrade an existing queue, including the exact old lock error. No
        // source revision changes or remote content reads are needed to repair it.
        w.database.0.lock().unwrap().execute("UPDATE file_space_index_jobs SET status='failed',error='Unable to replace semantic chunks: database is locked',retry_count=1",[]).unwrap();
        w.prepare(&sources).unwrap();
        let status = w.status().unwrap();
        assert_eq!((status.indexed_files, status.failed_files), (467, 0));
        let states: (i64,i64) = w.database.0.lock().unwrap().query_row(
            "SELECT count(*),sum(length(body_text)) FROM file_space_search_documents WHERE extraction_status='unsupported'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(states, (467, 0));
        let hits = w
            .search(&SearchRequest {
                query: "image-000".into(),
                scopes: vec!["name".into()],
            })
            .unwrap();
        assert!(hits.iter().any(|h| h.file_id == "file-000"));
        // An empty supported file is also complete without a pointless GET.
        sources[0].size = 0;
        sources[0].revision = 1000;
        w.prepare(&sources).unwrap();
        assert_eq!(w.status().unwrap().indexed_files, 468);
        w.clear_vectors().unwrap();
        assert_eq!(w.status().unwrap().indexed_files, 468);
        assert_eq!(w.next_source().unwrap(), Some(("file-467".into(), 469)));
        drop(w);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn index_job_claim_waits_for_replica_writer_without_read_to_write_upgrade_failure() {
        let root = std::env::temp_dir().join(format!("lumetrace-index-lock-{}", Uuid::new_v4()));
        let w = fixture(&root, &SemanticSearchRuntime::new(&root));
        w.prepare(&[source(1)]).unwrap();
        supply(&w, 1, "synthetic searchable text");
        let mut other = rusqlite::Connection::open(root.join("index.sqlite3")).unwrap();
        other.pragma_update(None, "journal_mode", "WAL").unwrap();
        let tx = other
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        tx.execute("UPDATE files SET updated_at=99", []).unwrap();
        std::thread::scope(|scope| {
            let (sent, received) = std::sync::mpsc::channel();
            let database = &w.database;
            scope.spawn(move || {
                sent.send(read_next_document(database).map(|d| d.map(|v| v.file_id)))
                    .unwrap();
            });
            assert!(
                matches!(
                    received.recv_timeout(Duration::from_millis(80)),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                ),
                "claim must wait, not fail immediately upgrading a stale read snapshot"
            );
            tx.commit().unwrap();
            assert_eq!(
                received
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .unwrap(),
                Some("one".into())
            );
        });
        drop((other, w));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn isolated_sources_invalidate_stale_content_and_do_not_repeat_unchanged_extraction() {
        let root =
            std::env::temp_dir().join(format!("lumetrace-isolated-index-{}", Uuid::new_v4()));
        let runtime = SemanticSearchRuntime::new(&root);
        let a = fixture(&root.join("a"), &runtime);
        let b = fixture(&root.join("b"), &runtime);
        a.prepare(&[source(1)]).unwrap();
        b.prepare(&[source(1)]).unwrap();
        assert!(supply(&a, 1, "unique_old_content retrieval sentinel"));
        a.prepare(&[source(1)]).unwrap();
        assert!(a.next_source().unwrap().is_none());
        assert!(!query(&a, "unique_old_content").is_empty());
        assert!(query(&b, "unique_old_content").is_empty());
        a.prepare(&[source(2)]).unwrap();
        assert!(query(&a, "unique_old_content").is_empty());
        assert!(!supply(&a, 1, "stale response"));
        assert!(supply(&a, 2, "replacement_new_content"));
        assert!(!query(&a, "replacement_new_content").is_empty());
        a.prepare(&[]).unwrap();
        assert!(query(&a, "replacement_new_content").is_empty());
        assert!(b.next_source().unwrap().is_some());
        drop((a, b));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    #[ignore = "requires explicitly supplied existing model assets; never downloads or reads user documents"]
    fn real_e5_isolated_owner_member_restart_and_version_invalidation() {
        let model = PathBuf::from(
            std::env::var_os("LUMETRACE_TEST_MODEL_DATA_DIR").expect("model assets required"),
        );
        let root = std::env::temp_dir().join(format!("lumetrace-isolated-e5-{}", Uuid::new_v4()));
        let owner_model = SemanticSearchRuntime::new(&model);
        let member_model = SemanticSearchRuntime::new(&model);
        let a = fixture(&root.join("owner"), &owner_model);
        let b = fixture(&root.join("member"), &member_model);
        for w in [&a, &b] {
            let sources = mixed_sources(w);
            w.prepare(&sources).unwrap();
            assert_eq!(w.status().unwrap().indexed_files, 467);
            supply(
                w,
                1,
                "You can reset your account password from the security settings page.",
            );
            assert!(w.step().unwrap());
            assert_eq!(w.status().unwrap().indexed_files, 468);
            assert_eq!(w.next_source().unwrap(), Some(("file-467".into(), 469)));
            assert!(w
                .store_extraction(
                    "file-467",
                    469,
                    ContentExtraction {
                        text: "A second synthetic document about team collaboration.".into(),
                        status: ExtractionStatus::Extracted,
                        error: None
                    }
                )
                .unwrap());
            assert!(w.step().unwrap());
            assert_eq!(w.status().unwrap().indexed_files, 469);
            let hits = query(w, "How can I change my password?");
            assert!(
                hits.iter().any(|h| h.semantic_similarity.is_some()),
                "real E5 must contribute semantic results: {hits:?}"
            );
        }
        a.prepare(&[source(2)]).unwrap();
        assert!(query(&a, "How can I change my password?").is_empty());
        assert!(!query(&b, "How can I change my password?").is_empty());
        b.clear_vectors().unwrap();
        assert!(query(&b, "How can I change my password?").is_empty());
        assert!(b.step().unwrap());
        assert!(!query(&b, "How can I change my password?").is_empty());
        drop(b);
        let b = fixture(&root.join("member"), &SemanticSearchRuntime::new(&model));
        assert!(!query(&b, "How can I change my password?").is_empty());
        b.prepare(&[]).unwrap();
        assert!(query(&b, "How can I change my password?").is_empty());
        drop((a, b));
        std::fs::remove_dir_all(root).unwrap();
    }
}

pub struct Workspace {
    database: Database,
    runtime: SemanticSearchRuntime,
}

// Metadata-only documents need neither a remote read nor E5 work. Complete them
// in one local transaction, including jobs left pending/failed by older builds.
// Keep their extraction status truthful; "ready" means the index is up to date,
// not that unsupported files acquired extracted body text or vectors.
fn complete_metadata_only(tx: &Transaction<'_>) -> Result<(), String> {
    tx.execute("UPDATE file_space_search_documents SET extraction_status='empty',body_text='',extraction_error=NULL
        WHERE size_bytes=0 AND extraction_status IN ('pending','failed')", [])
        .map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM file_space_search_chunks WHERE file_id IN
        (SELECT file_id FROM file_space_search_documents WHERE extraction_status IN ('unsupported','empty'))", [])
        .map_err(|e| e.to_string())?;
    tx.execute("INSERT INTO file_space_index_jobs
        (file_id,requested_document_indexed_at,status,retry_count,error,requested_at,started_at,completed_at)
        SELECT file_id,indexed_at,'ready',0,NULL,?1,NULL,?1 FROM file_space_search_documents
        WHERE extraction_status IN ('unsupported','empty')
        ON CONFLICT(file_id) DO UPDATE SET requested_document_indexed_at=excluded.requested_document_indexed_at,
        status='ready',retry_count=0,error=NULL,started_at=NULL,completed_at=excluded.completed_at
        WHERE file_space_index_jobs.status<>'ready' OR file_space_index_jobs.error IS NOT NULL
        OR file_space_index_jobs.requested_document_indexed_at<>excluded.requested_document_indexed_at",
        [now_millis()]).map_err(|e| e.to_string())?;
    Ok(())
}

/// Model lifecycle belongs to this device, not to an individual file space.
pub fn model_action(app: &tauri::AppHandle, action: &str) -> Result<Status, String> {
    let db = app.state::<Database>();
    let runtime = app.state::<SemanticSearchRuntime>();
    match action {
        "status" => Ok(status_record(db.inner(), runtime.inner())),
        "install" => install_semantic_search_model(app.clone(), db, runtime),
        "cancel" => Ok(cancel_semantic_search_model_download(db, runtime)),
        "remove" => remove_semantic_search_model(db, runtime),
        _ => Err("Unknown model action".into()),
    }
}

impl Workspace {
    pub fn open(app: &tauri::AppHandle, path: &Path, identity: String) -> Result<Self, String> {
        Self::with_runtime(path, identity, app.state::<SemanticSearchRuntime>().inner())
    }
    fn with_runtime(
        path: &Path,
        identity: String,
        shared: &SemanticSearchRuntime,
    ) -> Result<Self, String> {
        if !path.is_absolute() || identity.is_empty() {
            return Err("Invalid isolated workspace".into());
        }
        let database = crate::database::open_workspace_database(
            path.to_owned(),
            path.with_file_name("versions"),
            identity,
        )?;
        // One loaded E5 model per process; ANN snapshots remain space-specific.
        let runtime = SemanticSearchRuntime {
            model_directory: shared.model_directory.clone(),
            model: shared.model.clone(),
            progress: shared.progress.clone(),
            download_running: shared.download_running.clone(),
            cancel_download: shared.cancel_download.clone(),
            index_generation: shared.index_generation.clone(),
            ann_snapshot: Mutex::new(None),
        };
        reset_interrupted_jobs(&database)?;
        Ok(Self { database, runtime })
    }
    pub fn installed(&self) -> bool {
        self.runtime.is_installed() && !self.runtime.download_running.load(AtomicOrdering::Relaxed)
    }
    pub fn supports(name: &str) -> bool {
        content_extractor::supports_content(name)
    }

    /// Reconcile metadata before status/search/indexing. Invalidated chunks are
    /// removed in the same transaction, so an old version is never searchable.
    pub fn prepare(&self, sources: &[Source]) -> Result<(), String> {
        let mut db = self.database.0.lock().map_err(|_| "Index database busy")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let mut old: HashMap<String, i64> = tx
            .prepare("SELECT file_id,indexed_at FROM file_space_search_documents")
            .map_err(|e| e.to_string())?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<_>>()
            .map_err(|e| e.to_string())?;
        let mut seen = HashSet::new();
        for s in sources {
            if s.revision <= 0 || !seen.insert(&s.id) {
                return Err("Invalid source revision".into());
            }
            if old.remove(&s.id) == Some(s.revision) {
                continue;
            }
            tx.execute(
                "DELETE FROM file_space_search_chunks WHERE file_id=?1",
                [&s.id],
            )
            .map_err(|e| e.to_string())?;
            tx.execute(
                "DELETE FROM file_space_index_jobs WHERE file_id=?1",
                [&s.id],
            )
            .map_err(|e| e.to_string())?;
            let state = if !Self::supports(&s.name) {
                "unsupported"
            } else if s.size == 0 {
                "empty"
            } else {
                "pending"
            };
            tx.execute("INSERT INTO file_space_search_documents(file_id,file_name,body_text,extraction_status,extraction_error,extraction_version,tag_text,task_text,cell_text,file_updated_at,size_bytes,indexed_at)
                VALUES(?1,?2,'',?3,NULL,?4,?5,'','',?6,?7,?8) ON CONFLICT(file_id) DO UPDATE SET file_name=excluded.file_name,body_text='',extraction_status=excluded.extraction_status,extraction_error=NULL,extraction_version=excluded.extraction_version,tag_text=excluded.tag_text,task_text='',cell_text='',file_updated_at=excluded.file_updated_at,size_bytes=excluded.size_bytes,indexed_at=excluded.indexed_at",
                params![s.id,s.name,state,content_extractor::EXTRACTION_VERSION,s.tags,s.updated_at,s.size,s.revision]).map_err(|e|e.to_string())?;
        }
        for id in old.keys() {
            for table in [
                "file_space_search_chunks",
                "file_space_index_jobs",
                "file_space_search_documents",
            ] {
                tx.execute(&format!("DELETE FROM {table} WHERE file_id=?1"), [id])
                    .map_err(|e| e.to_string())?;
            }
        }
        complete_metadata_only(&tx)?;
        // Recover transient lock failures from previous builds without retrying
        // corrupt documents/model failures or looping forever on a locked DB.
        tx.execute("UPDATE file_space_index_jobs SET status='pending',error=NULL,started_at=NULL,completed_at=NULL,requested_at=?1
            WHERE status='failed' AND retry_count<3 AND
            (error LIKE '%database is locked' OR error LIKE '%database table is locked')", [now_millis()])
            .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn next_source(&self) -> Result<Option<(String, i64)>, String> {
        self.database.0.lock().map_err(|_|"Index database busy")?.query_row(
            "SELECT file_id,indexed_at FROM file_space_search_documents WHERE extraction_status='pending' ORDER BY indexed_at,file_id LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|e|e.to_string())
    }
    pub fn supply(&self, id: &str, revision: i64, name: &str, path: &Path) -> Result<bool, String> {
        self.store_extraction(
            id,
            revision,
            content_extractor::extract_file_content(path, name),
        )
    }
    pub fn fail(&self, id: &str, revision: i64, reason: &str) -> Result<bool, String> {
        self.store_extraction(
            id,
            revision,
            ContentExtraction {
                text: String::new(),
                status: ExtractionStatus::Failed,
                error: Some(reason.into()),
            },
        )
    }
    fn store_extraction(
        &self,
        id: &str,
        revision: i64,
        value: ContentExtraction,
    ) -> Result<bool, String> {
        let mut db = self.database.0.lock().map_err(|_| "Index database busy")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let changed=tx.execute("UPDATE file_space_search_documents SET body_text=?3,extraction_status=?4,extraction_error=?5 WHERE file_id=?1 AND indexed_at=?2 AND extraction_status='pending'",
            params![id,revision,value.text,value.status.as_str(),value.error]).map_err(|e|e.to_string())?;
        if changed > 0 {
            schedule_search_documents_in_transaction(&tx, &[id.into()], now_millis())?;
            complete_metadata_only(&tx)?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(changed > 0)
    }
    pub fn step(&self) -> Result<bool, String> {
        if !self.installed() {
            return Ok(false);
        }
        ensure_model_loaded(&self.runtime)?;
        let worked = process_next_document(&self.database, &self.runtime)?;
        reconcile_semantic_ann_index(&self.database, &self.runtime)?;
        Ok(worked)
    }
    pub fn retry(&self) -> Result<(), String> {
        self.database.0.lock().map_err(|_|"Index database busy")?.execute("UPDATE file_space_search_documents SET extraction_status='pending',extraction_error=NULL WHERE extraction_status='failed'",[]).map_err(|e|e.to_string())?;
        retry_failed_index_jobs(&self.database)
    }
    /// Removing the model from this space also removes its disposable local
    /// vectors, but retains extracted text and sources for a later reinstall.
    pub fn clear_vectors(&self) -> Result<(), String> {
        let mut db = self.database.0.lock().map_err(|_| "Index database busy")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM file_space_search_chunks", [])
            .map_err(|e| e.to_string())?;
        tx.execute("UPDATE file_space_index_jobs SET status='pending',retry_count=0,error=NULL,started_at=NULL,completed_at=NULL",[]).map_err(|e|e.to_string())?;
        complete_metadata_only(&tx)?;
        tx.commit().map_err(|e| e.to_string())?;
        self.runtime.clear_ann_snapshot();
        let path = semantic_ann_index_path_for_database(&self.database.location()?.database_path);
        if let Err(e) = std::fs::remove_file(path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                return Err(e.to_string());
            }
        }
        Ok(())
    }
    pub fn status(&self) -> Result<Status, String> {
        let mut status = status_record(&self.database, &self.runtime);
        let (pending,failed):(i64,i64)=self.database.0.lock().map_err(|_|"Index database busy")?.query_row(
            "SELECT COALESCE(SUM(extraction_status='pending'),0),COALESCE(SUM(extraction_status='failed'),0) FROM file_space_search_documents",[],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
        status.total_files += pending + failed;
        status.pending_files += pending;
        status.failed_files += failed;
        if failed > 0 && status.error.is_none() {
            status.error=self.database.0.lock().map_err(|_|"Index database busy")?.query_row("SELECT extraction_error FROM file_space_search_documents WHERE extraction_status='failed' ORDER BY indexed_at LIMIT 1",[],|r|r.get(0)).optional().map_err(|e|e.to_string())?.flatten();
        }
        if self.installed() && status.failed_files > 0 && status.pending_files == 0 {
            status.state = "failed".into();
        } else if self.installed() && status.pending_files > 0 {
            status.state = "indexing".into();
        }
        Ok(status)
    }
    pub fn search(&self, request: &SearchRequest) -> Result<Vec<SearchMatch>, String> {
        if request.query.chars().count() > 512 || request.scopes.len() > 8 {
            return Err("Search query is too long".into());
        }
        let lexical = crate::file_space::search_file_space_matches(&self.database, None, request)?
            .into_iter()
            .map(|v| v.file_id)
            .collect();
        let mut scores = HashMap::<String, f32>::new();
        if self.installed()
            && !request.query.trim().is_empty()
            && (request.scopes.is_empty() || request.scopes.iter().any(|s| s == "content"))
        {
            ensure_model_loaded(&self.runtime)?;
            reconcile_semantic_ann_index(&self.database, &self.runtime)?;
            if let Some(vector) = self
                .runtime
                .embed(&format!("query: {}", request.query.trim()))?
            {
                for (chunk, score) in
                    dense_ai_context_chunks_for_vector(&self.database, &self.runtime, &vector)?
                {
                    scores
                        .entry(chunk.file_id)
                        .and_modify(|s| *s = s.max(score))
                        .or_insert(score);
                }
            }
        }
        let mut semantic: Vec<_> = scores.into_iter().collect();
        semantic.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        Ok(merge_hybrid_matches(lexical, semantic)
            .into_iter()
            .map(|v| SearchMatch {
                file_id: v.file_id,
                lexical_match: v.lexical_match,
                semantic_similarity: v.semantic_similarity,
            })
            .collect())
    }
}
