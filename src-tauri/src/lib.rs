mod asr;
mod audio;
mod commands;
mod injector;
mod llm;
mod pipeline;
mod tray;

use commands::PipelineState;
use pipeline::Pipeline;
use std::sync::Mutex;
use tauri::Manager;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(PipelineState(Mutex::new(Pipeline::new())))
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::set_api_config,
            commands::start_recording,
            commands::stop_and_process,
        ])
        .setup(|app| {
            // System tray
            tray::setup_tray(app.handle())?;

            // Global hotkey: Cmd+Shift+Space (Mac) / Ctrl+Shift+Space (Win/Linux)
            let handle = app.handle().clone();
            app.global_shortcut().on_shortcut(
                "CmdOrCtrl+Shift+Space",
                move |_app, _shortcut, event| {
                    let state = handle.state::<PipelineState>();
                    match event.state {
                        ShortcutState::Pressed => {
                            let mut pipeline = state.0.lock().unwrap();
                            if let Err(e) = pipeline.start_recording() {
                                eprintln!("Failed to start recording: {e}");
                            }
                        }
                        ShortcutState::Released => {
                            // Spawn async task for stop + process
                            let handle2 = handle.clone();
                            tauri::async_runtime::spawn(async move {
                                let state = handle2.state::<PipelineState>();
                                // Extract data under lock, drop lock, then do async work
                                let (samples, asr_client, llm_client) = {
                                    let pipeline = state.0.lock().unwrap();
                                    let audio = match pipeline.audio_thread_ref() {
                                        Some(a) => a,
                                        None => {
                                            eprintln!("Audio not initialized");
                                            return;
                                        }
                                    };
                                    audio.buffer().stop();
                                    pipeline.set_status(pipeline::AppStatus::Processing);
                                    let samples = audio.buffer().samples();
                                    (
                                        samples,
                                        pipeline.asr_client().cloned(),
                                        pipeline.llm_client().cloned(),
                                    )
                                };

                                if samples.is_empty() {
                                    eprintln!("No audio data recorded");
                                    let pipeline = state.0.lock().unwrap();
                                    pipeline.set_status(pipeline::AppStatus::Idle);
                                    return;
                                }

                                // Encode WAV
                                let wav_data = match audio::encode_wav(&samples, 16000) {
                                    Ok(d) => d,
                                    Err(e) => {
                                        eprintln!("WAV encoding failed: {e}");
                                        let pipeline = state.0.lock().unwrap();
                                        pipeline.set_status(pipeline::AppStatus::Idle);
                                        return;
                                    }
                                };

                                // ASR
                                let asr = match asr_client {
                                    Some(a) => a,
                                    None => {
                                        eprintln!("ASR not configured");
                                        let pipeline = state.0.lock().unwrap();
                                        pipeline.set_status(pipeline::AppStatus::Idle);
                                        return;
                                    }
                                };

                                let raw_text = match asr.recognize(&wav_data).await {
                                    Ok(t) => t,
                                    Err(e) => {
                                        eprintln!("ASR failed: {e}");
                                        let pipeline = state.0.lock().unwrap();
                                        pipeline.set_status(pipeline::AppStatus::Idle);
                                        return;
                                    }
                                };

                                // LLM polish
                                let final_text = if let Some(llm) = &llm_client {
                                    match llm.polish(&raw_text).await {
                                        Ok(p) => p,
                                        Err(e) => {
                                            eprintln!("LLM polish failed: {e}");
                                            raw_text
                                        }
                                    }
                                } else {
                                    raw_text
                                };

                                // Inject text
                                match injector::TextInjector::new() {
                                    Ok(mut inj) => {
                                        if let Err(e) = inj.inject(&final_text) {
                                            eprintln!("Text injection failed: {e}");
                                        }
                                    }
                                    Err(e) => eprintln!("Failed to init injector: {e}"),
                                }

                                let pipeline = state.0.lock().unwrap();
                                pipeline.set_status(pipeline::AppStatus::Idle);
                            });
                        }
                    }
                },
            )?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
