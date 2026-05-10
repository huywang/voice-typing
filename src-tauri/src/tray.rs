use cpal::traits::{DeviceTrait, HostTrait};
use log::{debug, error, info, warn};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Wry,
};
use tauri_plugin_store::StoreExt;

use crate::commands::PipelineState;

/// App version string shown in the tray menu title item.
const APP_VERSION: &str = concat!("Voice Typing v", env!("CARGO_PKG_VERSION"));

/// Enumerate available audio input device names via cpal.
///
/// Returns an empty Vec if enumeration fails; errors are logged but not
/// propagated so that tray setup never fails due to audio issues.
fn enumerate_input_device_names() -> Vec<String> {
    let host = cpal::default_host();
    match host.input_devices() {
        Ok(devices) => {
            let names: Vec<String> = devices.filter_map(|d| d.name().ok()).collect();
            debug!("Enumerated {} audio input device(s) for tray", names.len());
            names
        }
        Err(e) => {
            warn!("Failed to enumerate audio input devices for tray: {e}");
            Vec::new()
        }
    }
}

/// Return the currently selected device name from the persistent store.
///
/// Returns `None` when the user has not explicitly chosen a device (system
/// default is in use).
fn get_selected_device(app: &AppHandle) -> Option<String> {
    app.store("config.json")
        .ok()
        .and_then(|s| s.get("selected_device"))
        .and_then(|v| {
            if v.is_null() {
                None
            } else {
                v.as_str().map(|s| s.to_string())
            }
        })
}

/// Build and register the system tray icon and context menu.
///
/// Menu layout:
/// ```text
/// Voice Typing v0.1.0   (disabled, info only)
/// ──────────────────────
/// Settings
/// Select Microphone ▶
///   ✓ Built-in Microphone
///     MacBook Pro Microphone
///     …
/// ──────────────────────
/// Quit
/// ```
pub fn setup_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    // --- Version info item (non-clickable) ---
    let version_item = MenuItem::with_id(app, "version", APP_VERSION, false, None::<&str>)?;

    // --- Settings item ---
    let settings_item = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;

    // --- Microphone submenu ---
    let mic_submenu = build_mic_submenu(app)?;

    // --- Separators ---
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;

    // --- Quit item ---
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

    // Assemble the top-level menu.
    let menu = Menu::with_items(
        app,
        &[
            &version_item,
            &sep1,
            &settings_item,
            &mic_submenu,
            &sep2,
            &quit_item,
        ],
    )?;

    let icon = Image::from_bytes(include_bytes!("../icons/32x32.png"))?;

    let _tray = TrayIconBuilder::new()
        .icon(icon)
        .menu(&menu)
        .tooltip("Voice Typing")
        .on_menu_event(|app: &AppHandle<Wry>, event| {
            let id = event.id.as_ref();

            if id == "settings" {
                info!("Tray: Settings clicked — showing main window");
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
                return;
            }

            if id == "quit" {
                info!("Tray: Quit clicked");
                app.exit(0);
                return;
            }

            // Microphone selection items are prefixed with "mic:".
            if let Some(device_name) = id.strip_prefix("mic:") {
                // "mic:__default__" is the sentinel for "system default".
                let selected: Option<String> = if device_name == "__default__" {
                    None
                } else {
                    Some(device_name.to_string())
                };

                info!(
                    "Tray: microphone selected — {}",
                    selected.as_deref().unwrap_or("system default")
                );

                // Update the live pipeline.
                if let Ok(state) = app.try_state::<PipelineState>().ok_or("no state") {
                    let mut pipeline = state.0.lock().unwrap();
                    pipeline.set_selected_device(selected.clone());
                } else {
                    warn!("Tray: PipelineState not available yet");
                }

                // Persist to store.
                match app.store("config.json") {
                    Ok(store) => {
                        store.set("selected_device", serde_json::json!(selected));
                        if let Err(e) = store.save() {
                            error!("Tray: failed to persist selected_device: {e}");
                        } else {
                            debug!("Tray: selected_device persisted");
                        }
                    }
                    Err(e) => {
                        error!("Tray: failed to open config store: {e}");
                    }
                }
            }
        })
        .on_tray_icon_event(|tray: &tauri::tray::TrayIcon<Wry>, event| {
            // Single-click on the tray icon toggles the main window.
            if let TrayIconEvent::Click { .. } = event {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    if window.is_visible().unwrap_or(false) {
                        let _ = window.hide();
                    } else {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
            }
        })
        .build(app)?;

    info!("System tray initialized with enhanced menu");
    Ok(())
}

/// Build the "Select Microphone" submenu.
///
/// Each item gets an id of the form `"mic:<device_name>"`.  The currently
/// selected device (or system default) is prefixed with a check-mark `✓`.
fn build_mic_submenu(app: &AppHandle) -> Result<Submenu<Wry>, Box<dyn std::error::Error>> {
    let selected = get_selected_device(app);
    let device_names = enumerate_input_device_names();

    // Build a vector of (id, label) pairs.
    // The first entry is always the system-default option.
    let mut items: Vec<(String, String)> = Vec::new();

    let default_label = if selected.is_none() {
        "✓ System Default".to_string()
    } else {
        "  System Default".to_string()
    };
    items.push(("mic:__default__".to_string(), default_label));

    for name in &device_names {
        let check = if selected.as_deref() == Some(name.as_str()) {
            "✓ "
        } else {
            "  "
        };
        items.push((format!("mic:{}", name), format!("{check}{name}")));
    }

    // Build the Submenu by creating each MenuItem and collecting references.
    // We create them in a Vec first so lifetimes are satisfied.
    let menu_items: Vec<MenuItem<Wry>> = items
        .iter()
        .map(|(id, label)| MenuItem::with_id(app, id.as_str(), label.as_str(), true, None::<&str>))
        .collect::<Result<Vec<_>, _>>()?;

    let item_refs: Vec<&dyn tauri::menu::IsMenuItem<Wry>> =
        menu_items.iter().map(|i| i as &dyn tauri::menu::IsMenuItem<Wry>).collect();

    let submenu = Submenu::with_items(app, "Select Microphone", true, &item_refs)?;

    debug!(
        "Mic submenu built with {} item(s) (selected: {})",
        menu_items.len(),
        selected.as_deref().unwrap_or("system default")
    );

    Ok(submenu)
}
