// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Song source configuration.
//!
//! A *source* is a place audio files are read from. Two kinds are supported:
//! `webdav` (a remote WebDAV collection, optionally used as the cloud-sync
//! target) and `local` (a directory on this machine). The list lives in
//! `settings.json` (`sources`), together with the id of the source the user
//! chose to hold the synced profile / playback state (`sources.syncId`).
//!
//! The backend reads this configuration; the frontend owns its CRUD through
//! `tauri-plugin-store`. Secrets never live here: a WebDAV password is stored in
//! the OS keychain keyed by the source id.

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

use crate::core::error::AppError;

/// `tauri-plugin-store` file that holds all non-secret settings.
pub const SETTINGS_FILE: &str = "settings.json";
/// Key holding the serialized `SourceConfig[]`.
pub const SOURCES_KEY: &str = "sources";
/// Key holding the id of the source used to store synced data.
pub const SYNC_SOURCE_KEY: &str = "sources.syncId";

/// Discriminator for the source implementation to use.
pub const WEBDAV_KIND: &str = "webdav";
pub const LOCAL_KIND: &str = "local";

/// A configured song source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceConfig {
    /// Stable identifier (UUID). Forms the profile key namespace and the
    /// keychain account for the password.
    pub id: String,
    /// `webdav` or `local`.
    pub kind: String,
    /// Human-readable label shown in the UI.
    #[serde(default)]
    pub name: String,
    /// WebDAV server collection URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// WebDAV username.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Whether plaintext HTTP to a public host is allowed for this source.
    #[serde(default)]
    pub allow_insecure: bool,
    /// Absolute directory path for a local source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_path: Option<String>,
}

impl SourceConfig {
    pub fn is_local(&self) -> bool {
        self.kind == LOCAL_KIND
    }
}

/// Read the configured sources (empty when none are set yet).
pub fn load_sources(app: &AppHandle) -> Vec<SourceConfig> {
    app.store(SETTINGS_FILE)
        .ok()
        .and_then(|store| store.get(SOURCES_KEY))
        .and_then(|value| serde_json::from_value::<Vec<SourceConfig>>(value).ok())
        .unwrap_or_default()
}

/// Read the id of the source configured to hold synced data.
pub fn read_sync_id(app: &AppHandle) -> Option<String> {
    app.store(SETTINGS_FILE)
        .ok()
        .and_then(|store| store.get(SYNC_SOURCE_KEY))
        .and_then(|value| value.as_str().map(str::to_string))
        .filter(|value| !value.trim().is_empty())
}

/// Look up one source by id.
pub fn find_source(app: &AppHandle, id: &str) -> Result<SourceConfig, AppError> {
    load_sources(app)
        .into_iter()
        .find(|source| source.id == id)
        .ok_or_else(|| AppError::invalid_argument("sourceId", "未知的歌曲来源"))
}

/// The source that stores the synced profile / playback state, when configured.
pub fn sync_source(app: &AppHandle) -> Option<SourceConfig> {
    let id = read_sync_id(app)?;
    load_sources(app)
        .into_iter()
        .find(|source| source.id == id)
}
