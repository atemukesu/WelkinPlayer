// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! OS integration: Android native bridges, the desktop tray icon and system
//! media controls.

#[cfg(target_os = "android")]
pub mod android;
pub mod media_control;
pub mod tray;
