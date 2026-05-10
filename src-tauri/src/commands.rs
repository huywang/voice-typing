use chrono::Utc;
use cpal::traits::{DeviceTrait, HostTrait};
use log::{debug, error, info, warn};
use serde::Serialize;
use std::sync::Mutex;
use std::time::Instant;
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt as AutostartManagerExt;
use tauri_plugin_store::StoreExt;

use crate::history::{HistoryDb, HistoryRecord};
use crate::pipeline::{AppStatus, Pipeline};

/// Shared history database state managed by Tauri.
pub struct HistoryState(pub HistoryDb);

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
pub async fn stop_and_process(
    app: AppHandle,
    state: State<'_, PipelineState>,
    history: State<'_, HistoryState>,
) -> Result<String, String> {
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

    // Keep a copy of the raw text for history before LLM may consume it.
    let raw_text_for_history = raw_text.clone();

    // Load personal dictionary from store (best-effort; empty on failure).
    let dictionary: Vec<String> = app
        .store("config.json")
        .ok()
        .and_then(|s| {
            s.get("dictionary")
                .and_then(|v| serde_json::from_value(v).ok())
        })
        .unwrap_or_default();
    if !dictionary.is_empty() {
        info!("Personal dictionary loaded: {} term(s)", dictionary.len());
    }

    // LLM polishing (with fallback)
    let final_text = if let Some(llm) = &llm_client {
        let llm_start = Instant::now();
        match llm.polish(&raw_text, &dictionary).await {
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

    // Update status and cache raw + polished text for the re-inject / AI-revert shortcuts.
    {
        let pipeline = state.0.lock().unwrap();
        pipeline.set_last_transcription(raw_text_for_history.clone(), final_text.clone());
        pipeline.set_status(AppStatus::Idle);
    }

    // Save to history
    let duration_secs = wav_data.len() as f64 / (16000.0 * 2.0); // 16kHz, 16-bit mono
    let record = HistoryRecord {
        id: 0,
        timestamp: Utc::now().to_rfc3339(),
        raw_text: raw_text_for_history,
        polished_text: final_text.clone(),
        duration_secs,
        app_name: String::new(),
    };
    if let Err(e) = history.0.insert(&record) {
        error!("Failed to save history record: {e}");
    } else {
        info!("History record saved");
    }

    Ok(final_text)
}

/// Get paginated history records, newest first.
#[tauri::command]
pub fn get_history(
    state: State<HistoryState>,
    limit: u32,
    offset: u32,
) -> Result<Vec<HistoryRecord>, String> {
    state.0.list(limit, offset)
}

/// Clear all history records.
#[tauri::command]
pub fn clear_history(state: State<HistoryState>) -> Result<(), String> {
    state.0.clear()
}

/// Get total number of history records.
#[tauri::command]
pub fn get_history_count(state: State<HistoryState>) -> Result<u32, String> {
    state.0.count()
}

/// Get whether launch-at-startup is currently enabled.
#[tauri::command]
pub fn get_autostart_enabled(app: AppHandle) -> Result<bool, String> {
    let enabled = app
        .autolaunch()
        .is_enabled()
        .map_err(|e| {
            error!("Failed to query autostart status: {e}");
            format!("Failed to query autostart status: {e}")
        })?;
    debug!("Autostart enabled: {}", enabled);
    Ok(enabled)
}

/// Enable or disable launch-at-startup.
#[tauri::command]
pub fn set_autostart_enabled(app: AppHandle, enabled: bool) -> Result<(), String> {
    let autolaunch = app.autolaunch();
    if enabled {
        autolaunch.enable().map_err(|e| {
            error!("Failed to enable autostart: {e}");
            format!("Failed to enable autostart: {e}")
        })?;
        info!("Autostart enabled");
    } else {
        autolaunch.disable().map_err(|e| {
            error!("Failed to disable autostart: {e}");
            format!("Failed to disable autostart: {e}")
        })?;
        info!("Autostart disabled");
    }
    Ok(())
}

/// Return true when running on macOS; used by the frontend to show macOS-only settings.
#[tauri::command]
pub fn is_macos() -> bool {
    cfg!(target_os = "macos")
}

/// Get whether the dock icon is visible (macOS only; always true on other platforms).
#[tauri::command]
pub fn get_dock_visible(app: AppHandle) -> bool {
    app.store("config.json")
        .ok()
        .and_then(|s| s.get("dock_visible"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

/// Show or hide the dock icon on macOS and persist the setting.
///
/// Uses `ActivationPolicy::Regular` to show and `ActivationPolicy::Accessory`
/// to hide. On non-macOS platforms this is a no-op (returns Ok).
#[tauri::command]
pub fn set_dock_visible(app: AppHandle, visible: bool) -> Result<(), String> {
    // Apply the activation policy on macOS only.
    #[cfg(target_os = "macos")]
    {
        use tauri::ActivationPolicy;
        let policy = if visible {
            ActivationPolicy::Regular
        } else {
            ActivationPolicy::Accessory
        };
        if let Err(e) = app.set_activation_policy(policy) {
            warn!("Failed to set activation policy: {e}");
        } else {
            info!("Dock icon visibility set to {visible}");
        }
    }

    // Persist the setting so it survives restarts.
    match app.store("config.json") {
        Ok(store) => {
            store.set("dock_visible", serde_json::json!(visible));
            if let Err(e) = store.save() {
                error!("Failed to save dock_visible setting: {e}");
                return Err(format!("Failed to save dock_visible: {e}"));
            }
            info!("dock_visible set to {visible} and persisted");
            Ok(())
        }
        Err(e) => {
            error!("Failed to open config store for dock_visible: {e}");
            Err(format!("Failed to open config store: {e}"))
        }
    }
}

/// Get whether sound effects are enabled.
#[tauri::command]
pub fn get_sound_enabled(app: AppHandle) -> bool {
    app.store("config.json")
        .ok()
        .and_then(|s| s.get("sound_enabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

/// Enable or disable interaction sound effects and persist the setting.
#[tauri::command]
pub fn set_sound_enabled(app: AppHandle, enabled: bool) -> Result<(), String> {
    match app.store("config.json") {
        Ok(store) => {
            store.set("sound_enabled", serde_json::json!(enabled));
            if let Err(e) = store.save() {
                error!("Failed to save sound_enabled setting: {e}");
                return Err(format!("Failed to save sound_enabled: {e}"));
            }
            info!("sound_enabled set to {enabled} and persisted");
            Ok(())
        }
        Err(e) => {
            error!("Failed to open config store for sound_enabled: {e}");
            Err(format!("Failed to open config store: {e}"))
        }
    }
}

/// Check whether the user has already completed the onboarding flow.
#[tauri::command]
pub fn is_onboarding_completed(app: AppHandle) -> bool {
    let completed = app
        .store("config.json")
        .ok()
        .and_then(|s| s.get("onboarding_completed"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    debug!("is_onboarding_completed: {completed}");
    completed
}

/// Mark the onboarding flow as completed and persist the flag.
#[tauri::command]
pub fn complete_onboarding(app: AppHandle) -> Result<(), String> {
    match app.store("config.json") {
        Ok(store) => {
            store.set("onboarding_completed", serde_json::json!(true));
            if let Err(e) = store.save() {
                error!("Failed to save onboarding_completed flag: {e}");
                return Err(format!("Failed to save onboarding flag: {e}"));
            }
            info!("Onboarding marked as completed");
            Ok(())
        }
        Err(e) => {
            error!("Failed to open config store for onboarding flag: {e}");
            Err(format!("Failed to open config store: {e}"))
        }
    }
}

/// Get the personal dictionary terms from the store.
#[tauri::command]
pub fn get_dictionary(app: AppHandle) -> Vec<String> {
    let terms: Vec<String> = app
        .store("config.json")
        .ok()
        .and_then(|s| {
            s.get("dictionary")
                .and_then(|v| serde_json::from_value(v).ok())
        })
        .unwrap_or_default();
    debug!("get_dictionary: {} term(s)", terms.len());
    terms
}

/// Save the personal dictionary terms to the store.
#[tauri::command]
pub fn set_dictionary(app: AppHandle, terms: Vec<String>) -> Result<(), String> {
    match app.store("config.json") {
        Ok(store) => {
            store.set("dictionary", serde_json::json!(terms));
            if let Err(e) = store.save() {
                error!("Failed to save dictionary: {e}");
                return Err(format!("Failed to save dictionary: {e}"));
            }
            info!("Personal dictionary saved: {} term(s)", terms.len());
            Ok(())
        }
        Err(e) => {
            error!("Failed to open config store for dictionary: {e}");
            Err(format!("Failed to open config store: {e}"))
        }
    }
}

/// Information about an available audio input device.
#[derive(Debug, Serialize)]
pub struct AudioDeviceInfo {
    /// Device name as reported by the OS.
    pub name: String,
    /// Whether this is the current system default input device.
    pub is_default: bool,
}

/// List all available audio input devices on this system.
#[tauri::command]
pub fn list_audio_devices() -> Result<Vec<AudioDeviceInfo>, String> {
    let host = cpal::default_host();

    let default_name = host
        .default_input_device()
        .and_then(|d| d.name().ok());

    let devices = host
        .input_devices()
        .map_err(|e| {
            error!("Failed to enumerate audio input devices: {e}");
            format!("Failed to enumerate audio devices: {e}")
        })?
        .filter_map(|d| d.name().ok())
        .map(|name| {
            let is_default = default_name.as_deref() == Some(name.as_str());
            AudioDeviceInfo { name, is_default }
        })
        .collect::<Vec<_>>();

    debug!("Enumerated {} audio input device(s)", devices.len());
    Ok(devices)
}

/// Persist the selected audio input device and update the live pipeline.
///
/// Pass `device_name = null` from the frontend to revert to the system default.
#[tauri::command]
pub fn set_audio_device(
    app: AppHandle,
    state: State<PipelineState>,
    device_name: Option<String>,
) -> Result<(), String> {
    info!(
        "Setting audio device: {}",
        device_name.as_deref().unwrap_or("system default")
    );

    // Update the live pipeline.
    {
        let mut pipeline = state.0.lock().unwrap();
        pipeline.set_selected_device(device_name.clone());
    }

    // Persist to store.
    match app.store("config.json") {
        Ok(store) => {
            store.set("selected_device", serde_json::json!(device_name));
            if let Err(e) = store.save() {
                error!("Failed to save selected_device: {e}");
                return Err(format!("Failed to save selected_device: {e}"));
            }
            info!("selected_device persisted to store");
        }
        Err(e) => {
            error!("Failed to open config store for selected_device: {e}");
            return Err(format!("Failed to open config store: {e}"));
        }
    }

    Ok(())
}

/// Get the currently persisted audio device selection (None = system default).
#[tauri::command]
pub fn get_audio_device(app: AppHandle) -> Option<String> {
    let result = app
        .store("config.json")
        .ok()
        .and_then(|s| s.get("selected_device"))
        .and_then(|v| {
            if v.is_null() {
                None
            } else {
                v.as_str().map(|s| s.to_string())
            }
        });
    debug!(
        "get_audio_device: {}",
        result.as_deref().unwrap_or("system default")
    );
    result
}

/// Set the translation target language and persist to store.
#[tauri::command]
pub fn set_translation_target(
    app: AppHandle,
    state: State<PipelineState>,
    language: String,
) -> Result<(), String> {
    info!("Setting translation target: {}", language);
    {
        let pipeline = state.0.lock().unwrap();
        pipeline.set_translation_target(language.clone());
    }
    match app.store("config.json") {
        Ok(store) => {
            store.set("translation_target", serde_json::json!(language));
            if let Err(e) = store.save() {
                error!("Failed to save translation_target: {e}");
                return Err(format!("Failed to save translation_target: {e}"));
            }
            info!("translation_target persisted to store");
        }
        Err(e) => {
            error!("Failed to open config store for translation_target: {e}");
            return Err(format!("Failed to open config store: {e}"));
        }
    }
    Ok(())
}

/// Get the currently persisted translation target language (default: "English").
#[tauri::command]
pub fn get_translation_target(app: AppHandle) -> String {
    let result = app
        .store("config.json")
        .ok()
        .and_then(|s| s.get("translation_target"))
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "English".to_string());
    debug!("get_translation_target: {}", result);
    result
}

/// Default push-to-talk hotkey string used when no custom value has been saved.
pub const DEFAULT_HOTKEY: &str = "CmdOrCtrl+Shift+Space";

/// Get the persisted push-to-talk hotkey string.
///
/// Returns the stored value or the built-in default if nothing has been saved yet.
#[tauri::command]
pub fn get_hotkey(app: AppHandle) -> String {
    let hotkey = app
        .store("config.json")
        .ok()
        .and_then(|s| s.get("push_to_talk_hotkey"))
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| DEFAULT_HOTKEY.to_string());
    debug!("get_hotkey: {hotkey}");
    hotkey
}

/// Persist the push-to-talk hotkey string to the store.
///
/// The new hotkey takes effect after the next application restart.
#[tauri::command]
pub fn set_hotkey(app: AppHandle, hotkey: String) -> Result<(), String> {
    info!("set_hotkey: {hotkey}");
    match app.store("config.json") {
        Ok(store) => {
            store.set("push_to_talk_hotkey", serde_json::json!(hotkey));
            if let Err(e) = store.save() {
                error!("Failed to save push_to_talk_hotkey: {e}");
                return Err(format!("Failed to save hotkey: {e}"));
            }
            info!("push_to_talk_hotkey persisted: {hotkey}");
            Ok(())
        }
        Err(e) => {
            error!("Failed to open config store for push_to_talk_hotkey: {e}");
            Err(format!("Failed to open config store: {e}"))
        }
    }
}
