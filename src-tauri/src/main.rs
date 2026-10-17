// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
#[tauri_runtime_cef::cef_entry_point]
fn main() {
    welkinplayer_lib::run()
}

#[cfg(not(target_os = "linux"))]
fn main() {
    welkinplayer_lib::run()
}
