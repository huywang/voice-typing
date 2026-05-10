use log::{debug, warn};

/// Map a frontmost application name to a tone descriptor used by the LLM polishing prompt.
///
/// Returns a static string describing the desired tone:
/// - Email clients → "formal, professional"
/// - Chat / messaging apps → "casual, conversational"
/// - Code editors / terminals → "technical, minimal editing"
/// - Everything else → "natural, clear" (standard polish)
pub fn get_tone_for_app(app_name: &str) -> &'static str {
    // Normalise to lower-case for case-insensitive matching.
    let lower = app_name.to_lowercase();

    // Email clients.
    if lower.contains("mail")
        || lower.contains("outlook")
        || lower.contains("gmail")
    {
        return "formal, professional";
    }

    // Chat / messaging apps.
    if lower.contains("slack")
        || lower.contains("discord")
        || lower.contains("wechat")
        || lower.contains("telegram")
        || lower.contains("messages")
    {
        return "casual, conversational";
    }

    // Code editors and terminals.
    if lower.contains("code")
        || lower.contains("xcode")
        || lower.contains("terminal")
        || lower.contains("iterm")
    {
        return "technical, minimal editing";
    }

    // Default: standard polish tone.
    "natural, clear"
}

/// Return the name of the frontmost (focused) application at the time of the call.
///
/// On macOS this uses `osascript` to query System Events. On other platforms
/// it returns an empty string for now.
///
/// Errors are logged and an empty string is returned so the caller never has
/// to handle a failure path.
pub fn get_frontmost_app_name() -> String {
    #[cfg(target_os = "macos")]
    {
        get_frontmost_app_name_macos()
    }
    #[cfg(not(target_os = "macos"))]
    {
        debug!("get_frontmost_app_name: non-macOS platform, returning empty string");
        String::new()
    }
}

/// macOS implementation: ask System Events for the frontmost process name.
#[cfg(target_os = "macos")]
fn get_frontmost_app_name_macos() -> String {
    use std::process::Command;

    let result = Command::new("osascript")
        .args([
            "-e",
            "tell application \"System Events\" to get name of first application process whose frontmost is true",
        ])
        .output();

    match result {
        Ok(output) if output.status.success() => {
            let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
            debug!("Frontmost app: {name}");
            name
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("osascript exited with non-zero status: {stderr}");
            String::new()
        }
        Err(e) => {
            warn!("Failed to run osascript to detect frontmost app: {e}");
            String::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tone_email_apps() {
        assert_eq!(get_tone_for_app("Mail"), "formal, professional");
        assert_eq!(get_tone_for_app("Microsoft Outlook"), "formal, professional");
        assert_eq!(get_tone_for_app("Gmail"), "formal, professional");
    }

    #[test]
    fn test_tone_chat_apps() {
        assert_eq!(get_tone_for_app("Slack"), "casual, conversational");
        assert_eq!(get_tone_for_app("Discord"), "casual, conversational");
        assert_eq!(get_tone_for_app("WeChat"), "casual, conversational");
        assert_eq!(get_tone_for_app("Telegram"), "casual, conversational");
        assert_eq!(get_tone_for_app("Messages"), "casual, conversational");
    }

    #[test]
    fn test_tone_code_editors() {
        assert_eq!(get_tone_for_app("Code"), "technical, minimal editing");
        assert_eq!(get_tone_for_app("Xcode"), "technical, minimal editing");
        assert_eq!(get_tone_for_app("Terminal"), "technical, minimal editing");
        assert_eq!(get_tone_for_app("iTerm2"), "technical, minimal editing");
    }

    #[test]
    fn test_tone_default() {
        assert_eq!(get_tone_for_app("Safari"), "natural, clear");
        assert_eq!(get_tone_for_app(""), "natural, clear");
        assert_eq!(get_tone_for_app("Notion"), "natural, clear");
    }
}
