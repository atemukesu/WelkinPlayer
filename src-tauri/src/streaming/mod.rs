// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Audio streaming and cache pipeline: the loopback streaming proxy, the
//! transient read-ahead cache and the persistent smart cache.

pub mod proxy;
pub mod smart_cache;
pub mod stream_cache;
