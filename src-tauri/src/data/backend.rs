// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Unified storage backend for a song source.
//!
//! Every source resolves to a [`Backend`] — either a WebDAV client or a local
//! directory. Higher-level commands (metadata, lyrics, profile, playback,
//! covers) are written once against this interface so adding a third source
//! kind later is a single match arm.
//!
//! Paths are always *relative* to the source root, using `/` separators. For a
//! local source they are resolved inside the chosen directory with a traversal
//! guard; for WebDAV they are resolved against the collection URL by
//! [`WebDavClient`].

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use tauri::AppHandle;

use crate::commands::webdav::{enforce_url_policy, source_password};
use crate::data::dav::{is_audio_file, RemoteEntry, WebDavClient};
use crate::core::error::AppError;
use crate::data::sources::{find_source, SourceConfig, LOCAL_KIND, WEBDAV_KIND};

/// A resolved source transport.
pub enum Backend {
    /// A remote WebDAV collection.
    Webdav(WebDavClient),
    /// A directory on this machine.
    Local { root: PathBuf },
}

impl Backend {
    /// Read a whole resource into memory.
    pub async fn read(&self, path: &str) -> Result<Vec<u8>, AppError> {
        match self {
            Backend::Webdav(client) => Ok(client.get(path).await?.bytes().await?.to_vec()),
            Backend::Local { root } => {
                let file = safe_join(root, path)?;
                std::fs::read(&file).map_err(map_local_read_error)
            }
        }
    }

    /// Read a whole text resource (UTF-8).
    pub async fn read_text(&self, path: &str) -> Result<String, AppError> {
        match self {
            Backend::Webdav(client) => Ok(client.get(path).await?.text().await?),
            Backend::Local { root } => {
                let file = safe_join(root, path)?;
                std::fs::read_to_string(&file).map_err(map_local_read_error)
            }
        }
    }

    /// Read at most `max_bytes` from the start of a resource.
    pub async fn read_range(&self, path: &str, max_bytes: u64) -> Result<Vec<u8>, AppError> {
        match self {
            Backend::Webdav(client) => client.get_range(path, max_bytes).await,
            Backend::Local { root } => {
                let file_path = safe_join(root, path)?;
                let file = std::fs::File::open(&file_path).map_err(map_local_read_error)?;
                let mut buffer = Vec::new();
                file.take(max_bytes).read_to_end(&mut buffer)?;
                Ok(buffer)
            }
        }
    }

    /// Write bytes to a resource, creating parent directories for local sources.
    pub async fn put(&self, path: &str, content_type: &str, body: Vec<u8>) -> Result<(), AppError> {
        match self {
            Backend::Webdav(client) => client.put(path, content_type, body).await,
            Backend::Local { root } => {
                let file = safe_join(root, path)?;
                if let Some(parent) = file.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(file, body)?;
                Ok(())
            }
        }
    }

    /// Delete a resource, treating "already gone" as success.
    pub async fn delete(&self, path: &str) -> Result<(), AppError> {
        match self {
            Backend::Webdav(client) => client.delete(path).await,
            Backend::Local { root } => {
                let file = safe_join(root, path)?;
                match std::fs::remove_file(file) {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(AppError::Io(error)),
                }
            }
        }
    }

    /// Confirm a resource can be read (one small ranged read).
    pub async fn probe(&self, path: &str) -> Result<(), AppError> {
        match self {
            Backend::Webdav(client) => {
                client.get_range(path, 1).await?;
                Ok(())
            }
            Backend::Local { root } => {
                let file = safe_join(root, path)?;
                if file.is_file() {
                    Ok(())
                } else {
                    Err(AppError::WebdavNotFound(path.to_string()))
                }
            }
        }
    }

    /// Recursively list audio files under the source root.
    pub async fn list_audio(&self) -> Result<Vec<RemoteEntry>, AppError> {
        match self {
            Backend::Webdav(client) => client.list_audio_files("").await,
            Backend::Local { root } => list_local_audio(root),
        }
    }
}

/// Build the backend for `source`.
pub fn build_backend(app: &AppHandle, source: &SourceConfig) -> Result<Backend, AppError> {
    match source.kind.as_str() {
        WEBDAV_KIND => {
            let url = source.url.clone().unwrap_or_default();
            let username = source.username.clone().unwrap_or_default();
            if url.trim().is_empty() || username.trim().is_empty() {
                return Err(AppError::MissingCredentials(
                    "请先填写并保存该来源的服务器地址与用户名".to_string(),
                ));
            }
            enforce_url_policy(app, &url, source.allow_insecure)?;
            let password = source_password(app, &source.id)?;
            let client = WebDavClient::new(&url, &username, &password)?;
            Ok(Backend::Webdav(client))
        }
        LOCAL_KIND => {
            let root = source.root_path.clone().unwrap_or_default();
            if root.trim().is_empty() {
                return Err(AppError::invalid_argument(
                    "rootPath",
                    "本地来源缺少目录",
                ));
            }
            let root = PathBuf::from(root);
            if !root.is_dir() {
                return Err(AppError::Store(format!(
                    "本地来源目录不存在：{}",
                    root.display()
                )));
            }
            Ok(Backend::Local { root })
        }
        other => Err(AppError::invalid_argument(
            "kind",
            format!("未知的来源类型：{other}"),
        )),
    }
}

/// Build the backend for a source id.
pub fn backend_for(app: &AppHandle, source_id: &str) -> Result<Backend, AppError> {
    let source = find_source(app, source_id)?;
    build_backend(app, &source)
}

/// Build the backend for the configured sync source, when one exists.
pub fn sync_backend(app: &AppHandle) -> Result<Option<Backend>, AppError> {
    match crate::data::sources::sync_source(app) {
        Some(source) => Ok(Some(build_backend(app, &source)?)),
        None => Ok(None),
    }
}

/// Resolve a relative path inside `root`, refusing absolute paths and traversal.
pub fn safe_join(root: &Path, path: &str) -> Result<PathBuf, AppError> {
    let relative = Path::new(path.trim());
    let mut out = root.to_path_buf();
    for component in relative.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            _ => {
                return Err(AppError::invalid_argument(
                    "path",
                    "路径超出了来源目录",
                ))
            }
        }
    }
    Ok(out)
}

/// Map an I/O error while reading a source resource; a missing file becomes the
/// same "not found" signal WebDAV emits, so callers handle both alike.
fn map_local_read_error(error: std::io::Error) -> AppError {
    if error.kind() == std::io::ErrorKind::NotFound {
        AppError::WebdavNotFound("来源中不存在该文件".to_string())
    } else {
        AppError::Io(error)
    }
}

/// Best-effort content type from a file name extension.
pub fn content_type_for(name: &str) -> Option<String> {
    let extension = Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let value = match extension.as_str() {
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "m4a" | "mp4" | "aac" => "audio/mp4",
        "ogg" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "wma" => "audio/x-ms-wma",
        _ => return None,
    };
    Some(value.to_string())
}

/// Walk a local directory (depth-first) collecting audio files as entries.
fn list_local_audio(root: &Path) -> Result<Vec<RemoteEntry>, AppError> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(directory) = stack.pop() {
        let read = match std::fs::read_dir(&directory) {
            Ok(read) => read,
            // A missing directory or one the app cannot read (common in shared
            // storage under Android scoped storage) must not abort the whole
            // scan, so both are skipped.
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    || error.kind() == std::io::ErrorKind::PermissionDenied =>
            {
                continue
            }
            Err(error) => return Err(AppError::Io(error)),
        };
        for entry in read.flatten() {
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if !is_audio_file(&name) {
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let metadata = entry.metadata().ok();
            let size = metadata.as_ref().map(|meta| meta.len());
            let modified = metadata
                .as_ref()
                .and_then(|meta| meta.modified().ok())
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs().to_string());
            files.push(RemoteEntry {
                name: name.clone(),
                path: relative.clone(),
                local_path: relative,
                is_dir: false,
                size,
                modified,
                content_type: content_type_for(&name),
            });
        }
    }

    files.sort_by(|a, b| a.local_path.cmp(&b.local_path));
    Ok(files)
}
