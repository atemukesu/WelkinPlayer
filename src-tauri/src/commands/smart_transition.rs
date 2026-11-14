// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Per-playlist smart-transition documents.
//!
//! Each analysed playlist owns exactly one small JSON file:
//!
//! ```text
//! welkin-transition-<sha256(playlistId)[..16]>.json
//! ```
//!
//! It holds the cached per-track features, the computed best order and the
//! transition plan for each adjacent pair. The file is cached locally (so the
//! UI works offline and instantly) and uploaded to the sync source's WebDAV root
//! when one is configured, mirroring how `welkin-playback.json` and playlist
//! covers are stored.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::analysis::{
    analyze_playlist, file_for_playlist, AnalysisDocument, AnalysisSummary, TrackInput,
    TransitionPlan, ALGORITHM_VERSION, SCHEMA_VERSION,
};
use crate::commands::media::resolve_cache_dir;
use crate::core::error::AppError;
use crate::data::backend::sync_backend;
use crate::streaming::state::StreamState;

/// Cancellation flags for in-flight analyses, keyed by playlist id.
#[derive(Default)]
pub struct TransitionJobs {
    cancel: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl TransitionJobs {
    fn begin(&self, playlist_id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.cancel
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(playlist_id.to_string(), flag.clone());
        flag
    }

    fn finish(&self, playlist_id: &str) {
        self.cancel
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(playlist_id);
    }

    fn cancel(&self, playlist_id: &str) {
        if let Some(flag) = self
            .cancel
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(playlist_id)
        {
            flag.store(true, Ordering::Relaxed);
        }
    }
}

/// Progress pushed to the webview while an analysis runs.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AnalysisProgress {
    playlist_id: String,
    done: usize,
    total: usize,
    phase: &'static str,
}

fn transitions_dir(app: &AppHandle) -> PathBuf {
    resolve_cache_dir(app).join("transitions")
}

fn local_path(app: &AppHandle, file: &str) -> PathBuf {
    transitions_dir(app).join(file)
}

/// In-progress snapshots live next to the final document but with a
/// `.partial` suffix, so a cancelled re-analysis never clobbers a finished
/// analysis and can still be resumed.
fn partial_path(app: &AppHandle, playlist_id: &str) -> PathBuf {
    transitions_dir(app).join(format!("{}.partial", file_for_playlist(playlist_id)))
}

fn read_local(app: &AppHandle, file: &str) -> Option<String> {
    std::fs::read_to_string(local_path(app, file)).ok()
}

fn read_partial(app: &AppHandle, playlist_id: &str) -> Option<String> {
    std::fs::read_to_string(partial_path(app, playlist_id)).ok()
}

fn write_local(app: &AppHandle, file: &str, text: &str) -> Result<(), AppError> {
    let dir = transitions_dir(app);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(file), text)?;
    Ok(())
}

fn write_partial(app: &AppHandle, playlist_id: &str, text: &str) -> Result<(), AppError> {
    let dir = transitions_dir(app);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(partial_path(app, playlist_id), text)?;
    Ok(())
}

fn remove_partial(app: &AppHandle, playlist_id: &str) {
    let path = partial_path(app, playlist_id);
    if path.exists() {
        if let Err(error) = std::fs::remove_file(&path) {
            log::warn!("smart-transition: failed to remove partial snapshot: {error}");
        }
    }
}

fn parse_document(text: &str) -> Option<AnalysisDocument> {
    let document: AnalysisDocument = serde_json::from_str(text).ok()?;
    if document.schema != SCHEMA_VERSION || document.algorithm_version != ALGORITHM_VERSION {
        return None;
    }
    Some(document)
}

/// Merge a finished document with a partial snapshot, keeping the newest
/// entry for each track so a resumed run reuses every feature it already has.
fn merge_cached(
    full: Option<AnalysisDocument>,
    partial: Option<AnalysisDocument>,
) -> Option<AnalysisDocument> {
    match (full, partial) {
        (Some(mut full), Some(partial)) => {
            let mut positions: HashMap<String, usize> = full
                .tracks
                .iter()
                .enumerate()
                .map(|(index, track)| (track.key.clone(), index))
                .collect();
            for track in partial.tracks {
                match positions.get(&track.key) {
                    Some(index) => full.tracks[*index] = track,
                    None => {
                        positions.insert(track.key.clone(), full.tracks.len());
                        full.tracks.push(track);
                    }
                }
            }
            full.revision = full.revision.max(partial.revision);
            Some(full)
        }
        (None, partial) => partial,
        (full, None) => full,
    }
}

/// Read a playlist's document, local cache first. The remote copy is preferred
/// when its revision/updated-at is newer, then written back to the cache.
async fn load_document(
    app: &AppHandle,
    playlist_id: &str,
) -> Result<Option<(AnalysisDocument, String)>, AppError> {
    let file = file_for_playlist(playlist_id);
    let full = read_local(app, &file).and_then(|text| parse_document(&text));
    let partial = read_partial(app, playlist_id).and_then(|text| parse_document(&text));
    // A partial snapshot with a newer revision means the last run was
    // cancelled or is still running; prefer it so the UI reflects that.
    let local = match (full, partial) {
        (Some(full), Some(partial)) => {
            if partial.revision >= full.revision {
                Some(partial)
            } else {
                Some(full)
            }
        }
        (full, partial) => full.or(partial),
    };
    let client = match sync_backend(app) {
        Ok(client) => client,
        Err(error) => {
            log::warn!("smart-transition: sync backend unavailable: {error}");
            None
        }
    };
    let Some(client) = client else {
        return Ok(local.map(|document| (document, "local".to_string())));
    };
    match client.read_text(&file).await {
        Ok(text) => {
            let remote = parse_document(&text);
            match remote {
                Some(remote) => {
                    let use_remote = local
                        .as_ref()
                        .map(|current| {
                            remote.revision > current.revision
                                || (remote.revision == current.revision
                                    && remote.updated_at > current.updated_at)
                        })
                        .unwrap_or(true);
                    if use_remote {
                        if let Err(error) = write_local(app, &file, &text) {
                            log::warn!(
                                "smart-transition: failed to cache remote document: {error}"
                            );
                        }
                        return Ok(Some((remote, "remote".to_string())));
                    }
                    Ok(local.map(|document| (document, "local".to_string())))
                }
                None => {
                    log::warn!("smart-transition: ignoring incompatible remote document {file}");
                    Ok(local.map(|document| (document, "local".to_string())))
                }
            }
        }
        Err(AppError::WebdavNotFound(_)) => {
            Ok(local.map(|document| (document, "local".to_string())))
        }
        Err(error) => {
            log::warn!("smart-transition: remote read failed ({error}); using local cache");
            Ok(local.map(|document| (document, "local".to_string())))
        }
    }
}

async fn store_document(
    app: &AppHandle,
    document: &AnalysisDocument,
) -> Result<(String, String), AppError> {
    let file = file_for_playlist(&document.playlist_id);
    let text = serde_json::to_string(document)?;
    write_local(app, &file, &text)?;
    match sync_backend(app) {
        Ok(Some(client)) => match client
            .put(&file, "application/json", text.into_bytes())
            .await
        {
            Ok(()) => Ok((file, "remote".to_string())),
            Err(error) => {
                log::warn!("smart-transition: upload failed ({error}); kept local copy");
                Ok((file, "local".to_string()))
            }
        },
        Ok(None) => Ok((file, "local".to_string())),
        Err(error) => {
            log::warn!("smart-transition: no sync backend ({error}); kept local copy");
            Ok((file, "local".to_string()))
        }
    }
}

/// Analyse a playlist and store the result (local cache + WebDAV when present).
///
/// Tracks are decoded only for their first and last minute. Tracks already
/// present in a previous document are reused, so re-running after an order-only
/// change performs no decoding at all.
#[tauri::command]
pub async fn smart_transition_analyze(
    app: AppHandle,
    jobs: tauri::State<'_, TransitionJobs>,
    playlist_id: String,
    playlist_name: String,
    tracks: Vec<TrackInput>,
    previous_order: Option<Vec<String>>,
    fixed_first: Option<String>,
) -> Result<AnalysisSummary, AppError> {
    if tracks.is_empty() {
        return Err(AppError::invalid_argument("tracks", "歌单没有可分析的曲目"));
    }
    let stream = app.state::<StreamState>();
    let cache = stream.cache.clone();
    let manual = stream.manual.clone();
    drop(stream);

    let flag = jobs.begin(&playlist_id);
    let cancel = flag.clone();
    let file = file_for_playlist(&playlist_id);
    let full = read_local(&app, &file).and_then(|text| parse_document(&text));
    let partial = read_partial(&app, &playlist_id).and_then(|text| parse_document(&text));
    let cached = merge_cached(full, partial);
    let revision = cached
        .as_ref()
        .map(|document| document.revision + 1)
        .unwrap_or(1);
    let previous_order =
        previous_order.unwrap_or_else(|| tracks.iter().map(|track| track.key.clone()).collect());
    if let Some(cached) = &cached {
        if cached.playlist_id != playlist_id {
            return Err(AppError::Other("分析文件与歌单不匹配".to_string()));
        }
    }

    let emit_app = app.clone();
    let emit_id = playlist_id.clone();
    let task = {
        let app = app.clone();
        let cached = cached.clone();
        let tracks = tracks.clone();
        let playlist_id = playlist_id.clone();
        let playlist_name = playlist_name.clone();
        let fixed_first = fixed_first.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let progress_app = emit_app.clone();
            let progress_id = emit_id.clone();
            let partial_app = emit_app;
            analyze_playlist(
                &app,
                &cache,
                &manual,
                &playlist_id,
                &playlist_name,
                &tracks,
                previous_order,
                cached.as_ref(),
                fixed_first.as_deref(),
                revision,
                &cancel,
                &move |done, total| {
                    let _ = progress_app.emit(
                        "smart-transition:progress",
                        AnalysisProgress {
                            playlist_id: progress_id.clone(),
                            done,
                            total,
                            phase: if done < total {
                                "analyzing"
                            } else {
                                "planning"
                            },
                        },
                    );
                },
                &move |document| {
                    // Persist the in-progress snapshot locally so a cancel or a
                    // closed dialog can resume without re-analysing. It is not
                    // uploaded and is marked `incomplete`, so it is never
                    // applied or used for playback.
                    let file = file_for_playlist(&document.playlist_id);
                    match serde_json::to_string(document) {
                        Ok(text) => {
                            if let Err(error) =
                                write_partial(&partial_app, &document.playlist_id, &text)
                            {
                                log::warn!(
                                    "smart-transition: failed to write partial snapshot: {error}"
                                );
                            }
                        }
                        Err(error) => {
                            log::warn!("smart-transition: partial snapshot encode failed: {error}");
                        }
                    }
                    let _ = partial_app.emit(
                        "smart-transition:partial",
                        document.summary(file, "local"),
                    );
                },
            )
        })
    };
    let result = task
        .await
        .map_err(|error| AppError::other(format!("分析任务失败：{error}")));
    jobs.finish(&playlist_id);
    let mut document = result??;
    // Keep the revision monotonic even if the task was cancelled midway.
    document.revision = revision;
    let (file, source) = store_document(&app, &document).await?;
    remove_partial(&app, &playlist_id);
    Ok(document.summary(file, &source))
}

/// Load a playlist's stored analysis summary (remote copy when newer).
#[tauri::command]
pub async fn smart_transition_load(
    app: AppHandle,
    playlist_id: String,
) -> Result<Option<AnalysisSummary>, AppError> {
    let Some((document, source)) = load_document(&app, &playlist_id).await? else {
        return Ok(None);
    };
    Ok(Some(
        document.summary(file_for_playlist(&playlist_id), &source),
    ))
}

/// The transition plans needed by the playback engine.
///
/// Reads the local cache only: it runs on every playlist start and must never
/// block on a WebDAV round trip. Analysis always writes the local copy first,
/// and opening the playlist's dialog refreshes it from the remote.
#[tauri::command]
pub async fn smart_transition_plans(
    app: AppHandle,
    playlist_id: String,
) -> Result<Vec<TransitionPlan>, AppError> {
    let file = file_for_playlist(&playlist_id);
    let Some(document) = read_local(&app, &file).and_then(|text| parse_document(&text)) else {
        return Ok(Vec::new());
    };
    if document.incomplete {
        return Ok(Vec::new());
    }
    Ok(document.transitions)
}

/// Delete a playlist's analysis file (local and remote).
#[tauri::command]
pub async fn smart_transition_delete(app: AppHandle, playlist_id: String) -> Result<(), AppError> {
    let file = file_for_playlist(&playlist_id);
    let path = local_path(&app, &file);
    if path.exists() {
        if let Err(error) = std::fs::remove_file(&path) {
            log::warn!("smart-transition: failed to remove local {file}: {error}");
        }
    }
    remove_partial(&app, &playlist_id);
    if let Ok(Some(client)) = sync_backend(&app) {
        if let Err(error) = client.delete(&file).await {
            log::warn!("smart-transition: failed to remove remote {file}: {error}");
        }
    }
    Ok(())
}

/// Request cancellation of an in-flight analysis.
#[tauri::command]
pub fn smart_transition_cancel(
    jobs: tauri::State<'_, TransitionJobs>,
    playlist_id: String,
) -> Result<(), AppError> {
    jobs.cancel(&playlist_id);
    Ok(())
}
