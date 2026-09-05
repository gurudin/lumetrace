use serde::Deserialize;
use std::process::Command;

const CHANNELS: &str = include_str!("../../src/shared/feedbackChannels.json");

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeedbackChannel {
    Github,
    Discord,
}

#[derive(Deserialize)]
struct FeedbackLinks {
    github: Option<String>,
    discord: Option<String>,
}

fn channel_url(channel: FeedbackChannel) -> Result<String, String> {
    let links: FeedbackLinks = serde_json::from_str(CHANNELS).map_err(|error| error.to_string())?;
    let url = match channel {
        FeedbackChannel::Github => links.github,
        FeedbackChannel::Discord => links.discord,
    }
    .ok_or_else(|| "This feedback channel is not available yet.".to_owned())?;
    if !url.starts_with("https://") {
        return Err("Feedback links must use HTTPS.".to_owned());
    }
    Ok(url)
}

fn browser_command(url: &str) -> Command {
    #[cfg(target_os = "macos")]
    let mut command = Command::new("/usr/bin/open");
    #[cfg(target_os = "windows")]
    let mut command = Command::new("explorer.exe");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut command = Command::new("xdg-open");
    command.arg(url);
    command
}

#[tauri::command]
pub async fn open_feedback_channel(channel: FeedbackChannel) -> Result<(), String> {
    // Only compiled-in destinations are accepted. No files, logs, or query data are attached.
    let url = channel_url(channel)?;
    tauri::async_runtime::spawn_blocking(move || {
        let result = browser_command(&url)
            .output()
            .map_err(|error| format!("Could not open the browser: {error}"))?;
        if result.status.success() {
            Ok(())
        } else {
            Err("The system could not open the feedback page.".to_owned())
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_opens_project_issues_without_attaching_data() {
        let url = channel_url(FeedbackChannel::Github).unwrap();
        assert_eq!(url, "https://github.com/gurudin/lumetrace/issues");
        let command = browser_command(&url);
        assert_eq!(command.get_args().collect::<Vec<_>>(), vec![url.as_str()]);
    }

    #[test]
    fn discord_opens_the_configured_community_without_attaching_data() {
        let url = channel_url(FeedbackChannel::Discord).unwrap();
        assert_eq!(url, "https://discord.gg/6pJVMTJ5UG");
        let command = browser_command(&url);
        assert_eq!(command.get_args().collect::<Vec<_>>(), vec![url.as_str()]);
    }

    #[test]
    fn callers_cannot_supply_arbitrary_urls_or_commands() {
        assert!(serde_json::from_str::<FeedbackChannel>(r#""https://example.com""#).is_err());
        assert!(serde_json::from_str::<FeedbackChannel>(r#""file:///tmp/test""#).is_err());
        assert!(serde_json::from_str::<FeedbackChannel>(r#""github; open /tmp""#).is_err());
        assert!(serde_json::from_str::<FeedbackChannel>(r#""github""#).is_ok());
        assert!(serde_json::from_str::<FeedbackChannel>(r#""discord""#).is_ok());
    }
}
