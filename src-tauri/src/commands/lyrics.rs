//! Lyric transport and persistence.
//!
//! All provider logic (search, platform-ID resolution, response shaping) lives
//! in the frontend (`src/lib/lyricSources.ts`). This module only provides the
//! two things the webview cannot do on its own:
//!
//! * [`lyric_http_get`] — music-platform endpoints do not send CORS headers, so
//!   requests are tunnelled through Rust (restricted to an allowlist of hosts).
//! * [`save_track_lyrics`] — write a fetched lyric to the local cache **and**
//!   upload it next to the audio file as a same-named `.lrc` on WebDAV, so the
//!   `local` source finds it on every later play.

use std::time::Duration;

use reqwest::{header, Client, Url};
use tauri::AppHandle;

use super::media::{cover_hash, lyric_path, resolve_cache_dir, write_lyric};
use super::webdav::require_credentials;
use crate::dav::WebDavClient;
use crate::error::AppError;

/// A desktop browser UA; the music platforms reject the default agent.
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// Hosts the lyric tunnel is allowed to reach.
const ALLOWED_HOSTS: [&str; 3] = ["api.amll.dev", "music.163.com", "c.y.qq.com"];

fn http_client() -> Result<Client, AppError> {
    Ok(Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .user_agent(USER_AGENT)
        .build()?)
}

/// GET a lyric-provider URL and return the response body as text.
///
/// The frontend builds the URL (query parameters, encoding); Rust only performs
/// transport so the webview never hits CORS. `referer` is required by some
/// platforms and is passed through unchanged.
#[tauri::command]
pub async fn lyric_http_get(url: String, referer: Option<String>) -> Result<String, AppError> {
    let parsed = Url::parse(&url)
        .map_err(|error| AppError::invalid_argument("url", format!("无效的歌词接口地址：{error}")))?;
    if parsed.scheme() != "https" {
        return Err(AppError::invalid_argument("url", "歌词接口必须使用 https"));
    }
    let host = parsed.host_str().unwrap_or_default();
    if !ALLOWED_HOSTS.contains(&host) {
        return Err(AppError::invalid_argument(
            "url",
            format!("不允许的歌词来源：{host}"),
        ));
    }

    log::info!("lyric http GET {}", parsed.as_str());

    let http = http_client()?;
    let mut request = http.get(parsed);
    if let Some(referer) = referer.filter(|value| !value.is_empty()) {
        request = request.header(header::REFERER, referer);
    }

    let response = request.send().await?;
    let status = response.status();
    if !status.is_success() {
        let bytes = response.bytes().await.unwrap_or_default();
        let body = String::from_utf8_lossy(&bytes);
        let preview: String = body.chars().take(200).collect();
        log::warn!("lyric http {status}: {preview}");
        return Err(AppError::other(format!("歌词接口返回 {status}")));
    }

    // Decode the bytes as UTF-8 explicitly. QQ answers with `text/html` and no
    // charset, and these payloads are UTF-8; going through a guessed legacy
    // encoding would turn the Chinese/Japanese text into mojibake.
    let bytes = response.bytes().await?;
    let body = String::from_utf8_lossy(&bytes).into_owned();
    log::info!("lyric http {status} -> {} chars", body.chars().count());
    Ok(body)
}

/// Persist a fetched lyric: local cache first, then the WebDAV sidecar.
///
/// The WebDAV upload is best-effort — a missing server or a failed write must
/// never break playback, because the local cache already holds the lyrics.
#[tauri::command]
pub async fn save_track_lyrics(
    app: AppHandle,
    path: String,
    content: String,
) -> Result<(), AppError> {
    let dir = resolve_cache_dir(&app);
    write_lyric(&dir, &cover_hash(&path), &content)?;
    log::info!(
        "lyrics cached for {path} ({} chars)",
        content.chars().count()
    );

    match upload_sidecar(&app, &path, &content).await {
        Ok(true) => log::info!("lyrics sidecar uploaded for {path}"),
        Ok(false) => log::info!("WebDAV not configured; lyrics cached locally only for {path}"),
        Err(error) => log::warn!("failed to upload lyrics for {path}: {error}"),
    }
    Ok(())
}

/// Upload the same-named `.lrc` file; `Ok(false)` when WebDAV is not configured.
async fn upload_sidecar(app: &AppHandle, path: &str, content: &str) -> Result<bool, AppError> {
    let Ok(credentials) = require_credentials(app) else {
        return Ok(false);
    };
    let client = WebDavClient::new(&credentials.url, &credentials.username, &credentials.password)?;
    let remote = lyric_path(path)?;
    client
        .put(
            &remote,
            "text/plain; charset=utf-8",
            content.as_bytes().to_vec(),
        )
        .await?;
    Ok(true)
}
