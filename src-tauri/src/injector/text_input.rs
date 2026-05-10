use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use log::debug;

/// Injects text into the currently focused input field by simulating keyboard input.
pub struct TextInjector {
    enigo: Enigo,
}

impl TextInjector {
    /// Create a new `TextInjector`.
    ///
    /// On macOS, this requires Accessibility permission to be granted.
    pub fn new() -> Result<Self, InjectionError> {
        let enigo = Enigo::new(&Settings::default())
            .map_err(|e| InjectionError::InitError(format!("{e}")))?;
        Ok(Self { enigo })
    }

    /// Inject text into the currently focused input field.
    ///
    /// Simulates keyboard input character by character, supporting Unicode.
    pub fn inject(&mut self, text: &str) -> Result<(), InjectionError> {
        if text.is_empty() {
            return Ok(());
        }

        self.enigo
            .text(text)
            .map_err(|e| InjectionError::InputError(format!("{e}")))?;

        Ok(())
    }

    /// Inject text via the system clipboard.
    ///
    /// Writes `text` to the system clipboard, then simulates Cmd+V (macOS) or
    /// Ctrl+V (other platforms) to paste it. This is the reliable fallback for
    /// applications such as terminals or some editors where direct enigo key
    /// simulation does not work well.
    pub fn inject_via_clipboard(&mut self, text: &str) -> Result<(), InjectionError> {
        if text.is_empty() {
            return Ok(());
        }

        // Write text to the system clipboard.
        let mut clipboard = arboard::Clipboard::new()
            .map_err(|e| InjectionError::ClipboardError(format!("{e}")))?;
        clipboard
            .set_text(text)
            .map_err(|e| InjectionError::ClipboardError(format!("{e}")))?;
        debug!("Clipboard set ({} chars), sending paste shortcut", text.len());

        // Small delay so the clipboard contents are available before pasting.
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Simulate Cmd+V on macOS, Ctrl+V everywhere else.
        #[cfg(target_os = "macos")]
        let modifier = Key::Meta;
        #[cfg(not(target_os = "macos"))]
        let modifier = Key::Control;

        self.enigo
            .key(modifier, Direction::Press)
            .map_err(|e| InjectionError::InputError(format!("{e}")))?;
        self.enigo
            .key(Key::Unicode('v'), Direction::Click)
            .map_err(|e| InjectionError::InputError(format!("{e}")))?;
        self.enigo
            .key(modifier, Direction::Release)
            .map_err(|e| InjectionError::InputError(format!("{e}")))?;

        debug!("Clipboard paste shortcut sent");
        Ok(())
    }

    /// Smart injection: choose clipboard paste or direct key simulation.
    ///
    /// Pass `use_clipboard = true` for apps known to have poor compatibility with
    /// enigo's direct key simulation (e.g. terminals and some code editors).
    /// Pass `false` for everything else to get the lower-latency direct path.
    pub fn inject_smart(&mut self, text: &str, use_clipboard: bool) -> Result<(), InjectionError> {
        if use_clipboard {
            debug!("inject_smart: clipboard path");
            self.inject_via_clipboard(text)
        } else {
            debug!("inject_smart: direct key-simulation path");
            self.inject(text)
        }
    }
}

/// Errors that can occur during text injection.
#[derive(Debug, thiserror::Error)]
pub enum InjectionError {
    /// Failed to initialize the input simulator.
    /// On macOS, this typically means Accessibility permission is not granted.
    #[error("Failed to initialize input simulator: {0}")]
    InitError(String),

    /// Failed to simulate keyboard input.
    #[error("Failed to inject text: {0}")]
    InputError(String),

    /// Failed to access or write to the system clipboard.
    #[error("Clipboard error: {0}")]
    ClipboardError(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_injector_creation() {
        // This test may fail in CI environments without display access.
        // It verifies the API compiles and the constructor works.
        let result = TextInjector::new();
        // We don't assert success because CI may lack display/accessibility.
        // Just verify it doesn't panic.
        let _ = result;
    }

    #[test]
    fn test_inject_empty_string() {
        if let Ok(mut injector) = TextInjector::new() {
            // Empty string should be a no-op
            let result = injector.inject("");
            assert!(result.is_ok());
        }
    }

    #[test]
    fn test_inject_smart_empty() {
        if let Ok(mut injector) = TextInjector::new() {
            // inject_smart with empty string is a no-op regardless of the flag.
            assert!(injector.inject_smart("", false).is_ok());
            assert!(injector.inject_smart("", true).is_ok());
        }
    }
}
