// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! NetEase Cloud Music cover search and fetch.

use serde::Serialize;

use crate::core::error::AppError;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

const ALLOWED_HOSTS: [&str; 5] = [
    "music.163.com",
    "p1.music.126.net",
    "p2.music.126.net",
    "p3.music.126.net",
    "p4.music.126.net",
];

fn http_client() -> Result<reqwest::Client, AppError> {
    Ok(reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(20))
        .user_agent(USER_AGENT)
        .build()?)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseSongHit {
    pub song_id: i64,
    pub album_id: i64,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub cover_url: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseCoverResult {
    pub songs: Vec<NeteaseSongHit>,
}

/// Resolve the album cover URL for a NetEase album ID.
async fn resolve_album_pic(album_id: i64) -> Option<String> {
    let http = http_client().ok()?;
    let url = format!("https://music.163.com/api/album/{album_id}/");
    let response = http
        .get(&url)
        .header("Referer", "https://music.163.com/")
        .header("Cookie", "appver=1.5.2")
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body = response.text().await.ok()?;
    let parsed = serde_json::from_str::<serde_json::Value>(&body).ok()?;
    parsed
        .pointer("/album/picUrl")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Search NetEase Cloud Music for songs matching `text`.
#[tauri::command]
pub async fn netease_search_songs(text: String) -> Result<NeteaseCoverResult, AppError> {
    let query = text.trim();
    if query.is_empty() {
        return Err(AppError::invalid_argument("text", "搜索关键词不能为空"));
    }
    let url = format!(
        "https://music.163.com/api/search/get?type=1&limit=10&offset=0&s={}",
        urlencoding::encode(query)
    );

    let http = http_client()?;
    let response = http
        .get(&url)
        .header("Referer", "https://music.163.com/")
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        return Err(AppError::other(format!("网易云接口返回 {status}")));
    }
    let body = response.text().await?;
    let parsed = serde_json::from_str::<serde_json::Value>(&body)
        .map_err(|error| AppError::Other(format!("网易云搜索响应解析失败：{error}")))?;

    let list = parsed
        .pointer("/result/songs")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    // Collect basic song info first.
    let basic: Vec<(i64, i64, String, String, String)> = list
        .iter()
        .filter_map(|song| {
            let song_id = song.get("id")?.as_i64()?;
            let album_id = song.get("album")?.get("id")?.as_i64()?;
            let title = song.get("name")?.as_str()?.to_string();
            let artist = song
                .get("artists")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|a| a.get("name")?.as_str())
                        .collect::<Vec<&str>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let album = song
                .get("album")
                .and_then(|a| a.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some((song_id, album_id, title, artist, album))
        })
        .collect();

    // Resolve cover URLs sequentially — the album endpoint is fast enough and
    // avoids pulling in a concurrent-runtime dependency.
    let mut pics = Vec::with_capacity(basic.len());
    for (_, album_id, _, _, _) in &basic {
        pics.push(resolve_album_pic(*album_id).await);
    }

    let songs: Vec<NeteaseSongHit> = basic
        .into_iter()
        .zip(pics.into_iter())
        .map(|((song_id, album_id, title, artist, album), cover_url)| NeteaseSongHit {
            song_id,
            album_id,
            title,
            artist,
            album,
            cover_url: cover_url.unwrap_or_default(),
        })
        .collect();

    Ok(NeteaseCoverResult { songs })
}

/// Fetch a NetEase album cover as raw image bytes.
#[tauri::command]
pub async fn netease_fetch_cover(album_id: i64) -> Result<Option<Vec<u8>>, AppError> {
    if album_id <= 0 {
        return Err(AppError::invalid_argument("albumId", "专辑 ID 无效"));
    }

    let pic_url = match resolve_album_pic(album_id).await {
        Some(url) if !url.is_empty() => url,
        _ => return Ok(None),
    };

    let http = http_client()?;
    let parsed = reqwest::Url::parse(&pic_url)
        .map_err(|error| AppError::invalid_argument("url", format!("无效的封面地址：{error}")))?;
    if parsed.scheme() != "https" {
        return Err(AppError::invalid_argument("url", "封面必须使用 https"));
    }
    let host = parsed.host_str().unwrap_or_default();
    if !ALLOWED_HOSTS.contains(&host) {
        return Err(AppError::invalid_argument(
            "url",
            format!("不允许的封面来源：{host}"),
        ));
    }

    let response = http.get(parsed).send().await?;
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