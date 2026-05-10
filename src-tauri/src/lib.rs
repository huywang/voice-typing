mod asr;
mod audio;
mod commands;
mod context;
mod history;
mod injector;
mod llm;
mod pipeline;
mod sound;
mod tray;

use chrono::Utc;
use commands::{CurrentHotkey, EditContext, FloatingState, FloatingStateData, HistoryState, PipelineState};
use history::{HistoryDb, HistoryRecord};
use log::{debug, error, info, warn};
use pipeline::Pipeline;
use std::sync::Mutex;
use std::time::Instant;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_store::StoreExt;


/// Show the transparent floating-bar window, positioned at the bottom center
/// of the monitor that currently contains the mouse cursor.
///
/// On multi-monitor setups this ensures the bar appears on the active screen
/// rather than always on the primary display.
fn show_floating_bar(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("floating-bar") {
        // Attempt to position the bar on the monitor that contains the cursor.
        // If any step fails we fall back to just showing the window at its
        // existing position — positioning is best-effort.
        match (win.available_monitors(), win.cursor_position()) {
            (Ok(monitors), Ok(cursor)) => {
                // Find which monitor the cursor is on.
                let target = monitors.iter().find(|m| {
                    let pos = m.position();
                    let size = m.size();
                    cursor.x >= pos.x as f64
                        && cursor.x < (pos.x + size.width as i32) as f64
                        && cursor.y >= pos.y as f64
                        && cursor.y < (pos.y + size.height as i32) as f64
                });

                if let Some(monitor) = target {
                    let pos = monitor.position();
                    let size = monitor.size();
                    // Bar dimensions must match tauri.conf.json (460 × 60).
                    let bar_width = 460.0_f64;
                    let bar_height = 60.0_f64;
                    // Center horizontally; place 40 px above the bottom edge.
                    let x = pos.x as f64 + (size.width as f64 - bar_width) / 2.0;
                    let y = pos.y as f64 + size.height as f64 - bar_height - 40.0;
                    if let Err(e) = win.set_position(tauri::Position::Physical(
                        tauri::PhysicalPosition::new(x as i32, y as i32),
                    )) {
                        warn!("Failed to reposition floating-bar: {e}");
                    } else {
                        debug!(
                            "Floating-bar positioned at ({}, {}) on monitor \"{}\"",
                            x as i32,
                            y as i32,
                            monitor.name().map_or("unknown", |v| v)
                        );
                    }
                } else {
                    debug!("Cursor not found on any monitor; keeping existing floating-bar position");
                }
            }
            (Err(e), _) => warn!("Failed to enumerate monitors: {e}"),
            (_, Err(e)) => warn!("Failed to get cursor position: {e}"),
        }

        if let Err(e) = win.show() {
            warn!("Failed to show floating-bar window: {e}");
        } else {
            debug!("Floating-bar window shown");
        }
    } else {
        warn!("floating-bar window not found");
    }
}

/// Set the floating bar to "result" state with the injected text.
/// The frontend will show the result card and auto-hide after 3 seconds.
fn set_floating_result(app: &AppHandle, text: &str) {
    let state = app.state::<FloatingState>();
    let mut inner = state.0.lock().unwrap();
    inner.status = "result".to_string();
    inner.text = text.to_string();
    debug!("FloatingState set to result with {} chars", text.len());
}

/// Reset the floating bar to idle state and hide the window.
/// Used on error paths where we skip the result preview card.
fn hide_floating_bar(app: &AppHandle) {
    // Reset state so the frontend sees idle on the next poll.
    let state = app.state::<FloatingState>();
    {
        let mut inner = state.0.lock().unwrap();
        inner.status = "idle".to_string();
        inner.text = String::new();
    }
    if let Some(win) = app.get_webview_window("floating-bar") {
        if let Err(e) = win.hide() {
            warn!("Failed to hide floating-bar window: {e}");
        } else {
            debug!("Floating-bar window hidden");
        }
    } else {
        warn!("floating-bar window not found");
    }
}

/// Register the push-to-talk shortcut with the given hotkey string.
///
/// The handler captures an `AppHandle` so it can access managed state and the
/// config store at runtime without holding any lock across await points.
/// This function is also called by [`commands::set_hotkey`] for live re-registration.
pub fn register_push_to_talk_shortcut(app: &AppHandle, hotkey: &str) -> Result<(), String> {
    let handle = app.clone();
    app.global_shortcut()
        .on_shortcut(hotkey, move |_app, _shortcut, event| {
            // Read sound_enabled from store on every event so Settings
            // changes are picked up without a restart.
            let sound_on = handle
                .store("config.json")
                .ok()
                .and_then(|s| s.get("sound_enabled"))
                .and_then(|v| v.as_bool())
                .unwrap_or(true);

            let state = handle.state::<PipelineState>();
            match event.state {
                ShortcutState::Pressed => {
                    info!("Hotkey pressed — starting recording");
                    let mut pipeline = state.0.lock().unwrap();
                    if let Err(e) = pipeline.start_recording() {
                        error!("Failed to start recording: {e}");
                        sound::play_error(sound_on);
                    } else {
                        sound::play_start(sound_on);
                        // Show floating bar so user sees recording status immediately.
                        show_floating_bar(&handle);
                    }
                }
                ShortcutState::Released => {
                    info!("Hotkey released — stopping recording and processing");
                    let handle2 = handle.clone();
                    tauri::async_runtime::spawn(async move {
                        // Re-read inside the async task (store reads are cheap).
                        let sound_on = handle2
                            .store("config.json")
                            .ok()
                            .and_then(|s| s.get("sound_enabled"))
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true);

                        let pipeline_start = Instant::now();
                        let state = handle2.state::<PipelineState>();

                        // Extract data under lock, drop lock, then do async work.
                        let (wav_result, asr_client, llm_client) = {
                            let pipeline = state.0.lock().unwrap();
                            let audio = match pipeline.audio_thread_ref() {
                                Some(a) => a,
                                None => {
                                    error!(
                                        "Audio not initialized — did you configure API key?"
                                    );
                                    sound::play_error(sound_on);
                                    hide_floating_bar(&handle2);
                                    return;
                                }
                            };
                            audio.buffer().stop();
                            pipeline.set_status(pipeline::AppStatus::Processing);
                            // get_wav_data handles mono mixing + resampling to 16kHz
                            let wav = audio.buffer().get_wav_data();
                            (
                                wav,
                                pipeline.asr_client().cloned(),
                                pipeline.llm_client().cloned(),
                            )
                        };

                        let wav_data = match wav_result {
                            Ok(d) => {
                                let duration_secs = d.len() as f64 / (16000.0 * 2.0); // 16kHz, 16-bit mono
                                info!(
                                    "WAV encoded: {} bytes (~{:.1}s audio)",
                                    d.len(),
                                    duration_secs
                                );
                                d
                            }
                            Err(e) => {
                                warn!("WAV encoding failed: {e}");
                                sound::play_error(sound_on);
                                let pipeline = state.0.lock().unwrap();
                                pipeline.set_status(pipeline::AppStatus::Idle);
                                hide_floating_bar(&handle2);
                                return;
                            }
                        };

                        // ASR
                        let asr = match asr_client {
                            Some(a) => a,
                            None => {
                                warn!("ASR not configured — please set API key in settings");
                                sound::play_error(sound_on);
                                let pipeline = state.0.lock().unwrap();
                                pipeline.set_status(pipeline::AppStatus::Idle);
                                hide_floating_bar(&handle2);
                                return;
                            }
                        };

                        let asr_start = Instant::now();
                        let raw_text = match asr.recognize(&wav_data).await {
                            Ok(t) => {
                                info!(
                                    "ASR completed in {:.1}s: \"{}\"",
                                    asr_start.elapsed().as_secs_f64(),
                                    t
                                );
                                t
                            }
                            Err(e) => {
                                error!("ASR failed: {e}");
                                sound::play_error(sound_on);
                                let pipeline = state.0.lock().unwrap();
                                pipeline.set_status(pipeline::AppStatus::Idle);
                                hide_floating_bar(&handle2);
                                return;
                            }
                        };

                        // Keep raw text for history before LLM may consume it.
                        let raw_text_for_history = raw_text.clone();

                        // Load personal dictionary (best-effort; empty on failure).
                        let dictionary: Vec<String> = handle2
                            .store("config.json")
                            .ok()
                            .and_then(|s| {
                                s.get("dictionary")
                                    .and_then(|v| serde_json::from_value(v).ok())
                            })
                            .unwrap_or_default();

                        // Detect the focused app, derive tone, and check blacklist.
                        let app_name = context::get_frontmost_app_name();
                        let tone = context::get_tone_for_app(&app_name);
                        debug!("Context-aware tone for \"{}\": {}", app_name, tone);

                        // Load custom blacklist from store (best-effort).
                        let custom_blacklist: Vec<String> = handle2
                            .store("config.json")
                            .ok()
                            .and_then(|s| {
                                s.get("app_blacklist")
                                    .and_then(|v| serde_json::from_value(v).ok())
                            })
                            .unwrap_or_default();
                        let use_clipboard =
                            context::is_blacklisted(&app_name, &custom_blacklist);
                        if use_clipboard {
                            info!(
                                "App '{}' is blacklisted — will use clipboard paste fallback",
                                app_name
                            );
                        }

                        // LLM polish
                        let final_text = if let Some(llm) = &llm_client {
                            let llm_start = Instant::now();
                            match llm.polish(&raw_text, &dictionary, tone).await {
                                Ok(p) => {
                                    if p != raw_text {
                                        info!(
                                            "LLM polished in {:.1}s: \"{}\" -> \"{}\"",
                                            llm_start.elapsed().as_secs_f64(),
                                            raw_text,
                                            p
                                        );
                                    } else {
                                        info!(
                                            "LLM returned unchanged text in {:.1}s",
                                            llm_start.elapsed().as_secs_f64()
                                        );
                                    }
                                    p
                                }
                                Err(e) => {
                                    warn!("LLM polish failed (using raw text): {e}");
                                    raw_text
                                }
                            }
                        } else {
                            debug!("LLM disabled, skipping polish");
                            raw_text
                        };

                        // Inject text — use clipboard paste for blacklisted apps.
                        let injection_ok;
                        match injector::TextInjector::new() {
                            Ok(mut inj) => {
                                if let Err(e) = inj.inject_smart(&final_text, use_clipboard) {
                                    error!("Text injection failed: {e}");
                                    sound::play_error(sound_on);
                                    injection_ok = false;
                                } else {
                                    info!(
                                        "Text injected successfully ({} chars)",
                                        final_text.len()
                                    );
                                    // Play stop sound on successful injection.
                                    sound::play_stop(sound_on);
                                    // Cache raw and polished text for re-inject / AI-revert shortcuts.
                                    let pipeline = state.0.lock().unwrap();
                                    pipeline.set_last_transcription(
                                        raw_text_for_history.clone(),
                                        final_text.clone(),
                                    );
                                    injection_ok = true;
                                }
                            }
                            Err(e) => {
                                error!("Failed to init injector: {e}");
                                sound::play_error(sound_on);
                                injection_ok = false;
                            }
                        }

                        // Save to history after successful injection.
                        if injection_ok {
                            let duration_secs = wav_data.len() as f64 / (16000.0 * 2.0);
                            let history_state = handle2.state::<HistoryState>();
                            let record = HistoryRecord {
                                id: 0,
                                timestamp: Utc::now().to_rfc3339(),
                                raw_text: raw_text_for_history,
                                polished_text: final_text.clone(),
                                duration_secs,
                                app_name: context::get_frontmost_app_name(),
                            };
                            if let Err(e) = history_state.0.insert(&record) {
                                error!("Failed to save history record: {e}");
                            } else {
                                info!("History record saved");
                            }
                        }

                        info!(
                            "Pipeline completed in {:.1}s",
                            pipeline_start.elapsed().as_secs_f64()
                        );

                        let pipeline = state.0.lock().unwrap();
                        pipeline.set_status(pipeline::AppStatus::Idle);
                        // On success show the result preview card; on failure hide immediately.
                        if injection_ok {
                            set_floating_result(&handle2, &final_text);
                        } else {
                            hide_floating_bar(&handle2);
                        }
                    });
                }
            }
        })
        .map_err(|e| format!("Failed to register hotkey '{hotkey}': {e}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize logger; level controlled via RUST_LOG env var (default: info).
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    info!("Voice Typing starting up");

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(PipelineState(Mutex::new(Pipeline::new())))
        // CurrentHotkey is initialised with the compile-time default; the real
        // persisted value is stored here after it's read from the store in setup.
        .manage(CurrentHotkey(Mutex::new(commands::DEFAULT_HOTKEY.to_string())))
        .manage(FloatingState(Mutex::new(FloatingStateData {
            status: "idle".to_string(),
            text: String::new(),
        })))
        .manage(EditContext(Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::set_api_config,
            commands::start_recording,
            commands::stop_and_process,
            commands::get_history,
            commands::clear_history,
            commands::get_history_count,
            commands::get_autostart_enabled,
            commands::set_autostart_enabled,
            commands::get_sound_enabled,
            commands::set_sound_enabled,
            commands::is_macos,
            commands::get_dock_visible,
            commands::set_dock_visible,
            commands::is_onboarding_completed,
            commands::complete_onboarding,
            commands::get_dictionary,
            commands::set_dictionary,
            commands::list_audio_devices,
            commands::set_audio_device,
            commands::get_audio_device,
            commands::get_hotkey,
            commands::set_hotkey,
            commands::set_translation_target,
            commands::get_translation_target,
            commands::test_api_key,
            commands::delete_history_item,
            commands::reinject_history_item,
            commands::check_permissions,
            commands::open_system_preferences,
            commands::get_retention_days,
            commands::set_retention_days,
            commands::get_floating_state,
            commands::set_floating_state,
            commands::get_blacklist,
            commands::set_blacklist,
            commands::check_for_update,
            commands::install_update,
            commands::get_selected_text,
        ])
        .setup(|app| {
            // Initialize history database
            let app_data_dir = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("Failed to get app data dir: {e}"))?;
            std::fs::create_dir_all(&app_data_dir)
                .map_err(|e| format!("Failed to create app data dir: {e}"))?;
            let history_db = HistoryDb::init(&app_data_dir)
                .map_err(|e| format!("Failed to init history DB: {e}"))?;

            // Run retention cleanup before making the DB available to the rest of the app.
            let retention_days: i32 = app
                .store("config.json")
                .ok()
                .and_then(|s| s.get("retention_days"))
                .and_then(|v| v.as_i64())
                .map(|v| v as i32)
                .unwrap_or(-1);
            if retention_days > 0 {
                match history_db.cleanup_older_than(retention_days as u32) {
                    Ok(count) => {
                        if count > 0 {
                            info!("Auto-cleanup removed {count} history record(s) older than {retention_days} days");
                        } else {
                            debug!("Auto-cleanup: no history records older than {retention_days} days");
                        }
                    }
                    Err(e) => warn!("Auto-cleanup failed: {e}"),
                }
            } else {
                debug!("History retention set to forever — skipping auto-cleanup");
            }

            app.manage(HistoryState(history_db));
            info!("History database ready");

            // System tray
            tray::setup_tray(app.handle())?;
            info!("System tray initialized");

            // Load persisted API config from store
            match app.store("config.json") {
                Ok(store) => {
                    let api_key = store
                        .get("api_key")
                        .and_then(|v| v.as_str().map(|s| s.to_string()));
                    let llm_enabled = store
                        .get("llm_enabled")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let sound_enabled = store
                        .get("sound_enabled")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);
                    debug!("sound_enabled restored from store: {sound_enabled}");

                    // Restore dock icon visibility on macOS (default: visible).
                    #[cfg(target_os = "macos")]
                    {
                        use tauri::ActivationPolicy;
                        let dock_visible = store
                            .get("dock_visible")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true);
                        let policy = if dock_visible {
                            ActivationPolicy::Regular
                        } else {
                            ActivationPolicy::Accessory
                        };
                        app.set_activation_policy(policy);
                        debug!("dock_visible restored from store: {dock_visible}");
                    }

                    // Restore selected audio device (None = system default).
                    let selected_device = store
                        .get("selected_device")
                        .and_then(|v| if v.is_null() { None } else { v.as_str().map(|s| s.to_string()) });
                    if let Some(ref dev) = selected_device {
                        debug!("selected_device restored from store: {dev}");
                    }

                    // Restore translation target language (default: "English").
                    let translation_target = store
                        .get("translation_target")
                        .and_then(|v| v.as_str().map(|s| s.to_string()))
                        .unwrap_or_else(|| "English".to_string());
                    debug!("translation_target restored from store: {translation_target}");
                    {
                        let state = app.state::<PipelineState>();
                        let pipeline = state.0.lock().unwrap();
                        pipeline.set_translation_target(translation_target);
                    }

                    if let Some(key) = api_key {
                        if !key.is_empty() {
                            info!(
                                "Restoring saved API config: api_key={}***, llm_enabled={}",
                                &key[..key.len().min(4)],
                                llm_enabled
                            );
                            let state = app.state::<PipelineState>();
                            let mut pipeline = state.0.lock().unwrap();
                            pipeline.configure(key, llm_enabled);
                            pipeline.set_selected_device(selected_device);
                        } else {
                            debug!("Stored API key is empty, skipping restore");
                            // Still restore device even without API key.
                            let state = app.state::<PipelineState>();
                            let mut pipeline = state.0.lock().unwrap();
                            pipeline.set_selected_device(selected_device);
                        }
                    } else {
                        info!("No saved API config found (first launch)");
                        // Still restore device even without API key.
                        let state = app.state::<PipelineState>();
                        let mut pipeline = state.0.lock().unwrap();
                        pipeline.set_selected_device(selected_device);
                    }
                }
                Err(e) => {
                    warn!("Failed to open config store (first launch?): {e}");
                }
            }

            // Read the push-to-talk hotkey from the store (falls back to the
            // built-in default when no custom value has been persisted yet).
            let push_to_talk_hotkey = app
                .store("config.json")
                .ok()
                .and_then(|s| s.get("push_to_talk_hotkey"))
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_else(|| commands::DEFAULT_HOTKEY.to_string());
            info!("Registering push-to-talk hotkey: {push_to_talk_hotkey}");

            // Register PTT shortcut via the shared helper so it can also be called
            // from set_hotkey for live re-registration without a restart.
            register_push_to_talk_shortcut(app.handle(), &push_to_talk_hotkey)?;

            // Update the CurrentHotkey managed state to reflect the actual hotkey
            // that was just registered (may differ from the compile-time default).
            {
                let current_hotkey = app.state::<CurrentHotkey>();
                let mut current = current_hotkey.0.lock().unwrap();
                *current = push_to_talk_hotkey.clone();
            }

            info!("Global shortcut registered: {push_to_talk_hotkey}");

            // Secondary hotkey: re-inject the last transcription.
            // Ctrl+Cmd+V on Mac / Ctrl+Alt+V on Win/Linux.
            let handle2 = app.handle().clone();
            app.global_shortcut().on_shortcut(
                "CmdOrCtrl+Alt+V",
                move |_app, _shortcut, event| {
                    if event.state != ShortcutState::Pressed {
                        return;
                    }
                    info!("Paste-last-transcription hotkey pressed");
                    let state = handle2.state::<PipelineState>();
                    let last = {
                        let pipeline = state.0.lock().unwrap();
                        pipeline.last_transcription()
                    };
                    match last {
                        Some(text) => {
                            match injector::TextInjector::new() {
                                Ok(mut inj) => {
                                    if let Err(e) = inj.inject(&text) {
                                        error!("Failed to re-inject last transcription: {e}");
                                    } else {
                                        info!(
                                            "Re-injected last transcription ({} chars)",
                                            text.len()
                                        );
                                    }
                                }
                                Err(e) => {
                                    error!("Failed to init injector for re-inject: {e}");
                                }
                            }
                        }
                        None => {
                            warn!("No previous transcription cached — nothing to re-inject");
                        }
                    }
                },
            )?;

            info!("Global shortcut registered: CmdOrCtrl+Alt+V (paste last transcription)");

            // Tertiary hotkey: AI revert — inject raw ASR text (undoing LLM polish).
            // CmdOrCtrl+Alt+Z on all platforms.
            let handle3 = app.handle().clone();
            app.global_shortcut().on_shortcut(
                "CmdOrCtrl+Alt+Z",
                move |_app, _shortcut, event| {
                    if event.state != ShortcutState::Pressed {
                        return;
                    }
                    info!("AI-revert hotkey pressed — injecting raw ASR text");
                    let state = handle3.state::<PipelineState>();
                    let (raw, polished) = {
                        let pipeline = state.0.lock().unwrap();
                        (
                            pipeline.last_raw_transcription(),
                            pipeline.last_transcription(),
                        )
                    };
                    match raw {
                        Some(raw_text) => {
                            let polished_text = polished.unwrap_or_default();
                            info!(
                                "AI revert: polished=\"{}\" -> raw=\"{}\"",
                                polished_text, raw_text
                            );
                            match injector::TextInjector::new() {
                                Ok(mut inj) => {
                                    if let Err(e) = inj.inject(&raw_text) {
                                        error!("Failed to inject raw ASR text (AI revert): {e}");
                                    } else {
                                        info!(
                                            "AI revert injected raw text ({} chars)",
                                            raw_text.len()
                                        );
                                    }
                                }
                                Err(e) => {
                                    error!("Failed to init injector for AI revert: {e}");
                                }
                            }
                        }
                        None => {
                            warn!("No previous transcription cached — nothing to revert");
                        }
                    }
                },
            )?;

            info!("Global shortcut registered: CmdOrCtrl+Alt+Z (AI revert)");

            // Translation hotkey: CmdOrCtrl+Shift+T
            // Hold to record, release to transcribe then translate to target language.
            let handle_trans = app.handle().clone();
            app.global_shortcut().on_shortcut(
                "CmdOrCtrl+Shift+T",
                move |_app, _shortcut, event| {
                    let sound_on = handle_trans
                        .store("config.json")
                        .ok()
                        .and_then(|s| s.get("sound_enabled"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);

                    let state = handle_trans.state::<PipelineState>();
                    match event.state {
                        ShortcutState::Pressed => {
                            info!("Translation hotkey pressed — starting recording");
                            let mut pipeline = state.0.lock().unwrap();
                            if let Err(e) = pipeline.start_recording() {
                                error!("Failed to start recording (translation): {e}");
                                sound::play_error(sound_on);
                            } else {
                                sound::play_start(sound_on);
                                // Show floating bar so user sees recording status immediately.
                                show_floating_bar(&handle_trans);
                            }
                        }
                        ShortcutState::Released => {
                            info!("Translation hotkey released — stopping recording and translating");
                            let handle2 = handle_trans.clone();
                            tauri::async_runtime::spawn(async move {
                                let sound_on = handle2
                                    .store("config.json")
                                    .ok()
                                    .and_then(|s| s.get("sound_enabled"))
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(true);

                                let pipeline_start = Instant::now();
                                let state = handle2.state::<PipelineState>();

                                // Extract data under lock then drop it.
                                let (wav_result, asr_client, llm_client, translation_target) = {
                                    let pipeline = state.0.lock().unwrap();
                                    let audio = match pipeline.audio_thread_ref() {
                                        Some(a) => a,
                                        None => {
                                            error!("Audio not initialized — did you configure API key?");
                                            sound::play_error(sound_on);
                                            return;
                                        }
                                    };
                                    audio.buffer().stop();
                                    pipeline.set_status(pipeline::AppStatus::Processing);
                                    let wav = audio.buffer().get_wav_data();
                                    (
                                        wav,
                                        pipeline.asr_client().cloned(),
                                        pipeline.llm_client().cloned(),
                                        pipeline.translation_target(),
                                    )
                                };

                                let wav_data = match wav_result {
                                    Ok(d) => {
                                        let duration_secs = d.len() as f64 / (16000.0 * 2.0);
                                        info!(
                                            "Translation WAV encoded: {} bytes (~{:.1}s audio)",
                                            d.len(),
                                            duration_secs
                                        );
                                        d
                                    }
                                    Err(e) => {
                                        warn!("WAV encoding failed (translation): {e}");
                                        sound::play_error(sound_on);
                                        let pipeline = state.0.lock().unwrap();
                                        pipeline.set_status(pipeline::AppStatus::Idle);
                                        hide_floating_bar(&handle2);
                                        return;
                                    }
                                };

                                // ASR
                                let asr = match asr_client {
                                    Some(a) => a,
                                    None => {
                                        warn!("ASR not configured — please set API key in settings");
                                        sound::play_error(sound_on);
                                        let pipeline = state.0.lock().unwrap();
                                        pipeline.set_status(pipeline::AppStatus::Idle);
                                        hide_floating_bar(&handle2);
                                        return;
                                    }
                                };

                                let asr_start = Instant::now();
                                let raw_text = match asr.recognize(&wav_data).await {
                                    Ok(t) => {
                                        info!(
                                            "ASR completed in {:.1}s: \"{}\"",
                                            asr_start.elapsed().as_secs_f64(),
                                            t
                                        );
                                        t
                                    }
                                    Err(e) => {
                                        error!("ASR failed (translation): {e}");
                                        sound::play_error(sound_on);
                                        let pipeline = state.0.lock().unwrap();
                                        pipeline.set_status(pipeline::AppStatus::Idle);
                                        hide_floating_bar(&handle2);
                                        return;
                                    }
                                };

                                // Translation via LLM
                                let translated_text = if let Some(llm) = &llm_client {
                                    let llm_start = Instant::now();
                                    match llm.translate(&raw_text, &translation_target).await {
                                        Ok(t) => {
                                            info!(
                                                "LLM translated in {:.1}s: \"{}\" -> \"{}\"",
                                                llm_start.elapsed().as_secs_f64(),
                                                raw_text,
                                                t
                                            );
                                            t
                                        }
                                        Err(e) => {
                                            warn!("LLM translation failed (using raw text): {e}");
                                            raw_text.clone()
                                        }
                                    }
                                } else {
                                    warn!("LLM not configured — translation requires LLM; using raw text");
                                    raw_text.clone()
                                };

                                // Check blacklist for the focused app (best-effort).
                                let trans_app_name = context::get_frontmost_app_name();
                                let trans_custom_blacklist: Vec<String> = handle2
                                    .store("config.json")
                                    .ok()
                                    .and_then(|s| {
                                        s.get("app_blacklist")
                                            .and_then(|v| serde_json::from_value(v).ok())
                                    })
                                    .unwrap_or_default();
                                let trans_use_clipboard = context::is_blacklisted(
                                    &trans_app_name,
                                    &trans_custom_blacklist,
                                );
                                if trans_use_clipboard {
                                    info!(
                                        "App '{}' is blacklisted — using clipboard paste for translation",
                                        trans_app_name
                                    );
                                }

                                // Inject translated text — clipboard fallback for blacklisted apps.
                                let injection_ok;
                                match injector::TextInjector::new() {
                                    Ok(mut inj) => {
                                        if let Err(e) =
                                            inj.inject_smart(&translated_text, trans_use_clipboard)
                                        {
                                            error!("Text injection failed (translation): {e}");
                                            sound::play_error(sound_on);
                                            injection_ok = false;
                                        } else {
                                            info!(
                                                "Translated text injected ({} chars)",
                                                translated_text.len()
                                            );
                                            sound::play_stop(sound_on);
                                            let pipeline = state.0.lock().unwrap();
                                            pipeline.set_last_transcription(
                                                raw_text.clone(),
                                                translated_text.clone(),
                                            );
                                            injection_ok = true;
                                        }
                                    }
                                    Err(e) => {
                                        error!("Failed to init injector (translation): {e}");
                                        sound::play_error(sound_on);
                                        injection_ok = false;
                                    }
                                }

                                // Save to history after successful injection
                                if injection_ok {
                                    let duration_secs = wav_data.len() as f64 / (16000.0 * 2.0);
                                    let history_state = handle2.state::<HistoryState>();
                                    let record = HistoryRecord {
                                        id: 0,
                                        timestamp: Utc::now().to_rfc3339(),
                                        raw_text,
                                        polished_text: translated_text.clone(),
                                        duration_secs,
                                        app_name: context::get_frontmost_app_name(),
                                    };
                                    if let Err(e) = history_state.0.insert(&record) {
                                        error!("Failed to save translation history record: {e}");
                                    } else {
                                        info!("Translation history record saved");
                                    }
                                }

                                info!(
                                    "Translation pipeline completed in {:.1}s",
                                    pipeline_start.elapsed().as_secs_f64()
                                );

                                let pipeline = state.0.lock().unwrap();
                                pipeline.set_status(pipeline::AppStatus::Idle);
                                // On success show the result preview card; on failure hide immediately.
                                if injection_ok {
                                    set_floating_result(&handle2, &translated_text);
                                } else {
                                    hide_floating_bar(&handle2);
                                }
                            });
                        }
                    }
                },
            )?;

            info!("Global shortcut registered: CmdOrCtrl+Shift+T (translation mode)");

            // Speak-to-edit hotkey: CmdOrCtrl+Shift+E
            // Press: capture selected text + start recording.
            // Release: stop recording, ASR -> LLM edit -> inject (replaces selection).
            let handle_edit = app.handle().clone();
            app.global_shortcut().on_shortcut(
                "CmdOrCtrl+Shift+E",
                move |_app, _shortcut, event| {
                    let sound_on = handle_edit
                        .store("config.json")
                        .ok()
                        .and_then(|s| s.get("sound_enabled"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);

                    let state = handle_edit.state::<PipelineState>();
                    match event.state {
                        ShortcutState::Pressed => {
                            info!("Speak-to-edit hotkey pressed — capturing selected text and starting recording");

                            // Capture the currently selected text and store it before starting
                            // recording (Cmd+C simulation in get_selected_text may change focus).
                            let selected = context::get_selected_text();
                            match &selected {
                                Some(t) => info!("Speak-to-edit: captured {} chars of selected text", t.len()),
                                None => warn!("Speak-to-edit: no text selected — edit will be skipped"),
                            }
                            {
                                let edit_ctx = handle_edit.state::<EditContext>();
                                let mut ctx = edit_ctx.0.lock().unwrap();
                                *ctx = selected;
                            }

                            let mut pipeline = state.0.lock().unwrap();
                            if let Err(e) = pipeline.start_recording() {
                                error!("Failed to start recording (speak-to-edit): {e}");
                                sound::play_error(sound_on);
                            } else {
                                sound::play_start(sound_on);
                                // Show the floating bar so the user sees recording status.
                                show_floating_bar(&handle_edit);
                            }
                        }
                        ShortcutState::Released => {
                            info!("Speak-to-edit hotkey released — stopping recording and processing");
                            let handle2 = handle_edit.clone();
                            tauri::async_runtime::spawn(async move {
                                let sound_on = handle2
                                    .store("config.json")
                                    .ok()
                                    .and_then(|s| s.get("sound_enabled"))
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(true);

                                let pipeline_start = Instant::now();
                                let state = handle2.state::<PipelineState>();

                                // Extract selected text from EditContext before any await points.
                                let selected_text = {
                                    let edit_ctx = handle2.state::<EditContext>();
                                    let mut ctx = edit_ctx.0.lock().unwrap();
                                    ctx.take() // consume and clear the stored text
                                };

                                let selected_text = match selected_text {
                                    Some(t) if !t.is_empty() => t,
                                    _ => {
                                        warn!("Speak-to-edit: no selected text was captured — aborting");
                                        sound::play_error(sound_on);
                                        // Still need to stop the audio recorder gracefully.
                                        let pipeline = state.0.lock().unwrap();
                                        if let Some(audio) = pipeline.audio_thread_ref() {
                                            audio.buffer().stop();
                                        }
                                        pipeline.set_status(pipeline::AppStatus::Idle);
                                        hide_floating_bar(&handle2);
                                        return;
                                    }
                                };

                                // Extract data under lock then drop it.
                                let (wav_result, asr_client, llm_client) = {
                                    let pipeline = state.0.lock().unwrap();
                                    let audio = match pipeline.audio_thread_ref() {
                                        Some(a) => a,
                                        None => {
                                            error!("Audio not initialized — did you configure API key?");
                                            sound::play_error(sound_on);
                                            hide_floating_bar(&handle2);
                                            return;
                                        }
                                    };
                                    audio.buffer().stop();
                                    pipeline.set_status(pipeline::AppStatus::Processing);
                                    let wav = audio.buffer().get_wav_data();
                                    (
                                        wav,
                                        pipeline.asr_client().cloned(),
                                        pipeline.llm_client().cloned(),
                                    )
                                };

                                let wav_data = match wav_result {
                                    Ok(d) => {
                                        let duration_secs = d.len() as f64 / (16000.0 * 2.0);
                                        info!(
                                            "Speak-to-edit WAV encoded: {} bytes (~{:.1}s audio)",
                                            d.len(),
                                            duration_secs
                                        );
                                        d
                                    }
                                    Err(e) => {
                                        warn!("WAV encoding failed (speak-to-edit): {e}");
                                        sound::play_error(sound_on);
                                        let pipeline = state.0.lock().unwrap();
                                        pipeline.set_status(pipeline::AppStatus::Idle);
                                        hide_floating_bar(&handle2);
                                        return;
                                    }
                                };

                                // ASR: transcribe the voice command.
                                let asr = match asr_client {
                                    Some(a) => a,
                                    None => {
                                        warn!("ASR not configured — please set API key in settings");
                                        sound::play_error(sound_on);
                                        let pipeline = state.0.lock().unwrap();
                                        pipeline.set_status(pipeline::AppStatus::Idle);
                                        hide_floating_bar(&handle2);
                                        return;
                                    }
                                };

                                let asr_start = Instant::now();
                                let voice_command = match asr.recognize(&wav_data).await {
                                    Ok(t) => {
                                        info!(
                                            "Speak-to-edit ASR completed in {:.1}s: \"{}\"",
                                            asr_start.elapsed().as_secs_f64(),
                                            t
                                        );
                                        t
                                    }
                                    Err(e) => {
                                        error!("ASR failed (speak-to-edit): {e}");
                                        sound::play_error(sound_on);
                                        let pipeline = state.0.lock().unwrap();
                                        pipeline.set_status(pipeline::AppStatus::Idle);
                                        hide_floating_bar(&handle2);
                                        return;
                                    }
                                };

                                if voice_command.is_empty() {
                                    warn!("Speak-to-edit: ASR returned empty command — aborting");
                                    sound::play_error(sound_on);
                                    let pipeline = state.0.lock().unwrap();
                                    pipeline.set_status(pipeline::AppStatus::Idle);
                                    hide_floating_bar(&handle2);
                                    return;
                                }

                                // LLM edit: apply the voice command to the selected text.
                                let edited_text = match &llm_client {
                                    Some(llm) => {
                                        let llm_start = Instant::now();
                                        match llm.edit_text(&selected_text, &voice_command).await {
                                            Ok(t) => {
                                                info!(
                                                    "LLM edit_text completed in {:.1}s",
                                                    llm_start.elapsed().as_secs_f64()
                                                );
                                                t
                                            }
                                            Err(e) => {
                                                error!("LLM edit_text failed: {e}");
                                                sound::play_error(sound_on);
                                                let pipeline = state.0.lock().unwrap();
                                                pipeline.set_status(pipeline::AppStatus::Idle);
                                                hide_floating_bar(&handle2);
                                                return;
                                            }
                                        }
                                    }
                                    None => {
                                        warn!("LLM not configured — speak-to-edit requires LLM; aborting");
                                        sound::play_error(sound_on);
                                        let pipeline = state.0.lock().unwrap();
                                        pipeline.set_status(pipeline::AppStatus::Idle);
                                        hide_floating_bar(&handle2);
                                        return;
                                    }
                                };

                                // Detect the focused app to decide injection method.
                                let edit_app_name = context::get_frontmost_app_name();
                                let custom_blacklist: Vec<String> = handle2
                                    .store("config.json")
                                    .ok()
                                    .and_then(|s| {
                                        s.get("app_blacklist")
                                            .and_then(|v| serde_json::from_value(v).ok())
                                    })
                                    .unwrap_or_default();
                                let use_clipboard =
                                    context::is_blacklisted(&edit_app_name, &custom_blacklist);
                                if use_clipboard {
                                    info!(
                                        "Speak-to-edit: app '{}' is blacklisted — using clipboard paste",
                                        edit_app_name
                                    );
                                }

                                // Inject the edited text — it replaces the current selection naturally.
                                let injection_ok;
                                match injector::TextInjector::new() {
                                    Ok(mut inj) => {
                                        if let Err(e) =
                                            inj.inject_smart(&edited_text, use_clipboard)
                                        {
                                            error!("Text injection failed (speak-to-edit): {e}");
                                            sound::play_error(sound_on);
                                            injection_ok = false;
                                        } else {
                                            info!(
                                                "Speak-to-edit: injected {} chars (replacing selection)",
                                                edited_text.len()
                                            );
                                            sound::play_stop(sound_on);
                                            let pipeline = state.0.lock().unwrap();
                                            pipeline.set_last_transcription(
                                                voice_command.clone(),
                                                edited_text.clone(),
                                            );
                                            injection_ok = true;
                                        }
                                    }
                                    Err(e) => {
                                        error!("Failed to init injector (speak-to-edit): {e}");
                                        sound::play_error(sound_on);
                                        injection_ok = false;
                                    }
                                }

                                // Save to history after successful injection.
                                if injection_ok {
                                    let duration_secs = wav_data.len() as f64 / (16000.0 * 2.0);
                                    let history_state = handle2.state::<HistoryState>();
                                    let record = HistoryRecord {
                                        id: 0,
                                        timestamp: Utc::now().to_rfc3339(),
                                        raw_text: voice_command,
                                        polished_text: edited_text.clone(),
                                        duration_secs,
                                        app_name: edit_app_name,
                                    };
                                    if let Err(e) = history_state.0.insert(&record) {
                                        error!("Failed to save speak-to-edit history record: {e}");
                                    } else {
                                        info!("Speak-to-edit history record saved");
                                    }
                                }

                                info!(
                                    "Speak-to-edit pipeline completed in {:.1}s",
                                    pipeline_start.elapsed().as_secs_f64()
                                );

                                let pipeline = state.0.lock().unwrap();
                                pipeline.set_status(pipeline::AppStatus::Idle);
                                if injection_ok {
                                    set_floating_result(&handle2, &edited_text);
                                } else {
                                    hide_floating_bar(&handle2);
                                }
                            });
                        }
                    }
                },
            )?;

            info!("Global shortcut registered: CmdOrCtrl+Shift+E (speak-to-edit)");

            // Check for updates in the background on startup so the About tab
            // can reflect availability without blocking the main window.
            let update_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use tauri_plugin_updater::UpdaterExt;
                match update_handle.updater() {
                    Ok(updater) => match updater.check().await {
                        Ok(Some(update)) => {
                            info!(
                                "Update available on startup: v{} — {}",
                                update.version,
                                update.body.as_deref().unwrap_or("no release notes")
                            );
                        }
                        Ok(None) => {
                            debug!("Startup update check: already on latest version");
                        }
                        Err(e) => {
                            // Non-fatal: updater may not be configured until release signing is set up.
                            debug!("Startup update check failed (expected before release setup): {e}");
                        }
                    },
                    Err(e) => {
                        debug!("Updater not available (expected before release setup): {e}");
                    }
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
