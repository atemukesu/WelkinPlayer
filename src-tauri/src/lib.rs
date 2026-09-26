mod commands;
mod dav;
mod error;
mod logging;
mod metadata;
mod proxy;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    logging::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .setup(|app| {
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
            commands::fonts::list_system_fonts,
            commands::webdav::load_webdav_password,
            commands::webdav::save_webdav_password,
            commands::webdav::webdav_url_risk,
            commands::webdav::test_webdav_connection,
            commands::webdav::list_webdav_audio,
            commands::profile::load_local_profile,
            commands::profile::load_remote_profile,
            commands::profile::save_profile,
            commands::profile::test_webdav_write,
            commands::covers::upload_playlist_cover,
            commands::covers::resolve_playlist_covers,
            commands::covers::delete_playlist_cover,
            commands::playback::load_local_playback,
            commands::playback::load_remote_playback,
            commands::playback::save_playback,
            commands::media::read_track_metadata,
            commands::media::download_track_metadata,
            commands::media::read_track_lyrics,
            commands::media::get_cached_lyrics,
            commands::media::get_cached_metadata,
            commands::media::get_cover,
            commands::media::get_cached_cover,
            commands::media::load_cached_tracks,
            commands::media::cover_path,
            commands::media::get_cache_dir,
            commands::media::set_cache_dir,
            commands::media::load_library_cache,
            commands::media::save_library_cache,
            commands::media::refresh_webdav_library,
            proxy::stream_endpoint
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
