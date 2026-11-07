// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Active-network classification.
//!
//! Speculative prefetch must not spend a metered (cellular) connection.
//! Desktop platforms are treated as unmetered; Android asks
//! `ConnectivityManager`. A failed query errs on the side of "unmetered" so a
//! missing permission never silently disables streaming.

use serde::Serialize;

/// Whether the active connection is metered, Wi-Fi, or present at all.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkStatus {
    pub metered: bool,
    pub wifi: bool,
    pub available: bool,
}

impl NetworkStatus {
    /// The permissive default used on desktop and when detection fails.
    pub const UNMETERED: Self = Self {
        metered: false,
        wifi: true,
        available: true,
    };
}

/// Classify the active network.
pub fn status() -> NetworkStatus {
    #[cfg(target_os = "android")]
    {
        crate::platform::android::network_status()
    }
    #[cfg(not(target_os = "android"))]
    {
        NetworkStatus::UNMETERED
    }
}
