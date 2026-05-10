use log::{debug, error, info, warn};
use std::sync::Mutex;
use std::time::Instant;
use tauri::{AppHandle, State};
use tauri_plugin_store::StoreExt;

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

/// Configure API keys and persist to store.
#[tauri::command]
pub fn set_api_config(
    app: AppHandle,
    state: State<PipelineState>,
    api_key: String,
    llm_enabled: bool,
) -> Result<(), String> {
    info!("Configuring API: llm_enabled={}", llm_enabled);
    let mut pipeline = state.0.lock().unwrap();
    pipeline.configure(api_key.clone(), llm_enabled);

    // Persist to store
    match app.store("config.json") {
        Ok(store) => {
            store.set("api_key", serde_json::json!(api_key));
            store.set("llm_enabled", serde_json::json!(llm_enabled));
            if let Err(e) = store.save() {
                error!("Failed to save config store: {e}");
                return Err(format!("Failed to save config: {e}"));
            }
            info!("API config persisted to store");
        }
        Err(e) => {
            error!("Failed to open config store: {e}");
            return Err(format!("Failed to open config store: {e}"));
        }
    }

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
    let pipeline_start = Instant::now();
    info!("stop_and_process command invoked");

    // Extract what we need from the pipeline under lock, then drop it.
    let (wav_result, asr_client, llm_client) = {
        let pipeline = state.0.lock().unwrap();

        let audio = pipeline
            .audio_thread_ref()
            .ok_or("Audio not initialized")?;

        audio.buffer().stop();
        pipeline.set_status(AppStatus::Processing);

        // get_wav_data handles mono mixing + resampling to 16kHz
        let wav = audio.buffer().get_wav_data();

        (
            wav,
            pipeline.asr_client().cloned(),
            pipeline.llm_client().cloned(),
        )
    };
    // MutexGuard is dropped here.

    // Encode WAV (with proper resampling)
    let wav_data = wav_result.map_err(|e| {
        warn!("WAV encoding failed: {e}");
        let pipeline = state.0.lock().unwrap();
        pipeline.set_status(AppStatus::Idle);
        format!("WAV encoding failed: {e}")
    })?;

    info!("WAV encoded: {} bytes", wav_data.len());

    // ASR
    let asr = asr_client.ok_or("ASR not configured. Please set your API key.")?;
    let asr_start = Instant::now();
    let raw_text = asr
        .recognize(&wav_data)
        .await
        .map_err(|e| {
            error!("ASR failed: {e}");
            format!("ASR failed: {e}")
        })?;
    info!(
        "ASR completed in {:.1}s: \"{}\"",
        asr_start.elapsed().as_secs_f64(),
        raw_text
    );

    // LLM polishing (with fallback)
    let final_text = if let Some(llm) = &llm_client {
        let llm_start = Instant::now();
        match llm.polish(&raw_text).await {
            Ok(polished) => {
                info!(
                    "LLM polished in {:.1}s: \"{}\"",
                    llm_start.elapsed().as_secs_f64(),
                    polished
                );
                polished
            }
            Err(e) => {
                warn!("LLM polishing failed, using raw text: {e}");
                raw_text
            }
        }
    } else {
        debug!("LLM disabled, skipping polish");
        raw_text
    };

    // Inject text
    let mut injector = crate::injector::TextInjector::new()
        .map_err(|e| {
            error!("Failed to init text injector: {e}");
            format!("Failed to init text injector: {e}")
        })?;
    injector
        .inject(&final_text)
        .map_err(|e| {
            error!("Failed to inject text: {e}");
            format!("Failed to inject text: {e}")
        })?;

    info!(
        "Pipeline (command) completed in {:.1}s, injected {} chars",
        pipeline_start.elapsed().as_secs_f64(),
        final_text.len()
    );

    // Update status
    {
        let pipeline = state.0.lock().unwrap();
        pipeline.set_status(AppStatus::Idle);
    }

    Ok(final_text)
}
