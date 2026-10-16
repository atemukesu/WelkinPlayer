// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Desktop playback engine.
//!
//! Audio is decoded in Rust (Symphonia, through `rodio`) and rendered straight
//! to the OS audio device (`cpal`: WASAPI / CoreAudio / ALSA on desktop, Oboe
//! on Android). The webview is no longer involved in playback: it only issues
//! transport commands and mirrors the state events.
//!
//! Decoding happens on a dedicated worker thread that fills a small ring buffer;
//! `rodio`'s mixer pulls that buffer from the realtime output callback. Network
//! reads therefore never run on the audio thread — a slow or stalled download
//! yields a silent underrun instead of blocking the device. The mixer's
//! `UniformSourceIterator` resamples and remaps channels to the device config,
//! so the worker only has to decode at the file's native rate.

pub mod commands;
pub mod engine;
pub mod source;
pub mod spectrum;
