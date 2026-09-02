use crate::database::Database;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    env,
    io::Read,
    path::PathBuf,
    process::{ExitStatus, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::State;

const AGENT_CLI_SETTINGS_KEY: &str = "ai.agent_cli";
const VERSION_CHECK_TIMEOUT: Duration = Duration::from_secs(5);

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

struct AgentCliDefinition {
    key: &'static str,
    executable: &'static str,
    health_args: &'static [&'static str],
    health_timeout: Duration,
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
    TimedOut,
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
    let connection = database
        .0
        .lock()
        .map_err(|_| "Unable to access Lume Trace database".to_owned())?;
    connection
        .execute(
            "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![AGENT_CLI_SETTINGS_KEY, value, now_millis()],
        )
        .map_err(|error| format!("Unable to save the Agent CLI setting: {error}"))?;
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

fn run_command(executable: &PathBuf, args: &[&str], timeout: Duration) -> CommandResult {
    let mut child = match std::process::Command::new(executable)
        .args(args)
        .stdin(Stdio::null())
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

    // Drain both pipes while the process is running. Some CLIs print enough diagnostic
    // output to fill an OS pipe and would otherwise block before they can exit.
    let stdout_reader = read_pipe(child.stdout.take());
    let stderr_reader = read_pipe(child.stderr.take());
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return CommandResult::Completed(read_process_output(
                    stdout_reader,
                    stderr_reader,
                    status,
                ))
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(40)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return CommandResult::TimedOut;
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return CommandResult::Failed;
            }
        }
    }
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

fn definitions() -> [AgentCliDefinition; 4] {
    [
        AgentCliDefinition {
            key: "claude",
            executable: "claude",
            health_args: &["auth", "status"],
            health_timeout: Duration::from_secs(8),
            validator: claude_is_authenticated,
        },
        AgentCliDefinition {
            key: "hermes",
            executable: "hermes",
            health_args: &["status"],
            health_timeout: Duration::from_secs(12),
            validator: hermes_is_configured,
        },
        AgentCliDefinition {
            key: "codex",
            executable: "codex",
            health_args: &["login", "status"],
            health_timeout: Duration::from_secs(8),
            validator: codex_is_authenticated,
        },
        AgentCliDefinition {
            key: "opencode",
            executable: "opencode",
            health_args: &["auth", "list"],
            health_timeout: Duration::from_secs(8),
            validator: opencode_is_authenticated,
        },
    ]
}

fn inspect(definition: AgentCliDefinition) -> AgentCliStatus {
    for candidate in executable_candidates(definition.executable) {
        match run_command(&candidate, &["--version"], VERSION_CHECK_TIMEOUT) {
            CommandResult::NotFound => continue,
            CommandResult::Completed(version_output) => {
                let version = version_output.first_line();
                let check_state = if !version_output.success {
                    AgentCliCheckState::CheckFailed
                } else {
                    match run_command(
                        &candidate,
                        definition.health_args,
                        definition.health_timeout,
                    ) {
                        CommandResult::Completed(output)
                            if output.success && (definition.validator)(&output.combined()) =>
                        {
                            AgentCliCheckState::Passed
                        }
                        CommandResult::Completed(output) if output.success => {
                            AgentCliCheckState::NotConfigured
                        }
                        CommandResult::Completed(_)
                        | CommandResult::NotFound
                        | CommandResult::TimedOut
                        | CommandResult::Failed => AgentCliCheckState::CheckFailed,
                    }
                };

                return AgentCliStatus {
                    key: definition.key.to_owned(),
                    installed: true,
                    reachable: check_state == AgentCliCheckState::Passed,
                    version,
                    check_state,
                };
            }
            CommandResult::TimedOut | CommandResult::Failed => {
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

#[tauri::command]
pub async fn check_agent_clis() -> Vec<AgentCliStatus> {
    tauri::async_runtime::spawn_blocking(|| {
        definitions()
            .into_iter()
            .map(|definition| thread::spawn(move || inspect(definition)))
            .collect::<Vec<_>>()
            .into_iter()
            .filter_map(|check| check.join().ok())
            .collect()
    })
    .await
    .unwrap_or_default()
}

#[tauri::command]
pub async fn check_agent_cli(key: String) -> Option<AgentCliStatus> {
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
    #[cfg(unix)]
    fn command_timeout_allows_a_slow_healthy_check_and_reports_a_real_timeout() {
        let shell = PathBuf::from("/bin/sh");
        let completed = run_command(
            &shell,
            &["-c", "sleep 0.05; printf 'ready\\n'"],
            Duration::from_millis(500),
        );
        assert!(matches!(
            completed,
            CommandResult::Completed(ProcessOutput { success: true, .. })
        ));

        let timed_out = run_command(
            &shell,
            &["-c", "sleep 0.5; printf 'too late\\n'"],
            Duration::from_millis(50),
        );
        assert!(matches!(timed_out, CommandResult::TimedOut));
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
            Some(saved)
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
