// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Linux-only platform integration.
//!
//! The CEF runtime needs two things the other platforms get from GTK/WebKit for
//! free: an off-screen renderer for the transparent desktop-lyric layer (see
//! [`desktop_lyric`]), and the process-wide opt-in that makes windowless
//! browsers possible at all.

pub mod desktop_lyric;
