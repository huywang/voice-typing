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

/// Central pipeline that coordinates recording → ASR → LLM → injection.
pub struct Pipeline {
    audio_thread: Option<AudioThread>,
    asr_client: Option<AsrClient>,
    llm_client: Option<LlmClient>,
    status: Arc<Mutex<AppStatus>>,
}

// Safety: The non-Send AudioRecorder lives inside a dedicated thread.
// We only interact with it through the Send+Sync RecordingBuffer.
unsafe impl Send for Pipeline {}
unsafe impl Sync for Pipeline {}

impl Pipeline {
    pub fn new() -> Self {
        Self {
            audio_thread: None,
            asr_client: None,
            llm_client: None,
            status: Arc::new(Mutex::new(AppStatus::Idle)),
        }
    }

    pub fn configure(&mut self, api_key: String, llm_enabled: bool) {
        self.asr_client = Some(AsrClient::new(api_key.clone()));
        let mut llm = LlmClient::new(api_key);
        llm.set_enabled(llm_enabled);
        self.llm_client = Some(llm);
    }

    pub fn status(&self) -> AppStatus {
        self.status.lock().unwrap().clone()
    }

    pub fn set_status(&self, status: AppStatus) {
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

    fn ensure_audio_thread(&mut self) -> Result<(), String> {
        if self.audio_thread.is_some() {
            return Ok(());
        }

        let buffer = RecordingBuffer::new();
        let buffer_clone = buffer.clone();

        let handle = thread::spawn(move || {
            let _recorder = match AudioRecorder::new(buffer_clone) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Failed to create audio recorder: {e}");
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

        Ok(())
    }

    pub fn start_recording(&mut self) -> Result<(), String> {
        self.ensure_audio_thread()?;
        self.set_status(AppStatus::Recording);
        self.audio_thread.as_ref().unwrap().buffer.start();
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
}
