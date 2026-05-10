use log::{debug, error, info};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::asr::AsrClient;
use crate::audio::{AudioRecorder, RecordingBuffer};
use crate::llm::LlmClient;

/// Application state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppStatus {
    Idle,
    Recording,
    Processing,
    Error(String),
}

/// Handle to the audio thread that manages the non-Send AudioRecorder.
pub struct AudioThread {
    buffer: RecordingBuffer,
    _handle: thread::JoinHandle<()>,
}

impl AudioThread {
    pub fn buffer(&self) -> &RecordingBuffer {
        &self.buffer
    }
}

/// Holds both the raw ASR output and the LLM-polished version for a single transcription.
#[derive(Clone, Debug)]
pub struct LastTranscription {
    /// Raw text as returned by ASR (before LLM polish).
    pub raw: String,
    /// Final text that was injected (may equal `raw` if LLM is disabled or failed).
    pub polished: String,
}

/// Central pipeline that coordinates recording -> ASR -> LLM -> injection.
pub struct Pipeline {
    audio_thread: Option<AudioThread>,
    asr_client: Option<AsrClient>,
    llm_client: Option<LlmClient>,
    status: Arc<Mutex<AppStatus>>,
    /// Cache of the most recent successfully injected transcription (both raw and polished).
    last_transcription: Arc<Mutex<Option<LastTranscription>>>,
    /// User-selected audio input device name. None means system default.
    selected_device: Option<String>,
    /// Target language for translation mode (e.g. "English", "中文").
    translation_target: Arc<Mutex<String>>,
}

// Safety: The non-Send AudioRecorder lives inside a dedicated thread.
// We only interact with it through the Send+Sync RecordingBuffer.
unsafe impl Send for Pipeline {}
unsafe impl Sync for Pipeline {}

impl Pipeline {
    pub fn new() -> Self {
        debug!("Pipeline created");
        Self {
            audio_thread: None,
            asr_client: None,
            llm_client: None,
            status: Arc::new(Mutex::new(AppStatus::Idle)),
            last_transcription: Arc::new(Mutex::new(None)),
            selected_device: None,
            translation_target: Arc::new(Mutex::new("English".to_string())),
        }
    }

    pub fn configure(&mut self, api_key: String, llm_enabled: bool) {
        info!(
            "Pipeline configured: api_key={}***, llm_enabled={}",
            &api_key[..api_key.len().min(4)],
            llm_enabled
        );
        self.asr_client = Some(AsrClient::new(api_key.clone()));
        let mut llm = LlmClient::new(api_key);
        llm.set_enabled(llm_enabled);
        self.llm_client = Some(llm);
    }

    pub fn status(&self) -> AppStatus {
        self.status.lock().unwrap().clone()
    }

    pub fn set_status(&self, status: AppStatus) {
        debug!("Status: {:?}", status);
        *self.status.lock().unwrap() = status;
    }

    pub fn asr_client(&self) -> Option<&AsrClient> {
        self.asr_client.as_ref()
    }

    pub fn llm_client(&self) -> Option<&LlmClient> {
        self.llm_client.as_ref()
    }

    pub fn audio_thread_ref(&self) -> Option<&AudioThread> {
        self.audio_thread.as_ref()
    }

    /// Set the preferred audio input device by name.
    ///
    /// Drops the current audio thread so the next recording uses the new device.
    pub fn set_selected_device(&mut self, device: Option<String>) {
        info!(
            "Selected audio device changed to: {}",
            device.as_deref().unwrap_or("system default")
        );
        self.selected_device = device;
        // Drop existing audio thread so it is re-created with the new device on next use.
        self.audio_thread = None;
    }

    /// Get the currently selected device name (None = system default).
    pub fn selected_device(&self) -> Option<&str> {
        self.selected_device.as_deref()
    }

    /// Get the current translation target language.
    pub fn translation_target(&self) -> String {
        self.translation_target.lock().unwrap().clone()
    }

    /// Set the translation target language (e.g. "English", "中文").
    pub fn set_translation_target(&self, language: String) {
        info!("Translation target set to: {}", language);
        *self.translation_target.lock().unwrap() = language;
    }

    fn ensure_audio_thread(&mut self) -> Result<(), String> {
        if self.audio_thread.is_some() {
            return Ok(());
        }

        info!("Initializing audio thread...");
        let buffer = RecordingBuffer::new();
        let buffer_clone = buffer.clone();
        // Clone the device name so it can be moved into the spawned thread.
        let device_name = self.selected_device.clone();

        let handle = thread::spawn(move || {
            let _recorder = match AudioRecorder::new(buffer_clone, device_name.as_deref()) {
                Ok(r) => {
                    info!("AudioRecorder initialized on dedicated thread");
                    r
                }
                Err(e) => {
                    error!("Failed to create audio recorder: {e}");
                    return;
                }
            };
            loop {
                thread::park();
            }
        });

        // Give the audio thread time to initialize.
        thread::sleep(std::time::Duration::from_millis(100));

        self.audio_thread = Some(AudioThread {
            buffer,
            _handle: handle,
        });

        info!("Audio thread ready");
        Ok(())
    }

    /// Cache the most recently injected transcription (both raw and polished text).
    pub fn set_last_transcription(&self, raw: String, polished: String) {
        debug!(
            "Caching last transcription: raw={} chars, polished={} chars",
            raw.len(),
            polished.len()
        );
        *self.last_transcription.lock().unwrap() = Some(LastTranscription { raw, polished });
    }

    /// Return the polished text from the most recently cached transcription, if any.
    pub fn last_transcription(&self) -> Option<String> {
        self.last_transcription
            .lock()
            .unwrap()
            .as_ref()
            .map(|t| t.polished.clone())
    }

    /// Return the raw ASR text from the most recently cached transcription, if any.
    pub fn last_raw_transcription(&self) -> Option<String> {
        self.last_transcription
            .lock()
            .unwrap()
            .as_ref()
            .map(|t| t.raw.clone())
    }

    pub fn start_recording(&mut self) -> Result<(), String> {
        self.ensure_audio_thread()?;
        self.set_status(AppStatus::Recording);
        self.audio_thread.as_ref().unwrap().buffer.start();
        info!("Recording started");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_creation() {
        let pipeline = Pipeline::new();
        assert_eq!(pipeline.status(), AppStatus::Idle);
    }

    #[test]
    fn test_pipeline_configure() {
        let mut pipeline = Pipeline::new();
        pipeline.configure("test-key".to_string(), true);
        assert!(pipeline.asr_client.is_some());
        assert!(pipeline.llm_client.is_some());
    }

    #[test]
    fn test_status_transitions() {
        let pipeline = Pipeline::new();
        assert_eq!(pipeline.status(), AppStatus::Idle);
        pipeline.set_status(AppStatus::Recording);
        assert_eq!(pipeline.status(), AppStatus::Recording);
        pipeline.set_status(AppStatus::Processing);
        assert_eq!(pipeline.status(), AppStatus::Processing);
    }

    #[test]
    fn test_set_selected_device() {
        let mut pipeline = Pipeline::new();
        assert!(pipeline.selected_device().is_none());
        pipeline.set_selected_device(Some("Test Mic".to_string()));
        assert_eq!(pipeline.selected_device(), Some("Test Mic"));
        pipeline.set_selected_device(None);
        assert!(pipeline.selected_device().is_none());
    }
}
