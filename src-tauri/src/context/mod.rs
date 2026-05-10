#[allow(unused_imports)]
use log::{debug, info, warn};

/// Built-in list of applications where enigo's direct key simulation is known
/// to behave poorly. For these apps the injector falls back to clipboard paste.
pub const DEFAULT_BLACKLIST: &[&str] = &[
    "Terminal",
    "iTerm",
    "iTerm2",
    "Alacritty",
    "kitty",
    "Warp",
    "Sublime Text",
    "Zed",
];

/// Check whether `app_name` should use the clipboard-paste injection fallback.
///
/// Returns `true` when the app name contains any entry from the built-in
/// `DEFAULT_BLACKLIST` or from the caller-supplied `custom_blacklist`.
/// Matching is case-insensitive substring search so that, for example,
/// `"iTerm2"` matches a blacklist entry of `"iterm"`.
pub fn is_blacklisted(app_name: &str, custom_blacklist: &[String]) -> bool {
    let lower = app_name.to_lowercase();

    // Check built-in blacklist.
    for entry in DEFAULT_BLACKLIST {
        if lower.contains(&entry.to_lowercase()) {
            debug!("App '{}' matched built-in blacklist entry '{}'", app_name, entry);
            return true;
        }
    }

    // Check user-defined custom blacklist.
    for entry in custom_blacklist {
        if !entry.is_empty() && lower.contains(&entry.to_lowercase()) {
            debug!("App '{}' matched custom blacklist entry '{}'", app_name, entry);
            return true;
        }
    }

    false
}

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

/// Get the currently selected text in the focused application.
///
/// On macOS, first tries the Accessibility API via AppleScript (requires
/// Accessibility permission). Falls back to simulating Cmd+C and reading the
/// clipboard when the AppleScript approach fails or returns nothing.
///
/// On non-macOS platforms, always returns `None`.
pub fn get_selected_text() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        debug!("get_selected_text: trying AppleScript first");

        // Primary approach: query AXSelectedText via System Events.
        // This works in most native text fields when Accessibility is granted.
        if let Some(text) = get_selected_text_via_applescript() {
            if !text.is_empty() {
                info!("get_selected_text: got {} chars via AppleScript", text.len());
                return Some(text);
            }
        }

        debug!("get_selected_text: AppleScript returned nothing, trying clipboard fallback");

        // Fallback: simulate Cmd+C, read clipboard, then restore original.
        let result = get_selected_text_via_clipboard();
        match &result {
            Some(t) => info!("get_selected_text: got {} chars via clipboard fallback", t.len()),
            None => debug!("get_selected_text: clipboard fallback also returned nothing"),
        }
        result
    }
    #[cfg(not(target_os = "macos"))]
    {
        debug!("get_selected_text: non-macOS platform, returning None");
        None
    }
}

/// macOS: query the selected text from the frontmost app via AppleScript.
///
/// Uses `AXSelectedText` attribute from the Accessibility API.  Returns `None`
/// when the script fails or the attribute is unavailable (e.g. the focused
/// element is not a text field, or Accessibility permission has not been granted).
#[cfg(target_os = "macos")]
fn get_selected_text_via_applescript() -> Option<String> {
    let script = r#"
        set selectedText to ""
        try
            tell application "System Events"
                set frontApp to first application process whose frontmost is true
                set focusedEl to value of attribute "AXFocusedUIElement" of frontApp
                set selectedText to value of attribute "AXSelectedText" of focusedEl
            end tell
        end try
        return selectedText
    "#;

    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .ok()?;

    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        debug!("get_selected_text_via_applescript: raw result = {:?}", text);
        if !text.is_empty() {
            return Some(text);
        }
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        debug!("get_selected_text_via_applescript: script failed: {}", stderr.trim());
    }

    None
}

/// Fallback: copy selected text via simulated Cmd+C, read the clipboard, then
/// restore the original clipboard contents.
///
/// This is less reliable than the Accessibility API approach because it
/// temporarily clobbers the clipboard and introduces a short delay, but it
/// works in more applications (e.g. Electron apps, web browsers).
fn get_selected_text_via_clipboard() -> Option<String> {
    use arboard::Clipboard;

    let mut clipboard = Clipboard::new().ok()?;

    // Save the current clipboard text so we can restore it afterwards.
    let original = clipboard.get_text().ok();

    // Simulate Cmd+C to copy the current selection.
    let mut enigo = enigo::Enigo::new(&enigo::Settings::default()).ok()?;
    use enigo::{Direction, Key, Keyboard};
    enigo.key(Key::Meta, Direction::Press).ok()?;
    enigo.key(Key::Unicode('c'), Direction::Click).ok()?;
    enigo.key(Key::Meta, Direction::Release).ok()?;

    // Give the OS a moment to update the clipboard.
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Read whatever is now on the clipboard.
    let selected = clipboard.get_text().ok();

    // Restore the previous clipboard contents (best-effort; ignore errors).
    match &original {
        Some(orig) => {
            let _ = clipboard.set_text(orig.as_str());
        }
        None => {
            // Original clipboard had no text; clear what we just wrote.
            let _ = clipboard.clear();
        }
    }

    // Return the selected text only if it differs from what was already on
    // the clipboard (i.e. the Cmd+C actually changed it).
    let result = selected.filter(|s| !s.is_empty() && Some(s) != original.as_ref());
    debug!(
        "get_selected_text_via_clipboard: result = {:?}",
        result.as_deref().map(|s| &s[..s.len().min(40)])
    );
    result
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

    #[test]
    fn test_is_blacklisted_default_list() {
        assert!(is_blacklisted("Terminal", &[]));
        assert!(is_blacklisted("iTerm2", &[]));
        assert!(is_blacklisted("Alacritty", &[]));
        assert!(is_blacklisted("Warp", &[]));
        assert!(is_blacklisted("Sublime Text", &[]));
        assert!(is_blacklisted("Zed", &[]));
    }

    #[test]
    fn test_is_blacklisted_case_insensitive() {
        assert!(is_blacklisted("terminal", &[]));
        assert!(is_blacklisted("ITERM2", &[]));
    }

    #[test]
    fn test_is_blacklisted_not_in_list() {
        assert!(!is_blacklisted("Safari", &[]));
        assert!(!is_blacklisted("Slack", &[]));
        assert!(!is_blacklisted("", &[]));
    }

    #[test]
    fn test_is_blacklisted_custom_list() {
        let custom = vec!["MyCustomApp".to_string(), "InternalTool".to_string()];
        assert!(is_blacklisted("MyCustomApp", &custom));
        assert!(is_blacklisted("internaltool", &custom)); // case-insensitive
        assert!(!is_blacklisted("Safari", &custom));
    }

    #[test]
    fn test_is_blacklisted_empty_custom_entries_ignored() {
        let custom = vec!["".to_string()];
        // Empty custom entries should not match everything.
        assert!(!is_blacklisted("Safari", &custom));
    }
}
