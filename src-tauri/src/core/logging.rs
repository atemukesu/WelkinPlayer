// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Minimal process-wide logger.
//!
//! Output format:
//! `[YYYY-MM-DD HH:MM:SS] LEVEL target: message`
//!
//! Writes to stderr on both debug and release builds, so command errors and
//! lifecycle events are visible while running under `tauri dev` or a packaged
//! binary. Kept dependency-free; swap for `tauri-plugin-log` later if file
//! rotation or per-module levels become necessary.

use std::time::{SystemTime, UNIX_EPOCH};

use log::{Level, LevelFilter, Metadata, Record};

struct StderrLogger;

impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Info
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        eprintln!(
            "[{}] {:<5} {}: {}",
            timestamp(),
            record.level(),
            record.target(),
            record.args(),
        );
    }

    fn flush(&self) {}
}

static LOGGER: StderrLogger = StderrLogger;

/// Install the logger and cap the level at `Info`. Safe to call repeatedly.
pub fn init() {
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(LevelFilter::Info);
    }
}

/// Current UTC time as `YYYY-MM-DD HH:MM:SS`.
fn timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default();

    let days = seconds.div_euclid(86_400);
    let clock = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);

    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        clock / 3_600,
        (clock % 3_600) / 60,
        clock % 60,
    )
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 -> (year, month, day).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;

    (if month <= 2 { year + 1 } else { year }, month, day)
}
