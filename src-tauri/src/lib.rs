// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

mod commands;
mod core;
mod data;
mod platform;
mod streaming;

// The Rust playback engine runs on desktop and Android. Both use `rodio` +
// `cpal` (WASAPI / CoreAudio / ALSA on desktop, Oboe / AAudio on Android), so
// decoding and output never touch the webview or hand-written JNI.
#[cfg(any(desktop, target_os = "android"))]
mod playback;

use tauri::Manager;

/// Extra browser arguments for the WebView2 runtime.
///
/// Disables Chromium's own system media controls so only the `souvlaki` session
/// remains. The background flags keep timers, rendering and audio alive when the
/// main window is hidden to the tray, so playback and the desktop-lyrics overlay
/// (which is driven by ticks from the main window and animates on its own rAF)
/// do not freeze. WebView2 requires every window sharing a data directory to use
/// identical options, so the main window's `additionalBrowserArgs` in
/// `tauri.conf.json` MUST stay byte-for-byte identical to this value. Linux/Cef
/// receives the equivalent switches in [`cef_runtime`] instead.
#[cfg(all(desktop, not(target_os = "linux")))]
pub(crate) const WEBVIEW_BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,HardwareMediaKeyHandling --autoplay-policy=no-user-gesture-required --disable-background-timer-throttling --disable-backgrounding-occluded-windows --disable-renderer-backgrounding";

/// Chromium switches for the Linux CEF runtime.
///
/// CEF ignores wry's `additionalBrowserArgs`, so the WebView2 flags above that
/// matter on Linux are repeated here: keep timers, rendering and audio running
/// while the window is hidden to the tray, allow autoplay without a user
/// gesture, and leave hardware media keys to `souvlaki`. `--no-first-run`
/// skips Chromium's first-run EULA, without which a fresh profile exits with
/// result code 28 on Linux.
#[cfg(target_os = "linux")]
fn cef_runtime() -> tauri_runtime_cef::Cef {
    tauri_runtime_cef::Cef::default()
        .command_line_arg("no-first-run", None::<String>)
        .command_line_args([
            ("autoplay-policy", Some("no-user-gesture-required")),
            ("disable-background-timer-throttling", None::<&str>),
            ("disable-backgrounding-occluded-windows", None::<&str>),
            ("disable-renderer-backgrounding", None::<&str>),
        ])
        .disable_features(["HardwareMediaKeyHandling"])
        // The desktop-lyric overlay is an off-screen browser: CEF paints it into
        // a buffer we composite into a transparent GTK window instead of into a
        // native view. CEF only creates windowless browsers when this
        // process-wide setting is on, and advises against setting it when
        // nothing renders off-screen, so it is always on here because the
        // overlay may open at any time.
        .with_settings(|settings| settings.windowless_rendering_enabled = 1)
        // Native input and redraws for that overlay: an off-screen browser has
        // no view of its own, so nothing else delivers them.
        .on_window_event(crate::platform::linux::desktop_lyric::on_window_event)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    core::logging::init();

    // Android has no keyring backend; install the Keystore-backed one before
    // anything touches the keychain. A failure is recorded so the WebDAV
    // commands report an unavailable keychain instead of silently writing to
    // keyring's non-persistent mock store.
    #[cfg(target_os = "android")]
    commands::webdav::set_keychain_init_error(platform::android::initialize_keychain());

    // Tauri 3 selects the webview runtime at startup. Linux ships Chromium via
    // CEF (so it never links WebKitGTK); every other target keeps the system
    // webview through wry.
    let builder = tauri::Builder::default();
    #[cfg(target_os = "linux")]
    let builder = builder.runtime(cef_runtime());
    #[cfg(not(target_os = "linux"))]
    let builder = builder.runtime(tauri_runtime_wry::Wry::default());

    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .manage(commands::desktop_lyric::DesktopLyricCache::default())
        .manage(platform::media_control::MediaControlState::default())
        .manage(platform::tray::CloseToTray::default())
        .setup(|app| {
            // Apply signed updates from GitHub Releases. macOS/Windows use the
            // Tauri updater plugin (in-app auto-install); Android and Linux query
            // GitHub directly and open the release page, so they need neither the
            // plugin nor the pending-update state.
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            {
                if let Err(error) = app
                    .handle()
                    .plugin(tauri_plugin_updater::Builder::new().build())
                {
                    log::error!("failed to initialize updater plugin: {error}");
                }
                app.manage(commands::update::desktop::PendingUpdate::default());
            }

            // Mint the per-install identifier on first launch so it is stable
            // from the very first run; later launches just read it back.
            match commands::sponsor::get_install_id() {
                Ok(status) if status.available => {}
                Ok(status) => log::warn!(
                    "install identifier unavailable at startup: {}",
                    status
                        .warning
                        .as_deref()
                        .unwrap_or("unknown keychain error")
                ),
                Err(error) => log::warn!("failed to prepare install identifier: {error}"),
            }

            // Let the webview load cached cover thumbnails as plain asset URLs.
            commands::media::allow_cache_dir(app.handle());
            match streaming::state::start(app.handle().clone()) {
                Ok(stream_state) => {
                    // The playback engine decodes in Rust and reads the
                    // stream/smart caches directly; it shares their handles.
                    #[cfg(any(desktop, target_os = "android"))]
                    let engine = playback::engine::start(
                        app.handle().clone(),
                        stream_state.cache.clone(),
                        stream_state.smart.clone(),
                    );
                    #[cfg(any(desktop, target_os = "android"))]
                    if !app.manage(engine.clone()) {
                        log::error!(
                            "playback: Engine was already managed; the newly started worker was dropped"
                        );
                    }
                    app.manage(stream_state);

                    // The playback context (queue / shuffle / repeat) is owned
                    // by Rust so playback survives a dead or throttled webview.
                    // It wraps the engine and drives it directly.
                    #[cfg(any(desktop, target_os = "android"))]
                    {
                        let context =
                            playback::context::PlaybackContext::new(engine.clone());
                        if !app.manage(context) {
                            log::error!("playback: PlaybackContext was already managed");
                        }
                        // Drain the native Android MediaSession action queue from
                        // a Rust timer (the webview cannot be relied on to poll).
                        playback::context::start_media_control_poller(app.handle().clone());
                    }
                }
                Err(error) => log::error!("failed to initialize streaming caches: {error}"),
            }
            // Desktop window lifecycle: a close either quits (taking the
            // floating lyrics down with it) or, when "minimise to tray" is on,
            // just hides the main window so playback continues in the tray.
            #[cfg(desktop)]
            {
                // System tray: left click restores the window; right click
                // opens the 打开 / 退出 menu.
                if let Err(error) = platform::tray::init(app) {
                    log::warn!("failed to create tray icon: {error}");
                }

                if let Some(main) = app.get_webview_window("main") {
                    let handle = app.handle().clone();
                    let window = main.clone();
                    main.on_window_event(move |event| {
                        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                            // "Minimise to tray": hide the main window and keep
                            // the process alive so audio and the floating lyrics
                            // keep running in the background.
                            let close_to_tray = handle
                                .try_state::<platform::tray::CloseToTray>()
                                .map(|state| state.get())
                                .unwrap_or(false);
                            if close_to_tray {
                                api.prevent_close();
                                let _ = window.hide();
                                return;
                            }
                            if let Some(overlay) = handle.get_webview_window("desktop-lyrics") {
                                let _ = overlay.close();
                            }
                            handle.exit(0);
                        }
                    });
                }
                // OS transport controls need the main window (Windows SMTC uses
                // its HWND) and must be created on the main thread.
                platform::media_control::init(app.handle());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::ping,
            commands::echo,
            commands::log_message,
            commands::fonts::list_system_fonts,
            commands::webdav::load_source_password,
            commands::webdav::save_source_password,
            commands::webdav::delete_source_password,
            commands::webdav::webdav_url_risk,
            commands::sources::pick_local_folder,
            commands::sources::local_file_access_granted,
            commands::sources::request_local_file_access,
            commands::sources::list_source_audio,
            commands::sources::list_source_directory,
            commands::sources::list_source_audio_under,
            commands::sources::test_source_connection,
            commands::sources::test_source_write,
            commands::sponsor::get_install_id,
            commands::sponsor::build_sponsor_claim,
            // macOS/Windows install updates in-app through the Tauri updater.
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            commands::update::check_update,
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            commands::update::install_update,
            // Linux opens the GitHub Release page instead of auto-installing, so
            // it only needs the check command (no installer).
            #[cfg(target_os = "linux")]
            commands::update::check_update,
            #[cfg(target_os = "android")]
            commands::update::check_update,
            commands::license::activate_pro,
            commands::license::get_pro_status,
            commands::profile::load_local_profile,
            commands::profile::load_remote_profile,
            commands::profile::save_profile,
            commands::covers::upload_playlist_cover,
            commands::covers::resolve_playlist_covers,
            commands::covers::delete_playlist_cover,
            commands::editor::read_track_tags,
            commands::editor::edit_track_metadata,
            commands::playback::load_local_playback,
            commands::playback::load_remote_playback,
            commands::playback::save_playback,
            commands::media::read_track_metadata,
            commands::media::read_track_metadata_batch,
            commands::media::download_track_metadata,
            commands::media::probe_track,
            commands::media::read_track_lyrics,
            commands::media::get_cached_lyrics,
            commands::lyrics::lyric_http_get,
            commands::lyrics::save_track_lyrics,
            commands::media::get_cached_metadata,
            commands::media::get_cover,
            commands::media::get_cached_cover,
            commands::media::load_cached_tracks,
            commands::media::cover_path,
            commands::media::get_cache_dir,
            commands::media::set_cache_dir,
            commands::media::load_library_cache,
            commands::media::save_library_cache,
            commands::media::refresh_source_library,
            streaming::state::prefetch_track,
            streaming::state::report_stream_progress,
            streaming::state::stream_progress,
            streaming::state::sync_smart_cache,
            streaming::state::pin_track,
            streaming::state::unpin_track,
            streaming::state::cache_status,
            streaming::state::cache_usage,
            streaming::state::network_status,
            streaming::state::get_cache_config,
            streaming::state::set_smart_cache_limit,
            streaming::state::set_smart_cache_enabled,
            streaming::state::set_stream_cache_enabled,
            streaming::state::set_prefetch_seconds,
            streaming::state::clear_cache,
            #[cfg(any(desktop, target_os = "android"))]
            playback::commands::audio_load,
            #[cfg(any(desktop, target_os = "android"))]
            playback::commands::audio_play,
            #[cfg(any(desktop, target_os = "android"))]
            playback::commands::audio_pause,
            #[cfg(any(desktop, target_os = "android"))]
            playback::commands::audio_seek,
            #[cfg(any(desktop, target_os = "android"))]
            playback::commands::audio_set_volume,
            #[cfg(any(desktop, target_os = "android"))]
            playback::commands::audio_set_mute,
            #[cfg(any(desktop, target_os = "android"))]
            playback::commands::audio_set_speed,
            #[cfg(any(desktop, target_os = "android"))]
            playback::commands::audio_device_info,
            #[cfg(any(desktop, target_os = "android"))]
            playback::commands::audio_devices,
            #[cfg(any(desktop, target_os = "android"))]
            playback::commands::audio_set_device,
            #[cfg(any(desktop, target_os = "android"))]
            playback::commands::audio_set_quality,
            #[cfg(any(desktop, target_os = "android"))]
            playback::commands::audio_stop,
            #[cfg(any(desktop, target_os = "android"))]
            playback::commands::audio_state,
            #[cfg(any(desktop, target_os = "android"))]
            playback::commands_context::playback_set_queue,
            #[cfg(any(desktop, target_os = "android"))]
            playback::commands_context::playback_select,
            #[cfg(any(desktop, target_os = "android"))]
            playback::commands_context::playback_toggle,
            #[cfg(any(desktop, target_os = "android"))]
            playback::commands_context::playback_play,
            #[cfg(any(desktop, target_os = "android"))]
            playback::commands_context::playback_pause,
            #[cfg(any(desktop, target_os = "android"))]
            playback::commands_context::playback_next,
            #[cfg(any(desktop, target_os = "android"))]
            playback::commands_context::playback_previous,
            #[cfg(any(desktop, target_os = "android"))]
            playback::commands_context::playback_toggle_shuffle,
            #[cfg(any(desktop, target_os = "android"))]
            playback::commands_context::playback_set_shuffle,
            #[cfg(any(desktop, target_os = "android"))]
            playback::commands_context::playback_cycle_repeat,
            #[cfg(any(desktop, target_os = "android"))]
            playback::commands_context::playback_restore,
            #[cfg(any(desktop, target_os = "android"))]
            playback::commands_context::playback_state,
            commands::desktop_lyric::desktop_lyric_open,
            commands::desktop_lyric::desktop_lyric_close,
            commands::desktop_lyric::desktop_lyric_load,
            commands::desktop_lyric::desktop_lyric_tick,
            commands::desktop_lyric::desktop_lyric_set_settings,
            commands::desktop_lyric::desktop_lyric_status,
            commands::desktop_lyric::desktop_lyric_take_control,
            commands::desktop_lyric::desktop_lyric_request_permission,
            platform::media_control::media_control_update,
            platform::media_control::media_control_position,
            platform::media_control::media_control_clear,
            platform::media_control::media_control_take_control,
            platform::tray::set_close_to_tray
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                #[cfg(any(desktop, target_os = "android"))]
                if let Some(engine) = app.try_state::<playback::engine::Engine>() {
                    engine.shutdown();
                }
            }
        });
}
