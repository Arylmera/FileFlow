mod commands;
mod state;
mod volume;
mod watchers;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, WindowEvent,
};

pub(crate) fn show_main(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize(); // a minimized window won't come back with show() alone
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Apply the Dock + menu-bar visibility from config. Called at startup and on
/// every save so the toggles take effect live. Callers normalize first
/// (`AppSettings::ensure_reachable`) so at least one surface stays visible.
pub(crate) fn apply_window_mode(app: &tauri::AppHandle, cfg: &fileflow_core::config::AppSettings) {
    #[cfg(target_os = "macos")]
    {
        let _ = app.set_activation_policy(if cfg.show_dock_icon {
            tauri::ActivationPolicy::Regular
        } else {
            tauri::ActivationPolicy::Accessory
        });
    }
    if let Some(tray) = app.tray_by_id("main-tray") {
        let _ = tray.set_visible(cfg.show_tray_icon);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // single-instance MUST be registered first; a second launch just refocuses us.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Boot as a menu-bar agent (no Dock icon) to avoid a launch flash;
            // apply_window_mode below sets the configured state once config loads.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let show = MenuItem::with_id(app, "show", "Open FileFlow", true, None::<&str>)?;
            let import_lr =
                MenuItem::with_id(app, "import_lr", "Import to Photos now", true, None::<&str>)?;
            let pause =
                MenuItem::with_id(app, "toggle_pause", "Pause / Resume watchers", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let menu = Menu::with_items(app, &[&show, &import_lr, &pause, &sep, &quit])?;

            TrayIconBuilder::with_id("main-tray")
                .icon(tauri::include_image!("icons/tray.png"))
                .icon_as_template(true)
                .tooltip("FileFlow")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main(app),
                    "import_lr" => {
                        let a = app.clone();
                        std::thread::spawn(move || {
                            let folders = a.state::<state::AppState>().snapshot().folders;
                            for (i, r) in folders.iter().enumerate() {
                                if r.is_photos() {
                                    watchers::run_now(&a, i);
                                }
                            }
                        });
                    }
                    "toggle_pause" => {
                        let st = app.state::<state::AppState>();
                        let now = !st.is_paused();
                        st.set_paused(now);
                        let _ = app.emit("paused-changed", now);
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main(tray.app_handle());
                    }
                })
                .build(app)?;

            // App state: load persisted config (missing file → defaults).
            let config_path = app.path().app_config_dir()?.join("config.toml");
            let mut config = fileflow_core::config::Config::load(&config_path).unwrap_or_default();
            config.app.ensure_reachable();

            // Apply the Dock/menu-bar visibility now that the tray exists.
            apply_window_mode(app.handle(), &config.app);

            // File logging to the app log dir (single non-rotating file).
            if let Ok(log_dir) = app.path().app_log_dir() {
                let _ = std::fs::create_dir_all(&log_dir);
                let (writer, guard) = tracing_appender::non_blocking(
                    tracing_appender::rolling::never(&log_dir, "fileflow.log"),
                );
                let level = config
                    .app
                    .log_level
                    .parse::<tracing::Level>()
                    .unwrap_or(tracing::Level::INFO);
                let _ = tracing_subscriber::fmt()
                    .with_writer(writer)
                    .with_ansi(false)
                    .with_max_level(level)
                    .try_init();
                app.manage(std::sync::Mutex::new(guard)); // keep the flush guard alive
                tracing::info!("FileFlow started");
            }

            app.manage(state::AppState::new(config, config_path));

            // File-system watchers: card ingest (/Volumes) + per-rule folder watchers.
            watchers::start(app)?;
            Ok(())
        })
        // On window close: hide to the menu bar (default) or quit, per the setting.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let keep = window
                    .app_handle()
                    .state::<state::AppState>()
                    .snapshot()
                    .app
                    .keep_running_on_close;
                if keep {
                    let _ = window.hide();
                    api.prevent_close();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::save_config,
            commands::list_mounted_cards,
            commands::prepare_ingest,
            commands::run_ingest_now,
            commands::run_photos_import_now,
            commands::run_folder_now,
            commands::get_activity,
            commands::dest_writable,
            commands::get_paused,
            commands::set_paused,
            commands::reveal_in_finder,
            commands::log_path,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
