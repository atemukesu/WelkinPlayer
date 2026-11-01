// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Persisted romanized sort keys for track titles.
//!
//! Reading a title (pinyin for hanzi, romaji for kana, see the frontend's
//! `lib/reading.ts`) is deterministic but not free, and a library can hold
//! thousands of titles that would be re-read on every sort. The frontend
//! therefore computes each title's key once and hands it here to persist under
//! the configured cache directory; later launches load the map back and never
//! recompute a known title.

use std::collections::BTreeMap;
use std::path::PathBuf;

use tauri::AppHandle;

use crate::commands::media::resolve_cache_dir;
use crate::core::error::AppError;

/// File (relative to the cache directory) holding the `title -> key` map.
const READINGS_FILE: &str = "readings.json";

fn readings_file(app: &AppHandle) -> PathBuf {
    resolve_cache_dir(app).join(READINGS_FILE)
}

/// Read the whole `title -> romanized key` cache. A missing or unreadable file
/// simply means nothing has been computed yet.
#[tauri::command]
pub fn load_sort_readings(app: AppHandle) -> Result<BTreeMap<String, String>, AppError> {
    let file = readings_file(&app);
    match std::fs::read(&file) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes).unwrap_or_default()),
        Err(_) => Ok(BTreeMap::new()),
    }
}

/// Merge freshly computed keys into the on-disk cache. Existing entries for
/// other titles are preserved so a partial flush never drops earlier work.
#[tauri::command]
pub fn save_sort_readings(
    app: AppHandle,
    readings: BTreeMap<String, String>,
) -> Result<(), AppError> {
    if readings.is_empty() {
        return Ok(());
    }
    let file = readings_file(&app);
    let mut existing: BTreeMap<String, String> = std::fs::read(&file)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    existing.extend(readings);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(file, serde_json::to_vec(&existing)?)?;
    Ok(())
}
