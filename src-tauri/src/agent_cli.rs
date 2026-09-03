use crate::{
    ai_service::{AI_SERVICE_MODE_AGENT_CLI, AI_SERVICE_MODE_KEY},
    database::Database,
};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    env,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::State;

const AGENT_CLI_SETTINGS_KEY: &str = "ai.agent_cli";
const CLAUDE_CONNECTION_MARKER: &str = "LUMETRACE_CLAUDE_OK";
const HERMES_CONNECTION_MARKER: &str = "LUMETRACE_HERMES_OK";
const CODEX_CONNECTION_MARKER: &str = "LUMETRACE_CODEX_OK";
const OPENCODE_CONNECTION_MARKER: &str = "LUMETRACE_OPENCODE_OK";
const CODEX_HOST_ENVIRONMENT_KEYS: [&str; 7] = [
    "CODEX_CI",
    "CODEX_INTERNAL_ORIGINATOR_OVERRIDE",
    "CODEX_PERMISSION_PROFILE",
    "CODEX_SANDBOX_NETWORK_DISABLED",
    "CODEX_SESSION_ID",
    "CODEX_SHELL",
    "CODEX_THREAD_ID",
];

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
enum AgentCliCheckState {
    Passed,
    NotConfigured,
    CheckFailed,
    Missing,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCliStatus {
    key: String,
    installed: bool,
    reachable: bool,
    version: Option<String>,
    check_state: AgentCliCheckState,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCliSettings {
    pub(crate) cli: String,
    pub(crate) permission: String,
    pub(crate) version: Option<String>,
}

#[derive(Clone, Copy)]
struct AgentCliDefinition {
    key: &'static str,
    executable: &'static str,
    health_args: &'static [&'static str],
    validator: fn(&str) -> bool,
}

struct ProcessOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

enum CommandResult {
    Completed(ProcessOutput),
    NotFound,
    Failed,
}

impl ProcessOutput {
    fn combined(&self) -> String {
        format!("{}\n{}", self.stdout, self.stderr)
    }

    fn first_line(&self) -> Option<String> {
        self.stdout
            .lines()
            .chain(self.stderr.lines())
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(str::to_owned)
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn validate_agent_cli_settings(mut settings: AgentCliSettings) -> Result<AgentCliSettings, String> {
    if !definitions()
        .iter()
        .any(|definition| definition.key == settings.cli)
    {
        return Err("Unsupported Agent CLI".to_owned());
    }
    if !matches!(settings.permission.as_str(), "readOnly" | "readWrite") {
        return Err("Unsupported Agent CLI file permission".to_owned());
    }
    settings.version = settings.version.and_then(|version| {
        let version = version.trim();
        (!version.is_empty() && version.len() <= 256 && !version.chars().any(char::is_control))
            .then(|| version.to_owned())
    });
    Ok(settings)
}

pub(crate) fn load_agent_cli_settings_record(
    database: &Database,
) -> Result<Option<AgentCliSettings>, String> {
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let value = connection
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [AGENT_CLI_SETTINGS_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("Unable to load the Agent CLI setting: {error}"))?;
    value
        .map(|value| {
            serde_json::from_str::<AgentCliSettings>(&value)
                .map_err(|error| format!("Unable to read the Agent CLI setting: {error}"))
                .and_then(validate_agent_cli_settings)
        })
        .transpose()
}

fn save_agent_cli_settings_record(
    database: &Database,
    settings: AgentCliSettings,
) -> Result<AgentCliSettings, String> {
    let settings = validate_agent_cli_settings(settings)?;
    let value = serde_json::to_string(&settings)
        .map_err(|error| format!("Unable to encode the Agent CLI setting: {error}"))?;
    let mut connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Unable to begin saving the Agent CLI setting: {error}"))?;
    let now = now_millis();
    transaction
        .execute(
            "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![AGENT_CLI_SETTINGS_KEY, value, now],
        )
        .map_err(|error| format!("Unable to save the Agent CLI setting: {error}"))?;
    transaction
        .execute(
            "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![AI_SERVICE_MODE_KEY, AI_SERVICE_MODE_AGENT_CLI, now],
        )
        .map_err(|error| format!("Unable to activate the Agent CLI setting: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("Unable to finish saving the Agent CLI setting: {error}"))?;
    Ok(settings)
}

fn executable_candidates(executable: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(home) = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")) {
        let home = PathBuf::from(home);
        candidates.push(home.join(".local").join("bin").join(executable));
        candidates.push(home.join(".cargo").join("bin").join(executable));
        candidates.push(home.join(".bun").join("bin").join(executable));
        candidates.push(home.join(".volta").join("bin").join(executable));
        if executable == "claude" {
            candidates.push(
                home.join("Applications")
                    .join("Claude Code URL Handler.app")
                    .join("Contents")
                    .join("MacOS")
                    .join("claude"),
            );
        }
    }

    if executable == "claude" {
        candidates.push(
            PathBuf::from("/Applications")
                .join("Claude Code URL Handler.app")
                .join("Contents")
                .join("MacOS")
                .join("claude"),
        );
    }
    candidates.push(PathBuf::from("/usr/local/bin").join(executable));
    candidates.push(PathBuf::from("/opt/homebrew/bin").join(executable));
    candidates.push(PathBuf::from(executable));
    candidates
}

pub(crate) fn resolve_agent_cli_executable(key: &str) -> Option<PathBuf> {
    let definition = definitions()
        .into_iter()
        .find(|definition| definition.key == key)?;
    executable_candidates(definition.executable)
        .into_iter()
        .find(|candidate| {
            std::process::Command::new(candidate)
                .arg("--version")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        })
}

fn codex_user_config_path() -> Option<PathBuf> {
    env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .or_else(|| env::var_os("USERPROFILE"))
                .map(PathBuf::from)
                .map(|home| home.join(".codex"))
        })
        .map(|home| home.join("config.toml"))
}

fn codex_mcp_server_keys(config: &str) -> Vec<String> {
    let mut keys = config
        .lines()
        .filter_map(|line| {
            let table = line
                .trim()
                .strip_prefix("[mcp_servers.")?
                .strip_suffix(']')?
                .trim();
            if table.is_empty() || table.chars().any(char::is_control) {
                return None;
            }

            let mut quote = None;
            let mut escaped = false;
            for character in table.chars() {
                if escaped {
                    escaped = false;
                    continue;
                }
                match (quote, character) {
                    (Some('"'), '\\') => escaped = true,
                    (Some(current), value) if current == value => quote = None,
                    (None, '"' | '\'') => quote = Some(character),
                    (None, '.') => return None,
                    _ => {}
                }
            }
            quote.is_none().then(|| table.to_owned())
        })
        .collect::<Vec<_>>();
    keys.sort();
    keys.dedup();
    keys
}

fn configured_codex_mcp_server_keys() -> Vec<String> {
    codex_user_config_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|config| codex_mcp_server_keys(&config))
        .unwrap_or_default()
}

fn read_pipe<T>(pipe: Option<T>) -> thread::JoinHandle<String>
where
    T: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut output = String::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_string(&mut output);
        }
        output
    })
}

fn read_process_output(
    stdout_reader: thread::JoinHandle<String>,
    stderr_reader: thread::JoinHandle<String>,
    status: ExitStatus,
) -> ProcessOutput {
    ProcessOutput {
        success: status.success(),
        stdout: stdout_reader.join().unwrap_or_default(),
        stderr: stderr_reader.join().unwrap_or_default(),
    }
}

fn run_command(executable: &PathBuf, args: &[&str]) -> CommandResult {
    let mut command = Command::new(executable);
    command.args(args);
    run_prepared_command(command, None)
}

fn run_prepared_command(mut command: Command, stdin_input: Option<&str>) -> CommandResult {
    let mut child = match command
        .stdin(if stdin_input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return CommandResult::NotFound
        }
        Err(_) => return CommandResult::Failed,
    };
    if let Some(input) = stdin_input {
        let wrote_input = child
            .stdin
            .take()
            .is_some_and(|mut stdin| stdin.write_all(input.as_bytes()).is_ok());
        if !wrote_input {
            let _ = child.kill();
            let _ = child.wait();
            return CommandResult::Failed;
        }
    }

    // Drain both pipes while the process is running. Some CLIs print enough diagnostic
    // output to fill an OS pipe and would otherwise block before they can exit.
    let stdout_reader = read_pipe(child.stdout.take());
    let stderr_reader = read_pipe(child.stderr.take());
    match child.wait() {
        Ok(status) => {
            CommandResult::Completed(read_process_output(stdout_reader, stderr_reader, status))
        }
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            CommandResult::Failed
        }
    }
}

fn configured_codex_exec_command(
    executable: &Path,
    working_dir: &Path,
    include_reasoning_summary: bool,
) -> Command {
    let mut command = Command::new(executable);
    command
        .arg("-a")
        .arg("never")
        .arg("exec")
        .arg("--sandbox")
        .arg("read-only")
        .arg("--skip-git-repo-check")
        .arg("--ephemeral")
        .arg("--ignore-rules")
        .arg("--color")
        .arg("never")
        .arg("--json")
        .arg("--disable")
        .arg("shell_tool")
        .arg("--disable")
        .arg("multi_agent")
        .arg("--disable")
        .arg("apps")
        .arg("--disable")
        .arg("hooks")
        .arg("--disable")
        .arg("remote_plugin")
        .arg("--disable")
        .arg("memories")
        .arg("-c")
        .arg("web_search=\"disabled\"")
        .current_dir(working_dir);
    if include_reasoning_summary {
        command
            .arg("-c")
            .arg("model_reasoning_effort=\"low\"")
            .arg("-c")
            .arg("model_reasoning_summary=\"auto\"")
            .arg("-c")
            .arg("hide_agent_reasoning=false");
    }
    // Keep the user's model/provider/auth configuration, but do not start the
    // user's unrelated MCP servers for a RAG answer whose complete evidence is
    // already supplied by Lume Trace in the prompt.
    for server in configured_codex_mcp_server_keys() {
        command
            .arg("-c")
            .arg(format!("mcp_servers.{server}.enabled=false"));
    }
    command.arg("-");
    for key in CODEX_HOST_ENVIRONMENT_KEYS {
        command.env_remove(key);
    }
    command
}

pub(crate) fn codex_exec_command(executable: &Path, working_dir: &Path) -> Command {
    configured_codex_exec_command(executable, working_dir, false)
}

pub(crate) fn codex_answer_command(executable: &Path, working_dir: &Path) -> Command {
    configured_codex_exec_command(executable, working_dir, true)
}

fn configured_claude_command(
    executable: &Path,
    working_dir: &Path,
    include_partial_messages: bool,
) -> Command {
    let mut command = Command::new(executable);
    command
        .arg("--print")
        .arg("--safe-mode")
        .arg("--tools")
        .arg("")
        .arg("--output-format")
        .arg("stream-json")
        .arg("--verbose")
        .current_dir(working_dir);
    if include_partial_messages {
        command.arg("--include-partial-messages");
    }
    command
}

pub(crate) fn claude_connection_command(executable: &Path, working_dir: &Path) -> Command {
    configured_claude_command(executable, working_dir, false)
}

pub(crate) fn claude_answer_command(executable: &Path, working_dir: &Path) -> Command {
    configured_claude_command(executable, working_dir, true)
}

fn configured_opencode_command(
    executable: &Path,
    working_dir: &Path,
    include_thinking: bool,
) -> Command {
    let mut command = Command::new(executable);
    command.arg("run").arg("--format").arg("json");
    if include_thinking {
        command.arg("--thinking");
    }
    command
        .current_dir(working_dir)
        .env(
            "OPENCODE_CONFIG_CONTENT",
            r#"{"permission":{"*":"deny"},"share":"disabled"}"#,
        )
        .env("OPENCODE_DISABLE_AUTOUPDATE", "true")
        .env("OPENCODE_DISABLE_CLAUDE_CODE", "true")
        .env("OPENCODE_DISABLE_DEFAULT_PLUGINS", "true")
        .env("OPENCODE_DISABLE_EXTERNAL_SKILLS", "true")
        .env("OPENCODE_DISABLE_PROJECT_CONFIG", "true");
    command
}

pub(crate) fn opencode_connection_command(executable: &Path, working_dir: &Path) -> Command {
    configured_opencode_command(executable, working_dir, false)
}

pub(crate) fn opencode_answer_command(executable: &Path, working_dir: &Path) -> Command {
    configured_opencode_command(executable, working_dir, true)
}

fn claude_is_authenticated(output: &str) -> bool {
    output.contains("\"loggedIn\": true")
}

fn codex_is_authenticated(output: &str) -> bool {
    output.lines().any(|line| line.contains("Logged in"))
}

fn opencode_is_authenticated(output: &str) -> bool {
    let output = output.to_ascii_lowercase();
    output.contains("credentials")
        && !output.contains("0 credentials")
        && !output.contains("unable to connect")
}

fn hermes_is_configured(output: &str) -> bool {
    let provider = output
        .lines()
        .find_map(|line| line.trim().strip_prefix("Provider:"))
        .map(str::trim)
        .map(str::to_ascii_lowercase);

    let Some(provider) = provider else {
        return false;
    };

    let provider_label = if provider.contains("kimi") || provider.contains("moonshot") {
        "kimi"
    } else if provider.contains("openrouter") {
        "openrouter"
    } else if provider.contains("anthropic") || provider.contains("claude") {
        "anthropic"
    } else if provider.contains("google") || provider.contains("gemini") {
        "google / gemini"
    } else if provider.contains("deepseek") {
        "deepseek"
    } else if provider.contains("nvidia") {
        "nvidia"
    } else if provider.contains("minimax") {
        "minimax"
    } else if provider.contains("step") {
        "stepfun"
    } else if provider.contains("zai") || provider.contains("z.ai") || provider.contains("glm") {
        "z.ai"
    } else if provider.contains("xai") || provider.contains("grok") {
        "xai"
    } else if provider.contains("codex") {
        "openai codex"
    } else if provider.contains("openai") {
        "openai"
    } else if provider.contains("nous") {
        "nous portal"
    } else {
        return false;
    };

    output.lines().any(|line| {
        let normalized = line.to_ascii_lowercase();
        normalized.contains(provider_label)
            && line.contains('✓')
            && !normalized.contains("not configured")
            && !normalized.contains("not logged in")
    })
}

fn hermes_connection_succeeded(output: &str) -> bool {
    output
        .lines()
        .any(|line| line.trim() == HERMES_CONNECTION_MARKER)
}

fn codex_agent_message(line: &str) -> Option<String> {
    let event = serde_json::from_str::<serde_json::Value>(line).ok()?;
    if event.get("type")?.as_str()? != "item.completed" {
        return None;
    }
    let item = event.get("item")?;
    (item.get("type")?.as_str()? == "agent_message")
        .then(|| {
            item.get("text")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .flatten()
}

fn codex_connection_succeeded(output: &str) -> bool {
    output
        .lines()
        .filter_map(codex_agent_message)
        .any(|message| message.trim() == CODEX_CONNECTION_MARKER)
}

fn claude_connection_succeeded(output: &str) -> bool {
    output.lines().any(|line| {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            return false;
        };
        event.get("type").and_then(serde_json::Value::as_str) == Some("result")
            && event.get("is_error").and_then(serde_json::Value::as_bool) != Some(true)
            && event
                .get("result")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|result| result.trim() == CLAUDE_CONNECTION_MARKER)
    })
}

fn opencode_connection_succeeded(output: &str) -> bool {
    output.lines().any(|line| {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            return false;
        };
        event.get("type").and_then(serde_json::Value::as_str) == Some("text")
            && event
                .get("part")
                .and_then(|part| part.get("text"))
                .and_then(serde_json::Value::as_str)
                .is_some_and(|text| text.trim() == OPENCODE_CONNECTION_MARKER)
    })
}

fn definitions() -> [AgentCliDefinition; 4] {
    [
        AgentCliDefinition {
            key: "claude",
            executable: "claude",
            health_args: &["auth", "status"],
            validator: claude_is_authenticated,
        },
        AgentCliDefinition {
            key: "hermes",
            executable: "hermes",
            health_args: &["status"],
            validator: hermes_is_configured,
        },
        AgentCliDefinition {
            key: "codex",
            executable: "codex",
            health_args: &["login", "status"],
            validator: codex_is_authenticated,
        },
        AgentCliDefinition {
            key: "opencode",
            executable: "opencode",
            health_args: &["auth", "list"],
            validator: opencode_is_authenticated,
        },
    ]
}

fn inspect_health(definition: &AgentCliDefinition, candidate: &PathBuf) -> AgentCliCheckState {
    match run_command(candidate, definition.health_args) {
        CommandResult::Completed(output)
            if output.success && (definition.validator)(&output.combined()) =>
        {
            AgentCliCheckState::Passed
        }
        CommandResult::Completed(output) if output.success => AgentCliCheckState::NotConfigured,
        CommandResult::Completed(_) | CommandResult::NotFound | CommandResult::Failed => {
            AgentCliCheckState::CheckFailed
        }
    }
}

fn inspect(definition: AgentCliDefinition) -> AgentCliStatus {
    for candidate in executable_candidates(definition.executable) {
        match run_command(&candidate, &["--version"]) {
            CommandResult::NotFound => continue,
            CommandResult::Completed(version_output) => {
                let version = version_output.first_line();
                let check_state = if !version_output.success {
                    AgentCliCheckState::CheckFailed
                } else {
                    inspect_health(&definition, &candidate)
                };

                return AgentCliStatus {
                    key: definition.key.to_owned(),
                    installed: true,
                    reachable: check_state == AgentCliCheckState::Passed,
                    version,
                    check_state,
                };
            }
            CommandResult::Failed => {
                return AgentCliStatus {
                    key: definition.key.to_owned(),
                    installed: true,
                    reachable: false,
                    version: None,
                    check_state: AgentCliCheckState::CheckFailed,
                };
            }
        }
    }

    AgentCliStatus {
        key: definition.key.to_owned(),
        installed: false,
        reachable: false,
        version: None,
        check_state: AgentCliCheckState::Missing,
    }
}

fn failed_check_status(definition: AgentCliDefinition) -> AgentCliStatus {
    AgentCliStatus {
        key: definition.key.to_owned(),
        installed: false,
        reachable: false,
        version: None,
        check_state: AgentCliCheckState::CheckFailed,
    }
}

fn inspect_connection(definition: AgentCliDefinition) -> AgentCliStatus {
    let status = inspect(definition);
    if !status.installed {
        return status;
    }
    let Some(executable) = resolve_agent_cli_executable(definition.key) else {
        return AgentCliStatus {
            check_state: AgentCliCheckState::CheckFailed,
            reachable: false,
            ..status
        };
    };
    let neutral_directory = std::env::temp_dir();
    let result = match definition.key {
        "claude" => run_prepared_command(
            claude_connection_command(&executable, &neutral_directory),
            Some("Reply with exactly LUMETRACE_CLAUDE_OK."),
        ),
        "hermes" => {
            let Some(neutral_directory) = neutral_directory.to_str() else {
                return AgentCliStatus {
                    check_state: AgentCliCheckState::CheckFailed,
                    reachable: false,
                    ..status
                };
            };
            run_command(
                &executable,
                &[
                    "chat",
                    "-q",
                    "Reply with exactly LUMETRACE_HERMES_OK.",
                    "-Q",
                    "--safe-mode",
                    "--reasoning",
                    "none",
                    "--max-turns",
                    "1",
                    "--source",
                    "tool",
                    "--toolsets",
                    "context_engine",
                    "--in",
                    neutral_directory,
                ],
            )
        }
        "codex" => run_prepared_command(
            codex_exec_command(&executable, &neutral_directory),
            Some("Reply with exactly LUMETRACE_CODEX_OK. Do not call any tools."),
        ),
        "opencode" => run_prepared_command(
            opencode_connection_command(&executable, &neutral_directory),
            Some("Reply with exactly LUMETRACE_OPENCODE_OK."),
        ),
        _ => CommandResult::Failed,
    };
    let passed = matches!(
        result,
        CommandResult::Completed(ref output)
            if output.success && match definition.key {
                "claude" => claude_connection_succeeded(&output.stdout),
                "hermes" => hermes_connection_succeeded(&output.combined()),
                "codex" => codex_connection_succeeded(&output.stdout),
                "opencode" => opencode_connection_succeeded(&output.stdout),
                _ => false,
            }
    );
    AgentCliStatus {
        reachable: passed,
        check_state: if passed {
            AgentCliCheckState::Passed
        } else {
            AgentCliCheckState::CheckFailed
        },
        ..status
    }
}

#[tauri::command]
pub async fn check_agent_clis() -> Vec<AgentCliStatus> {
    tauri::async_runtime::spawn_blocking(|| {
        let workers = definitions()
            .into_iter()
            .map(|definition| {
                let worker = thread::spawn(move || inspect(definition));
                (definition, worker)
            })
            .collect::<Vec<_>>();
        workers
            .into_iter()
            .map(|(definition, worker)| {
                worker
                    .join()
                    .unwrap_or_else(|_| failed_check_status(definition))
            })
            .collect()
    })
    .await
    .unwrap_or_else(|_| definitions().into_iter().map(failed_check_status).collect())
}

#[tauri::command]
pub async fn check_agent_cli_status(key: String) -> Option<AgentCliStatus> {
    tauri::async_runtime::spawn_blocking(move || {
        definitions()
            .into_iter()
            .find(|definition| definition.key == key)
            .map(inspect)
    })
    .await
    .ok()
    .flatten()
}

#[tauri::command]
pub async fn check_agent_cli(key: String) -> Option<AgentCliStatus> {
    tauri::async_runtime::spawn_blocking(move || {
        definitions()
            .into_iter()
            .find(|definition| definition.key == key)
            .map(inspect_connection)
    })
    .await
    .ok()
    .flatten()
}

#[tauri::command]
pub fn get_agent_cli_settings(
    database: State<'_, Database>,
) -> Result<Option<AgentCliSettings>, String> {
    load_agent_cli_settings_record(database.inner())
}

#[tauri::command]
pub fn save_agent_cli_settings(
    settings: AgentCliSettings,
    database: State<'_, Database>,
) -> Result<AgentCliSettings, String> {
    save_agent_cli_settings_record(database.inner(), settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn authentication_validators_reject_unconfigured_outputs() {
        assert!(claude_is_authenticated(r#"{"loggedIn": true}"#));
        assert!(!claude_is_authenticated(r#"{"loggedIn": false}"#));
        assert!(codex_is_authenticated("Logged in using ChatGPT"));
        assert!(!opencode_is_authenticated("Credentials\n0 credentials"));
        assert!(!opencode_is_authenticated(
            "Credentials\n1 credential\nUnable to connect"
        ));
    }

    #[test]
    fn hermes_validator_requires_the_active_provider_to_be_configured() {
        let configured = "[photon] warning: sidecar unavailable\nProvider:     kimi-plan\nKimi / Moonshot  ✓ configured";
        let missing = "Provider: kimi-plan\nKimi / Moonshot  ✗ not configured";
        assert!(hermes_is_configured(configured));
        assert!(!hermes_is_configured(missing));
    }

    #[test]
    fn hermes_connection_requires_the_exact_health_marker() {
        assert!(hermes_connection_succeeded(
            "session_id: test\nLUMETRACE_HERMES_OK\n"
        ));
        assert!(!hermes_connection_succeeded(
            "session_id: test\nHermes is available\n"
        ));
    }

    #[test]
    fn codex_connection_requires_an_exact_jsonl_agent_message() {
        let valid = r#"{"type":"thread.started","thread_id":"test"}
{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"LUMETRACE_CODEX_OK"}}"#;
        assert!(codex_connection_succeeded(valid));
        assert!(!codex_connection_succeeded("LUMETRACE_CODEX_OK"));
        assert!(!codex_connection_succeeded(
            r#"{"type":"item.completed","item":{"type":"reasoning","text":"LUMETRACE_CODEX_OK"}}"#
        ));
    }

    #[test]
    fn claude_connection_requires_an_exact_success_result() {
        assert!(claude_connection_succeeded(
            r#"{"type":"result","is_error":false,"result":"LUMETRACE_CLAUDE_OK"}"#
        ));
        assert!(!claude_connection_succeeded(
            r#"{"type":"result","is_error":true,"result":"LUMETRACE_CLAUDE_OK"}"#
        ));
        assert!(!claude_connection_succeeded("LUMETRACE_CLAUDE_OK"));
    }

    #[test]
    fn opencode_connection_requires_an_exact_json_text_event() {
        assert!(opencode_connection_succeeded(
            r#"{"type":"text","part":{"type":"text","text":"LUMETRACE_OPENCODE_OK"}}"#
        ));
        assert!(!opencode_connection_succeeded(
            r#"{"type":"reasoning","part":{"type":"reasoning","text":"LUMETRACE_OPENCODE_OK"}}"#
        ));
        assert!(!opencode_connection_succeeded("LUMETRACE_OPENCODE_OK"));
    }

    #[test]
    fn codex_prompt_is_received_from_stdin_instead_of_process_arguments() {
        let command = codex_exec_command(Path::new("codex"), Path::new("/tmp"));
        let arguments = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(arguments.last().map(String::as_str), Some("-"));
        assert!(!arguments
            .iter()
            .any(|argument| argument.contains("LUMETRACE")));
    }

    #[test]
    fn claude_uses_stream_json_with_tools_disabled() {
        let command = claude_answer_command(Path::new("claude"), Path::new("/tmp"));
        let arguments = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(arguments.windows(2).any(|pair| pair == ["--tools", ""]));
        assert!(arguments
            .windows(2)
            .any(|pair| pair == ["--output-format", "stream-json"]));
        assert!(arguments
            .iter()
            .any(|argument| argument == "--include-partial-messages"));
        assert!(!arguments
            .iter()
            .any(|argument| argument.contains("LUMETRACE")));
    }

    #[test]
    fn opencode_uses_json_stdin_and_denies_tools() {
        let command = opencode_answer_command(Path::new("opencode"), Path::new("/tmp"));
        let arguments = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(arguments
            .windows(3)
            .any(|pair| pair == ["run", "--format", "json"]));
        assert!(arguments.iter().any(|argument| argument == "--thinking"));
        assert!(!arguments
            .iter()
            .any(|argument| argument.contains("LUMETRACE")));
        let config = command
            .get_envs()
            .find_map(|(key, value)| {
                (key == "OPENCODE_CONFIG_CONTENT")
                    .then(|| value.map(|value| value.to_string_lossy().into_owned()))
                    .flatten()
            })
            .expect("OpenCode isolation config");
        assert!(config.contains(r#""*":"deny""#));
    }

    #[test]
    fn codex_keeps_user_provider_configuration_but_disables_runtime_extensions() {
        let command = codex_exec_command(Path::new("codex"), Path::new("/tmp"));
        let arguments = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(!arguments
            .iter()
            .any(|argument| argument == "--ignore-user-config"));
        for feature in [
            "shell_tool",
            "multi_agent",
            "apps",
            "hooks",
            "remote_plugin",
            "memories",
        ] {
            assert!(arguments.iter().any(|argument| argument == feature));
        }
    }

    #[test]
    fn codex_answers_request_low_reasoning_with_visible_summaries() {
        let command = codex_answer_command(Path::new("codex"), Path::new("/tmp"));
        let arguments = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        for config in [
            "model_reasoning_effort=\"low\"",
            "model_reasoning_summary=\"auto\"",
            "hide_agent_reasoning=false",
        ] {
            assert!(arguments.iter().any(|argument| argument == config));
        }
        assert_eq!(arguments.last().map(String::as_str), Some("-"));
    }

    #[test]
    fn codex_connection_check_does_not_enable_reasoning() {
        let command = codex_exec_command(Path::new("codex"), Path::new("/tmp"));
        let arguments = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(!arguments
            .iter()
            .any(|argument| argument.starts_with("model_reasoning_")));
    }

    #[test]
    fn codex_mcp_config_parser_returns_only_top_level_server_tables() {
        let config = r#"
[mcp_servers.claude-mem]
command = "node"

[mcp_servers.node_repl]
command = "node"

[mcp_servers.node_repl.env]
TOKEN = "not-a-real-token"

[mcp_servers."server.with.dots"]
url = "https://example.invalid/mcp"
"#;
        assert_eq!(
            codex_mcp_server_keys(config),
            vec![
                "\"server.with.dots\"".to_owned(),
                "claude-mem".to_owned(),
                "node_repl".to_owned(),
            ]
        );
    }

    #[test]
    fn codex_child_does_not_inherit_the_outer_codex_host_environment() {
        let command = codex_exec_command(Path::new("codex"), Path::new("/tmp"));
        let removed_keys = command
            .get_envs()
            .filter_map(|(key, value)| value.is_none().then(|| key.to_string_lossy().into_owned()))
            .collect::<Vec<_>>();
        for key in CODEX_HOST_ENVIRONMENT_KEYS {
            assert!(removed_keys.iter().any(|removed| removed == key));
        }
        assert!(!removed_keys.iter().any(|removed| removed == "CODEX_HOME"));
    }

    #[test]
    #[cfg(unix)]
    fn command_waits_for_a_slow_healthy_check() {
        let shell = PathBuf::from("/bin/sh");
        let completed = run_command(&shell, &["-c", "sleep 0.05; printf 'ready\\n'"]);
        assert!(matches!(
            completed,
            CommandResult::Completed(ProcessOutput { success: true, .. })
        ));
    }

    #[test]
    fn definitions_keep_the_requested_cli_order() {
        assert_eq!(
            definitions()
                .into_iter()
                .map(|definition| definition.key)
                .collect::<Vec<_>>(),
            vec!["claude", "hermes", "codex", "opencode"]
        );
    }

    #[test]
    fn agent_cli_settings_round_trip_through_sqlite() {
        let root =
            std::env::temp_dir().join(format!("lumetrace-agent-cli-settings-{}", Uuid::new_v4()));
        let database = crate::database::open_for_test(&root.join("lumetrace.sqlite3")).unwrap();
        let saved = save_agent_cli_settings_record(
            &database,
            AgentCliSettings {
                cli: "codex".to_owned(),
                permission: "readOnly".to_owned(),
                version: Some("  codex-cli 1.2.3  ".to_owned()),
            },
        )
        .unwrap();

        assert_eq!(saved.version.as_deref(), Some("codex-cli 1.2.3"));
        assert_eq!(
            load_agent_cli_settings_record(&database).unwrap(),
            Some(saved.clone())
        );
        assert_eq!(
            crate::ai_service::load_active_ai_service_record(&database).unwrap(),
            Some(crate::ai_service::ActiveAiService::AgentCli(saved))
        );
        drop(database);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn agent_cli_settings_reject_unknown_values() {
        assert!(validate_agent_cli_settings(AgentCliSettings {
            cli: "unknown".to_owned(),
            permission: "readOnly".to_owned(),
            version: None,
        })
        .is_err());
        assert!(validate_agent_cli_settings(AgentCliSettings {
            cli: "codex".to_owned(),
            permission: "everything".to_owned(),
            version: None,
        })
        .is_err());
    }
}
