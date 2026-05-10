use log::{debug, warn};

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
