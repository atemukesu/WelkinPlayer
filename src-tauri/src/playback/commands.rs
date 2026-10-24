// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Tauri transport commands for the desktop playback engine.
//!
//! The frontend is a thin mirror: it loads a track, skips/pauses/seeks, and
//! receives state on the `audio:state` / `audio:ended` / `audio:error` events.

use tauri::State;

use crate::core::error::AppError;
use crate::playback::engine::{device_info, AudioState, Command, DeviceInfo, Engine};

/// Load a track with an explicit initial playback intent. Building the byte
/// source happens on the worker, so this returns as soon as the command is
/// queued.
#[tauri::command]
pub fn audio_load(
    engine: State<Engine>,
    source_id: String,
    path: String,
    autoplay: Option<bool>,
) -> Result<(), AppError> {
    log::info!(
        "playback: audio_load queued source_id={source_id} path={path} autoplay={}",
        autoplay.unwrap_or(false)
    );
    engine.send(Command::Load {
        source_id,
        path,
        autoplay: autoplay.unwrap_or(false),
    })
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
    log::debug!("playback: audio_seek received {position_secs:.3}s");
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

/// Set the playback-rate multiplier (1.0 = unchanged). Pitch is preserved by
/// the engine's time-stretcher.
#[tauri::command]
pub fn audio_set_speed(engine: State<Engine>, rate: f64) -> Result<(), AppError> {
    engine.send(Command::SetTempo { rate: rate as f32 })
}

/// Probe the output device for the sample rates it supports.
#[tauri::command]
pub fn audio_device_info(engine: State<Engine>) -> DeviceInfo {
    let shared = engine.shared();
    let mut info = device_info();
    info.current_rate = shared.device_rate();
    info.preferred_rate = shared.preferred_rate();
    info
}

/// Choose the output quality. `rate == 0` follows the device default;
/// otherwise the stream is reopened at that exact rate.
#[tauri::command]
pub fn audio_set_quality(engine: State<Engine>, rate: u32) -> Result<(), AppError> {
    engine.send(Command::SetQuality { rate })
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
