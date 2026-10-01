#[cfg(target_os = "android")]
mod android;
mod backend;
mod commands;
mod dav;
mod error;
mod logging;
mod metadata;
mod network;
mod proxy;
mod smart_cache;
mod sources;
mod stream_cache;

use tauri::Manager;

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
        .setup(|app| {
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
            commands::sources::list_source_audio,
            commands::sources::test_source_connection,
            commands::sources::test_source_write,
            commands::sponsor::get_install_id,
            commands::sponsor::build_sponsor_claim,
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
            proxy::clear_stream_cache
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
