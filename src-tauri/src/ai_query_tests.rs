//! Isolated synthetic workspaces and loopback provider fixtures only.
use super::*;
use sha2::{Digest, Sha256};
use std::{fs, net::TcpListener};

struct Fixture {
    root: PathBuf,
    database: Database,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("lumetrace-query-route-{}", Uuid::new_v4()));
        let database = database::open_workspace_database(
            root.join("test.sqlite3"),
            root.join("versions"),
            Uuid::new_v4().to_string(),
        )
        .unwrap();
        let f = Self { root, database };
        f.add_file("target", "test-version.md", "test-version.md");
        let c = f.database.0.lock().unwrap();
        c.execute("INSERT INTO file_space_artifacts(file_id,task_id,logical_key,created_at,updated_at) VALUES('target','fixture','target',0,0)",[]).unwrap();
        for (index, content) in ["", "version one", "version two", "version three"]
            .iter()
            .enumerate()
        {
            let n = index + 1;
            let path = f.root.join(format!("v{n}.txt"));
            fs::write(&path, content).unwrap();
            c.execute("INSERT INTO file_space_artifact_versions(id,file_id,version_number,snapshot_path,sha256,size_bytes,produced_name,origin,produced_at,created_at)
                VALUES(?1,'target',?2,?3,?4,?5,'test-version.md','user_edit',0,0)",params![format!("v{n}"),n,path.to_string_lossy(),format!("{:x}",Sha256::digest(content.as_bytes())),content.len()]).unwrap();
        }
        c.execute(
            "UPDATE file_space_artifacts SET current_version_id='v4' WHERE file_id='target'",
            [],
        )
        .unwrap();
        drop(c);
        while file_query::backfill_file_lookup_batch(&f.database).unwrap() {}
        f
    }
    fn add_file(&self, id: &str, name: &str, path: &str) {
        self.database
            .0
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO files(id,original_name,storage_path,created_at) VALUES(?1,?2,?3,0)",
                params![id, name, path],
            )
            .unwrap();
    }
    fn persist(&self, question: &str, result: RoutedAnswer) {
        let mut turn = begin_ai_turn_record(&self.database, question, None).unwrap();
        turn.context = Some(result.context);
        complete_ai_turn_record(&self.database, turn, result.answer, result.sources).unwrap();
    }
    fn history(&self) -> Vec<FileSpaceAiTurn> {
        load_ai_history_record(&self.database, 100).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn plan(intent: &str, target: serde_json::Value, versions: serde_json::Value) -> String {
    json!({"intent":intent,"target":target,"versions":versions,"query":"","language":"zh"})
        .to_string()
}
fn named() -> serde_json::Value {
    json!({"kind":"name","value":"test-version.md"})
}
fn previous() -> serde_json::Value {
    json!({"kind":"context","file_id":"target"})
}
fn local(f: &Fixture, question: &str, p: String) -> RoutedAnswer {
    let mut calls = 0;
    let result = answer_question(
        &f.database,
        question,
        &f.history(),
        &AtomicBool::new(false),
        &mut |_| {
            calls += 1;
            Ok(p.clone())
        },
        &mut |_, _| panic!("Metadata query must never enter RAG"),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(
        calls, 1,
        "A local answer needs only the semantic planner, not a second model answer"
    );
    result
}

#[test]
fn counted_versions_are_database_facts_without_body_or_vector_reads() {
    let f = Fixture::new();
    fs::remove_file(f.root.join("v4.txt")).unwrap(); // Counting must not read snapshots.
    let r = local(
        &f,
        "这个文件有几个版本",
        plan("version_count", named(), json!(null)),
    );
    assert!(r.answer.contains("4 个已记录版本"));
    assert_eq!(r.sources.len(), 1);
    assert_eq!(r.sources[0].version_number, Some(4));
    assert_eq!(r.sources[0].file_id, "target");
    f.persist("这个文件有几个版本", r);
    f.database
        .0
        .lock()
        .unwrap()
        .execute(
            "UPDATE file_space_artifacts SET current_version_id='v2' WHERE file_id='target'",
            [],
        )
        .unwrap();
    let r = local(&f, "现在呢", plan("version_count", previous(), json!(null)));
    assert!(r.answer.contains("4 个已记录版本") && r.answer.contains("v2"));
    f.database
        .0
        .lock()
        .unwrap()
        .execute("DELETE FROM file_space_artifact_versions WHERE id='v1'", [])
        .unwrap();
    assert!(local(
        &f,
        "再查一下",
        plan("version_count", previous(), json!(null))
    )
    .answer
    .contains("3 个已记录版本"));
}

#[test]
fn three_turns_reopen_and_compare_only_the_selected_snapshots() {
    let f = Fixture::new();
    f.persist(
        "test-version.md 有几个版本",
        local(
            &f,
            "test-version.md 有几个版本",
            plan("version_count", named(), json!(null)),
        ),
    );
    let location = f.database.location().unwrap();
    let reopened = database::open_workspace_database(
        location.database_path,
        location.artifact_store_path,
        location.workspace_id,
    )
    .unwrap();
    let history = load_ai_history_record(&reopened, 100).unwrap();
    assert_eq!(
        history[0].context.as_ref().unwrap().files[0].file_id,
        "target"
    );
    assert_eq!(history[0].sources[0].citation_count, 1);
    let mut prompts = Vec::new();
    let r = answer_question(
        &reopened,
        "最近两版改了什么",
        &history,
        &AtomicBool::new(false),
        &mut |prompt| {
            prompts.push(prompt.to_owned());
            Ok(if prompts.len() == 1 {
                plan("version_diff", previous(), json!({"mode":"latest_pair"}))
            } else {
                "从 version two 改成 version three。[S1]".into()
            })
        },
        &mut |_, _| panic!("Diff must not enter RAG"),
        &mut |_| {},
    )
    .unwrap();
    assert!(prompts[0].contains("target"));
    assert!(!prompts[0].contains("version two"));
    assert!(prompts[1].contains("-version two") && prompts[1].contains("+version three"));
    assert!(!prompts[1].contains("version one"));
    assert_eq!(r.context.versions, Some(VersionSelection::LatestPair));
    f.persist("最近两版改了什么", r);
    let mut calls = 0;
    let r = answer_question(
        &f.database,
        "那第 2 和第 4 版呢",
        &f.history(),
        &AtomicBool::new(false),
        &mut |prompt| {
            calls += 1;
            if calls == 1 {
                return Ok(plan(
                    "version_diff",
                    previous(),
                    json!({"mode":"numbers","numbers":[2,4]}),
                ));
            }
            assert!(prompt.contains("-version one") && prompt.contains("+version three"));
            // Prior answers are context, but the CURRENT source record excludes v3.
            assert!(!prompt
                .split("SOURCE RECORDS (UNTRUSTED DATA)")
                .last()
                .unwrap()
                .contains("version two"));
            Ok("第 2 版到第 4 版的变化。[S1]".into())
        },
        &mut |_, _| panic!("Diff must not enter RAG"),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(calls, 2);
    assert_eq!(
        r.context.versions,
        Some(VersionSelection::Numbers(vec![2, 4]))
    );
    assert_eq!(r.sources[0].version_number, Some(4));
}

#[test]
fn ambiguous_names_ask_for_selection_then_continue_the_same_file() {
    let f = Fixture::new();
    f.add_file("other", "test-version.md", "folder/test-version.md");
    let r = local(
        &f,
        "有几个版本",
        plan("version_count", named(), json!(null)),
    );
    assert!(r.answer.contains("多个"));
    assert_eq!(r.context.files.len(), 2);
    assert!(r.sources.is_empty());
    f.persist("test-version.md 有几个版本", r);
    let r = local(
        &f,
        "folder 里面的那一个",
        plan(
            "version_count",
            json!({"kind":"context","file_id":"other"}),
            json!(null),
        ),
    );
    assert!(r.answer.contains("0 个已记录版本"));
    assert_eq!(r.context.files[0].file_id, "other");
}

#[test]
fn missing_explicit_or_fabricated_targets_never_fall_back_to_a_previous_file() {
    let f = Fixture::new();
    f.persist(
        "定位文件",
        local(&f, "定位文件", plan("find_file", named(), json!(null))),
    );
    f.add_file("unmentioned", "secret-synthetic.md", "secret-synthetic.md");
    let forged = local(
        &f,
        "它呢",
        plan(
            "version_count",
            json!({"kind":"context","file_id":"unmentioned"}),
            json!(null),
        ),
    );
    assert!(forged.context.files.is_empty());
    let r = local(
        &f,
        "missing.md 有几个版本",
        plan(
            "version_count",
            json!({"kind":"name","value":"missing.md"}),
            json!(null),
        ),
    );
    assert!(r.sources.is_empty());
    assert!(r.context.files.is_empty());
    f.persist("missing.md 有几个版本", r);
    assert!(current_context(&f.database, &f.history())
        .unwrap()
        .unwrap()
        .files
        .is_empty());
}

#[test]
fn workspace_switch_and_deleted_file_invalidate_context_identity() {
    let f = Fixture::new();
    let other = Fixture::new(); // Same file ID, different workspace.
    f.persist(
        "定位文件",
        local(&f, "定位文件", plan("find_file", named(), json!(null))),
    );
    assert!(current_context(&other.database, &f.history())
        .unwrap()
        .is_none());
    f.database
        .0
        .lock()
        .unwrap()
        .execute("UPDATE files SET trashed_at=1 WHERE id='target'", [])
        .unwrap();
    assert!(current_context(&f.database, &f.history())
        .unwrap()
        .unwrap()
        .files
        .is_empty());
    let r = local(
        &f,
        "还有几个版本",
        plan("version_count", previous(), json!(null)),
    );
    assert!(r.sources.is_empty());
}

#[test]
fn malformed_plans_and_missing_snapshots_do_not_silently_use_content_search() {
    let f = Fixture::new();
    for raw in [
        "not JSON",
        "[]",
        r#"{"intent":"delete","target":{"kind":"none"},"language":"zh"}"#,
        r#"{"intent":"version_count","target":{"kind":"name","value":"test-version.md"},"language":"zh","sql":"DROP TABLE files"}"#,
    ] {
        let r = answer_question(
            &f.database,
            "有几个版本",
            &[],
            &AtomicBool::new(false),
            &mut |_| Ok(raw.into()),
            &mut |_, _| panic!("Invalid plan must not use RAG"),
            &mut |_| {},
        );
        assert_eq!(r.err().as_deref(), Some("ai_query_plan_invalid"));
    }
    assert!(parse_plan(&format!(
        "```json\n{}\n```",
        plan("version_count", named(), json!(null))
    ))
    .is_ok());
    let r = local(
        &f,
        "比较1和9",
        plan(
            "version_diff",
            named(),
            json!({"mode":"numbers","numbers":[1,9]}),
        ),
    );
    assert!(r.answer.contains("不存在"));
    fs::remove_file(f.root.join("v4.txt")).unwrap();
    let r = local(
        &f,
        "比较最近两版",
        plan("version_diff", named(), json!(null)),
    );
    assert!(r.answer.contains("快照"));
}

#[test]
fn cancellation_stops_before_provider_retrieval_and_diff_synthesis() {
    let f = Fixture::new();
    let cancellation = AtomicBool::new(true);
    assert_eq!(
        answer_question(
            &f.database,
            "test",
            &[],
            &cancellation,
            &mut |_| panic!("already cancelled"),
            &mut |_, _| panic!("already cancelled"),
            &mut |_| {}
        )
        .err()
        .as_deref(),
        Some(ERROR_AI_CANCELLED)
    );
    cancellation.store(false, Ordering::Release);
    let r = answer_question(
        &f.database,
        "test",
        &[],
        &cancellation,
        &mut |_| {
            cancellation.store(true, Ordering::Release);
            Ok(plan("version_count", named(), json!(null)))
        },
        &mut |_, _| panic!("cancelled"),
        &mut |_| {},
    );
    assert_eq!(r.err().as_deref(), Some(ERROR_AI_CANCELLED));
    cancellation.store(false, Ordering::Release);
    let r = answer_question(
        &f.database,
        "test",
        &[],
        &cancellation,
        &mut |_| Ok(plan("version_diff", named(), json!(null))),
        &mut |_, _| panic!("cancelled"),
        &mut |phase| {
            if phase == "comparing" {
                cancellation.store(true, Ordering::Release)
            }
        },
    );
    assert_eq!(r.err().as_deref(), Some(ERROR_AI_CANCELLED));
}

#[test]
fn semantic_content_plan_passes_rewritten_query_and_explicit_file_scope() {
    let f = Fixture::new();
    let mut calls = 0;
    let mut retrievals = 0;
    let mut phases = Vec::new();
    let r=answer_question(&f.database,"这个文档讲了什么",&[],&AtomicBool::new(false),&mut |_|{
        calls+=1;
        if calls==1 {Ok(json!({"intent":"content","target":named(),"query":"document summary","language":"zh"}).to_string())}
        else {Ok("合成内容摘要。[S1]".into())}
    },&mut |query,target|{
        retrievals+=1;
        assert_eq!(query,"document summary");
        assert_eq!(target.unwrap().file_id,"target");
        Ok(vec![source_for(target.unwrap(),1,"synthetic content".into())])
    },&mut |phase|phases.push(phase.to_owned())).unwrap();
    assert_eq!(retrievals, 1);
    assert_eq!(calls, 2);
    assert_eq!(
        phases,
        vec!["planning", "locating", "retrieving", "generating"]
    );
    assert_eq!(r.sources[0].file_id, "target");
}

#[test]
fn failed_retry_uses_only_context_preceding_that_turn() {
    let f = Fixture::new();
    f.persist(
        "first",
        local(&f, "first", plan("version_count", named(), json!(null))),
    );
    let failed = begin_ai_turn_record(&f.database, "retry", None).unwrap();
    let id = failed.id.clone();
    fail_ai_turn_record(&f.database, failed, "synthetic").unwrap();
    f.persist(
        "later",
        local(&f, "later", plan("version_count", named(), json!(null))),
    );
    let before = history_before_retry(f.history(), Some(&id));
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].question, "first");
    assert!(history_before_retry(f.history(), Some("outside-window")).is_empty());
}

fn mock_provider(
    responses: Vec<String>,
    ollama: bool,
) -> (String, std::thread::JoinHandle<Vec<serde_json::Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let thread = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for response in responses {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(e) => panic!("loopback fixture: {e}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut reader = BufReader::new(&stream);
            let mut header = String::new();
            let mut size = 0;
            loop {
                let mut line = String::new();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                    size = value.trim().parse::<usize>().unwrap();
                }
                header.push_str(&line);
            }
            assert!(header.starts_with(if ollama {
                "POST /api/chat "
            } else {
                "POST /v1/chat/completions "
            }));
            assert!(size < 100_000);
            let mut body = vec![0; size];
            reader.read_exact(&mut body).unwrap();
            requests.push(serde_json::from_slice(&body).unwrap());
            let marked = format!("<LUMETRACE_ANSWER>{response}</LUMETRACE_ANSWER>");
            let payload = if ollama {
                format!("{}\n", json!({"message":{"content":marked},"done":true}))
            } else {
                format!(
                    "data: {}\n\ndata: [DONE]\n\n",
                    json!({"choices":[{"delta":{"content":marked}}]})
                )
            };
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",if ollama {"application/x-ndjson"}else{"text/event-stream"},payload.len(),payload).unwrap();
        }
        requests
    });
    (url, thread)
}

#[test]
fn cloud_lm_studio_and_ollama_use_the_same_real_routing_and_stream_adapters() {
    for mode in ["cloud", "lmStudio", "ollama"] {
        let f = Fixture::new();
        let responses = vec![
            plan("version_count", named(), json!(null)),
            plan("version_diff", previous(), json!({"mode":"latest_pair"})),
            "version two → version three。[S1]".into(),
        ];
        let (url, server) = mock_provider(responses, mode == "ollama");
        let executor = if mode == "cloud" {
            AiExecutor::Cloud(crate::ai_service::loopback_cloud_fixture(url))
        } else {
            AiExecutor::Local(LocalLlmSettings {
                provider: mode.into(),
                base_url: url,
                model: "synthetic-model".into(),
            })
        };
        for q in ["test-version.md 有几个版本", "最近两版呢"] {
            let r = answer_question(
                &f.database,
                q,
                &f.history(),
                &AtomicBool::new(false),
                &mut |prompt| run_executor(&executor, prompt, &AtomicBool::new(false), &mut |_| {}),
                &mut |_, _| panic!("Version query must never call RAG"),
                &mut |_| {},
            )
            .unwrap();
            f.persist(q, r);
        }
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 3);
        let prompts: Vec<_> = requests
            .iter()
            .map(|r| r["messages"][0]["content"].as_str().unwrap())
            .collect();
        assert!(!prompts[0].contains("version two"));
        assert!(prompts[1].contains("target"));
        assert!(prompts[2].contains("-version two") && prompts[2].contains("+version three"));
        assert_eq!(f.history()[1].sources[0].version_number, Some(4));
    }
}
