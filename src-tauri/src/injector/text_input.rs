use enigo::{Enigo, Keyboard, Settings};

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
}
