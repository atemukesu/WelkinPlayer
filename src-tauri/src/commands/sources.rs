//! Song source management commands.
//!
//! CRUD of the source list itself lives in the frontend (it writes
//! `settings.json` through `tauri-plugin-store`). This module provides the
//! operations that need the backend: the native folder picker, connection /
//! write probes and per-source library listing.

use tauri::AppHandle;
#[cfg(desktop)]
use tauri_plugin_dialog::DialogExt;

use crate::backend::{backend_for, build_backend};
use crate::dav::{Depth, RemoteEntry};
use crate::error::AppError;

/// Temporary object used to prove a source allows writes.
const WRITE_TEST_PATH: &str = "welkin-write-test.tmp";

/// Show the native folder picker and return the chosen directory.
#[tauri::command]
pub async fn pick_local_folder(app: AppHandle) -> Result<Option<String>, AppError> {
    #[cfg(desktop)]
    {
        let folder = app.dialog().file().blocking_pick_folder();
        return match folder {
            Some(path) => {
                let path = path
                    .into_path()
                    .map_err(|error| AppError::Other(format!("无法读取所选目录：{error}")))?;
                Ok(Some(path.to_string_lossy().to_string()))
            }
            None => Ok(None),
        };
    }
    #[cfg(not(desktop))]
    {
        let _ = app;
        Ok(None)
    }
}

/// List the audio files under a source.
#[tauri::command]
pub async fn list_source_audio(
    app: AppHandle,
    source_id: String,
) -> Result<Vec<RemoteEntry>, AppError> {
    let backend = backend_for(&app, &source_id)?;
    let files = backend.list_audio().await?;
    log::info!("listed {} audio files from source {source_id}", files.len());
    Ok(files)
}

/// Verify a source is reachable and readable. Returns a display string.
#[tauri::command]
pub async fn test_source_connection(app: AppHandle, source_id: String) -> Result<String, AppError> {
    let source = crate::sources::find_source(&app, &source_id)?;
    if source.is_local() {
        // `build_backend` already validates the directory exists.
        let backend = build_backend(&app, &source)?;
        let files = backend.list_audio().await?;
        return Ok(format!("本地来源可用（{} 个音频文件）", files.len()));
    }

    let backend = build_backend(&app, &source)?;
    match backend {
        crate::backend::Backend::Webdav(client) => {
            let xml = client.propfind("", Depth::Zero).await?;
            if xml.trim().is_empty() {
                return Err(AppError::Xml("服务器返回了空的 PROPFIND 响应".to_string()));
            }
            Ok(source.url.unwrap_or_default())
        }
        _ => unreachable!("webdav source resolved to a non-webdav backend"),
    }
}

/// Verify a source allows writes: PUT a throwaway object, read it back, DELETE.
#[tauri::command]
pub async fn test_source_write(app: AppHandle, source_id: String) -> Result<String, AppError> {
    let backend = backend_for(&app, &source_id)?;
    let payload = b"welkin write probe".to_vec();
    backend
        .put(WRITE_TEST_PATH, "text/plain", payload.clone())
        .await?;
    let echoed = backend.read(WRITE_TEST_PATH).await?;
    let delete_result = backend.delete(WRITE_TEST_PATH).await;

    if echoed != payload {
        return Err(AppError::Other("写入校验失败：回读内容不一致".to_string()));
    }
    delete_result?;
    log::info!("source write test succeeded for {source_id}");
    Ok(source_id)
}
