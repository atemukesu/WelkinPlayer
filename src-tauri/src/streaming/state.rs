// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Streaming/cache state shared by the playback engine and the frontend.
//!
//! There is no loopback HTTP proxy any more: the playback engine reads the
//! read-ahead cache file in-process (see [`crate::playback::source`]). This
//! module only owns the two cache tiers and the commands that configure and
//! inspect them.

use std::sync::Arc;

use serde::Serialize;
use tauri::AppHandle;

use crate::commands::media::asset_hash;
use crate::data::sources::find_source;
use crate::streaming::smart_cache::{SmartCache, SmartCandidate};
use crate::streaming::stream_cache::{
    read_lead_seconds, read_smart_enabled, read_smart_limit_mb, read_stream_enabled,
    write_lead_seconds, write_smart_enabled, write_smart_limit_mb, write_stream_enabled,
    StreamCache, STREAM_BUDGET_MB,
};

/// Cache tiers managed as Tauri state.
pub struct StreamState {
    pub cache: Arc<StreamCache>,
    pub smart: Arc<SmartCache>,
}

/// Build the cache tiers. Called once at startup.
pub fn start(app: AppHandle) -> Result<StreamState, String> {
    // The read-ahead buffer has its own fixed budget: it is required for smooth
    // streaming and never eats into the user's smart-cache limit.
    let hot_bytes = STREAM_BUDGET_MB.saturating_mul(1024 * 1024);
    let smart_bytes = read_smart_limit_mb(&app).saturating_mul(1024 * 1024);
    let cache = Arc::new(StreamCache::new(&app, hot_bytes)?);
    cache.set_enabled(read_stream_enabled(&app));
    let smart = SmartCache::new(&app, smart_bytes)?;
    smart.set_enabled(read_smart_enabled(&app));
    Ok(StreamState { cache, smart })
}

/// Aggressively prefetch a track's bytes into the read-ahead cache. Called for
/// the current track and, shortly before it ends, for the next queue entry
/// (`lead_secs` shortens that speculative preload). Skips tracks already in the
/// smart cache.
#[tauri::command]
pub fn prefetch_track(
    state: tauri::State<StreamState>,
    app: AppHandle,
    source_id: String,
    path: String,
    lead_secs: Option<f64>,
) -> Result<(), crate::core::error::AppError> {
    let source = find_source(&app, &source_id)?;
    if source.is_local() {
        return Ok(());
    }
    if !state.cache.is_enabled() {
        return Ok(());
    }
    if state.smart.is_complete(&asset_hash(&source_id, &path)) {
        return Ok(());
    }
    state.cache.prefetch(&app, source, &path, lead_secs);
    Ok(())
}

/// Replace the smart cache's candidate set (ranked play counts from the
/// frontend). Downloads are gated on an unmetered connection by Rust.
#[tauri::command]
pub fn sync_smart_cache(
    state: tauri::State<StreamState>,
    candidates: Vec<SmartCandidate>,
) -> Result<(), crate::core::error::AppError> {
    state.smart.sync(candidates);
    Ok(())
}

/// Pin a track: downloaded first and never evicted.
#[tauri::command]
pub fn pin_track(
    state: tauri::State<StreamState>,
    app: AppHandle,
    source_id: String,
    path: String,
) -> Result<(), crate::core::error::AppError> {
    let source = find_source(&app, &source_id)?;
    if !source.is_local() {
        state.smart.pin(&source_id, &path);
    }
    Ok(())
}

/// Stop protecting a pinned track.
#[tauri::command]
pub fn unpin_track(
    state: tauri::State<StreamState>,
    source_id: String,
    path: String,
) -> Result<(), crate::core::error::AppError> {
    state.smart.unpin(&asset_hash(&source_id, &path));
    Ok(())
}

/// Complete/pinned keys for the track-list badges.
#[tauri::command]
pub fn cache_status(
    state: tauri::State<StreamState>,
) -> Result<crate::streaming::smart_cache::CacheStatus, crate::core::error::AppError> {
    Ok(state.smart.status())
}

/// Disk usage snapshot for the settings progress bars.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheUsage {
    /// Read-ahead buffer currently on disk (the smooth-playback working set).
    pub stream_bytes: u64,
    /// Fixed read-ahead buffer budget.
    pub stream_limit_bytes: u64,
    /// Automatic whole-track cache currently on disk.
    pub smart_bytes: u64,
    /// Configured budget for the automatic whole-track cache.
    pub smart_limit_bytes: u64,
    /// Manually pinned (user-cached) tracks. Never evicted, so reported apart.
    pub pinned_bytes: u64,
}

/// Separate usage figures for the read-ahead buffer, the automatic whole-track
/// cache, and the manual (pinned) cache, in bytes.
#[tauri::command]
pub fn cache_usage(
    state: tauri::State<StreamState>,
) -> Result<CacheUsage, crate::core::error::AppError> {
    Ok(CacheUsage {
        stream_bytes: state.cache.used_bytes(),
        stream_limit_bytes: state.cache.limit_bytes(),
        smart_bytes: state.smart.automatic_bytes(),
        smart_limit_bytes: state.smart.limit_bytes(),
        pinned_bytes: state.smart.pinned_bytes(),
    })
}

/// Active network classification (metered / Wi-Fi).
#[tauri::command]
pub fn network_status() -> crate::core::network::NetworkStatus {
    crate::core::network::status()
}

/// Cache coverage (0-100) for the progress bar's prefill. Local files need no
/// buffering, so they report 100.
#[tauri::command]
pub fn stream_progress(
    app: AppHandle,
    state: tauri::State<StreamState>,
    source_id: String,
    path: String,
) -> Result<f64, crate::core::error::AppError> {
    let local = find_source(&app, &source_id)
        .map(|source| source.is_local())
        .unwrap_or(false);
    if local {
        return Ok(100.0);
    }
    let key = asset_hash(&source_id, &path);
    if state.smart.is_complete(&key) {
        return Ok(100.0);
    }
    Ok(state.cache.coverage_percent(&key).unwrap_or(0.0))
}

/// Report the playhead so the cache can keep its lead window ahead of it.
#[tauri::command]
pub fn report_stream_progress(
    state: tauri::State<StreamState>,
    source_id: String,
    path: String,
    position_secs: f64,
    duration_secs: f64,
) -> Result<(), crate::core::error::AppError> {
    state
        .cache
        .report(&source_id, &path, position_secs, duration_secs);
    Ok(())
}

/// Cache preferences shown in settings.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheConfig {
    /// Whether automatic whole-track caching is enabled.
    pub smart_enabled: bool,
    /// Smart cache budget in mebibytes.
    pub smart_limit_mb: u64,
    /// Whether the read-ahead stream buffer is enabled.
    pub stream_enabled: bool,
    /// Read-ahead pre-buffer window in seconds.
    pub prefetch_seconds: u64,
}

/// Read the cache toggles, the smart-cache budget, and the pre-buffer window.
#[tauri::command]
pub fn get_cache_config(app: AppHandle) -> Result<CacheConfig, crate::core::error::AppError> {
    Ok(CacheConfig {
        smart_enabled: read_smart_enabled(&app),
        smart_limit_mb: read_smart_limit_mb(&app),
        stream_enabled: read_stream_enabled(&app),
        prefetch_seconds: read_lead_seconds(&app),
    })
}

/// Persist a new smart-cache budget (MiB) and evict down to it.
#[tauri::command]
pub fn set_smart_cache_limit(
    state: tauri::State<StreamState>,
    app: AppHandle,
    limit_mb: u64,
) -> Result<u64, crate::core::error::AppError> {
    write_smart_limit_mb(&app, limit_mb).map_err(crate::core::error::AppError::Store)?;
    state
        .smart
        .set_limit_bytes(limit_mb.saturating_mul(1024 * 1024));
    Ok(limit_mb)
}

/// Enable or disable automatic whole-track caching. Manual pins are unaffected.
#[tauri::command]
pub fn set_smart_cache_enabled(
    state: tauri::State<StreamState>,
    app: AppHandle,
    enabled: bool,
) -> Result<bool, crate::core::error::AppError> {
    write_smart_enabled(&app, enabled).map_err(crate::core::error::AppError::Store)?;
    state.smart.set_enabled(enabled);
    Ok(enabled)
}

/// Enable or disable the read-ahead stream buffer.
#[tauri::command]
pub fn set_stream_cache_enabled(
    state: tauri::State<StreamState>,
    app: AppHandle,
    enabled: bool,
) -> Result<bool, crate::core::error::AppError> {
    write_stream_enabled(&app, enabled).map_err(crate::core::error::AppError::Store)?;
    state.cache.set_enabled(enabled);
    Ok(enabled)
}

/// Persist the read-ahead pre-buffer window (seconds) and apply it live. The
/// value is clamped to its bounds by the store helper.
#[tauri::command]
pub fn set_prefetch_seconds(
    state: tauri::State<StreamState>,
    app: AppHandle,
    seconds: u64,
) -> Result<u64, crate::core::error::AppError> {
    let seconds = write_lead_seconds(&app, seconds).map_err(crate::core::error::AppError::Store)?;
    state.cache.set_lead_seconds(seconds);
    Ok(seconds)
}

/// Drop the automatic caches (read-ahead buffer and non-pinned whole tracks);
/// manually pinned tracks are kept.
#[tauri::command]
pub fn clear_cache(state: tauri::State<StreamState>) -> Result<(), crate::core::error::AppError> {
    state.cache.clear();
    state.smart.clear();
    Ok(())
}
