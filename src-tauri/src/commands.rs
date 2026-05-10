use std::sync::Mutex;
use tauri::State;

use crate::pipeline::{AppStatus, Pipeline};

/// Shared pipeline state managed by Tauri.
pub struct PipelineState(pub Mutex<Pipeline>);

/// Get the current application status.
#[tauri::command]
pub fn get_status(state: State<PipelineState>) -> String {
    let pipeline = state.0.lock().unwrap();
    match pipeline.status() {
        AppStatus::Idle => "idle".to_string(),
        AppStatus::Recording => "recording".to_string(),
        AppStatus::Processing => "processing".to_string(),
        AppStatus::Error(msg) => format!("error:{msg}"),
    }
}

/// Configure API keys.
#[tauri::command]
pub fn set_api_config(
    state: State<PipelineState>,
    api_key: String,
    llm_enabled: bool,
) -> Result<(), String> {
    let mut pipeline = state.0.lock().unwrap();
    pipeline.configure(api_key, llm_enabled);
    Ok(())
}

/// Start recording audio.
#[tauri::command]
pub fn start_recording(state: State<PipelineState>) -> Result<(), String> {
    let mut pipeline = state.0.lock().unwrap();
    pipeline.start_recording()
}

/// Stop recording and process the audio (ASR -> LLM -> inject).
///
/// We extract the data we need from the locked pipeline, drop the lock,
/// then perform async operations without holding the MutexGuard.
#[tauri::command]
pub async fn stop_and_process(state: State<'_, PipelineState>) -> Result<String, String> {
    // Extract what we need from the pipeline under lock, then drop it.
    let (samples, asr_client, llm_client) = {
        let pipeline = state.0.lock().unwrap();

        let audio = pipeline
            .audio_thread_ref()
            .ok_or("Audio not initialized")?;

        audio.buffer().stop();
        pipeline.set_status(AppStatus::Processing);

        let samples = audio.buffer().samples();
        if samples.is_empty() {
            pipeline.set_status(AppStatus::Idle);
            return Err("No audio data recorded".to_string());
        }

        (
            samples,
            pipeline.asr_client().cloned(),
            pipeline.llm_client().cloned(),
        )
    };
    // MutexGuard is dropped here.

    // Encode WAV
    let wav_data = crate::audio::encode_wav(&samples, 16000)
        .map_err(|e| format!("WAV encoding failed: {e}"))?;

    // ASR
    let asr = asr_client.ok_or("ASR not configured. Please set your API key.")?;
    let raw_text = asr
        .recognize(&wav_data)
        .await
        .map_err(|e| format!("ASR failed: {e}"))?;

    // LLM polishing (with fallback)
    let final_text = if let Some(llm) = &llm_client {
        match llm.polish(&raw_text).await {
            Ok(polished) => polished,
            Err(e) => {
                eprintln!("LLM polishing failed, using raw text: {e}");
                raw_text
            }
        }
    } else {
        raw_text
    };

    // Inject text
    let mut injector = crate::injector::TextInjector::new()
        .map_err(|e| format!("Failed to init text injector: {e}"))?;
    injector
        .inject(&final_text)
        .map_err(|e| format!("Failed to inject text: {e}"))?;

    // Update status
    {
        let pipeline = state.0.lock().unwrap();
        pipeline.set_status(AppStatus::Idle);
    }

    Ok(final_text)
}
