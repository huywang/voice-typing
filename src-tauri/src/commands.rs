use chrono::Utc;
use cpal::traits::{DeviceTrait, HostTrait};
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::Instant;
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt as AutostartManagerExt;
use tauri_plugin_global_shortcut::GlobalShortcutExt;
use tauri_plugin_store::StoreExt;

use crate::asr::{AsrClient, AsrError};
use crate::audio::encode_wav;
use crate::history::{HistoryDb, HistoryRecord};
use crate::pipeline::{AppStatus, Pipeline};

/// Shared history database state managed by Tauri.
pub struct HistoryState(pub HistoryDb);

/// Shared pipeline state managed by Tauri.
pub struct PipelineState(pub Mutex<Pipeline>);

/// Managed state holding the currently active push-to-talk hotkey string.
///
/// Protected by a `Mutex` so `set_hotkey` can atomically swap the old hotkey
/// for the new one while preventing concurrent re-registration races.
pub struct CurrentHotkey(pub Mutex<String>);

/// Inner data for the floating bar state, serialisable for IPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FloatingStateData {
    /// Current status: "idle", "recording", "processing", or "result".
    pub status: String,
    /// Injected text, populated when status == "result".
    pub text: String,
}

/// Managed state for the floating bar, shared between the hotkey handler and commands.
pub struct FloatingState(pub Mutex<FloatingStateData>);

/// Managed state that temporarily holds the selected text captured when the
/// speak-to-edit hotkey is pressed. Cleared after each edit pipeline run.
pub struct EditContext(pub Mutex<Option<String>>);

/// Get the current floating bar state (status + injected text).
#[tauri::command]
pub fn get_floating_state(state: State<FloatingState>) -> FloatingStateData {
    state.0.lock().unwrap().clone()
}

/// Set the floating bar state. Called by lib.rs after injection or by the frontend on dismiss.
#[tauri::command]
pub fn set_floating_state(
    state: State<FloatingState>,
    status: String,
    text: String,
) -> Result<(), String> {
    debug!("set_floating_state: status={status}");
    let mut inner = state.0.lock().unwrap();
    inner.status = status;
    inner.text = text;
    Ok(())
}

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

    // Detect the focused app and derive the appropriate tone for polishing.
    let app_name = crate::context::get_frontmost_app_name();
    let tone = crate::context::get_tone_for_app(&app_name);
    debug!("Context-aware tone for \"{}\": {}", app_name, tone);

    // Load custom blacklist from store (best-effort; falls back to empty).
    let custom_blacklist: Vec<String> = app
        .store("config.json")
        .ok()
        .and_then(|s| {
            s.get("app_blacklist")
                .and_then(|v| serde_json::from_value(v).ok())
        })
        .unwrap_or_default();
    let use_clipboard = crate::context::is_blacklisted(&app_name, &custom_blacklist);
    if use_clipboard {
        info!("App '{}' is blacklisted — using clipboard paste fallback", app_name);
    }

    // LLM polishing (with fallback)
    let final_text = if let Some(llm) = &llm_client {
        let llm_start = Instant::now();
        match llm.polish(&raw_text, &dictionary, tone).await {
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

    // Inject text — use clipboard paste for blacklisted apps, direct simulation otherwise.
    let mut injector = crate::injector::TextInjector::new()
        .map_err(|e| {
            error!("Failed to init text injector: {e}");
            format!("Failed to init text injector: {e}")
        })?;
    injector
        .inject_smart(&final_text, use_clipboard)
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
        app_name: crate::context::get_frontmost_app_name(),
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
/// Delete a single history record by id.
#[tauri::command]
pub fn delete_history_item(state: State<HistoryState>, id: i64) -> Result<(), String> {
    info!("delete_history_item: id={id}");
    state.0.delete(id)
}

/// Re-inject the polished text of a history record into the currently focused input field.
#[tauri::command]
pub fn reinject_history_item(state: State<HistoryState>, id: i64) -> Result<(), String> {
    info!("reinject_history_item: id={id}");

    // Look up the record first.
    let record = state
        .0
        .get(id)?
        .ok_or_else(|| format!("History record {id} not found"))?;

    // Inject the polished text.
    let mut injector = crate::injector::TextInjector::new().map_err(|e| {
        error!("Failed to init text injector for re-inject: {e}");
        format!("Failed to init text injector: {e}")
    })?;
    injector.inject(&record.polished_text).map_err(|e| {
        error!("Failed to re-inject history record {id}: {e}");
        format!("Failed to inject text: {e}")
    })?;

    info!(
        "Re-injected history record {id} ({} chars)",
        record.polished_text.len()
    );
    Ok(())
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

/// Test whether the given DashScope API key can reach the ASR service.
///
/// Generates a short silent WAV (500 ms at 16 kHz) and sends it to the ASR
/// endpoint.  Because the audio is silence, the API may return an empty result
/// — that is still treated as a successful connection.  Only network errors or
/// HTTP 401/403 authentication failures are reported as errors.
#[tauri::command]
pub async fn test_api_key(api_key: String) -> Result<String, String> {
    info!("test_api_key: validating API key connectivity");

    if api_key.trim().is_empty() {
        return Err("API key cannot be empty".to_string());
    }

    // Generate 500 ms of silence at 16 kHz — small enough to be fast,
    // large enough for the API to accept as valid audio.
    let silence_samples = vec![0.0f32; 8000]; // 0.5 s × 16 000 Hz
    let wav_data = encode_wav(&silence_samples, 16000)
        .map_err(|e| format!("Failed to encode test WAV: {e}"))?;

    debug!("test_api_key: sending {} bytes WAV to ASR", wav_data.len());

    let client = AsrClient::new(api_key.trim().to_string());

    match client.recognize(&wav_data).await {
        Ok(text) => {
            info!("test_api_key: ASR returned text: {:?}", text);
            Ok("ok".to_string())
        }
        Err(AsrError::EmptyResult) => {
            // Silence produces no transcript — the API accepted the request,
            // so the key is valid.
            info!("test_api_key: ASR returned empty result (silence) — key is valid");
            Ok("ok".to_string())
        }
        Err(AsrError::ApiError(msg)) => {
            warn!("test_api_key: API error: {}", msg);
            // Surface HTTP 401/403 as authentication failures.
            if msg.contains("401") || msg.contains("403") {
                Err(format!("Authentication failed — check your API key ({})", msg))
            } else {
                Err(format!("API error: {}", msg))
            }
        }
        Err(AsrError::NetworkError(msg)) => {
            warn!("test_api_key: network error: {}", msg);
            Err(format!("Network error — check your internet connection ({})", msg))
        }
        Err(e) => {
            warn!("test_api_key: unexpected error: {}", e);
            Err(format!("Connection test failed: {e}"))
        }
    }
}

/// Permission status for microphone and accessibility.
#[derive(Debug, Serialize)]
pub struct PermissionStatus {
    /// Whether microphone access is granted.
    pub microphone: bool,
    /// Whether accessibility access is granted.
    pub accessibility: bool,
}

/// Check whether microphone access is available.
///
/// On macOS, we probe the default input device via cpal as a lightweight
/// proxy for permission status. If cpal can enumerate a device, the OS has
/// granted (or not yet blocked) microphone access.
fn check_microphone_permission() -> bool {
    let host = cpal::default_host();
    let has_device = host.default_input_device().is_some();
    debug!("check_microphone_permission: has_device={has_device}");
    has_device
}

/// Check whether accessibility access is granted (macOS only).
///
/// Runs a trivial AppleScript via `osascript`. If accessibility is not
/// granted, System Events will deny the request and the command will fail.
fn check_accessibility_permission() -> bool {
    #[cfg(target_os = "macos")]
    {
        let result = std::process::Command::new("osascript")
            .arg("-e")
            .arg("tell application \"System Events\" to return true")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        debug!("check_accessibility_permission: {result}");
        result
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// Return the current microphone and accessibility permission status.
#[tauri::command]
pub fn check_permissions() -> PermissionStatus {
    let status = PermissionStatus {
        microphone: check_microphone_permission(),
        accessibility: check_accessibility_permission(),
    };
    info!(
        "check_permissions: microphone={}, accessibility={}",
        status.microphone, status.accessibility
    );
    status
}

/// Open a specific macOS System Preferences / System Settings pane.
///
/// `pane` can be `"microphone"` or `"accessibility"`.
/// On non-macOS platforms this is a no-op (returns Ok).
#[tauri::command]
pub fn open_system_preferences(pane: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let url = match pane.as_str() {
            "microphone" => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
            }
            "accessibility" => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
            }
            other => {
                warn!("open_system_preferences: unknown pane '{other}'");
                return Err(format!("Unknown pane: {other}"));
            }
        };
        info!("open_system_preferences: opening pane '{pane}' → {url}");
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|e| {
                error!("Failed to open system preferences: {e}");
                format!("Failed to open system preferences: {e}")
            })?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        debug!("open_system_preferences: no-op on non-macOS (pane={pane})");
        Ok(())
    }
}

/// Get the user-defined app compatibility blacklist from the store.
///
/// Returns the custom list of app names for which the clipboard-paste injection
/// fallback should be used. The built-in default list is always active in
/// addition to this custom list.
#[tauri::command]
pub fn get_blacklist(app: AppHandle) -> Vec<String> {
    let list: Vec<String> = app
        .store("config.json")
        .ok()
        .and_then(|s| {
            s.get("app_blacklist")
                .and_then(|v| serde_json::from_value(v).ok())
        })
        .unwrap_or_default();
    debug!("get_blacklist: {} entry/entries", list.len());
    list
}

/// Persist the user-defined app compatibility blacklist to the store.
///
/// The provided list is merged at runtime with the built-in `DEFAULT_BLACKLIST`
/// inside [`crate::context::is_blacklisted`].
#[tauri::command]
pub fn set_blacklist(app: AppHandle, apps: Vec<String>) -> Result<(), String> {
    info!("set_blacklist: {} entry/entries", apps.len());
    match app.store("config.json") {
        Ok(store) => {
            store.set("app_blacklist", serde_json::json!(apps));
            if let Err(e) = store.save() {
                error!("Failed to save app_blacklist: {e}");
                return Err(format!("Failed to save app_blacklist: {e}"));
            }
            info!("app_blacklist persisted");
            Ok(())
        }
        Err(e) => {
            error!("Failed to open config store for app_blacklist: {e}");
            Err(format!("Failed to open config store: {e}"))
        }
    }
}

/// Get the history retention period in days from the store.
///
/// Returns -1 to indicate "forever" (no cleanup). Default is -1.
#[tauri::command]
pub fn get_retention_days(app: AppHandle) -> i32 {
    let days = app
        .store("config.json")
        .ok()
        .and_then(|s| s.get("retention_days"))
        .and_then(|v| v.as_i64())
        .map(|v| v as i32)
        .unwrap_or(-1);
    debug!("get_retention_days: {days}");
    days
}

/// Set the history retention period in days and persist to store.
///
/// Use -1 to disable cleanup (retain forever).
#[tauri::command]
pub fn set_retention_days(app: AppHandle, days: i32) -> Result<(), String> {
    info!("set_retention_days: {days}");
    match app.store("config.json") {
        Ok(store) => {
            store.set("retention_days", serde_json::json!(days));
            if let Err(e) = store.save() {
                error!("Failed to save retention_days: {e}");
                return Err(format!("Failed to save retention_days: {e}"));
            }
            info!("retention_days persisted: {days}");
            Ok(())
        }
        Err(e) => {
            error!("Failed to open config store for retention_days: {e}");
            Err(format!("Failed to open config store: {e}"))
        }
    }
}

/// Persist the push-to-talk hotkey and live-reload it without requiring a restart.
///
/// Steps:
/// 1. Acquire the `CurrentHotkey` mutex to prevent concurrent re-registration.
/// 2. Unregister the old shortcut.
/// 3. Register the new shortcut via [`crate::register_push_to_talk_shortcut`].
/// 4. If registration fails, attempt to restore the old shortcut so the app
///    remains functional.
/// 5. Persist the new hotkey to the config store.
#[tauri::command]
pub fn set_hotkey(
    app: AppHandle,
    current_hotkey: State<CurrentHotkey>,
    hotkey: String,
) -> Result<(), String> {
    let hotkey = hotkey.trim().to_string();
    if hotkey.is_empty() {
        return Err("Hotkey cannot be empty".to_string());
    }

    info!("set_hotkey: changing hotkey to '{hotkey}'");

    // Hold the lock for the entire operation so concurrent calls are serialised.
    let mut current = current_hotkey.0.lock().unwrap();
    let old_hotkey = current.clone();

    if old_hotkey == hotkey {
        info!("set_hotkey: hotkey unchanged, nothing to do");
        return Ok(());
    }

    // Unregister the old push-to-talk shortcut.
    if let Err(e) = app.global_shortcut().unregister(old_hotkey.as_str()) {
        // Non-fatal: the old hotkey may already be gone (e.g. on first run with
        // a default that was never explicitly registered through this path).
        warn!("set_hotkey: failed to unregister old hotkey '{old_hotkey}': {e}");
    } else {
        info!("set_hotkey: unregistered old hotkey '{old_hotkey}'");
    }

    // Attempt to register the new hotkey.
    if let Err(e) = crate::register_push_to_talk_shortcut(&app, &hotkey) {
        error!("set_hotkey: failed to register new hotkey '{hotkey}': {e}");

        // Rollback: try to restore the old hotkey so the app stays functional.
        if let Err(re) = crate::register_push_to_talk_shortcut(&app, &old_hotkey) {
            error!("set_hotkey: rollback also failed, old hotkey '{old_hotkey}' is unregistered: {re}");
        } else {
            info!("set_hotkey: rolled back to old hotkey '{old_hotkey}'");
        }

        return Err(format!(
            "Invalid hotkey '{hotkey}' — could not register it: {e}"
        ));
    }

    info!("set_hotkey: registered new hotkey '{hotkey}'");
    *current = hotkey.clone();

    // Persist the new hotkey so it survives restarts.
    match app.store("config.json") {
        Ok(store) => {
            store.set("push_to_talk_hotkey", serde_json::json!(hotkey));
            if let Err(e) = store.save() {
                error!("Failed to save push_to_talk_hotkey: {e}");
                return Err(format!("Failed to save hotkey: {e}"));
            }
            info!("push_to_talk_hotkey persisted: {hotkey}");
        }
        Err(e) => {
            error!("Failed to open config store for push_to_talk_hotkey: {e}");
            return Err(format!("Failed to open config store: {e}"));
        }
    }

    Ok(())
}

// ─── Feedback ─────────────────────────────────────────────────────────────────

/// Submit a feature request or bug report as a GitHub Issue.
///
/// If a `github_token` is stored in the config, the issue is created directly
/// via the GitHub API and the issue URL is returned.  When no token is
/// configured the command returns an error asking the user to set one up.
#[tauri::command]
pub async fn submit_feedback(
    app: AppHandle,
    feedback_type: String, // "feature" or "bug"
    title: String,
    description: String,
) -> Result<String, String> {
    info!(
        "submit_feedback: type={} title=\"{}\"",
        feedback_type, title
    );

    let title = title.trim().to_string();
    let description = description.trim().to_string();

    if title.is_empty() {
        return Err("Title cannot be empty".to_string());
    }

    // Use the built-in bot token (embedded at compile time) so end users
    // can submit feedback without configuring anything.  Falls back to
    // a user-configured token in the store if the env var was not set at
    // build time.
    let token = option_env!("GITHUB_BOT_TOKEN")
        .map(String::from)
        .filter(|t| !t.is_empty())
        .or_else(|| {
            app.store("config.json")
                .ok()
                .and_then(|s| s.get("github_token"))
                .and_then(|v| v.as_str().map(String::from))
                .filter(|t| !t.trim().is_empty())
        });

    let token = match token {
        Some(t) => t,
        None => {
            warn!("submit_feedback: no GitHub token available");
            return Err(
                "Feedback submission is not available. Please contact the developer."
                    .to_string(),
            );
        }
    };

    let label = if feedback_type == "bug" {
        "bug"
    } else {
        "enhancement"
    };

    let body_text = format!("**Type:** {}\n\n{}", feedback_type, description);

    let client = reqwest::Client::new();
    let payload = serde_json::json!({
        "title": title,
        "body": body_text,
        "labels": [label, "user-feedback"]
    });

    let resp = client
        .post("https://api.github.com/repos/huywang/voice-typing/issues")
        .header("Authorization", format!("Bearer {token}"))
        .header("User-Agent", "voice-typing-app")
        .header("Accept", "application/vnd.github.v3+json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| {
            warn!("submit_feedback: network error: {e}");
            format!("Network error: {e}")
        })?;

    if resp.status().is_success() {
        let issue: serde_json::Value = resp.json().await.map_err(|e| {
            error!("submit_feedback: failed to parse API response: {e}");
            format!("Parse error: {e}")
        })?;
        let url = issue["html_url"].as_str().unwrap_or("").to_string();
        info!("submit_feedback: issue created at {url}");
        Ok(url)
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        warn!("submit_feedback: GitHub API error {status}: {body}");
        if status.as_u16() == 401 || status.as_u16() == 403 {
            Err(format!(
                "GitHub authentication failed ({status}). Check your token has 'repo' or 'public_repo' scope."
            ))
        } else {
            Err(format!("GitHub API error {status}: {body}"))
        }
    }
}

/// Persist the GitHub personal access token to the config store.
#[tauri::command]
pub fn set_github_token(app: AppHandle, token: String) -> Result<(), String> {
    info!("set_github_token: updating token");
    match app.store("config.json") {
        Ok(store) => {
            store.set("github_token", serde_json::json!(token.trim()));
            if let Err(e) = store.save() {
                error!("Failed to save github_token: {e}");
                return Err(format!("Failed to save token: {e}"));
            }
            info!("github_token persisted");
            Ok(())
        }
        Err(e) => {
            error!("Failed to open config store for github_token: {e}");
            Err(format!("Failed to open config store: {e}"))
        }
    }
}

// ─── Updater ──────────────────────────────────────────────────────────────────

/// Information about an available update returned to the frontend.
#[derive(Debug, Serialize)]
pub struct UpdateInfo {
    /// Version string of the available update (e.g. "1.2.0").
    pub version: String,
    /// Release notes / changelog body from the update manifest.
    pub body: String,
}

/// Check for a new application update using the configured update endpoint.
///
/// Returns `Some(UpdateInfo)` when a newer version is available, or `None`
/// when the app is already up to date.  Returns an `Err` string if the check
/// itself fails (e.g. no network, endpoint not yet configured).
#[tauri::command]
pub async fn check_for_update(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    use tauri_plugin_updater::UpdaterExt;

    info!("Checking for updates (user-initiated)");

    let updater = app.updater().map_err(|e| {
        debug!("Updater not available: {e}");
        format!("Updater not available: {e}")
    })?;

    match updater.check().await {
        Ok(Some(update)) => {
            let version = update.version.clone();
            let body = update.body.clone().unwrap_or_default();
            info!("Update available: v{version}");
            Ok(Some(UpdateInfo { version, body }))
        }
        Ok(None) => {
            info!("Already on the latest version");
            Ok(None)
        }
        Err(e) => {
            warn!("Update check failed: {e}");
            Err(format!("Update check failed: {e}"))
        }
    }
}

/// Get the currently selected text in the focused application.
///
/// On macOS, first tries the Accessibility API via AppleScript, then falls back
/// to simulating Cmd+C and reading the clipboard.  Returns `None` when no text
/// is selected or the feature is not supported on the current platform.
///
/// This is used by the speak-to-edit feature to provide surrounding context to
/// the LLM so it can make edits that are aware of existing text.
#[tauri::command]
pub fn get_selected_text() -> Option<String> {
    debug!("get_selected_text command invoked");
    let result = crate::context::get_selected_text();
    match &result {
        Some(t) => info!("get_selected_text: returning {} chars", t.len()),
        None => debug!("get_selected_text: no selected text"),
    }
    result
}

/// Download and install the latest available update, then restart the app.
///
/// Should only be called after `check_for_update` returns `Some(...)`.
/// Downloads the update package, installs it, and requests a full application
/// restart so the new version becomes active immediately.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;

    info!("Installing update (user-initiated)");

    let updater = app
        .updater()
        .map_err(|e| format!("Updater not available: {e}"))?;

    let update = updater
        .check()
        .await
        .map_err(|e| format!("Failed to fetch update info: {e}"))?;

    match update {
        Some(update) => {
            info!("Downloading and installing update v{}", update.version);
            update
                .download_and_install(|_chunk, _total| {}, || {})
                .await
                .map_err(|e| {
                    error!("Update installation failed: {e}");
                    format!("Installation failed: {e}")
                })?;
            info!("Update installed — restarting app");
            app.restart();
        }
        None => {
            info!("install_update called but no update is available");
        }
    }

    Ok(())
}
