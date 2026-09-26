//! System font enumeration for the lyrics font picker.
//!
//! The frontend renders lyrics with whatever fonts the user picks, so it needs
//! the list of families actually installed on this machine. [`fontdb`] handles
//! the platform-specific discovery (Windows Fonts dirs, macOS font dirs,
//! fontconfig on Linux) and parses each font's `name` table, which is exactly
//! what "read the system directly" means without hand-rolling per-OS code.

use std::sync::OnceLock;

use crate::error::AppError;

/// Sorted, de-duplicated family names. The scan parses every installed font
/// file, so the result is cached for the lifetime of the process.
fn installed_families() -> &'static Vec<String> {
    static FAMILIES: OnceLock<Vec<String>> = OnceLock::new();
    FAMILIES.get_or_init(|| {
        let started = std::time::Instant::now();
        let mut db = fontdb::Database::new();
        db.load_system_fonts();

        let mut names: Vec<String> = db
            .faces()
            .flat_map(|face| face.families.iter().map(|(name, _)| name.clone()))
            .filter(|name| !name.trim().is_empty())
            .collect();

        // Case-insensitive ordering keeps localized duplicates (e.g. "微软雅黑"
        // and "Microsoft YaHei") next to each other so `dedup_by` removes them.
        names.sort_unstable_by_key(|name| name.to_lowercase());
        names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));

        log::debug!(
            "loaded {} system font families in {:?}",
            names.len(),
            started.elapsed()
        );
        names
    })
}

/// Return every installed font family name, newest scan cached in-process.
#[tauri::command]
pub fn list_system_fonts() -> Result<Vec<String>, AppError> {
    Ok(installed_families().clone())
}
