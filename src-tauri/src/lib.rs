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
use commands::{HistoryState, PipelineState};
use history::{HistoryDb, HistoryRecord};
use log::{debug, error, info, warn};
use pipeline::Pipeline;
use std::sync::Mutex;
use std::time::Instant;
use tauri::Manager;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_store::StoreExt;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize logger; level controlled via RUST_LOG env var (default: info).
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    info!("Voice Typing starting up");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(PipelineState(Mutex::new(Pipeline::new())))
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

            // Global hotkey: configurable push-to-talk (default CmdOrCtrl+Shift+Space)
            let handle = app.handle().clone();
            app.global_shortcut().on_shortcut(
                push_to_talk_hotkey.as_str(),
                move |_app, _shortcut, event| {
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

                                // Extract data under lock, drop lock, then do async work
                                let (wav_result, asr_client, llm_client) = {
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

                                // LLM polish
                                let final_text = if let Some(llm) = &llm_client {
                                    let llm_start = Instant::now();
                                    match llm.polish(&raw_text, &dictionary).await {
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
                                            warn!(
                                                "LLM polish failed (using raw text): {e}"
                                            );
                                            raw_text
                                        }
                                    }
                                } else {
                                    debug!("LLM disabled, skipping polish");
                                    raw_text
                                };

                                // Inject text
                                let injection_ok;
                                match injector::TextInjector::new() {
                                    Ok(mut inj) => {
                                        if let Err(e) = inj.inject(&final_text) {
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

                                // Save to history after successful injection
                                if injection_ok {
                                    let duration_secs =
                                        wav_data.len() as f64 / (16000.0 * 2.0);
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
                            });
                        }
                    }
                },
            )?;

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

                                // Inject translated text
                                let injection_ok;
                                match injector::TextInjector::new() {
                                    Ok(mut inj) => {
                                        if let Err(e) = inj.inject(&translated_text) {
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
                            });
                        }
                    }
                },
            )?;

            info!("Global shortcut registered: CmdOrCtrl+Shift+T (translation mode)");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
