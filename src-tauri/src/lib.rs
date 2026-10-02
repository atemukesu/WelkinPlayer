// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

#[cfg(target_os = "android")]
mod android;
mod backend;
mod commands;
mod dav;
mod error;
mod logging;
mod media_control;
mod metadata;
mod network;
mod proxy;
mod smart_cache;
mod sources;
mod stream_cache;
mod tray;

use tauri::Manager;

/// Extra WebView2 browser arguments applied to every window.
///
/// Disables Chromium's own system media controls so only the `souvlaki` session
/// remains. The background flags keep timers, rendering and audio alive when the
/// main window is hidden to the tray, so playback and the desktop-lyrics overlay
/// (which is driven by ticks from the main window and animates on its own rAF)
/// do not freeze. WebView2 requires every window sharing a data directory to use
/// identical options, so the main window's `additionalBrowserArgs` in
/// `tauri.conf.json` MUST stay byte-for-byte identical to this value.
#[cfg(desktop)]
pub(crate) const WEBVIEW_BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,HardwareMediaKeyHandling --autoplay-policy=no-user-gesture-required --disable-background-timer-throttling --disable-backgrounding-occluded-windows --disable-renderer-backgrounding";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    logging::init();

    // Android has no keyring backend; install the Keystore-backed one before
    // anything touches the keychain. A failure is recorded so the WebDAV
    // commands report an unavailable keychain instead of silently writing to
    // keyring's non-persistent mock store.
    #[cfg(target_os = "android")]
    commands::webdav::set_keychain_init_error(android::initialize_keychain());

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .manage(commands::desktop_lyric::DesktopLyricCache::default())
        .manage(media_control::MediaControlState::default())
        .manage(tray::CloseToTray::default())
        .setup(|app| {
            // Apply signed updates from GitHub Releases. Registered here (rather
            // than in the builder chain) so the whole block stays desktop-only.
            #[cfg(desktop)]
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
                    status.warning.as_deref().unwrap_or("unknown keychain error")
                ),
                Err(error) => log::warn!("failed to prepare install identifier: {error}"),
            }

            // Let the webview load cached cover thumbnails as plain asset URLs.
            commands::media::allow_cache_dir(app.handle());
            match proxy::start(app.handle().clone()) {
                Ok(stream_proxy) => {
                    log::info!(
                        "streaming proxy listening on 127.0.0.1:{}",
                        stream_proxy.port
                    );
                    app.manage(stream_proxy);
                }
                Err(error) => log::error!("failed to start streaming proxy: {error}"),
            }
            // Desktop window lifecycle: a close either quits (taking the
            // floating lyrics down with it) or, when "minimise to tray" is on,
            // just hides the main window so playback continues in the tray.
            #[cfg(desktop)]
            {
                // System tray: left click restores the window; right click
                // opens the 打开 / 退出 menu.
                if let Err(error) = tray::init(app) {
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
                                .try_state::<tray::CloseToTray>()
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
                media_control::init(app.handle());
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
            commands::sources::test_source_connection,
            commands::sources::test_source_write,
            commands::sponsor::get_install_id,
            commands::sponsor::build_sponsor_claim,
            #[cfg(desktop)]
            commands::update::check_update,
            #[cfg(desktop)]
            commands::update::install_update,
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
            proxy::stream_endpoint,
            proxy::prefetch_track,
            proxy::report_stream_progress,
            proxy::stream_progress,
            proxy::sync_smart_cache,
            proxy::pin_track,
            proxy::unpin_track,
            proxy::cache_status,
            proxy::cache_usage,
            proxy::network_status,
            proxy::get_stream_cache_limit,
            proxy::set_stream_cache_limit,
            proxy::clear_stream_cache,
            commands::desktop_lyric::desktop_lyric_open,
            commands::desktop_lyric::desktop_lyric_close,
            commands::desktop_lyric::desktop_lyric_load,
            commands::desktop_lyric::desktop_lyric_tick,
            commands::desktop_lyric::desktop_lyric_set_settings,
            commands::desktop_lyric::desktop_lyric_status,
            commands::desktop_lyric::desktop_lyric_take_control,
            commands::desktop_lyric::desktop_lyric_request_permission,
            media_control::media_control_update,
            media_control::media_control_position,
            media_control::media_control_clear,
            media_control::media_control_take_control,
            tray::set_close_to_tray
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
