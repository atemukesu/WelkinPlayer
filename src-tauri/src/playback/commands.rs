// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Tauri transport commands for the desktop playback engine.
//!
//! The frontend is a thin mirror: it loads a track, skips/pauses/seeks, and
//! receives state on the `audio:state` / `audio:ended` / `audio:error` events.

use tauri::State;

use crate::core::error::AppError;
use crate::playback::engine::{AudioState, Command, Engine};

/// Load a track (starts paused; follow with `audio_play`). Building the byte
/// source happens on the worker, so this returns as soon as the command is
/// queued.
#[tauri::command]
pub fn audio_load(engine: State<Engine>, source_id: String, path: String) -> Result<(), AppError> {
    engine.send(Command::Load { source_id, path })
}

/// Resume playback.
#[tauri::command]
pub fn audio_play(engine: State<Engine>) -> Result<(), AppError> {
    engine.send(Command::Play)
}

/// Pause playback.
#[tauri::command]
pub fn audio_pause(engine: State<Engine>) -> Result<(), AppError> {
    engine.send(Command::Pause)
}

/// Seek to an absolute position (seconds).
#[tauri::command]
pub fn audio_seek(engine: State<Engine>, position_secs: f64) -> Result<(), AppError> {
    engine.send(Command::Seek { position_secs })
}

/// Set the output volume from a percentage (0-100).
#[tauri::command]
pub fn audio_set_volume(engine: State<Engine>, volume: f64) {
    engine.shared().set_volume_percent(volume);
}

/// Mute or unmute the output without losing the volume level.
#[tauri::command]
pub fn audio_set_mute(engine: State<Engine>, muted: bool) {
    engine.shared().set_muted(muted);
}

/// Stop playback and clear the loaded track.
#[tauri::command]
pub fn audio_stop(engine: State<Engine>) -> Result<(), AppError> {
    engine.send(Command::Stop)
}

/// Current playback snapshot, for the frontend's initial sync.
#[tauri::command]
pub fn audio_state(engine: State<Engine>) -> AudioState {
    engine.shared().snapshot()
}

/// Turn the spectrum analyser on/off and size it for the visualiser.
///
/// Analysis only runs while a visualiser is on screen, so this is what keeps a
/// closed full-screen player free of the extra per-sample work.
#[tauri::command]
pub fn audio_spectrum_config(
    engine: State<Engine>,
    enabled: bool,
    bands: Option<usize>,
    fps: Option<u32>,
) {
    let shared = engine.shared();
    let spectrum = &shared.spectrum;
    spectrum.set_enabled(enabled);
    if let Some(bands) = bands {
        spectrum.set_bands(bands);
    }
    if let Some(fps) = fps {
        spectrum.set_fps(fps);
    }
}
