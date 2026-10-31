// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Tauri command surface for the playback context (queue / shuffle / repeat).
//!
//! These replace the webview's per-call queue logic. Each command forwards to
//! the managed [`crate::playback::context::PlaybackContext`], which mutates the
//! queue and drives the engine, then emits `playback:changed` for the UI.

use tauri::State;

use crate::core::error::AppError;
use crate::playback::context::{PlaybackContext, KEY_SEPARATOR};

/// Replace the play queue and start at `start_key` (a `sourceId::path` key).
///
/// `keys` is the full list of track keys in playback order. When `shuffle` is
/// on and a `start_key` is given, the start track is pinned first and the rest
/// are shuffled.
#[tauri::command]
pub fn playback_set_queue(
    context: State<PlaybackContext>,
    keys: Vec<String>,
    start_key: Option<String>,
) -> Result<(), AppError> {
    context.set_queue(keys, start_key.as_deref());
    Ok(())
}

/// Select a track by its index in the current (possibly shuffled) queue.
#[tauri::command]
pub fn playback_select(context: State<PlaybackContext>, index: i64) -> Result<(), AppError> {
    context.select_index(index);
    Ok(())
}

/// Toggle play/pause.
#[tauri::command]
pub fn playback_toggle(context: State<PlaybackContext>) -> Result<(), AppError> {
    context.toggle();
    Ok(())
}

/// Resume playback.
#[tauri::command]
pub fn playback_play(context: State<PlaybackContext>) -> Result<(), AppError> {
    context.play();
    Ok(())
}

/// Pause playback.
#[tauri::command]
pub fn playback_pause(context: State<PlaybackContext>) -> Result<(), AppError> {
    context.pause();
    Ok(())
}

/// Advance to the next track.
#[tauri::command]
pub fn playback_next(context: State<PlaybackContext>) -> Result<(), AppError> {
    context.next();
    Ok(())
}

/// Go to the previous track.
#[tauri::command]
pub fn playback_previous(context: State<PlaybackContext>) -> Result<(), AppError> {
    context.previous();
    Ok(())
}

/// Toggle shuffle.
#[tauri::command]
pub fn playback_toggle_shuffle(context: State<PlaybackContext>) -> Result<(), AppError> {
    context.toggle_shuffle();
    Ok(())
}

/// Set shuffle explicitly (used when restoring persisted state).
#[tauri::command]
pub fn playback_set_shuffle(context: State<PlaybackContext>, on: bool) -> Result<(), AppError> {
    context.set_shuffle(on);
    Ok(())
}

/// Cycle repeat mode (off -> all -> one -> off).
#[tauri::command]
pub fn playback_cycle_repeat(context: State<PlaybackContext>) -> Result<(), AppError> {
    context.cycle_repeat();
    Ok(())
}

/// Restore a persisted queue + position. Mirrors the old `restoreQueue`.
#[tauri::command]
pub fn playback_restore(
    context: State<PlaybackContext>,
    keys: Vec<String>,
    index: i64,
    current_key: Option<String>,
) -> Result<bool, AppError> {
    Ok(context.restore(&keys, index, current_key.as_deref()))
}

/// Current context snapshot for the UI's initial sync.
#[tauri::command]
pub fn playback_state(
    context: State<PlaybackContext>,
) -> Result<crate::playback::context::ContextSnapshot, AppError> {
    Ok(context.snapshot())
}

// Re-export the separator so the frontend can build keys consistently.
pub const SEPARATOR: &str = KEY_SEPARATOR;
