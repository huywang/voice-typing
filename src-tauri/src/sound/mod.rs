//! Sound effects module.
//!
//! On macOS, plays system sounds via `afplay`. On other platforms, sound
//! playback is silently skipped so the rest of the app is unaffected.

#[allow(unused_imports)]
use log::{debug, warn};

/// Plays the "recording started" sound.
pub fn play_start(enabled: bool) {
    if !enabled {
        return;
    }
    play_system_sound("Tink");
}

/// Plays the "recording stopped / processing complete" sound.
pub fn play_stop(enabled: bool) {
    if !enabled {
        return;
    }
    play_system_sound("Glass");
}

/// Plays the "error" sound.
pub fn play_error(enabled: bool) {
    if !enabled {
        return;
    }
    play_system_sound("Basso");
}

/// Invokes `afplay` with the named macOS system sound (e.g. "Tink").
/// The sound file is located at `/System/Library/Sounds/<name>.aiff`.
/// On non-macOS platforms this function is a no-op.
fn play_system_sound(name: &str) {
    #[cfg(target_os = "macos")]
    {
        let path = format!("/System/Library/Sounds/{name}.aiff");
        debug!("Playing system sound: {path}");
        match std::process::Command::new("afplay").arg(&path).spawn() {
            Ok(_) => {} // fire-and-forget; we don't wait for completion
            Err(e) => warn!("Failed to play sound '{name}': {e}"),
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = name; // suppress unused variable warning
        debug!("Sound playback not supported on this platform — skipping");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_functions_do_not_panic_when_disabled() {
        play_start(false);
        play_stop(false);
        play_error(false);
    }

    #[test]
    fn play_functions_do_not_panic_when_enabled() {
        // On macOS this spawns afplay; on other platforms it is a no-op.
        // Either way it must not panic.
        play_start(true);
        play_stop(true);
        play_error(true);
    }
}
