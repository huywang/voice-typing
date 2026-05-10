mod asr;
mod audio;
mod commands;
mod history;
mod injector;
mod llm;
mod pipeline;
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
                        } else {
                            debug!("Stored API key is empty, skipping restore");
                        }
                    } else {
                        info!("No saved API config found (first launch)");
                    }
                }
                Err(e) => {
                    warn!("Failed to open config store (first launch?): {e}");
                }
            }

            // Global hotkey: Cmd+Shift+Space (Mac) / Ctrl+Shift+Space (Win/Linux)
            let handle = app.handle().clone();
            app.global_shortcut().on_shortcut(
                "CmdOrCtrl+Shift+Space",
                move |_app, _shortcut, event| {
                    let state = handle.state::<PipelineState>();
                    match event.state {
                        ShortcutState::Pressed => {
                            info!("Hotkey pressed — starting recording");
                            let mut pipeline = state.0.lock().unwrap();
                            if let Err(e) = pipeline.start_recording() {
                                error!("Failed to start recording: {e}");
                            }
                        }
                        ShortcutState::Released => {
                            info!("Hotkey released — stopping recording and processing");
                            let handle2 = handle.clone();
                            tauri::async_runtime::spawn(async move {
                                let pipeline_start = Instant::now();
                                let state = handle2.state::<PipelineState>();

                                // Extract data under lock, drop lock, then do async work
                                let (wav_result, asr_client, llm_client) = {
                                    let pipeline = state.0.lock().unwrap();
                                    let audio = match pipeline.audio_thread_ref() {
                                        Some(a) => a,
                                        None => {
                                            error!("Audio not initialized — did you configure API key?");
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
                                        let pipeline = state.0.lock().unwrap();
                                        pipeline.set_status(pipeline::AppStatus::Idle);
                                        return;
                                    }
                                };

                                // Keep raw text for history before LLM may consume it.
                                let raw_text_for_history = raw_text.clone();

                                // LLM polish
                                let final_text = if let Some(llm) = &llm_client {
                                    let llm_start = Instant::now();
                                    match llm.polish(&raw_text).await {
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
                                            injection_ok = false;
                                        } else {
                                            info!(
                                                "Text injected successfully ({} chars)",
                                                final_text.len()
                                            );
                                            injection_ok = true;
                                        }
                                    }
                                    Err(e) => {
                                        error!("Failed to init injector: {e}");
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
                                        app_name: String::new(),
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

            info!("Global shortcut registered: CmdOrCtrl+Shift+Space");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
