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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(PipelineState(Mutex::new(Pipeline::new())))
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::set_api_config,
            commands::start_recording,
            commands::stop_and_process,
        ])
        .setup(|app| {
            tray::setup_tray(app.handle())?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
