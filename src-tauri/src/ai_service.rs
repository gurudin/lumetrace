use crate::{
    agent_cli::{load_agent_cli_settings_record, AgentCliSettings},
    database::Database,
};
use reqwest::{blocking::Client, Url};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, ErrorKind},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::State;

pub(crate) const AI_SERVICE_MODE_KEY: &str = "ai.service_mode";
pub(crate) const AI_SERVICE_MODE_AGENT_CLI: &str = "agentCli";
const AI_SERVICE_MODE_CLOUD: &str = "cloud";
const AI_SERVICE_MODE_LOCAL: &str = "local";
const CLOUD_AI_API_KEY_KEY: &str = "ai.cloud_api_key";
const CLOUD_AI_SETTINGS_KEY: &str = "ai.cloud";
const LOCAL_LLM_SETTINGS_KEY: &str = "ai.local_llm";
const CLOUD_AI_CONNECTION_TIMEOUT: Duration = Duration::from_secs(20);
const LOCAL_LLM_CONNECTION_TIMEOUT: Duration = Duration::from_secs(12);
const LOCAL_LLM_MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const THINKING_EMIT_INTERVAL: Duration = Duration::from_millis(100);
const ANSWER_START_MARKER: &str = "<LUMETRACE_ANSWER>";
const ANSWER_END_MARKER: &str = "</LUMETRACE_ANSWER>";

pub(crate) const ERROR_LOCAL_LLM_UNAVAILABLE: &str = "ai_local_llm_unavailable";
pub(crate) const ERROR_LOCAL_LLM_TIMEOUT: &str = "ai_local_llm_timeout";
pub(crate) const ERROR_LOCAL_LLM_FAILED: &str = "ai_local_llm_failed";
pub(crate) const ERROR_LOCAL_LLM_EMPTY: &str = "ai_local_llm_empty";
pub(crate) const ERROR_LOCAL_LLM_OUTPUT_TOO_LARGE: &str = "ai_local_llm_output_too_large";

const ERROR_LOCAL_LLM_INVALID_URL: &str = "local_llm_invalid_url";
const ERROR_LOCAL_LLM_CONNECTION_FAILED: &str = "local_llm_connection_failed";
const ERROR_LOCAL_LLM_NO_MODELS: &str = "local_llm_no_models";
const ERROR_CLOUD_AI_CONNECTION_FAILED: &str = "cloud_ai_connection_failed";
const ERROR_CLOUD_AI_INVALID_KEY: &str = "cloud_ai_invalid_key";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredCloudAiSettings {
    provider: String,
    base_url: String,
    model: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudAiConnectionRequest {
    provider: String,
    base_url: String,
    api_key: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudAiSettingsRequest {
    provider: String,
    base_url: String,
    api_key: Option<String>,
    model: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudAiSettingsSnapshot {
    provider: String,
    base_url: String,
    model: String,
    has_api_key: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalLlmSettings {
    pub(crate) provider: String,
    pub(crate) base_url: String,
    pub(crate) model: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalLlmConnectionRequest {
    provider: String,
    base_url: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalLlmConnectionResult {
    base_url: String,
    models: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiServiceSettingsSnapshot {
    mode: Option<String>,
    cloud: Option<CloudAiSettingsSnapshot>,
    local: Option<LocalLlmSettings>,
    agent_cli: Option<AgentCliSettings>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ActiveAiService {
    Local(LocalLlmSettings),
    AgentCli(AgentCliSettings),
}

#[derive(Debug, Default, PartialEq)]
struct LocalLlmStreamChunk {
    content: String,
    thinking: String,
    finish_reason: Option<String>,
    done: bool,
}

#[derive(Debug, Default)]
struct LocalLlmStreamResult {
    content: String,
    thinking: String,
    finish_reason: Option<String>,
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn validate_provider(provider: String) -> Result<String, String> {
    let provider = provider.trim();
    matches!(provider, "ollama" | "lmStudio")
        .then(|| provider.to_owned())
        .ok_or_else(|| ERROR_LOCAL_LLM_CONNECTION_FAILED.to_owned())
}

fn validate_cloud_provider(provider: String) -> Result<String, String> {
    let provider = provider.trim();
    (provider == "openai")
        .then(|| provider.to_owned())
        .ok_or_else(|| ERROR_CLOUD_AI_CONNECTION_FAILED.to_owned())
}

fn normalize_base_url(provider: &str, value: &str) -> Result<String, String> {
    let mut url = Url::parse(value.trim()).map_err(|_| ERROR_LOCAL_LLM_INVALID_URL.to_owned())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ERROR_LOCAL_LLM_INVALID_URL.to_owned());
    }
    let path = url.path().trim_end_matches('/').to_owned();
    if provider == "ollama" {
        let native_path = path.strip_suffix("/v1").unwrap_or(&path);
        url.set_path(native_path);
    } else if path.is_empty() {
        url.set_path("/v1");
    }
    let normalized = url.as_str().trim_end_matches('/').to_owned();
    if normalized.len() > 2_048 || normalized.chars().any(char::is_control) {
        return Err(ERROR_LOCAL_LLM_INVALID_URL.to_owned());
    }
    Ok(normalized)
}

fn validate_model(model: String) -> Result<String, String> {
    let model = model.trim();
    (!model.is_empty() && model.len() <= 512 && !model.chars().any(char::is_control))
        .then(|| model.to_owned())
        .ok_or_else(|| ERROR_LOCAL_LLM_NO_MODELS.to_owned())
}

fn validate_api_key(api_key: String) -> Result<String, String> {
    let api_key = api_key.trim();
    (api_key.len() >= 8 && api_key.len() <= 4_096 && !api_key.chars().any(char::is_control))
        .then(|| api_key.to_owned())
        .ok_or_else(|| ERROR_CLOUD_AI_INVALID_KEY.to_owned())
}

fn validate_stored_cloud_ai_settings(
    settings: StoredCloudAiSettings,
) -> Result<StoredCloudAiSettings, String> {
    let provider = validate_cloud_provider(settings.provider)?;
    Ok(StoredCloudAiSettings {
        base_url: normalize_base_url("lmStudio", &settings.base_url)?,
        provider,
        model: validate_model(settings.model)?,
    })
}

fn validate_local_llm_settings(settings: LocalLlmSettings) -> Result<LocalLlmSettings, String> {
    let provider = validate_provider(settings.provider)?;
    Ok(LocalLlmSettings {
        base_url: normalize_base_url(&provider, &settings.base_url)?,
        provider,
        model: validate_model(settings.model)?,
    })
}

fn read_setting(database: &Database, key: &str) -> Result<Option<String>, String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    connection
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("Unable to load the AI service setting: {error}"))
}

fn load_service_mode_record(database: &Database) -> Result<Option<String>, String> {
    let mode = read_setting(database, AI_SERVICE_MODE_KEY)?;
    match mode.as_deref() {
        Some(AI_SERVICE_MODE_CLOUD | AI_SERVICE_MODE_LOCAL | AI_SERVICE_MODE_AGENT_CLI) | None => {
            Ok(mode)
        }
        Some(_) => Err("Unsupported AI service mode".to_owned()),
    }
}

fn load_stored_cloud_ai_settings_record(
    database: &Database,
) -> Result<Option<StoredCloudAiSettings>, String> {
    read_setting(database, CLOUD_AI_SETTINGS_KEY)?
        .map(|value| {
            serde_json::from_str::<StoredCloudAiSettings>(&value)
                .map_err(|error| format!("Unable to read the cloud AI setting: {error}"))
                .and_then(validate_stored_cloud_ai_settings)
        })
        .transpose()
}

fn cloud_ai_is_configured(database: &Database) -> Result<bool, String> {
    let has_settings = load_stored_cloud_ai_settings_record(database)?.is_some();
    let has_api_key = read_setting(database, CLOUD_AI_API_KEY_KEY)?
        .is_some_and(|api_key| validate_api_key(api_key).is_ok());
    Ok(has_settings && has_api_key)
}

pub(crate) fn load_local_llm_settings_record(
    database: &Database,
) -> Result<Option<LocalLlmSettings>, String> {
    read_setting(database, LOCAL_LLM_SETTINGS_KEY)?
        .map(|value| {
            serde_json::from_str::<LocalLlmSettings>(&value)
                .map_err(|error| format!("Unable to read the local model setting: {error}"))
                .and_then(validate_local_llm_settings)
        })
        .transpose()
}

fn resolved_mode(
    stored_mode: Option<String>,
    cloud_configured: bool,
    local: &Option<LocalLlmSettings>,
    agent_cli: &Option<AgentCliSettings>,
) -> Option<String> {
    stored_mode.or_else(|| {
        if cloud_configured {
            Some(AI_SERVICE_MODE_CLOUD.to_owned())
        } else if local.is_some() {
            Some(AI_SERVICE_MODE_LOCAL.to_owned())
        } else if agent_cli.is_some() {
            Some(AI_SERVICE_MODE_AGENT_CLI.to_owned())
        } else {
            None
        }
    })
}

pub(crate) fn load_active_ai_service_record(
    database: &Database,
) -> Result<Option<ActiveAiService>, String> {
    let cloud_configured = cloud_ai_is_configured(database)?;
    let local = load_local_llm_settings_record(database)?;
    let agent_cli = load_agent_cli_settings_record(database)?;
    let mode = resolved_mode(
        load_service_mode_record(database)?,
        cloud_configured,
        &local,
        &agent_cli,
    );
    Ok(match mode.as_deref() {
        Some(AI_SERVICE_MODE_LOCAL) => local.map(ActiveAiService::Local),
        Some(AI_SERVICE_MODE_AGENT_CLI) => agent_cli.map(ActiveAiService::AgentCli),
        _ => None,
    })
}

fn resolve_cloud_api_key(database: &Database, api_key: Option<String>) -> Result<String, String> {
    match api_key.filter(|value| !value.trim().is_empty()) {
        Some(api_key) => validate_api_key(api_key),
        None => read_setting(database, CLOUD_AI_API_KEY_KEY)?
            .ok_or_else(|| ERROR_CLOUD_AI_INVALID_KEY.to_owned())
            .and_then(validate_api_key),
    }
}

fn save_cloud_ai_settings_record(
    database: &Database,
    request: CloudAiSettingsRequest,
) -> Result<CloudAiSettingsSnapshot, String> {
    let api_key = resolve_cloud_api_key(database, request.api_key)?;
    let settings = validate_stored_cloud_ai_settings(StoredCloudAiSettings {
        provider: request.provider,
        base_url: request.base_url,
        model: request.model,
    })?;
    let value = serde_json::to_string(&settings)
        .map_err(|error| format!("Unable to encode the cloud AI setting: {error}"))?;
    let mut connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Unable to begin saving the cloud AI setting: {error}"))?;
    let now = now_millis();
    for (key, setting_value) in [
        (CLOUD_AI_SETTINGS_KEY, value.as_str()),
        (CLOUD_AI_API_KEY_KEY, api_key.as_str()),
        (AI_SERVICE_MODE_KEY, AI_SERVICE_MODE_CLOUD),
    ] {
        transaction
            .execute(
                "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                params![key, setting_value, now],
            )
            .map_err(|error| format!("Unable to save the cloud AI setting: {error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("Unable to finish saving the cloud AI setting: {error}"))?;
    Ok(CloudAiSettingsSnapshot {
        provider: settings.provider,
        base_url: settings.base_url,
        model: settings.model,
        has_api_key: true,
    })
}

fn save_local_llm_settings_record(
    database: &Database,
    settings: LocalLlmSettings,
) -> Result<LocalLlmSettings, String> {
    let settings = validate_local_llm_settings(settings)?;
    let value = serde_json::to_string(&settings)
        .map_err(|error| format!("Unable to encode the local model setting: {error}"))?;
    let mut connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Unable to begin saving the local model setting: {error}"))?;
    let now = now_millis();
    transaction
        .execute(
            "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![LOCAL_LLM_SETTINGS_KEY, value, now],
        )
        .map_err(|error| format!("Unable to save the local model setting: {error}"))?;
    transaction
        .execute(
            "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![AI_SERVICE_MODE_KEY, AI_SERVICE_MODE_LOCAL, now],
        )
        .map_err(|error| format!("Unable to activate the local model setting: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("Unable to finish saving the local model setting: {error}"))?;
    Ok(settings)
}

fn models_endpoint(base_url: &str) -> String {
    format!("{}/models", base_url.trim_end_matches('/'))
}

fn chat_endpoint(base_url: &str) -> String {
    format!("{}/chat/completions", base_url.trim_end_matches('/'))
}

fn ollama_endpoint(base_url: &str, endpoint: &str) -> Result<String, String> {
    let mut url = Url::parse(base_url).map_err(|_| ERROR_LOCAL_LLM_UNAVAILABLE.to_owned())?;
    let path = url.path().trim_end_matches('/');
    url.set_path(&format!("{path}/api/{endpoint}"));
    Ok(url.as_str().to_owned())
}

fn ollama_chat_endpoint(base_url: &str) -> Result<String, String> {
    ollama_endpoint(base_url, "chat")
}

fn ollama_models_endpoint(base_url: &str) -> Result<String, String> {
    ollama_endpoint(base_url, "tags")
}

fn parse_model_list(body: &[u8]) -> Result<Vec<String>, String> {
    let document = serde_json::from_slice::<Value>(body)
        .map_err(|_| ERROR_LOCAL_LLM_CONNECTION_FAILED.to_owned())?;
    let mut models = document
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("id").and_then(Value::as_str))
        .filter_map(|model| validate_model(model.to_owned()).ok())
        .collect::<Vec<_>>();
    models.sort_by_key(|model| model.to_lowercase());
    models.dedup();
    if models.is_empty() {
        Err(ERROR_LOCAL_LLM_NO_MODELS.to_owned())
    } else {
        models.truncate(1_000);
        Ok(models)
    }
}

fn parse_ollama_model_list(body: &[u8]) -> Result<Vec<String>, String> {
    let document = serde_json::from_slice::<Value>(body)
        .map_err(|_| ERROR_LOCAL_LLM_CONNECTION_FAILED.to_owned())?;
    let mut models = document
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            entry
                .get("name")
                .or_else(|| entry.get("model"))
                .and_then(Value::as_str)
        })
        .filter_map(|model| validate_model(model.to_owned()).ok())
        .collect::<Vec<_>>();
    models.sort_by_key(|model| model.to_lowercase());
    models.dedup();
    if models.is_empty() {
        Err(ERROR_LOCAL_LLM_NO_MODELS.to_owned())
    } else {
        models.truncate(1_000);
        Ok(models)
    }
}

fn read_bounded_response(
    response: reqwest::blocking::Response,
    limit: usize,
    error_code: &str,
) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|content_length| content_length > limit as u64)
    {
        return Err(error_code.to_owned());
    }
    let bytes = response.bytes().map_err(|_| error_code.to_owned())?;
    if bytes.len() > limit {
        return Err(error_code.to_owned());
    }
    Ok(bytes.to_vec())
}

fn request_local_models(
    provider: String,
    base_url: String,
) -> Result<LocalLlmConnectionResult, String> {
    let provider = validate_provider(provider)?;
    let base_url = normalize_base_url(&provider, &base_url)?;
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(LOCAL_LLM_CONNECTION_TIMEOUT)
        .build()
        .map_err(|_| ERROR_LOCAL_LLM_CONNECTION_FAILED.to_owned())?;
    let endpoint = if provider == "ollama" {
        ollama_models_endpoint(&base_url)?
    } else {
        models_endpoint(&base_url)
    };
    let response = client
        .get(endpoint)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|_| ERROR_LOCAL_LLM_CONNECTION_FAILED.to_owned())?;
    let body = read_bounded_response(response, 1024 * 1024, ERROR_LOCAL_LLM_CONNECTION_FAILED)?;
    let models = if provider == "ollama" {
        parse_ollama_model_list(&body)?
    } else {
        parse_model_list(&body)?
    };
    Ok(LocalLlmConnectionResult { base_url, models })
}

fn request_cloud_models(
    provider: String,
    base_url: String,
    api_key: String,
) -> Result<LocalLlmConnectionResult, String> {
    validate_cloud_provider(provider)?;
    let base_url = normalize_base_url("lmStudio", &base_url)?;
    let api_key = validate_api_key(api_key)?;
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(CLOUD_AI_CONNECTION_TIMEOUT)
        .build()
        .map_err(|_| ERROR_CLOUD_AI_CONNECTION_FAILED.to_owned())?;
    let response = client
        .get(models_endpoint(&base_url))
        .bearer_auth(api_key)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|_| ERROR_CLOUD_AI_CONNECTION_FAILED.to_owned())?;
    let body = read_bounded_response(response, 1024 * 1024, ERROR_CLOUD_AI_CONNECTION_FAILED)?;
    Ok(LocalLlmConnectionResult {
        base_url,
        models: parse_model_list(&body).map_err(|_| ERROR_CLOUD_AI_CONNECTION_FAILED.to_owned())?,
    })
}

fn message_content(message: &Value) -> String {
    match message.get("content") {
        Some(Value::String(content)) => content.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| match part {
                Value::String(text) => Some(text.as_str()),
                Value::Object(_) => part.get("text").and_then(Value::as_str),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn strip_think_blocks(content: &str) -> String {
    let mut remaining = content;
    let mut output = String::with_capacity(content.len());
    loop {
        let lowercase = remaining.to_ascii_lowercase();
        let Some(start) = lowercase.find("<think>") else {
            output.push_str(remaining);
            break;
        };
        output.push_str(&remaining[..start]);
        let reasoning = &remaining[start + "<think>".len()..];
        let reasoning_lowercase = reasoning.to_ascii_lowercase();
        let Some(end) = reasoning_lowercase.find("</think>") else {
            break;
        };
        remaining = &reasoning[end + "</think>".len()..];
    }
    output.replace("</think>", "").trim().to_owned()
}

fn extract_marked_or_plain_answer(content: &str) -> Option<String> {
    let content = strip_think_blocks(content);
    if content.is_empty() {
        return None;
    }
    if let Some(start) = content.rfind(ANSWER_START_MARKER) {
        let answer_start = start + ANSWER_START_MARKER.len();
        if let Some(relative_end) = content[answer_start..].find(ANSWER_END_MARKER) {
            let answer = content[answer_start..answer_start + relative_end].trim();
            return (!answer.is_empty()).then(|| answer.to_owned());
        }
    }
    let answer = content
        .replace(ANSWER_START_MARKER, "")
        .replace(ANSWER_END_MARKER, "");
    let answer = answer.trim();
    (!answer.is_empty()).then(|| answer.to_owned())
}

fn parse_chat_answer(body: &[u8]) -> Result<String, String> {
    let document =
        serde_json::from_slice::<Value>(body).map_err(|_| ERROR_LOCAL_LLM_FAILED.to_owned())?;
    let choice = document
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .ok_or_else(|| ERROR_LOCAL_LLM_EMPTY.to_owned())?;
    let content = choice
        .get("message")
        .map(message_content)
        .filter(|content| !content.trim().is_empty())
        .or_else(|| {
            choice
                .get("text")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_default();
    extract_marked_or_plain_answer(&content).ok_or_else(|| ERROR_LOCAL_LLM_EMPTY.to_owned())
}

fn reasoning_content(value: &Value) -> String {
    ["reasoning", "reasoning_content", "thinking"]
        .iter()
        .filter_map(|key| value.get(*key).and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("")
}

fn parse_openai_stream_line(line: &str) -> Result<Option<LocalLlmStreamChunk>, String> {
    let Some(payload) = line.trim().strip_prefix("data:").map(str::trim) else {
        return Ok(None);
    };
    if payload == "[DONE]" {
        return Ok(Some(LocalLlmStreamChunk {
            done: true,
            ..LocalLlmStreamChunk::default()
        }));
    }
    let document =
        serde_json::from_str::<Value>(payload).map_err(|_| ERROR_LOCAL_LLM_FAILED.to_owned())?;
    let Some(choice) = document
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
    else {
        return Ok(None);
    };
    let delta = choice
        .get("delta")
        .or_else(|| choice.get("message"))
        .unwrap_or(&Value::Null);
    Ok(Some(LocalLlmStreamChunk {
        content: message_content(delta),
        thinking: reasoning_content(delta),
        finish_reason: choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .map(str::to_owned),
        done: choice
            .get("finish_reason")
            .is_some_and(|reason| !reason.is_null()),
    }))
}

fn parse_ollama_stream_line(line: &str) -> Result<Option<LocalLlmStreamChunk>, String> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(None);
    }
    let document =
        serde_json::from_str::<Value>(line).map_err(|_| ERROR_LOCAL_LLM_FAILED.to_owned())?;
    let message = document.get("message").unwrap_or(&Value::Null);
    Ok(Some(LocalLlmStreamChunk {
        content: message_content(message),
        thinking: reasoning_content(message),
        finish_reason: document
            .get("done_reason")
            .and_then(Value::as_str)
            .map(str::to_owned),
        done: document
            .get("done")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }))
}

fn stream_read_error(error: std::io::Error) -> String {
    if matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) {
        ERROR_LOCAL_LLM_TIMEOUT.to_owned()
    } else {
        ERROR_LOCAL_LLM_FAILED.to_owned()
    }
}

fn apply_stream_chunk(
    result: &mut LocalLlmStreamResult,
    chunk: LocalLlmStreamChunk,
    last_thinking_emit: &mut Option<Instant>,
    on_thinking: &mut dyn FnMut(&str),
) -> Result<bool, String> {
    result.content.push_str(&chunk.content);
    if !chunk.thinking.is_empty() {
        result.thinking.push_str(&chunk.thinking);
        if result.content.is_empty()
            && last_thinking_emit
                .is_none_or(|last_emit| last_emit.elapsed() >= THINKING_EMIT_INTERVAL)
        {
            on_thinking(&result.thinking);
            *last_thinking_emit = Some(Instant::now());
        }
    }
    if let Some(finish_reason) = chunk.finish_reason {
        result.finish_reason = Some(finish_reason);
    }
    Ok(chunk.done)
}

fn openai_chat_request_payload(settings: &LocalLlmSettings, prompt: &str) -> Value {
    json!({
        "model": settings.model,
        "messages": [{ "role": "user", "content": prompt }],
        "temperature": 0.2,
        "reasoning_effort": "none",
        "stream": true,
    })
}

fn ollama_chat_request_payload(settings: &LocalLlmSettings, prompt: &str) -> Value {
    json!({
        "model": settings.model,
        "messages": [{ "role": "user", "content": prompt }],
        "stream": true,
        "think": false,
        "options": {
            "temperature": 0.2,
        },
    })
}

fn run_openai_stream(
    client: &Client,
    settings: &LocalLlmSettings,
    prompt: &str,
    on_thinking: &mut dyn FnMut(&str),
) -> Result<LocalLlmStreamResult, String> {
    let request_body = serde_json::to_vec(&openai_chat_request_payload(settings, prompt))
        .map_err(|_| ERROR_LOCAL_LLM_FAILED.to_owned())?;
    let response = client
        .post(chat_endpoint(&settings.base_url))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(request_body)
        .send()
        .map_err(|error| {
            if error.is_timeout() {
                ERROR_LOCAL_LLM_TIMEOUT.to_owned()
            } else if error.is_connect() {
                ERROR_LOCAL_LLM_UNAVAILABLE.to_owned()
            } else {
                ERROR_LOCAL_LLM_FAILED.to_owned()
            }
        })?
        .error_for_status()
        .map_err(|_| ERROR_LOCAL_LLM_FAILED.to_owned())?;
    let is_event_stream = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|content_type| content_type.contains("text/event-stream"));
    if !is_event_stream {
        let body = read_bounded_response(
            response,
            LOCAL_LLM_MAX_RESPONSE_BYTES,
            ERROR_LOCAL_LLM_OUTPUT_TOO_LARGE,
        )?;
        return Ok(LocalLlmStreamResult {
            content: parse_chat_answer(&body)?,
            ..LocalLlmStreamResult::default()
        });
    }

    let mut reader = BufReader::new(response);
    let mut line = String::new();
    let mut result = LocalLlmStreamResult::default();
    let mut last_thinking_emit = None;
    loop {
        line.clear();
        if reader.read_line(&mut line).map_err(stream_read_error)? == 0 {
            break;
        }
        let Some(chunk) = parse_openai_stream_line(&line)? else {
            continue;
        };
        if apply_stream_chunk(&mut result, chunk, &mut last_thinking_emit, on_thinking)? {
            break;
        }
    }
    if result.content.is_empty() && !result.thinking.is_empty() {
        on_thinking(&result.thinking);
    }
    Ok(result)
}

fn run_ollama_stream(
    client: &Client,
    settings: &LocalLlmSettings,
    prompt: &str,
    on_thinking: &mut dyn FnMut(&str),
) -> Result<LocalLlmStreamResult, String> {
    let request_body = serde_json::to_vec(&ollama_chat_request_payload(settings, prompt))
        .map_err(|_| ERROR_LOCAL_LLM_FAILED.to_owned())?;
    let response = client
        .post(ollama_chat_endpoint(&settings.base_url)?)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(request_body)
        .send()
        .map_err(|error| {
            if error.is_timeout() {
                ERROR_LOCAL_LLM_TIMEOUT.to_owned()
            } else if error.is_connect() {
                ERROR_LOCAL_LLM_UNAVAILABLE.to_owned()
            } else {
                ERROR_LOCAL_LLM_FAILED.to_owned()
            }
        })?
        .error_for_status()
        .map_err(|_| ERROR_LOCAL_LLM_FAILED.to_owned())?;
    let mut reader = BufReader::new(response);
    let mut line = String::new();
    let mut result = LocalLlmStreamResult::default();
    let mut last_thinking_emit = None;
    loop {
        line.clear();
        if reader.read_line(&mut line).map_err(stream_read_error)? == 0 {
            break;
        }
        let Some(chunk) = parse_ollama_stream_line(&line)? else {
            continue;
        };
        let done = apply_stream_chunk(&mut result, chunk, &mut last_thinking_emit, on_thinking)?;
        if done {
            break;
        }
    }
    if result.content.is_empty() && !result.thinking.is_empty() {
        on_thinking(&result.thinking);
    }
    Ok(result)
}

fn final_stream_answer(result: LocalLlmStreamResult) -> Result<String, String> {
    extract_marked_or_plain_answer(&result.content).ok_or_else(|| ERROR_LOCAL_LLM_EMPTY.to_owned())
}

pub(crate) fn run_local_llm(
    settings: &LocalLlmSettings,
    prompt: &str,
    on_thinking: &mut dyn FnMut(&str),
) -> Result<String, String> {
    let settings = validate_local_llm_settings(settings.clone())
        .map_err(|_| ERROR_LOCAL_LLM_UNAVAILABLE.to_owned())?;
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .build()
        .map_err(|_| ERROR_LOCAL_LLM_UNAVAILABLE.to_owned())?;
    if settings.provider == "ollama" {
        final_stream_answer(run_ollama_stream(&client, &settings, prompt, on_thinking)?)
    } else {
        final_stream_answer(run_openai_stream(&client, &settings, prompt, on_thinking)?)
    }
}

#[tauri::command]
pub fn get_ai_service_settings(
    database: State<'_, Database>,
) -> Result<AiServiceSettingsSnapshot, String> {
    let cloud = load_stored_cloud_ai_settings_record(database.inner())?;
    let cloud_api_key_exists = read_setting(database.inner(), CLOUD_AI_API_KEY_KEY)?
        .is_some_and(|api_key| validate_api_key(api_key).is_ok());
    let cloud_configured = cloud_ai_is_configured(database.inner())?;
    let local = load_local_llm_settings_record(database.inner())?;
    let agent_cli = load_agent_cli_settings_record(database.inner())?;
    let mode = resolved_mode(
        load_service_mode_record(database.inner())?,
        cloud_configured,
        &local,
        &agent_cli,
    );
    Ok(AiServiceSettingsSnapshot {
        mode,
        cloud: cloud.map(|settings| CloudAiSettingsSnapshot {
            provider: settings.provider,
            base_url: settings.base_url,
            model: settings.model,
            has_api_key: cloud_api_key_exists,
        }),
        local,
        agent_cli,
    })
}

#[tauri::command]
pub async fn check_cloud_ai_connection(
    request: CloudAiConnectionRequest,
    database: State<'_, Database>,
) -> Result<LocalLlmConnectionResult, String> {
    let api_key = resolve_cloud_api_key(database.inner(), request.api_key)?;
    tauri::async_runtime::spawn_blocking(move || {
        request_cloud_models(request.provider, request.base_url, api_key)
    })
    .await
    .map_err(|_| ERROR_CLOUD_AI_CONNECTION_FAILED.to_owned())?
}

#[tauri::command]
pub fn save_cloud_ai_settings(
    settings: CloudAiSettingsRequest,
    database: State<'_, Database>,
) -> Result<CloudAiSettingsSnapshot, String> {
    save_cloud_ai_settings_record(database.inner(), settings)
}

#[tauri::command]
pub async fn check_local_llm_connection(
    request: LocalLlmConnectionRequest,
) -> Result<LocalLlmConnectionResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        request_local_models(request.provider, request.base_url)
    })
    .await
    .map_err(|_| ERROR_LOCAL_LLM_CONNECTION_FAILED.to_owned())?
}

#[tauri::command]
pub fn save_local_llm_settings(
    settings: LocalLlmSettings,
    database: State<'_, Database>,
) -> Result<LocalLlmSettings, String> {
    save_local_llm_settings_record(database.inner(), settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn base_urls_are_normalized_and_unsafe_shapes_are_rejected() {
        assert_eq!(
            normalize_base_url("ollama", " http://192.168.1.10:11434 ").unwrap(),
            "http://192.168.1.10:11434"
        );
        assert_eq!(
            normalize_base_url("ollama", "http://192.168.1.10:11434/v1").unwrap(),
            "http://192.168.1.10:11434"
        );
        assert_eq!(
            normalize_base_url("lmStudio", "http://127.0.0.1:1234/v1/").unwrap(),
            "http://127.0.0.1:1234/v1"
        );
        assert!(normalize_base_url("ollama", "file:///tmp/model").is_err());
        assert!(normalize_base_url("ollama", "http://user:secret@127.0.0.1:11434/v1").is_err());
        assert!(normalize_base_url("ollama", "http://127.0.0.1:11434/v1?token=secret").is_err());
    }

    #[test]
    fn openai_compatible_model_lists_are_sorted_and_deduplicated() {
        let models = parse_model_list(
            br#"{"data":[{"id":"qwen3.5:27b"},{"id":"Qwen3:4b"},{"id":"qwen3.5:27b"}]}"#,
        )
        .unwrap();
        assert_eq!(models, vec!["qwen3.5:27b", "Qwen3:4b"]);
        assert_eq!(
            parse_model_list(br#"{"data":[]}"#),
            Err(ERROR_LOCAL_LLM_NO_MODELS.to_owned())
        );
    }

    #[test]
    fn ollama_model_lists_are_read_from_the_native_tags_response() {
        let models = parse_ollama_model_list(
            br#"{"models":[{"name":"qwen3.5:27b"},{"model":"Qwen3:4b"},{"name":"qwen3.5:27b"}]}"#,
        )
        .unwrap();
        assert_eq!(models, vec!["qwen3.5:27b", "Qwen3:4b"]);
        assert_eq!(
            parse_ollama_model_list(br#"{"models":[]}"#),
            Err(ERROR_LOCAL_LLM_NO_MODELS.to_owned())
        );
    }

    #[test]
    fn ollama_uses_native_model_and_chat_endpoints() {
        assert_eq!(
            ollama_models_endpoint("http://192.168.1.10:11434").unwrap(),
            "http://192.168.1.10:11434/api/tags"
        );
        assert_eq!(
            ollama_chat_endpoint("http://192.168.1.10:11434").unwrap(),
            "http://192.168.1.10:11434/api/chat"
        );
    }

    #[test]
    fn chat_response_uses_final_content_and_ignores_reasoning_fields() {
        let normal = r#"{"choices":[{"message":{"content":"<LUMETRACE_ANSWER>普通回答 [S1]</LUMETRACE_ANSWER>"}}]}"#;
        assert_eq!(
            parse_chat_answer(normal.as_bytes()).unwrap(),
            "普通回答 [S1]"
        );

        let reasoning_content = r#"{"choices":[{"message":{"reasoning_content":"内部推理","content":"最终回答 [S1]"}}]}"#;
        assert_eq!(
            parse_chat_answer(reasoning_content.as_bytes()).unwrap(),
            "最终回答 [S1]"
        );

        let thinking = r#"{"choices":[{"message":{"thinking":"内部思考","content":"可见正文"}}]}"#;
        assert_eq!(parse_chat_answer(thinking.as_bytes()).unwrap(), "可见正文");
    }

    #[test]
    fn inline_think_blocks_are_removed_without_hiding_the_final_answer() {
        let response = r#"{"choices":[{"message":{"content":"<think>先分析来源</think>\n<LUMETRACE_ANSWER>结论 [S2]</LUMETRACE_ANSWER>"}}]}"#;
        assert_eq!(parse_chat_answer(response.as_bytes()).unwrap(), "结论 [S2]");

        let reasoning_only = r#"{"choices":[{"message":{"reasoning":"仍在思考","content":""}}]}"#;
        assert_eq!(
            parse_chat_answer(reasoning_only.as_bytes()),
            Err(ERROR_LOCAL_LLM_EMPTY.to_owned())
        );
    }

    #[test]
    fn streaming_formats_keep_reasoning_separate_from_final_content() {
        let openai_thinking = parse_openai_stream_line(
            r#"data: {"choices":[{"delta":{"reasoning":"先检查来源"},"finish_reason":null}]}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(openai_thinking.thinking, "先检查来源");
        assert!(openai_thinking.content.is_empty());
        assert!(!openai_thinking.done);

        let openai_answer = parse_openai_stream_line(
            r#"data: {"choices":[{"delta":{"content":"最终回答 [S1]"},"finish_reason":"stop"}]}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(openai_answer.content, "最终回答 [S1]");
        assert!(openai_answer.thinking.is_empty());
        assert!(openai_answer.done);

        let ollama_thinking = parse_ollama_stream_line(
            r#"{"message":{"role":"assistant","thinking":"比较证据","content":""},"done":false}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(ollama_thinking.thinking, "比较证据");
        assert!(ollama_thinking.content.is_empty());

        let ollama_answer = parse_ollama_stream_line(
            r#"{"message":{"role":"assistant","thinking":"","content":"结论 [S2]"},"done":true,"done_reason":"stop"}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(ollama_answer.content, "结论 [S2]");
        assert!(ollama_answer.done);
    }

    #[test]
    fn local_chat_requests_use_provider_appropriate_reasoning_without_output_limits() {
        let settings = LocalLlmSettings {
            provider: "lmStudio".to_owned(),
            base_url: "http://192.168.1.10:11434/v1".to_owned(),
            model: "qwen3.5:27b".to_owned(),
        };
        let openai = openai_chat_request_payload(&settings, "question");
        assert_eq!(openai["reasoning_effort"], "none");
        assert!(openai.get("max_tokens").is_none());

        let ollama = ollama_chat_request_payload(&settings, "question");
        assert_eq!(ollama["think"], false);
        assert!(ollama["options"].get("num_predict").is_none());
    }

    #[test]
    fn thinking_is_preserved_until_the_model_emits_its_final_answer() {
        let mut result = LocalLlmStreamResult::default();
        let mut last_emit = None;
        let mut on_thinking = |_: &str| {};
        let long_thinking = "继续分析证据。".repeat(2_000);
        assert!(!apply_stream_chunk(
            &mut result,
            LocalLlmStreamChunk {
                thinking: long_thinking.clone(),
                ..LocalLlmStreamChunk::default()
            },
            &mut last_emit,
            &mut on_thinking,
        )
        .unwrap());
        assert_eq!(result.thinking, long_thinking);
        assert!(apply_stream_chunk(
            &mut result,
            LocalLlmStreamChunk {
                content: "最终回答".to_owned(),
                done: true,
                ..LocalLlmStreamChunk::default()
            },
            &mut last_emit,
            &mut on_thinking,
        )
        .unwrap());
        assert_eq!(final_stream_answer(result).unwrap(), "最终回答");
    }

    #[test]
    fn local_settings_persist_and_become_the_active_service() {
        let root =
            std::env::temp_dir().join(format!("lumetrace-local-llm-settings-{}", Uuid::new_v4()));
        let database = crate::database::open_for_test(&root.join("lumetrace.sqlite3")).unwrap();
        let saved = save_local_llm_settings_record(
            &database,
            LocalLlmSettings {
                provider: "ollama".to_owned(),
                base_url: "http://192.168.1.10:11434/v1/".to_owned(),
                model: " qwen3.5:27b ".to_owned(),
            },
        )
        .unwrap();
        assert_eq!(saved.base_url, "http://192.168.1.10:11434");
        assert_eq!(saved.model, "qwen3.5:27b");
        assert_eq!(
            load_active_ai_service_record(&database).unwrap(),
            Some(ActiveAiService::Local(saved))
        );
        drop(database);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cloud_settings_persist_locally_without_exposing_the_api_key() {
        let root =
            std::env::temp_dir().join(format!("lumetrace-cloud-ai-settings-{}", Uuid::new_v4()));
        let database = crate::database::open_for_test(&root.join("lumetrace.sqlite3")).unwrap();
        let saved = save_cloud_ai_settings_record(
            &database,
            CloudAiSettingsRequest {
                provider: "openai".to_owned(),
                base_url: "https://api.openai.com".to_owned(),
                api_key: Some("sk-local-test-value".to_owned()),
                model: "gpt-5-mini".to_owned(),
            },
        )
        .unwrap();
        assert_eq!(saved.base_url, "https://api.openai.com/v1");
        assert!(saved.has_api_key);
        assert!(!serde_json::to_string(&saved)
            .unwrap()
            .contains("sk-local-test-value"));
        assert!(cloud_ai_is_configured(&database).unwrap());
        assert_eq!(
            load_service_mode_record(&database).unwrap().as_deref(),
            Some("cloud")
        );
        assert_eq!(
            read_setting(&database, CLOUD_AI_API_KEY_KEY)
                .unwrap()
                .as_deref(),
            Some("sk-local-test-value")
        );

        let updated = save_cloud_ai_settings_record(
            &database,
            CloudAiSettingsRequest {
                provider: "openai".to_owned(),
                base_url: "https://api.openai.com/v1".to_owned(),
                api_key: None,
                model: "gpt-5.1".to_owned(),
            },
        )
        .unwrap();
        assert_eq!(updated.model, "gpt-5.1");
        assert_eq!(
            read_setting(&database, CLOUD_AI_API_KEY_KEY)
                .unwrap()
                .as_deref(),
            Some("sk-local-test-value")
        );

        drop(database);
        std::fs::remove_dir_all(root).unwrap();
    }
}
