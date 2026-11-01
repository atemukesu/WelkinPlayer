// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! QQ Music cover search and fetch.
//!
//! The music platforms do not send CORS headers, so every HTTP request is
//! tunnelled through Rust (restricted to an allowlist of hosts). The frontend
//! builds the query and receives a resolved cover URL — or `null` when no
//! cover could be found.

use serde::Serialize;

use crate::core::error::AppError;

/// A desktop browser UA; the music platforms reject the default agent.
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64.0; Win64; x64) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// Hosts the cover tunnel is allowed to reach.
const ALLOWED_HOSTS: [&str; 2] = ["c.y.qq.com", "y.gtimg.cn"];

fn http_client() -> Result<reqwest::Client, AppError> {
    Ok(reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(20))
        .user_agent(USER_AGENT)
        .build()?)
}

/// One song hit from QQ Music's search.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QqSongHit {
    pub song_mid: String,
    pub song_id: Option<String>,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_mid: String,
}

/// Result of a QQ Music cover search.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QqCoverResult {
    pub songs: Vec<QqSongHit>,
}

/// Search QQ Music for songs matching `text`.
///
/// Uses the `search_for_qq_cp` endpoint, which still returns plain results.
#[tauri::command]
pub async fn qq_search_songs(text: String) -> Result<QqCoverResult, AppError> {
    let query = text.trim();
    if query.is_empty() {
        return Err(AppError::invalid_argument("text", "搜索关键词不能为空"));
    }
    let url = format!(
        "https://c.y.qq.com/soso/fcgi-bin/search_for_qq_cp?format=json&w={}&n=10&p=1",
        urlencoding::encode(query)
    );

    let body = tunnel(&url, "https://y.qq.com/").await?;
    let parsed = serde_json::from_str::<serde_json::Value>(&body)
        .map_err(|error| AppError::Other(format!("QQ 搜索响应解析失败：{error}")))?;

    let list = parsed
        .pointer("/data/song/list")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    let songs: Vec<QqSongHit> = list
        .iter()
        .filter_map(|song| {
            let song_mid = song.get("songmid")?.as_str()?.to_string();
            let song_id = song
                .get("songid")
                .and_then(|value| value.as_i64())
                .map(|value| value.to_string());
            let title = song.get("songname")?.as_str()?.to_string();
            let artist = song
                .get("singer")
                .and_then(|value| value.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|s| s.get("name")?.as_str())
                        .collect::<Vec<&str>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let album = song.get("albumname")?.as_str()?.to_string();
            let album_mid = song.get("albummid")?.as_str()?.to_string();
            Some(QqSongHit {
                song_mid,
                song_id,
                title,
                artist,
                album,
                album_mid,
            })
        })
        .collect();

    Ok(QqCoverResult { songs })
}

/// Build a QQ Music album cover URL from an album MID.
///
/// The URL pattern is `https://y.gtimg.cn/music/photo_new/T002R{size}x{size}M000{albumMid}.jpg`.
/// The `000` prefix before the album MID is required — without it the server
/// returns 404.
pub fn qq_cover_url(album_mid: &str, size: u32) -> String {
    format!(
        "https://y.gtimg.cn/music/photo_new/T002R{size}x{size}M000{albumMid}.jpg?max_age=2592000",
        albumMid = album_mid,
        size = size
    )
}

/// Fetch a QQ Music album cover as raw image bytes.
///
/// Tunnels the request through Rust (bypasses CORS) and returns the JPEG
/// bytes, or `null` when the server does not carry a cover.
#[tauri::command]
pub async fn qq_fetch_cover(album_mid: String, size: Option<u32>) -> Result<Option<Vec<u8>>, AppError> {
    let mid = album_mid.trim();
    if mid.is_empty() {
        return Err(AppError::invalid_argument("albumMid", "专辑 MID 不能为空"));
    }
    let size = size.unwrap_or(300).clamp(150, 800);
    let url = qq_cover_url(mid, size);

    let http = http_client()?;
    let response = http.get(&url).send().await?;
    let status = response.status();
    if !status.is_success() {
        return Ok(None);
    }
    let bytes = response.bytes().await?;
    if bytes.is_empty() {
        return Ok(None);
    }
    Ok(Some(bytes.to_vec()))
}

/// Raw text GET through the Rust tunnel (bypasses CORS).
async fn tunnel(url: &str, referer: &str) -> Result<String, AppError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|error| AppError::invalid_argument("url", format!("无效的接口地址：{error}")))?;
    if parsed.scheme() != "https" {
        return Err(AppError::invalid_argument("url", "接口必须使用 https"));
    }
    let host = parsed.host_str().unwrap_or_default();
    if !ALLOWED_HOSTS.contains(&host) {
        return Err(AppError::invalid_argument(
            "url",
            format!("不允许的封面来源：{host}"),
        ));
    }

    log::info!("qq cover http GET {}", parsed.as_str());

    let http = http_client()?;
    let response = http
        .get(parsed)
        .header(reqwest::header::REFERER, referer)
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let preview: String = body.chars().take(200).collect();
        log::warn!("qq cover http {status}: {preview}");
        return Err(AppError::other(format!("QQ 接口返回 {status}")));
    }

    let body = response.text().await?;
    log::info!("qq cover http {} -> {} chars", status, body.chars().count());
    Ok(body)
}